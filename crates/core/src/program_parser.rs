//! Reads Northeastern's CourseLeaf academic catalog: editions, programs, requirements.

use crate::html::{plain_text, table_rows};
use crate::model::*;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

pub fn current_edition(html: &str) -> Option<CatalogYear> {
    let text = crate::html::decode_entities(html);
    for pattern in [
        r"Academic Catalog\s+\d{4}\s*[-–]\s*\d{2,4}",
        r"\d{4}\s*[-–]\s*\d{2,4}\s+Edition",
    ] {
        if let Some(m) = Regex::new(pattern).ok().and_then(|re| re.find(&text)) {
            if let Some(year) = CatalogYear::parse(m.as_str()) {
                return Some(year);
            }
        }
    }
    None
}

pub fn archived_editions(html: &str) -> Vec<CatalogYear> {
    let re = Regex::new(r"archive/(\d{4})-(\d{4})").unwrap();
    let mut years = BTreeSet::new();
    for cap in re.captures_iter(html) {
        let start: i32 = cap[1].parse().unwrap_or(0);
        let end: i32 = cap[2].parse().unwrap_or(0);
        if end == start + 1 {
            years.insert(CatalogYear::new(start));
        }
    }
    years.into_iter().rev().collect()
}

pub fn edition_relative_path(href: &str) -> String {
    let path = if let Some(idx) = href.find("://") {
        href[idx + 3..]
            .find('/')
            .map(|i| &href[idx + 3 + i..])
            .unwrap_or("/")
    } else {
        href
    };
    let path = path.split('#').next().unwrap_or(path);
    let re = Regex::new(r"^/?archive/\d{4}-\d{4}/?").unwrap();
    let mut path = re.replace(path, "").to_string();
    if path.starts_with('/') {
        path.remove(0);
    }
    path
}

pub fn programs_from_az(html: &str) -> Vec<CatalogProgram> {
    let re = Regex::new(r###"(?is)<a\s[^>]*href="([^"#]+)"[^>]*>([\s\S]*?)</a>"###).unwrap();
    let mut by_path = BTreeMap::new();
    for cap in re.captures_iter(html) {
        let path = edition_relative_path(&cap[1]);
        if !(path.starts_with("undergraduate/") || path.starts_with("graduate/"))
            || !path.ends_with('/')
        {
            continue;
        }
        let title = plain_text(&cap[2]);
        let program = CatalogProgram {
            title,
            path: path.clone(),
        };
        let Some(degree) = program.degree() else {
            continue;
        };
        if !degree.chars().next().is_some_and(|c| c.is_uppercase()) {
            continue;
        }
        if degree.chars().filter(|c| c.is_uppercase()).count() < 2 {
            continue;
        }
        by_path.entry(path).or_insert(program);
    }
    let mut programs: Vec<_> = by_path.into_values().collect();
    programs.sort_by(|a, b| crate::model::natural_cmp(&a.title, &b.title));
    programs
}

/// Programs whose degree matches an audit code such as `CS-B-CSCI-BSCS` (the last segment).
/// Campus-less listings come first, then Boston, then anywhere else.
pub fn suggested_programs(code: Option<&str>, programs: &[CatalogProgram]) -> Vec<CatalogProgram> {
    let Some(degree) = code
        .and_then(|text| text.split('-').next_back())
        .filter(|degree| degree.len() >= 2)
    else {
        return vec![];
    };
    let mut matched: Vec<CatalogProgram> = programs
        .iter()
        .filter(|program| {
            program
                .degree()
                .is_some_and(|name| name.eq_ignore_ascii_case(degree))
        })
        .cloned()
        .collect();
    matched.sort_by(|left, right| {
        campus_rank(left)
            .cmp(&campus_rank(right))
            .then_with(|| left.title.cmp(&right.title))
    });
    matched
}

fn campus_rank(program: &CatalogProgram) -> i32 {
    match program.campus().as_deref() {
        None => 0,
        Some("Boston") => 1,
        Some(_) => 2,
    }
}

pub fn universitywide_link(html: &str) -> Option<String> {
    let re = Regex::new(r###"(?is)<a\s[^>]*href="([^"#]+)"[^>]*>"###).unwrap();
    let cue = Regex::new(r"(?i)university-?wide-?requirements").unwrap();
    for cap in re.captures_iter(html) {
        if cue.is_match(&cap[1]) {
            let path = edition_relative_path(cap[1].split('#').next().unwrap_or(&cap[1]));
            if !path.is_empty() {
                return Some(if path.ends_with('/') {
                    path
                } else {
                    format!("{path}/")
                });
            }
        }
    }
    None
}

pub fn universitywide_requirements(html: &str) -> Option<UniversitywideRequirements> {
    let text = plain_text(&requirements_container(html));
    let total = first_number(
        &text,
        r"(?i)minimum\s+(?:of\s+)?(\d+(?:\.\d+)?)\s+total\s+semester\s+hours",
    );
    let gpa = first_number(&text, r"(?i)minimum\s+(?:cumulative\s+)?(\d\.\d+)\s+GPA");
    let residency = first_number(
        &text,
        r"(?i)minimum\s+of\s+(\d+(?:\.\d+)?)\s+Northeastern\s+University\s+semester\s+hours",
    );
    let experiential = Regex::new(r"(?i)experiential\s+learning\s+graduation\s+requirement")
        .unwrap()
        .is_match(&text);
    if total.is_none() && gpa.is_none() && residency.is_none() && !experiential {
        return None;
    }
    Some(UniversitywideRequirements {
        total_hours: total,
        gpa,
        residency_hours: residency,
        requires_experiential_learning: experiential,
        fetched_at: 0.0,
    })
}

fn first_number(text: &str, pattern: &str) -> Option<f64> {
    Regex::new(pattern)
        .ok()?
        .captures(text)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

pub fn requirements_container(html: &str) -> String {
    let start_re = Regex::new(r#"(?i)<div[^>]*id="programrequirementstextcontainer""#).unwrap();
    let alt = Regex::new(r#"(?i)<div[^>]*id="textcontainer""#).unwrap();
    let start = start_re.find(html).or_else(|| alt.find(html));
    let Some(start) = start else {
        return html.to_string();
    };
    let rest = &html[start.end()..];
    let next = Regex::new(r#"(?i)<div[^>]*id="[a-z]+textcontainer""#)
        .unwrap()
        .find(rest);
    let footer = rest.to_ascii_lowercase().find("<footer");
    let end = match (next.map(|m| m.start()), footer) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => rest.len(),
    };
    rest[..end].to_string()
}

pub fn requirements_from_html(
    html: &str,
    year: CatalogYear,
    program: CatalogProgram,
) -> Option<ProgramRequirements> {
    let mut builder = Builder::default();
    for token in tokens(&requirements_container(html)) {
        match token {
            Token::Heading(text) => builder.start_section(text),
            Token::Paragraph(text) => builder.add_paragraph(text),
            Token::Table(table) => builder.add_table(&table),
        }
    }
    let sections = builder.finish();
    if !sections.iter().any(|s| !s.groups.is_empty()) {
        return None;
    }
    Some(ProgramRequirements {
        year,
        program,
        sections,
        total_hours: builder.total_hours,
        fetched_at: 0.0,
        course_titles: builder.course_titles,
        universitywide: None,
    })
}

enum Token {
    Heading(String),
    Paragraph(String),
    Table(String),
}

fn tokens(html: &str) -> Vec<Token> {
    let mut found: Vec<(usize, Token)> = Vec::new();
    for level in 2..=4 {
        let re = Regex::new(&format!(r"(?is)<h{level}[^>]*>([\s\S]*?)</h{level}>")).unwrap();
        for cap in re.captures_iter(html) {
            let whole = cap.get(0).unwrap();
            found.push((whole.start(), Token::Heading(plain_text(&cap[1]))));
        }
    }
    let table =
        Regex::new(r#"(?is)<table[^>]*class="[^"]*sc_courselist[^"]*"[^>]*>([\s\S]*?)</table>"#)
            .unwrap();
    for cap in table.captures_iter(html) {
        found.push((
            cap.get(0).unwrap().start(),
            Token::Table(cap[1].to_string()),
        ));
    }
    let para = Regex::new(r"(?is)<p[^>]*>([\s\S]*?)</p>").unwrap();
    for cap in para.captures_iter(html) {
        let text = plain_text(&cap[1]);
        if !text.is_empty() {
            found.push((cap.get(0).unwrap().start(), Token::Paragraph(text)));
        }
    }
    found.sort_by_key(|item| item.0);
    found.into_iter().map(|(_, token)| token).collect()
}

#[derive(Clone, Debug)]
enum RowKind {
    Skip,
    AreaHeader(String),
    Subheader(String),
    Comment(String),
    Courses {
        sets: Vec<Vec<CourseKey>>,
        alternative: bool,
        titles: Vec<String>,
    },
}

struct Row {
    kind: RowKind,
    hours: Option<f64>,
}

pub fn course_keys(text: &str) -> Vec<CourseKey> {
    Regex::new(r"\b([A-Z]{2,5})\s+(\d{4}[A-Z]?)\b")
        .unwrap()
        .captures_iter(text)
        .map(|c| CourseKey {
            subject: c[1].to_string(),
            number: c[2].to_string(),
        })
        .collect()
}

pub fn course_sets(text: &str) -> Vec<Vec<CourseKey>> {
    let mut cleaned = text.replace('\u{00A0}', " ");
    if cleaned.to_ascii_lowercase().starts_with("or ") {
        cleaned = cleaned[3..].to_string();
    }
    cleaned
        .split(" or ")
        .filter_map(|part| {
            let keys = course_keys(part);
            if keys.is_empty() {
                None
            } else {
                Some(keys)
            }
        })
        .collect()
}

pub fn hours_value(text: &str) -> Option<f64> {
    Regex::new(r"\d+(\.\d+)?")
        .ok()?
        .find(text)?
        .as_str()
        .parse()
        .ok()
}

pub fn course_range(text: &str) -> Option<CourseRange> {
    let cleaned = text.replace('\u{00A0}', " ").trim().to_string();
    let higher = Regex::new(
        r"^([A-Z]{2,5}) (\d{4}) or (?:higher|above)(?:,? (?:except|excluding|but not) (.+?))?\.?$",
    )
    .unwrap();
    if let Some(m) = higher.captures(&cleaned) {
        return Some(CourseRange {
            subject: m[1].to_string(),
            lowest: m[2].parse().ok()?,
            highest: None,
            except: m
                .get(3)
                .map(|g| course_keys(g.as_str()))
                .unwrap_or_default(),
            label: cleaned,
        });
    }
    let span = Regex::new(r"^([A-Z]{2,5}) (\d{4}) (?:to|through|-|–) (?:([A-Z]{2,5}) )?(\d{4})(?:,? (?:except|excluding|but not) (.+?))?\.?$").unwrap();
    if let Some(m) = span.captures(&cleaned) {
        if m.get(3).is_some_and(|g| g.as_str() != &m[1]) {
            return None;
        }
        return Some(CourseRange {
            subject: m[1].to_string(),
            lowest: m[2].parse().ok()?,
            highest: m[4].parse().ok(),
            except: m
                .get(5)
                .map(|g| course_keys(g.as_str()))
                .unwrap_or_default(),
            label: cleaned,
        });
    }
    None
}

pub fn choose_amount(text: &str) -> Option<(Option<i32>, Option<f64>)> {
    let lower = text.to_ascii_lowercase();
    if !(lower.starts_with("complete")
        || lower.starts_with("choose")
        || lower.starts_with("select")
        || lower.starts_with("take"))
    {
        return None;
    }
    if let Some(m) = Regex::new(
        r"(\d+(?:\.\d+)?)(?:\s*[-–]\s*\d+)?\s+(?:semester hours?|credits?|credit hours?|sh)\b",
    )
    .unwrap()
    .captures(&lower)
    {
        return Some((None, m[1].parse().ok()));
    }
    let words = [
        ("one", 1),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
        ("seven", 7),
        ("eight", 8),
        ("nine", 9),
        ("ten", 10),
        ("eleven", 11),
        ("twelve", 12),
    ];
    if let Some(m) = Regex::new(r"^(?:complete|choose|select|take)\s+(?:any\s+)?(\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)\b").unwrap().captures(&lower) {
        let token = &m[1];
        let count = token.parse().ok().or_else(|| words.iter().find(|(w, _)| *w == token).map(|(_, n)| *n));
        if let Some(count) = count {
            return Some((Some(count), None));
        }
    }
    None
}

fn spaced_text(html: &str) -> String {
    let spaced = html.replace('<', " <");
    let text = plain_text(&spaced);
    let text = Regex::new(r"\s+([,.:;])").unwrap().replace_all(&text, "$1");
    Regex::new(r"\s+")
        .unwrap()
        .replace_all(&text, " ")
        .trim()
        .to_string()
}

fn rows_in_table(html: &str) -> Vec<Row> {
    let re = Regex::new(r"(?is)<tr([^>]*)>([\s\S]*?)</tr>").unwrap();
    re.captures_iter(html)
        .map(|c| parse_row(&c[1], &c[2]))
        .collect()
}

fn parse_row(attributes: &str, body: &str) -> Row {
    let row_class = Regex::new(r#"class="([^"]*)""#)
        .unwrap()
        .captures(attributes)
        .map(|c| c[1].to_string())
        .unwrap_or_default();
    let cell_re = Regex::new(r"(?is)<t([dh])([^>]*)>([\s\S]*?)</t[dh]>").unwrap();
    let class_re = Regex::new(r#"class="([^"]*)""#).unwrap();
    let cells: Vec<(bool, String, String)> = cell_re
        .captures_iter(body)
        .map(|c| {
            let cls = class_re
                .captures(&c[2])
                .map(|m| m[1].to_string())
                .unwrap_or_default();
            (c[1].eq_ignore_ascii_case("h"), cls, c[3].to_string())
        })
        .collect();
    let hours = cells
        .iter()
        .find(|c| c.1.contains("hourscol"))
        .and_then(|c| hours_value(&plain_text(&c.2)));
    if row_class.contains("hidden") || cells.iter().any(|c| c.0) {
        return Row {
            kind: RowKind::Skip,
            hours,
        };
    }
    if row_class.contains("areasubheader") || body.contains("areasubheader") {
        return Row {
            kind: RowKind::Subheader(plain_text(body)),
            hours,
        };
    }
    if row_class.contains("areaheader") || body.contains("courselistcomment areaheader") {
        return Row {
            kind: RowKind::AreaHeader(plain_text(body)),
            hours,
        };
    }
    if let Some(code) = cells.iter().find(|c| c.1.contains("codecol")) {
        let text = spaced_text(&code.2);
        let alternative = row_class.contains("orclass")
            || code.1.contains("orclass")
            || text.to_ascii_lowercase().starts_with("or ");
        let sets = course_sets(&text);
        if !sets.is_empty() {
            let titles = cells
                .iter()
                .find(|c| !c.1.contains("codecol") && !c.1.contains("hourscol") && !c.0)
                .map(|c| {
                    plain_text(&c.2)
                        .lines()
                        .map(|l| l.trim())
                        .map(|l| {
                            if l.to_ascii_lowercase().starts_with("and ") {
                                &l[4..]
                            } else {
                                l
                            }
                        })
                        .filter(|l| !l.is_empty())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            return Row {
                kind: RowKind::Courses {
                    sets,
                    alternative,
                    titles,
                },
                hours,
            };
        }
    }
    if body.contains("courselistcomment") {
        let text = spaced_text(cells.first().map(|c| c.2.as_str()).unwrap_or(body));
        if text.is_empty() {
            return Row {
                kind: RowKind::Skip,
                hours,
            };
        }
        return Row {
            kind: RowKind::Comment(text),
            hours,
        };
    }
    Row {
        kind: RowKind::Skip,
        hours,
    }
}

#[derive(Clone)]
enum DraftKind {
    Required(Vec<Vec<CourseKey>>),
    Choose {
        count: Option<i32>,
        hours: Option<f64>,
        options: Vec<Vec<Vec<CourseKey>>>,
        ranges: Vec<CourseRange>,
    },
    General(f64),
}

struct RuleDraft {
    kind: DraftKind,
    label: String,
    hours: Option<f64>,
    notes: Vec<String>,
    option_categories: Vec<Option<String>>,
    current_category: Option<String>,
}

struct GroupDraft {
    title: Option<String>,
    rules: Vec<RuleDraft>,
    notes: Vec<String>,
}

struct SectionDraft {
    title: String,
    groups: Vec<GroupDraft>,
    notes: Vec<String>,
}

#[derive(Default)]
struct Builder {
    sections: Vec<SectionDraft>,
    total_hours: Option<f64>,
    course_titles: BTreeMap<String, String>,
    choose_index: Option<usize>,
}

impl Builder {
    fn start_section(&mut self, title: String) {
        self.sections.push(SectionDraft {
            title: title.trim().to_string(),
            groups: vec![],
            notes: vec![],
        });
        self.choose_index = None;
    }
    fn add_paragraph(&mut self, text: String) {
        if let Some(m) = Regex::new(r"(\d+(?:\.\d+)?)\s+total semester hours")
            .unwrap()
            .captures(&text)
        {
            if let Ok(hours) = m[1].parse::<f64>() {
                self.total_hours = Some(self.total_hours.unwrap_or(0.0).max(hours));
            }
        }
        if let Some(section) = self.sections.last_mut() {
            section.notes.push(text);
        }
    }
    fn add_table(&mut self, html: &str) {
        if self.sections.is_empty() {
            self.start_section("Program Requirements".into());
        }
        self.choose_index = None;
        let mut started = false;
        for row in rows_in_table(html) {
            match row.kind {
                RowKind::Skip => {}
                RowKind::AreaHeader(title) => {
                    self.append_group(Some(title));
                    started = true;
                }
                RowKind::Subheader(title) => {
                    if let Some(idx) = self.choose_index {
                        let rule = &mut self.current_group().rules[idx];
                        rule.notes.push(title.clone());
                        rule.current_category = Some(title);
                    } else {
                        self.current_group().notes.push(title);
                    }
                }
                RowKind::Comment(text) => {
                    if !started {
                        self.append_group(None);
                        started = true;
                    }
                    self.add_comment(text, row.hours);
                }
                RowKind::Courses {
                    sets,
                    alternative,
                    titles,
                } => {
                    if !started {
                        self.append_group(None);
                        started = true;
                    }
                    self.add_courses(sets.clone(), alternative, row.hours);
                    for (key, title) in sets.into_iter().flatten().zip(titles) {
                        self.course_titles.entry(key.to_string()).or_insert(title);
                    }
                }
            }
        }
    }
    fn append_group(&mut self, title: Option<String>) {
        let section = self.sections.last_mut().unwrap();
        section.groups.push(GroupDraft {
            title,
            rules: vec![],
            notes: vec![],
        });
        self.choose_index = None;
    }
    fn current_group(&mut self) -> &mut GroupDraft {
        let section = self.sections.last_mut().unwrap();
        section.groups.last_mut().unwrap()
    }
    fn add_comment(&mut self, text: String, hours: Option<f64>) {
        if text.to_ascii_lowercase().contains("general elective") {
            if let Some(amount) = hours.or_else(|| choose_amount(&text).and_then(|(_, h)| h)) {
                self.current_group().rules.push(RuleDraft {
                    kind: DraftKind::General(amount),
                    label: text,
                    hours: Some(amount),
                    notes: vec![],
                    option_categories: vec![],
                    current_category: None,
                });
                self.choose_index = None;
                return;
            }
        }
        if let Some(range) = course_range(&text) {
            if let Some(idx) = self.choose_index {
                if let DraftKind::Choose { ranges, .. } = &mut self.current_group().rules[idx].kind
                {
                    ranges.push(range);
                    return;
                }
            }
            self.current_group().notes.push(text);
            return;
        }
        if let Some((count, amount_hours)) = choose_amount(&text) {
            let hours = hours.or(amount_hours);
            self.current_group().rules.push(RuleDraft {
                kind: DraftKind::Choose {
                    count,
                    hours: amount_hours,
                    options: vec![],
                    ranges: vec![],
                },
                label: text,
                hours,
                notes: vec![],
                option_categories: vec![],
                current_category: None,
            });
            self.choose_index = Some(self.current_group().rules.len() - 1);
            return;
        }
        if let Some(idx) = self.choose_index {
            self.current_group().rules[idx].notes.push(text);
        } else {
            self.current_group().notes.push(text);
        }
    }
    fn add_courses(&mut self, sets: Vec<Vec<CourseKey>>, alternative: bool, hours: Option<f64>) {
        if alternative {
            if let Some(idx) = self.choose_index {
                if let DraftKind::Choose { options, .. } = &mut self.current_group().rules[idx].kind
                {
                    if let Some(last) = options.last_mut() {
                        last.extend(sets);
                        return;
                    }
                }
            }
            let group = self.current_group();
            if let Some(last) = group.rules.last_mut() {
                if let DraftKind::Required(alts) = &mut last.kind {
                    alts.extend(sets);
                    return;
                }
            }
        }
        if let Some(idx) = self.choose_index {
            if hours.is_none() {
                if let DraftKind::Choose { options, .. } = &mut self.current_group().rules[idx].kind
                {
                    options.push(sets);
                    let cat = self.current_group().rules[idx].current_category.clone();
                    self.current_group().rules[idx].option_categories.push(cat);
                    return;
                }
            }
        }
        self.current_group().rules.push(RuleDraft {
            kind: DraftKind::Required(sets),
            label: String::new(),
            hours,
            notes: vec![],
            option_categories: vec![],
            current_category: None,
        });
        self.choose_index = None;
    }
    fn finish(&self) -> Vec<RequirementSection> {
        let mut section_ids = BTreeSet::new();
        self.sections
            .iter()
            .map(|draft| {
                let section_id = unique(&slug(&draft.title), &mut section_ids);
                let mut group_ids = BTreeSet::new();
                let groups: Vec<RequirementGroup> = draft
                    .groups
                    .iter()
                    .filter_map(|group| {
                        let group_id = unique(
                            &format!(
                                "{section_id}/{}",
                                slug(group.title.as_deref().unwrap_or("courses"))
                            ),
                            &mut group_ids,
                        );
                        let mut rule_ids = BTreeSet::new();
                        let rules: Vec<RequirementRule> = group
                            .rules
                            .iter()
                            .filter_map(|rule| {
                                let (kind, mut label) = match &rule.kind {
                                    DraftKind::Required(alts) => {
                                        if alts.is_empty() {
                                            return None;
                                        }
                                        let option = CourseOption {
                                            alternatives: alts.clone(),
                                        };
                                        (RuleKind::Required(option.clone()), option.label())
                                    }
                                    DraftKind::Choose {
                                        count,
                                        hours,
                                        options,
                                        ranges,
                                    } => (
                                        RuleKind::Choose {
                                            count: *count,
                                            hours: *hours,
                                            options: options
                                                .iter()
                                                .map(|a| CourseOption {
                                                    alternatives: a.clone(),
                                                })
                                                .collect(),
                                            ranges: ranges.clone(),
                                        },
                                        rule.label.clone(),
                                    ),
                                    DraftKind::General(hours) => (
                                        RuleKind::GeneralElectives { hours: *hours },
                                        rule.label.clone(),
                                    ),
                                };
                                if label.is_empty() {
                                    label = rule.label.clone();
                                }
                                let id =
                                    unique(&format!("{group_id}/{}", slug(&label)), &mut rule_ids);
                                let categories =
                                    if rule.option_categories.iter().any(|c| c.is_some()) {
                                        Some(rule.option_categories.clone())
                                    } else {
                                        None
                                    };
                                Some(RequirementRule {
                                    id,
                                    kind,
                                    label,
                                    hours: rule.hours,
                                    notes: rule.notes.clone(),
                                    option_categories: categories,
                                })
                            })
                            .collect();
                        if rules.is_empty() && group.notes.is_empty() {
                            return None;
                        }
                        Some(RequirementGroup {
                            id: group_id,
                            title: group.title.clone(),
                            rules,
                            notes: group.notes.clone(),
                        })
                    })
                    .collect();
                let title = draft
                    .title
                    .trim_matches(|c: char| c == '*' || c.is_whitespace())
                    .to_string();
                let is_concentration = !groups.is_empty() && is_choosable(&title);
                RequirementSection {
                    id: section_id,
                    title,
                    is_concentration,
                    groups,
                    notes: draft.notes.clone(),
                }
            })
            .collect()
    }
}

fn is_choosable(title: &str) -> bool {
    Regex::new(r"(?i)^(concentration|track|option|pathway)( in\b|\s*:)|\b(concentration|track|option|pathway)$").unwrap().is_match(title)
}

fn slug(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let lowered = Regex::new(r"[^a-z0-9]+").unwrap().replace_all(&lower, "-");
    let trimmed = lowered.trim_matches('-');
    trimmed
        .chars()
        .take(60)
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn unique(id: &str, used: &mut BTreeSet<String>) -> String {
    let base = if id.is_empty() { "item" } else { id };
    let mut candidate = base.to_string();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{id}-{n}");
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

/// Unused import guard for table_rows if a caller wants cell extraction.
#[allow(dead_code)]
fn _rows(html: &str) -> Option<Vec<Vec<String>>> {
    table_rows(html)
}
