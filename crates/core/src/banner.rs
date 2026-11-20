//! Banner HTML parsers and a small client for nubanner.neu.edu.
//!
//! Portions derived from SearchNEU (AGPL-3.0), packages/scraper.

use crate::catalog::{self, BannerMeetingFaculty, BannerSection};
use crate::html::{plain_text, table_rows};
use crate::model::*;
use regex::Regex;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct SubjectLookup {
    by_name: HashMap<String, String>,
}

impl SubjectLookup {
    pub fn new(subjects: &[Subject]) -> Self {
        let mut by_name = HashMap::new();
        for s in subjects {
            by_name.insert(s.name.to_ascii_lowercase(), s.code.clone());
            by_name.insert(s.code.to_ascii_lowercase(), s.code.clone());
        }
        Self { by_name }
    }
    pub fn code(&self, name: &str) -> String {
        let trimmed = name.trim();
        if let Some(code) = self.by_name.get(&trimmed.to_ascii_lowercase()) {
            return code.clone();
        }
        if let Some(m) = Regex::new(r"\s+-\s+[A-Z]{2,5}$").unwrap().find(trimmed) {
            let base = trimmed[..m.start()].to_ascii_lowercase();
            if let Some(code) = self.by_name.get(&base) {
                return code.clone();
            }
        }
        trimmed.to_string()
    }
}

struct Node {
    is_and: bool,
    items: Vec<Item>,
}
enum Item {
    Leaf(Requisite),
    Node(usize),
}

pub fn prerequisites(html: &str, subjects: &SubjectLookup) -> Option<Requisite> {
    let rows = table_rows(html)?;
    if rows.is_empty() {
        return None;
    }
    let mut nodes = vec![Node {
        is_and: false,
        items: vec![],
    }];
    let mut stack = vec![0usize];
    for cells in rows {
        if cells.len() < 6 {
            continue;
        }
        let cell = |i: usize| {
            cells
                .get(i)
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        let and_or = cell(0).to_ascii_lowercase();
        let current = *stack.last().unwrap();
        if and_or == "and" {
            nodes[current].is_and = true;
        } else if and_or == "or" {
            nodes[current].is_and = false;
        }
        if !cell(1).is_empty() {
            nodes.push(Node {
                is_and: false,
                items: vec![],
            });
            let child = nodes.len() - 1;
            nodes[current].items.push(Item::Node(child));
            stack.push(child);
        }
        let current = *stack.last().unwrap();
        if !cell(2).is_empty() {
            let score = cell(3).parse().ok();
            nodes[current].items.push(Item::Leaf(Requisite::Test {
                name: cell(2),
                score,
            }));
        }
        if !cell(4).is_empty() && !cell(5).is_empty() {
            let key = CourseKey {
                subject: subjects.code(&cell(4)),
                number: cell(5),
            };
            nodes[current]
                .items
                .push(Item::Leaf(Requisite::Course(key)));
        }
        if !cell(8).is_empty() && stack.len() > 1 {
            stack.pop();
        }
    }
    build(&nodes, 0)?.simplified()
}

fn build(nodes: &[Node], index: usize) -> Option<Requisite> {
    let node = &nodes[index];
    let built: Vec<Requisite> = node
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Leaf(r) => Some(r.clone()),
            Item::Node(i) => build(nodes, *i),
        })
        .collect();
    if built.is_empty() {
        None
    } else if node.is_and {
        Some(Requisite::And(built))
    } else {
        Some(Requisite::Or(built))
    }
}

pub fn corequisites(html: &str, subjects: &SubjectLookup) -> Option<Requisite> {
    let rows = table_rows(html)?;
    let items: Vec<Requisite> = rows
        .into_iter()
        .filter_map(|cells| {
            let (subject, number) = if cells.len() == 3 {
                (cells[0].clone(), cells[1].clone())
            } else if cells.len() >= 4 {
                (cells[1].clone(), cells[2].clone())
            } else {
                return None;
            };
            if subject.trim().is_empty() || number.trim().is_empty() {
                return None;
            }
            Some(Requisite::Course(CourseKey {
                subject: subjects.code(subject.trim()),
                number: number.trim().to_string(),
            }))
        })
        .collect();
    match items.len() {
        0 => None,
        1 => Some(items.into_iter().next().unwrap()),
        _ => Some(Requisite::And(items)),
    }
}

/// Headings such as "Must be enrolled in one of the following Levels:" and the values under them.
/// `None` when the HTML is not a restrictions page, so a failed response stays unknown.
pub fn enrollment_restrictions(html: &str) -> Option<Vec<EnrollmentRestriction>> {
    let text = plain_text(html);
    let lowered = text.to_ascii_lowercase();
    if !lowered.contains("enrolled in one of the following")
        && !lowered.contains("not all restrictions")
        && !lowered.contains("no restrictions")
        && !lowered.contains("no restriction")
    {
        return None;
    }
    let heading =
        Regex::new(r"(?i)^(must|cannot) be enrolled in one of the following ([^:]+):\s*$").unwrap();
    let mut restrictions = Vec::new();
    let mut current: Option<EnrollmentRestriction> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(caps) = heading.captures(line) {
            if let Some(done) = current.take() {
                if !done.values.is_empty() {
                    restrictions.push(done);
                }
            }
            let kind = if caps[1].eq_ignore_ascii_case("must") {
                RestrictionKind::Must
            } else {
                RestrictionKind::Cannot
            };
            current = Some(EnrollmentRestriction {
                kind,
                category: caps[2].trim().to_string(),
                values: Vec::new(),
            });
            continue;
        }
        if let Some(restriction) = current.as_mut() {
            restriction.values.push(line.to_string());
        }
    }
    if let Some(done) = current {
        if !done.values.is_empty() {
            restrictions.push(done);
        }
    }
    Some(restrictions)
}

pub fn description(html: &str) -> String {
    let text = plain_text(html).trim().to_string();
    let lowered = text.to_ascii_lowercase();
    if lowered.starts_with("no course description")
        || lowered.starts_with("no description")
        || lowered.contains("no course description available")
        || lowered.contains("no description available")
        || lowered.contains("no information available")
    {
        String::new()
    } else {
        text
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CourseFee {
    pub description: String,
    pub amount: Option<String>,
}

pub fn course_fees(html: &str) -> Vec<CourseFee> {
    let text = plain_text(html);
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("no fee information")
        || lowered.contains("no fees")
        || lowered.contains("no information available")
    {
        return Vec::new();
    }
    let mut fees = Vec::new();
    let row_re = Regex::new(r"(?is)<tr[^>]*>([\s\S]*?)</tr>").unwrap();
    let cell_re = Regex::new(r"(?is)<t[dh][^>]*>([\s\S]*?)</t[dh]>").unwrap();
    for row in row_re.captures_iter(html) {
        let cells: Vec<String> = cell_re
            .captures_iter(&row[1])
            .map(|c| plain_text(&c[1]).trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();
        if cells.is_empty() {
            continue;
        }
        let first = &cells[0];
        if first.eq_ignore_ascii_case("fee")
            || first.eq_ignore_ascii_case("fee type")
            || first.eq_ignore_ascii_case("description")
        {
            continue;
        }
        let description = first.clone();
        let amount = cells.get(1).cloned();
        fees.push(CourseFee {
            description,
            amount,
        });
    }
    if fees.is_empty() {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.to_ascii_lowercase().starts_with("fee")
                || trimmed.to_ascii_lowercase().contains("no fee")
            {
                continue;
            }
            if trimmed.contains('$') {
                fees.push(CourseFee {
                    description: trimmed.to_string(),
                    amount: None,
                });
            }
        }
    }
    fees
}

fn is_attribute_code(s: &str) -> bool {
    (1..=6).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn course_attributes(html: &str) -> Vec<String> {
    let text = plain_text(html);
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("no attribute information")
        || lowered.contains("no attributes")
        || lowered.contains("no information available")
    {
        return Vec::new();
    }
    let mut attrs = Vec::new();
    let row_re = Regex::new(r"(?is)<tr[^>]*>([\s\S]*?)</tr>").unwrap();
    let cell_re = Regex::new(r"(?is)<t[dh][^>]*>([\s\S]*?)</t[dh]>").unwrap();
    for row in row_re.captures_iter(html) {
        let cells: Vec<String> = cell_re
            .captures_iter(&row[1])
            .map(|c| plain_text(&c[1]).trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();
        if cells.is_empty() {
            continue;
        }
        if cells[0].eq_ignore_ascii_case("attribute")
            || cells[0].eq_ignore_ascii_case("code")
            || cells[0].eq_ignore_ascii_case("description")
        {
            continue;
        }
        let item = if cells.len() >= 2 && is_attribute_code(&cells[0]) {
            cells[1].clone()
        } else {
            cells[0].clone()
        };
        if !attrs.contains(&item) {
            attrs.push(item);
        }
    }
    if attrs.is_empty() {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.to_ascii_lowercase().starts_with("attribute")
                || trimmed.to_ascii_lowercase().contains("no attribute")
            {
                continue;
            }
            if !attrs.contains(&trimmed.to_string()) {
                attrs.push(trimmed.to_string());
            }
        }
    }
    attrs
}

#[derive(Clone, Debug, Default)]
pub struct CatalogDetails {
    pub title: Option<String>,
    pub college: Option<String>,
    pub department: Option<String>,
    pub min_credits: Option<f64>,
    pub max_credits: Option<f64>,
}

pub fn catalog_details(html: &str) -> CatalogDetails {
    let lines: Vec<String> = plain_text(html)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    fn value(lines: &[String], label: &str) -> Option<String> {
        let i = lines.iter().position(|l| l.starts_with(label))?;
        let inline = lines[i][label.len()..].trim().to_string();
        if !inline.is_empty() {
            return Some(inline);
        }
        let next = lines.get(i + 1)?;
        if next.ends_with(':') {
            None
        } else {
            Some(next.clone())
        }
    }
    fn strip_code(s: Option<String>) -> Option<String> {
        let s = s?;
        let mut parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() > 1 {
            if let Some(last) = parts.last() {
                if last.len() <= 4
                    && last
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                {
                    parts.pop();
                }
            }
        }
        Some(parts.join(" "))
    }
    let credits = value(&lines, "Credit Hours:").and_then(|t| credit_range(&t));
    CatalogDetails {
        title: value(&lines, "Title:"),
        college: strip_code(value(&lines, "College:")).map(|s| expand_college_name(&s)),
        department: strip_code(value(&lines, "Department:")).map(|s| expand_college_name(&s)),
        min_credits: credits.map(|c| c.0),
        max_credits: credits.map(|c| c.1),
    }
}

pub fn credit_range(text: &str) -> Option<(f64, f64)> {
    let nums: Vec<f64> = Regex::new(r"\d+(?:\.\d+)?")
        .unwrap()
        .find_iter(text)
        .filter_map(|m| m.as_str().parse().ok())
        .collect();
    let first = *nums.first()?;
    let second = nums.get(1).copied().unwrap_or(first);
    Some((first.min(second), first.max(second)))
}

pub fn expand_college_name(s: &str) -> String {
    let mut out = format!(" {s} ");
    for (from, to) in [
        (" Coll of ", " College of "),
        (" Col of ", " College of "),
        (" Coll ", " College "),
        (" Comp Sciences ", " Computer Sciences "),
        (" Hlth ", " Health "),
        (" Sci ", " Science "),
        (" Engr ", " Engineering "),
        (" Prof ", " Professional "),
        (" School Business ", " School of Business "),
        (" Soc Sci ", " Social Sciences "),
    ] {
        out = out.replace(from, to);
    }
    out.trim().to_string()
}

pub fn mutual_exclusions(html: &str, subjects: &SubjectLookup) -> Vec<CourseKey> {
    let text = plain_text(html);
    if text.to_ascii_lowercase().contains("no mutual exclusion") {
        return vec![];
    }
    let mut keys = Vec::new();
    if let Some(rows) = table_rows(html) {
        for cells in rows {
            let cells: Vec<String> = cells.iter().map(|c| c.trim().to_string()).collect();
            for i in 0..cells.len().saturating_sub(1) {
                if is_subject_text(&cells[i]) && is_course_number(&cells[i + 1]) {
                    push_unique(
                        &mut keys,
                        CourseKey {
                            subject: subjects.code(&cells[i]),
                            number: cells[i + 1].clone(),
                        },
                    );
                    break;
                }
            }
        }
    }
    if keys.is_empty() {
        for cap in Regex::new(r"\b([A-Z]{2,6})\s?(\d{3,4}[A-Z]?)\b")
            .unwrap()
            .captures_iter(&text)
        {
            push_unique(
                &mut keys,
                CourseKey {
                    subject: cap[1].to_string(),
                    number: cap[2].to_string(),
                },
            );
        }
    }
    keys
}

fn coerce_section_ints(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    for key in [
        "maximumEnrollment",
        "enrollment",
        "seatsAvailable",
        "waitCapacity",
        "waitCount",
        "waitAvailable",
        "crossListCapacity",
        "crossListCount",
        "crossListAvailable",
    ] {
        if let Some(number) = obj.get(key).and_then(|v| v.as_f64()) {
            obj.insert(key.to_string(), serde_json::json!(number.round() as i64));
        }
    }
}

fn push_unique(keys: &mut Vec<CourseKey>, key: CourseKey) {
    if !keys.contains(&key) {
        keys.push(key);
    }
}

fn is_subject_text(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_alphabetic()) && !is_course_number(text)
}

fn is_course_number(text: &str) -> bool {
    (3..=5).contains(&text.len())
        && text.chars().next().is_some_and(|c| c.is_ascii_digit())
        && text.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn catalog_lists(items: &[(String, String)], course: &CourseKey) -> bool {
    let wanted = format!("{}{}", course.subject, course.number)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    items.iter().any(|(code, _)| {
        code.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase()
            == wanted
    })
}

/// Attempts in `BannerClient.perform` before a retryable failure is given up.
pub const MAX_ATTEMPTS: u32 = 4;
/// Independent search sessions used by `fetch_all_sections` (one request in flight per cookie).
pub const SECTION_PARALLELISM: usize = 4;

/// 429 and 5xx are retried. Other HTTP statuses are not.
pub fn should_retry_status(code: u16) -> bool {
    code == 429 || code >= 500
}

/// Sleep length and the next base delay. `attempt` is 1-based. `None` on the last try,
/// so four attempts sleep after the first three only. Jitter is clamped to `0..=0.5`.
pub fn backoff(attempt: u32, delay: f64, jitter: f64) -> Option<(f64, f64)> {
    if attempt == 0 || attempt >= MAX_ATTEMPTS {
        return None;
    }
    let jitter = jitter.clamp(0.0, 0.5);
    Some((delay + jitter, (delay * 2.0).min(20.0)))
}

/// Pages for each search session. Page `n` is session `n % sessions`, so one cookie
/// never has two requests in flight. Extra parallelism beyond the page count is unused.
pub fn session_pages(page_count: usize, parallelism: usize) -> Vec<Vec<usize>> {
    if page_count == 0 {
        return Vec::new();
    }
    let sessions = parallelism.max(1).min(page_count);
    let mut pages = vec![Vec::new(); sessions];
    for page in 0..page_count {
        pages[page % sessions].push(page);
    }
    pages
}

fn jitter_seconds() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    f64::from(nanos % 501) / 1000.0
}

enum Attempt {
    Ready(Box<ureq::Response>),
    Retry(String),
    Stop(String),
}

fn classify(result: Result<ureq::Response, ureq::Error>) -> Attempt {
    match result {
        Ok(response) => Attempt::Ready(Box::new(response)),
        Err(ureq::Error::Status(code, _)) if !should_retry_status(code) => {
            Attempt::Stop(format!("HTTP {code}"))
        }
        Err(error) => Attempt::Retry(error.to_string()),
    }
}

/// Runs a request, retrying 429, 5xx, and transport errors. Other HTTP statuses return immediately.
fn perform(build: impl Fn() -> Attempt) -> Result<ureq::Response, String> {
    let mut delay = 1.0;
    let mut last = String::from("Banner request failed");
    for attempt in 1..=MAX_ATTEMPTS {
        match build() {
            Attempt::Ready(response) => return Ok(*response),
            Attempt::Stop(message) => return Err(message),
            Attempt::Retry(message) => last = message,
        }
        if let Some((sleep_for, next)) = backoff(attempt, delay, jitter_seconds()) {
            std::thread::sleep(std::time::Duration::from_secs_f64(sleep_for));
            delay = next;
        }
    }
    Err(last)
}

fn sections_of(value: &serde_json::Value) -> Result<Vec<crate::catalog::BannerSection>, String> {
    let rows = value
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    let mut sections = Vec::new();
    for mut row in rows {
        coerce_section_ints(&mut row);
        sections.push(serde_json::from_value(row).map_err(|e| e.to_string())?);
    }
    Ok(sections)
}

fn course_sections(
    page: &serde_json::Value,
    key: &CourseKey,
) -> Result<Vec<BannerSection>, String> {
    if page.get("success").and_then(|value| value.as_bool()) == Some(false) {
        return Err("Banner did not return a successful section search".into());
    }
    match page.get("data") {
        Some(serde_json::Value::Array(_)) => {}
        None | Some(serde_json::Value::Null)
            if page.get("success").and_then(|value| value.as_bool()) == Some(true) =>
        {
            return Ok(Vec::new());
        }
        _ => return Err("Banner did not return a section list".into()),
    }
    Ok(sections_of(page)?
        .into_iter()
        .filter(|section| {
            section.subject == key.subject
                && section.course_number == key.number
                && !section.course_reference_number.is_empty()
        })
        .collect())
}

fn faculty_instructors(body: &str) -> Result<Vec<Instructor>, String> {
    #[derive(serde::Deserialize)]
    struct FacultyMeetingTimes {
        fmt: Option<Vec<BannerMeetingFaculty>>,
    }
    let response: FacultyMeetingTimes =
        serde_json::from_str(body).map_err(|error| error.to_string())?;
    let mut seen = HashSet::new();
    let mut instructors: Vec<_> = response
        .fmt
        .unwrap_or_default()
        .into_iter()
        .flat_map(|meeting| meeting.faculty.unwrap_or_default())
        .filter_map(|faculty| faculty.to_instructor())
        .filter(|instructor| seen.insert(instructor.name.clone()))
        .collect();
    instructors.sort_by_key(|instructor| !instructor.primary);
    Ok(instructors)
}

#[derive(Clone, Debug)]
pub struct BannerClient {
    pub base: String,
}

impl Default for BannerClient {
    fn default() -> Self {
        Self {
            base: "https://nubanner.neu.edu".into(),
        }
    }
}

/// Fall 2026 Computer Science, BSCS (Boston) page on catalog.northeastern.edu.
pub const DEFAULT_CATALOG_PATH: &str =
    "undergraduate/computer-information-science/computer-science/bscs/";

/// Join a site origin and a catalog path. A missing slash used to produce
/// `catalog.northeastern.eduundergraduate/...`, which does not resolve.
pub fn join_url(base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

impl BannerClient {
    pub fn get(&self, path: &str) -> Result<String, String> {
        let url = join_url(&self.base, path);
        perform(|| {
            classify(
                ureq::get(&url)
                    .timeout(std::time::Duration::from_secs(20))
                    .call(),
            )
        })?
        .into_string()
        .map_err(|e| e.to_string())
    }

    pub fn refresh_course(
        &self,
        catalog: &mut TermCatalog,
        key: &CourseKey,
        now: f64,
    ) -> Result<bool, String> {
        let term = catalog.term.code.clone();
        let fresh = self.fetch_course_sections(&term, key)?;
        let cross_listed: HashSet<_> = fresh
            .iter()
            .filter(|section| {
                section
                    .cross_list
                    .as_deref()
                    .is_some_and(|group| !group.trim().is_empty())
            })
            .map(|section| section.course_reference_number.clone())
            .collect();
        let cached_cross_lists: HashMap<_, _> = catalog
            .sections
            .iter()
            .filter(|section| cross_listed.contains(&section.crn))
            .map(|section| (section.crn.clone(), section.cross_list_crns.clone()))
            .collect();
        if !catalog::apply_course_refresh(catalog, key, &fresh, now) {
            return Ok(false);
        }
        if !cross_listed.is_empty() {
            let subjects = SubjectLookup::new(&catalog.subjects);
            for section in catalog
                .sections
                .iter_mut()
                .filter(|section| cross_listed.contains(&section.crn))
            {
                if let Some(cached) = cached_cross_lists.get(&section.crn) {
                    for crn in cached {
                        if !section.cross_list_crns.contains(crn) {
                            section.cross_list_crns.push(crn.clone());
                        }
                    }
                }
                if let Ok(html) = self.post_section_form("getXlstSections", &term, &section.crn) {
                    let links: Vec<_> = catalog::cross_listed_sections(&html, &subjects)
                        .into_iter()
                        .map(|link| link.crn)
                        .filter(|crn| crn != &section.crn)
                        .collect();
                    if !links.is_empty() {
                        section.cross_list_crns = links;
                    }
                }
            }
        }
        let refresh_restrictions = catalog
            .courses
            .iter()
            .any(|course| &course.key() == key && course.details_loaded);
        // Section availability remains useful if optional metadata is unavailable.
        let _ = self.load_course_metadata(catalog, key);
        self.fill_section_details(catalog, key, refresh_restrictions);
        Ok(true)
    }

    /// Load missing course and section details on demand.
    pub fn load_course_details(
        &self,
        catalog: &mut TermCatalog,
        key: &CourseKey,
    ) -> Result<(), String> {
        self.load_course_metadata(catalog, key)?;
        self.fill_section_details(catalog, key, false);
        Ok(())
    }

    fn load_course_metadata(
        &self,
        catalog: &mut TermCatalog,
        key: &CourseKey,
    ) -> Result<(), String> {
        let Some(course) = catalog
            .courses
            .iter_mut()
            .find(|course| &course.key() == key)
        else {
            return Ok(());
        };
        let Some(first) = catalog
            .sections
            .iter()
            .find(|section| &section.course_key() == key)
        else {
            return Ok(());
        };
        let term = &catalog.term.code;
        if !course.details_loaded {
            let crn = &first.crn;
            let subjects = SubjectLookup::new(&catalog.subjects);
            let description_html = self.post_section_form("getCourseDescription", term, crn)?;
            let prereq_html = self.post_section_form("getSectionPrerequisites", term, crn)?;
            let coreq_html = self.post_section_form("getCorequisites", term, crn)?;
            let details_html = self.post_section_form("getSectionCatalogDetails", term, crn)?;
            let details = catalog_details(&details_html);
            course.description = description(&description_html);
            course.prereqs = prerequisites(&prereq_html, &subjects);
            course.coreqs = corequisites(&coreq_html, &subjects);
            if let Some(title) = details.title.filter(|_| !course.special_topics) {
                course.title = title;
            }
            course.college = details.college.or(course.college.take());
            course.department = details.department.or(course.department.take());
            course.details_loaded = true;
        }
        if course.special_topics {
            for section in catalog
                .sections
                .iter_mut()
                .filter(|section| &section.course_key() == key && section.description.is_none())
            {
                let html = self.post_section_form("getCourseDescription", term, &section.crn)?;
                section.description = Some(description(&html));
            }
        }
        Ok(())
    }

    fn fill_section_details(
        &self,
        catalog: &mut TermCatalog,
        key: &CourseKey,
        refresh_restrictions: bool,
    ) {
        let term = &catalog.term.code;
        let mut restriction_requested = false;
        for section in catalog
            .sections
            .iter_mut()
            .filter(|section| &section.course_key() == key)
        {
            if section.instructors.is_none() {
                let url = format!(
                    "{}searchResults/getFacultyMeetingTimes?term={term}&courseReferenceNumber={}",
                    self.ssb(),
                    section.crn
                );
                if let Ok(instructors) = self.get(&url).and_then(|body| faculty_instructors(&body))
                {
                    section.instructors = Some(instructors);
                }
            }
            if refresh_restrictions || section.restrictions.is_none() {
                if restriction_requested {
                    std::thread::sleep(Self::RESTRICTION_GAP);
                }
                restriction_requested = true;
                if let Some(restrictions) = self
                    .post_section_form("getRestrictions", term, &section.crn)
                    .ok()
                    .and_then(|html| enrollment_restrictions(&html))
                {
                    section.restrictions = Some(restrictions);
                }
            }
        }
    }

    /// Open a Banner search session and read current seats for one course.
    /// Falls back to the bundled snapshot when Banner is unreachable.
    pub fn live_seats(
        &self,
        term: &str,
        subject: &str,
        number: &str,
    ) -> Result<Vec<LiveSeat>, String> {
        let cookie = self.open_session(term)?;
        let page = self.search_page(&cookie, term, Some(subject), Some(number), 0, 500)?;
        parse_live_seats(
            &serde_json::to_string(&page).map_err(|e| e.to_string())?,
            subject,
            number,
        )
    }

    /// Current seats for each distinct subject and number. One failure fails the batch
    /// so a watchlist check is not mixed with stale snapshot counts.
    pub fn live_seats_for_keys(
        &self,
        term: &str,
        keys: &[(String, String)],
    ) -> Result<Vec<LiveSeat>, String> {
        let mut all = Vec::new();
        let mut seen = HashSet::new();
        for (subject, number) in keys {
            for seat in self.live_seats(term, subject, number)? {
                if seen.insert(seat.crn.clone()) {
                    all.push(seat);
                }
            }
        }
        Ok(all)
    }

    fn ssb(&self) -> String {
        if self.base.contains("StudentRegistrationSsb") {
            let base = self.base.trim_end_matches('/');
            format!("{base}/")
        } else {
            format!(
                "{}/StudentRegistrationSsb/ssb/",
                self.base.trim_end_matches('/')
            )
        }
    }

    pub fn fetch_terms(&self, max: i32) -> Result<Vec<Term>, String> {
        let url = format!(
            "{}classSearch/getTerms?offset=1&max={max}&searchTerm=",
            self.ssb()
        );
        let body = self.get(&url)?;
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(&body).map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let code = row.get("code").and_then(|v| v.as_str())?.to_string();
                let description = crate::html::decode_entities(
                    row.get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                );
                Some(Term {
                    code,
                    banner_description: description,
                })
            })
            .collect())
    }

    pub fn open_session(&self, term: &str) -> Result<String, String> {
        let url = format!("{}term/search?mode=search", self.ssb());
        let body = format!("term={term}&studyPath=&studyPathText=&startDatepicker=&endDatepicker=");
        let opened = perform(|| {
            classify(
                ureq::post(&url)
                    .set(
                        "Content-Type",
                        "application/x-www-form-urlencoded; charset=UTF-8",
                    )
                    .timeout(std::time::Duration::from_secs(30))
                    .send_string(&body),
            )
        })?;
        let cookie = opened
            .all("set-cookie")
            .into_iter()
            .filter_map(|header| header.split(';').next().map(str::trim))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        let _ = opened.into_string();
        if cookie.is_empty() {
            Err("Banner did not start a search session".into())
        } else {
            Ok(cookie)
        }
    }

    pub fn search_page(
        &self,
        cookie: &str,
        term: &str,
        subject: Option<&str>,
        number: Option<&str>,
        offset: i32,
        page_size: i32,
    ) -> Result<serde_json::Value, String> {
        let mut url = format!(
            "{}searchResults/searchResults?txt_term={term}&pageOffset={offset}&pageMaxSize={page_size}&sortColumn=subjectDescription&sortDirection=asc",
            self.ssb()
        );
        if let Some(subject) = subject {
            url.push_str(&format!("&txt_subject={subject}"));
        }
        if let Some(number) = number {
            url.push_str(&format!("&txt_courseNumber={number}"));
        }
        let cookie = cookie.to_string();
        let body = perform(|| {
            classify(
                ureq::get(&url)
                    .set("Cookie", &cookie)
                    .timeout(std::time::Duration::from_secs(40))
                    .call(),
            )
        })?
        .into_string()
        .map_err(|e| e.to_string())?;
        serde_json::from_str(&body).map_err(|e| e.to_string())
    }

    /// Every section in a term. A one-row probe reads `totalCount`, then up to
    /// `SECTION_PARALLELISM` sessions each walk their own pages (`page % sessions`).
    pub fn fetch_all_sections(
        &self,
        term: &str,
    ) -> Result<Vec<crate::catalog::BannerSection>, String> {
        let first = self.open_session(term)?;
        let probe = self.search_page(&first, term, None, None, 0, 1)?;
        let total = probe
            .get("totalCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if total <= 0 {
            return Ok(vec![]);
        }
        let page_size = 500i64;
        let page_count = ((total + page_size - 1) / page_size) as usize;
        let slots = session_pages(page_count, SECTION_PARALLELISM);
        let mut cookies = Vec::with_capacity(slots.len());
        cookies.push(first);
        for _ in 1..slots.len() {
            cookies.push(self.open_session(term)?);
        }
        let mut by_page: Vec<Vec<crate::catalog::BannerSection>> = vec![Vec::new(); page_count];
        std::thread::scope(|scope| -> Result<(), String> {
            let mut joins = Vec::new();
            for (cookie, pages) in cookies.into_iter().zip(slots) {
                let client = self.clone();
                let term = term.to_string();
                joins.push(scope.spawn(
                    move || -> Result<Vec<(usize, Vec<crate::catalog::BannerSection>)>, String> {
                        let mut found = Vec::new();
                        for page in pages {
                            let value = client.search_page(
                                &cookie,
                                &term,
                                None,
                                None,
                                (page as i64 * page_size) as i32,
                                page_size as i32,
                            )?;
                            found.push((page, sections_of(&value)?));
                        }
                        Ok(found)
                    },
                ));
            }
            for join in joins {
                let part = join
                    .join()
                    .map_err(|_| String::from("A section fetch stopped."))??;
                for (page, sections) in part {
                    if page < by_page.len() {
                        by_page[page] = sections;
                    }
                }
            }
            Ok(())
        })?;
        let mut all = Vec::new();
        let mut seen = HashSet::new();
        for sections in by_page {
            for section in sections {
                if seen.insert(section.course_reference_number.clone()) {
                    all.push(section);
                }
            }
        }
        Ok(all)
    }

    /// Fetch every section returned for one exact course number.
    pub fn fetch_course_sections(
        &self,
        term: &str,
        key: &CourseKey,
    ) -> Result<Vec<crate::catalog::BannerSection>, String> {
        const PAGE_SIZE: i32 = 500;
        let cookie = self.open_session(term)?;
        let mut offset = 0;
        let mut sections = Vec::new();
        let mut seen = HashSet::new();
        loop {
            let page = self.search_page(
                &cookie,
                term,
                Some(&key.subject),
                Some(&key.number),
                offset,
                PAGE_SIZE,
            )?;
            let total = page
                .get("totalCount")
                .and_then(|value| value.as_i64())
                .unwrap_or(0);
            let success = page.get("success").and_then(|value| value.as_bool());
            let rows = page.get("data").and_then(|value| value.as_array());
            if success == Some(false)
                || (rows.is_none() && success != Some(true))
                || (total > i64::from(offset) && rows.is_none_or(Vec::is_empty))
            {
                return Err("Banner returned an incomplete section list".into());
            }
            if rows.into_iter().flatten().any(|row| {
                row.get("term")
                    .and_then(|value| value.as_str())
                    .is_some_and(|row_term| !row_term.is_empty() && row_term != term)
            }) {
                return Err("Banner returned sections for a different term".into());
            }
            sections.extend(
                course_sections(&page, key)?
                    .into_iter()
                    .filter(|section| seen.insert(section.course_reference_number.clone())),
            );
            offset += PAGE_SIZE;
            if total <= i64::from(offset) {
                return Ok(sections);
            }
        }
    }

    pub fn fetch_offered(
        &self,
        term: &str,
        courses: &[CourseKey],
    ) -> Result<HashSet<CourseKey>, String> {
        if courses.is_empty() {
            return Ok(HashSet::new());
        }
        let cookie = self.open_session(term)?;
        let mut offered = HashSet::new();
        for (index, key) in courses.iter().enumerate() {
            if index > 0 {
                let url = format!("{}classSearch/resetDataForm", self.ssb());
                let _ = perform(|| {
                    classify(
                        ureq::post(&url)
                            .set("Cookie", &cookie)
                            .timeout(std::time::Duration::from_secs(20))
                            .call(),
                    )
                });
            }
            let page =
                self.search_page(&cookie, term, Some(&key.subject), Some(&key.number), 0, 20)?;
            let rows = page.get("data").and_then(|d| d.as_array());
            let hit = rows.is_some_and(|rows| {
                rows.iter().any(|row| {
                    row.get("subject").and_then(|v| v.as_str()) == Some(key.subject.as_str())
                        && row.get("courseNumber").and_then(|v| v.as_str())
                            == Some(key.number.as_str())
                })
            });
            if hit {
                offered.insert(key.clone());
            }
        }
        Ok(offered)
    }

    pub fn catalog_lists_course(&self, course: &CourseKey, term: &str) -> Result<bool, String> {
        let url = format!(
            "{}courseSearch/get_subjectcoursecombo?searchTerm={}%20{}&term={term}&offset=1&max=10",
            self.ssb(),
            course.subject,
            course.number
        );
        let body = self.get(&url)?;
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let items: Vec<(String, String)> = rows
            .into_iter()
            .filter_map(|row| {
                Some((
                    row.get("code")?.as_str()?.to_string(),
                    row.get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ))
            })
            .collect();
        Ok(catalog_lists(&items, course))
    }

    pub fn fetch_subjects(&self, term: &str) -> Result<Vec<Subject>, String> {
        self.code_list(&format!(
            "{}classSearch/get_subject?searchTerm=&term={term}&offset=1&max=900",
            self.ssb()
        ))
        .map(|rows| {
            rows.into_iter()
                .map(|(code, name)| Subject { code, name })
                .collect()
        })
    }

    pub fn fetch_campuses(&self, term: &str) -> Result<Vec<Campus>, String> {
        self.code_list(&format!(
            "{}classSearch/get_campus?searchTerm=&term={term}&offset=1&max=200",
            self.ssb()
        ))
        .map(|rows| {
            rows.into_iter()
                .map(|(code, name)| Campus { code, name })
                .collect()
        })
    }

    pub fn fetch_attributes(&self, term: &str) -> Result<Vec<AttributeInfo>, String> {
        self.code_list(&format!(
            "{}classSearch/get_attribute?searchTerm=&term={term}&offset=1&max=500",
            self.ssb()
        ))
        .map(|rows| {
            rows.into_iter()
                .map(|(code, name)| AttributeInfo { code, name })
                .collect()
        })
    }

    fn code_list(&self, url: &str) -> Result<Vec<(String, String)>, String> {
        let body = self.get(url)?;
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(&body).map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some((
                    row.get("code")?.as_str()?.to_string(),
                    crate::html::decode_entities(
                        row.get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                    )
                    .trim()
                    .to_string(),
                ))
            })
            .collect())
    }

    pub fn catalog_page_url(term: &str, course: &CourseKey) -> String {
        format!(
            "https://bnrordsp.neu.edu/ssb-prod/bwckctlg.p_disp_course_detail?cat_term_in={}&subj_code_in={}&crse_numb_in={}",
            term.trim(),
            course.subject.trim(),
            course.number.trim()
        )
    }

    pub fn fetch_course_description_html(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<String, String> {
        self.post_course_form("courseSearchResults/getCourseDescription", term, course)
    }

    pub fn fetch_course_catalog_details_html(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<String, String> {
        self.post_course_form("courseSearchResults/getCourseCatalogDetails", term, course)
    }

    pub fn fetch_course_prerequisites_html(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<String, String> {
        self.post_course_form("courseSearchResults/getPrerequisites", term, course)
    }

    pub fn fetch_course_corequisites_html(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<String, String> {
        self.post_course_form("courseSearchResults/getCorequisites", term, course)
    }

    pub fn fetch_course_mutual_exclusions_html(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<String, String> {
        self.post_course_form(
            "courseSearchResults/getCourseMutuallyExclusions",
            term,
            course,
        )
    }

    pub fn fetch_course_attributes_html(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<String, String> {
        self.post_course_form("courseSearchResults/getCourseAttributes", term, course)
    }

    pub fn fetch_course_attributes(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<Vec<String>, String> {
        let html = self.fetch_course_attributes_html(term, course)?;
        Ok(course_attributes(&html))
    }

    pub fn fetch_course_restrictions_html(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<String, String> {
        self.post_course_form("courseSearchResults/getRestrictions", term, course)
    }

    pub fn fetch_course_restrictions(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<Option<Vec<EnrollmentRestriction>>, String> {
        let html = self.fetch_course_restrictions_html(term, course)?;
        Ok(enrollment_restrictions(&html))
    }

    pub fn fetch_course_fees_html(&self, term: &str, course: &CourseKey) -> Result<String, String> {
        self.post_course_form("courseSearchResults/getFees", term, course)
    }

    pub fn fetch_course_fees(
        &self,
        term: &str,
        course: &CourseKey,
    ) -> Result<Vec<CourseFee>, String> {
        let html = self.fetch_course_fees_html(term, course)?;
        Ok(course_fees(&html))
    }

    pub fn fetch_description_html(&self, term: &str, crn: &str) -> Result<String, String> {
        self.post_section_form("getCourseDescription", term, crn)
    }

    pub fn fetch_section_catalog_details_html(
        &self,
        term: &str,
        crn: &str,
    ) -> Result<String, String> {
        self.post_section_form("getSectionCatalogDetails", term, crn)
    }

    pub fn fetch_section_prerequisites_html(
        &self,
        term: &str,
        crn: &str,
    ) -> Result<String, String> {
        self.post_section_form("getSectionPrerequisites", term, crn)
    }

    pub fn fetch_corequisites_html(&self, term: &str, crn: &str) -> Result<String, String> {
        self.post_section_form("getCorequisites", term, crn)
    }

    pub fn fetch_restrictions_html(&self, term: &str, crn: &str) -> Result<String, String> {
        self.post_section_form("getRestrictions", term, crn)
    }

    pub fn fetch_cross_listed_sections_html(
        &self,
        term: &str,
        crn: &str,
    ) -> Result<String, String> {
        self.post_section_form("getXlstSections", term, crn)
    }

    pub fn fetch_instructors(&self, term: &str, crn: &str) -> Result<Vec<Instructor>, String> {
        let url = format!(
            "{}searchResults/getFacultyMeetingTimes?term={term}&courseReferenceNumber={crn}",
            self.ssb()
        );
        self.get(&url).and_then(|body| faculty_instructors(&body))
    }

    fn post_course_form(
        &self,
        path: &str,
        term: &str,
        course: &CourseKey,
    ) -> Result<String, String> {
        let url = format!("{}{path}", self.ssb());
        let body = format!(
            "term={term}&subjectCode={}&courseNumber={}",
            course.subject, course.number
        );
        perform(|| {
            classify(
                ureq::post(&url)
                    .set(
                        "Content-Type",
                        "application/x-www-form-urlencoded; charset=UTF-8",
                    )
                    .timeout(std::time::Duration::from_secs(25))
                    .send_string(&body),
            )
        })?
        .into_string()
        .map_err(|e| e.to_string())
    }

    /// Pause between section restriction requests so a course open does not hammer Banner.
    pub const RESTRICTION_GAP: std::time::Duration = std::time::Duration::from_millis(300);

    fn post_section_form(&self, endpoint: &str, term: &str, crn: &str) -> Result<String, String> {
        let url = format!("{}searchResults/{endpoint}", self.ssb());
        let body = format!("term={term}&courseReferenceNumber={crn}");
        perform(|| {
            classify(
                ureq::post(&url)
                    .set(
                        "Content-Type",
                        "application/x-www-form-urlencoded; charset=UTF-8",
                    )
                    .timeout(std::time::Duration::from_secs(25))
                    .send_string(&body),
            )
        })?
        .into_string()
        .map_err(|e| e.to_string())
    }

    /// Catalog entry for one course in one term. `None` when that catalog does not list it.
    pub fn fetch_course_info(
        &self,
        course: &CourseKey,
        term: &str,
        subjects: &SubjectLookup,
        now: f64,
    ) -> Result<Option<crate::course_info::CourseInfo>, String> {
        if !self.catalog_lists_course(course, term)? {
            return Ok(None);
        }
        let details_html =
            self.post_course_form("courseSearchResults/getCourseCatalogDetails", term, course)?;
        let prereq_html =
            self.post_course_form("courseSearchResults/getPrerequisites", term, course)?;
        let coreq_html =
            self.post_course_form("courseSearchResults/getCorequisites", term, course)?;
        let exclusion_html = self.post_course_form(
            "courseSearchResults/getCourseMutuallyExclusions",
            term,
            course,
        )?;
        let description_html = self.fetch_course_description_html(term, course).ok();
        let details = catalog_details(&details_html);
        let desc = description_html
            .as_deref()
            .map(description)
            .filter(|d| !d.is_empty());
        Ok(Some(crate::course_info::CourseInfo {
            key: course.clone(),
            title: details.title.unwrap_or_default(),
            description: desc,
            min_credits: details.min_credits,
            max_credits: details.max_credits,
            prerequisites: prerequisites(&prereq_html, subjects),
            corequisites: corequisites(&coreq_html, subjects),
            equivalents: mutual_exclusions(&exclusion_html, subjects),
            source_term: term.to_string(),
            is_retired: None,
            fetched_at: now,
        }))
    }
}

impl crate::offering::OfferingSource for BannerClient {
    fn terms(&self) -> Result<Vec<Term>, String> {
        self.fetch_terms(120)
    }
    fn offered_courses(
        &self,
        term: &str,
        courses: &[CourseKey],
    ) -> Result<HashSet<CourseKey>, String> {
        self.fetch_offered(term, courses)
    }
}

impl crate::course_info::CourseInfoSource for BannerClient {
    fn terms(&self) -> Result<Vec<Term>, String> {
        self.fetch_terms(120)
    }
    fn catalog_entry(
        &self,
        key: &CourseKey,
        term: &str,
    ) -> Result<Option<crate::course_info::CourseInfo>, String> {
        let subjects = self.fetch_subjects(term).unwrap_or_default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        self.fetch_course_info(key, term, &SubjectLookup::new(&subjects), now)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSeat {
    pub crn: String,
    pub seats_remaining: i32,
    pub waitlist_remaining: i32,
    pub seat_capacity: i32,
}

pub fn parse_live_seats(json: &str, subject: &str, number: &str) -> Result<Vec<LiveSeat>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut seats = Vec::new();
    let Some(rows) = value.get("data").and_then(|d| d.as_array()) else {
        if value.get("success").and_then(|v| v.as_bool()) == Some(true) {
            return Ok(Vec::new());
        }
        return Err("Banner did not return a section list".into());
    };
    for item in rows {
        let crn = item
            .get("courseReferenceNumber")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if crn.is_empty() {
            continue;
        }
        let same_subject = item.get("subject").and_then(|v| v.as_str()) == Some(subject);
        let same_number = item.get("courseNumber").and_then(|v| v.as_str()) == Some(number);
        if !same_subject || !same_number {
            continue;
        }
        let seats_remaining = item
            .get("crossListAvailable")
            .and_then(|v| v.as_i64())
            .or_else(|| item.get("seatsAvailable").and_then(|v| v.as_i64()))
            .unwrap_or(0) as i32;
        let waitlist_remaining = item
            .get("waitAvailable")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let seat_capacity = item
            .get("maximumEnrollment")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        seats.push(LiveSeat {
            crn,
            seats_remaining,
            waitlist_remaining,
            seat_capacity,
        });
    }
    Ok(seats)
}

/// Overlay a live seat response onto the snapshot sections a watchlist cares about.
/// A watched CRN that Banner did not return is omitted, so the comparison can mark it missing.
pub fn sections_with_live_seats(
    snapshot: &[CourseSection],
    watched: &[(String, CourseKey)],
    seats: &[LiveSeat],
) -> Vec<CourseSection> {
    let by_crn: HashMap<&str, &LiveSeat> =
        seats.iter().map(|seat| (seat.crn.as_str(), seat)).collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (crn, key) in watched {
        let Some(seat) = by_crn.get(crn.as_str()) else {
            continue;
        };
        let mut section = snapshot
            .iter()
            .find(|section| &section.crn == crn)
            .cloned()
            .unwrap_or_else(|| section_from_seat(crn, key, seat));
        section.seats_remaining = seat.seats_remaining;
        section.waitlist_remaining = seat.waitlist_remaining;
        section.seat_capacity = seat.seat_capacity;
        if seen.insert(section.crn.clone()) {
            out.push(section);
        }
    }
    out
}

fn section_from_seat(crn: &str, key: &CourseKey, seat: &LiveSeat) -> CourseSection {
    CourseSection {
        crn: crn.to_string(),
        subject: key.subject.clone(),
        number: key.number.clone(),
        title: String::new(),
        section_number: String::new(),
        part_of_term: String::new(),
        seat_capacity: seat.seat_capacity,
        seats_remaining: seat.seats_remaining,
        enrollment: 0,
        waitlist_capacity: 0,
        waitlist_count: 0,
        waitlist_remaining: seat.waitlist_remaining,
        class_type: String::new(),
        honors: false,
        campus: String::new(),
        instruction_method: None,
        meetings: Vec::new(),
        instructors: None,
        cross_list_crns: Vec::new(),
        attributes: Vec::new(),
        description: None,
        prereqs: None,
        coreqs: None,
        credit_hours: None,
        credit_hour_low: None,
        credit_hour_high: None,
        restrictions: None,
    }
}

/// `09/09/2026` becomes `2026-09-09`. An ISO date is kept. Empty stays absent.
fn banner_slash_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.len() == 3 && parts[0].len() <= 2 && parts[1].len() <= 2 && parts[2].len() == 4 {
        return Some(format!(
            "{:0>4}-{:0>2}-{:0>2}",
            parts[2], parts[0], parts[1]
        ));
    }
    Some(raw.to_string())
}

/// Meeting from Banner's compact day string and HHMM times.
pub fn meeting_from_banner(
    days: &str,
    start_hhmm: i32,
    end_hhmm: i32,
    building: &str,
    room: &str,
    start_date: &str,
    end_date: &str,
) -> Meeting {
    let mut set = DaySet(0);
    for ch in days.chars() {
        let bit = match ch {
            'U' => 0,
            'M' => 1,
            'T' => 2,
            'W' => 3,
            'R' => 4,
            'F' => 5,
            'S' => 6,
            _ => continue,
        };
        set.0 |= 1 << bit;
    }
    Meeting {
        days: set,
        start: Some(minutes_from_hhmm(start_hhmm)),
        end: Some(minutes_from_hhmm(end_hhmm)),
        building_code: if building.is_empty() {
            None
        } else {
            Some(building.into())
        },
        building: None,
        room: if room.is_empty() {
            None
        } else {
            Some(room.into())
        },
        campus_code: None,
        kind: "Class".into(),
        start_date: banner_slash_date(start_date),
        end_date: banner_slash_date(end_date),
    }
}

#[cfg(test)]
mod tests {
    use super::{course_sections, faculty_instructors, parse_live_seats, BannerClient, LiveSeat};
    use crate::{catalog, model::*};
    use std::collections::HashMap;

    #[test]
    fn course_refresh_reads_banner_endpoints_and_preserves_cached_optional_details() {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = BannerClient {
            base: format!("http://{}", listener.local_addr().unwrap()),
        };
        let server = std::thread::spawn(move || {
            let responses = [
                (
                    "POST",
                    "/term/search",
                    "200 OK",
                    "Set-Cookie: JSESSIONID=fixture\r\n",
                    "{}",
                ),
                (
                    "GET",
                    "/searchResults/searchResults",
                    "200 OK",
                    "",
                    r#"{"success":true,"data":[{"courseReferenceNumber":"12345","subject":"CS","courseNumber":"3000","enrollment":23,"seatsAvailable":7}]}"#,
                ),
                (
                    "GET",
                    "/searchResults/getFacultyMeetingTimes",
                    "200 OK",
                    "",
                    r#"{"fmt":[{"faculty":[{"displayName":"Example, Alex","emailAddress":"alex@neu.edu"}]}]}"#,
                ),
                (
                    "POST",
                    "/searchResults/getRestrictions",
                    "404 Not Found",
                    "",
                    "",
                ),
                (
                    "POST",
                    "/term/search",
                    "200 OK",
                    "Set-Cookie: JSESSIONID=fixture\r\n",
                    "{}",
                ),
                (
                    "GET",
                    "/searchResults/searchResults",
                    "200 OK",
                    "",
                    r#"{"success":true,"data":null}"#,
                ),
                (
                    "POST",
                    "/term/search",
                    "200 OK",
                    "Set-Cookie: JSESSIONID=fixture\r\n",
                    "{}",
                ),
                (
                    "GET",
                    "/searchResults/searchResults",
                    "200 OK",
                    "",
                    r#"{"success":true,"data":[{"term":"202710","courseReferenceNumber":"99999","subject":"CS","courseNumber":"3000"}]}"#,
                ),
            ];
            for (method, path, status, headers, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert!(request.starts_with(&format!("{method} /StudentRegistrationSsb/ssb{path}")));
                let mut request_headers = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    request_headers.push_str(&line);
                }
                let request_headers = request_headers.to_ascii_lowercase();
                if let Some(size) = request_headers.lines().find_map(|line| {
                    line.strip_prefix("content-length:")?
                        .trim()
                        .parse::<usize>()
                        .ok()
                }) {
                    reader.read_exact(&mut vec![0; size]).unwrap();
                }
                assert_eq!(
                    request_headers.contains("cookie: jsessionid=fixture"),
                    path.ends_with("/searchResults")
                );
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\n{headers}Connection: close\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                ).unwrap();
            }
        });
        let mut catalog: TermCatalog = serde_json::from_value(serde_json::json!({
            "version": 1,
            "term": {"code": "202730", "bannerDescription": "Spring"},
            "courses": [{"subject": "CS", "number": "3000", "title": "Example"}],
            "sections": [],
            "subjects": [], "attributes": [], "campuses": []
        }))
        .unwrap();
        let key = CourseKey::parse("CS 3000").unwrap();
        let mut section = section_shell("12345", 4, 30);
        section.description = Some("Cached description".into());
        section.restrictions = Some(Vec::new());
        catalog.sections.push(section);
        assert!(client.refresh_course(&mut catalog, &key, 1.0).unwrap());
        let section = &catalog.sections[0];
        assert_eq!(section.enrollment, 23);
        assert_eq!(section.seats_remaining, 7);
        assert_eq!(section.description.as_deref(), Some("Cached description"));
        assert_eq!(section.restrictions, Some(Vec::new()));
        assert_eq!(
            section.instructors.as_ref().unwrap()[0].email.as_deref(),
            Some("alex@neu.edu")
        );
        assert!(!client.refresh_course(&mut catalog, &key, 2.0).unwrap());
        assert!(client.refresh_course(&mut catalog, &key, 3.0).is_err());
        assert_eq!(catalog.sections_updated_at, 1.0);
        assert_eq!(catalog.sections[0].enrollment, 23);
        server.join().unwrap();
    }

    #[test]
    fn live_course_search_keeps_only_exact_matches_and_full_counts() {
        let key = CourseKey::parse("CS 3000").unwrap();
        let page = serde_json::json!({
            "success": true,
            "data": [
                {"courseReferenceNumber":"12345","subject":"CS","courseNumber":"3000",
                 "crossListCapacity":50.0,"crossListCount":43,"crossListAvailable":7,
                 "maximumEnrollment":25,"enrollment":21,"seatsAvailable":4,
                 "waitCapacity":12,"waitCount":2,"waitAvailable":10},
                {"courseReferenceNumber":"23456","subject":"CS","courseNumber":"3000A"},
                {"courseReferenceNumber":"34567","subject":"MATH","courseNumber":"3000"}
            ]
        });
        let rows = course_sections(&page, &key).unwrap();
        let sections = catalog::make_sections(&rows, &HashMap::new());
        assert_eq!(sections.len(), 1);
        let section = &sections[0];
        assert_eq!(
            (
                section.seat_capacity,
                section.enrollment,
                section.seats_remaining
            ),
            (50, 43, 7)
        );
        assert_eq!(
            (
                section.waitlist_capacity,
                section.waitlist_count,
                section.waitlist_remaining
            ),
            (12, 2, 10)
        );
        assert!(course_sections(&serde_json::json!({"success": false, "data": []}), &key).is_err());
        assert!(
            course_sections(&serde_json::json!({"success": true, "data": null}), &key)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn faculty_details_deduplicate_sort_and_keep_only_institutional_email() {
        let faculty = faculty_instructors(r#"{"fmt":[{"faculty":[
            {"displayName":"Example, Assistant","emailAddress":"assistant@example.org"},
            {"displayName":"Example, Primary","primaryIndicator":true,"emailAddress":"primary@NEU.EDU"},
            {"displayName":"Example, Primary","emailAddress":"duplicate@northeastern.edu"},
            {"displayName":"Example, Third","emailAddress":"third@northeastern.edu"},
            {"displayName":"Example, Fourth","emailAddress":"fourth@northeastern.edu.example.org"}
        ]}]}"#).unwrap();
        assert_eq!(faculty.len(), 4);
        assert!(faculty[0].primary);
        assert_eq!(faculty[0].email.as_deref(), Some("primary@NEU.EDU"));
        assert_eq!(faculty[1].email, None);
        assert_eq!(faculty[2].email.as_deref(), Some("third@northeastern.edu"));
        assert_eq!(faculty[3].email, None);
        assert!(faculty_instructors("<html>Unavailable</html>").is_err());
        assert!(faculty_instructors(r#"{"fmt":[]}"#).unwrap().is_empty());
    }

    #[test]
    fn live_seat_json_keeps_exact_course() {
        let json = r#"{"success":true,"totalCount":2,"data":[
            {"courseReferenceNumber":"12345","subject":"CS","courseNumber":"3000","seatsAvailable":3,"waitAvailable":1,"maximumEnrollment":40},
            {"courseReferenceNumber":"99999","subject":"CS","courseNumber":"3001","seatsAvailable":8,"waitAvailable":0,"maximumEnrollment":30}
        ]}"#;
        let seats = parse_live_seats(json, "CS", "3000").unwrap();
        assert_eq!(seats.len(), 1);
        assert_eq!(seats[0].crn, "12345");
        assert_eq!(seats[0].seats_remaining, 3);
        assert_eq!(seats[0].waitlist_remaining, 1);
        assert_eq!(seats[0].seat_capacity, 40);
    }

    #[test]
    #[ignore = "contacts nubanner.neu.edu"]
    fn live_cs3000_when_banner_is_up() {
        let seats = super::BannerClient::default()
            .live_seats("202710", "CS", "3000")
            .expect("banner");
        assert!(!seats.is_empty(), "expected Fall 2026 CS 3000 sections");
    }

    #[test]
    fn null_data_is_an_empty_list() {
        let seats = parse_live_seats(
            r#"{"success":true,"totalCount":0,"data":null}"#,
            "CS",
            "3000",
        )
        .unwrap();
        assert!(seats.is_empty());
    }

    #[test]
    fn backoff_doubles_and_stops_after_four_attempts() {
        let (sleep, next) = super::backoff(1, 1.0, 0.25).unwrap();
        assert!((sleep - 1.25).abs() < 1e-9);
        assert!((next - 2.0).abs() < 1e-9);
        let (sleep, next) = super::backoff(2, 2.0, 0.0).unwrap();
        assert!((sleep - 2.0).abs() < 1e-9);
        assert!((next - 4.0).abs() < 1e-9);
        let (sleep, next) = super::backoff(3, 16.0, 0.5).unwrap();
        assert!((sleep - 16.5).abs() < 1e-9);
        assert!((next - 20.0).abs() < 1e-9);
        assert!(super::backoff(4, 20.0, 0.1).is_none());
        let (sleep, _) = super::backoff(1, 1.0, 9.0).unwrap();
        assert!((sleep - 1.5).abs() < 1e-9);
    }

    #[test]
    fn session_pages_assign_one_stream_per_cookie() {
        let pages = super::session_pages(10, 4);
        assert_eq!(
            pages,
            vec![vec![0, 4, 8], vec![1, 5, 9], vec![2, 6], vec![3, 7]]
        );
        assert_eq!(super::session_pages(2, 4).len(), 2);
        assert!(super::session_pages(0, 4).is_empty());
        assert_eq!(super::session_pages(3, 0), vec![vec![0, 1, 2]]);
    }

    #[test]
    fn only_429_and_5xx_are_retried() {
        assert!(super::should_retry_status(429));
        assert!(super::should_retry_status(500));
        assert!(super::should_retry_status(503));
        assert!(!super::should_retry_status(404));
        assert!(!super::should_retry_status(400));
        assert!(!super::should_retry_status(200));
    }

    #[test]
    fn catalog_urls_keep_a_single_slash() {
        assert_eq!(
            super::join_url("https://catalog.northeastern.edu", "undergraduate/computer-information-science/computer-science/bscs/"),
            "https://catalog.northeastern.edu/undergraduate/computer-information-science/computer-science/bscs/"
        );
        assert_eq!(
            super::join_url("https://catalog.northeastern.edu/", "/undergraduate/foo/"),
            "https://catalog.northeastern.edu/undergraduate/foo/"
        );
        assert_eq!(
            super::join_url("https://catalog.northeastern.edu", "https://example.edu/x"),
            "https://example.edu/x"
        );
        assert!(!super::DEFAULT_CATALOG_PATH.contains("computer-science-bs"));
        assert!(super::DEFAULT_CATALOG_PATH.ends_with("bscs/"));
    }

    #[test]
    fn live_counts_replace_the_snapshot_and_drop_missing_crns() {
        let snapshot = vec![section_shell("12726", 0, 40), section_shell("99999", 2, 30)];
        let watched = vec![
            (
                "12726".into(),
                CourseKey {
                    subject: "CS".into(),
                    number: "3000".into(),
                },
            ),
            (
                "99999".into(),
                CourseKey {
                    subject: "CS".into(),
                    number: "3000".into(),
                },
            ),
        ];
        let seats = vec![LiveSeat {
            crn: "12726".into(),
            seats_remaining: 4,
            waitlist_remaining: 1,
            seat_capacity: 42,
        }];
        let updated = super::sections_with_live_seats(&snapshot, &watched, &seats);
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].crn, "12726");
        assert_eq!(updated[0].seats_remaining, 4);
        assert_eq!(updated[0].waitlist_remaining, 1);
        assert_eq!(updated[0].seat_capacity, 42);
        assert_eq!(updated[0].subject, "CS");
    }

    #[test]
    #[ignore = "contacts catalog.northeastern.edu"]
    fn live_bscs_page_parses_requirements() {
        let client = super::BannerClient {
            base: "https://catalog.northeastern.edu".into(),
        };
        let html = client.get(super::DEFAULT_CATALOG_PATH).expect("catalog");
        let program = CatalogProgram {
            title: "Computer Science, BSCS (Boston)".into(),
            path: super::DEFAULT_CATALOG_PATH.into(),
        };
        let requirements = crate::program_parser::requirements_from_html(
            &html,
            CatalogYear::containing(2026, 10),
            program,
        )
        .expect("requirements");
        assert!(requirements
            .sections
            .iter()
            .any(|section| !section.groups.is_empty()));
    }

    fn section_shell(crn: &str, seats: i32, capacity: i32) -> CourseSection {
        CourseSection {
            crn: crn.into(),
            subject: "CS".into(),
            number: "3000".into(),
            title: String::new(),
            section_number: "01".into(),
            part_of_term: String::new(),
            seat_capacity: capacity,
            seats_remaining: seats,
            enrollment: 0,
            waitlist_capacity: 0,
            waitlist_count: 0,
            waitlist_remaining: 0,
            class_type: "Lecture".into(),
            honors: false,
            campus: "Boston".into(),
            instruction_method: None,
            meetings: Vec::new(),
            instructors: None,
            cross_list_crns: Vec::new(),
            attributes: Vec::new(),
            description: None,
            prereqs: None,
            coreqs: None,
            credit_hours: None,
            credit_hour_low: None,
            credit_hour_high: None,
            restrictions: None,
        }
    }

    #[test]
    fn parses_course_attributes_and_fees() {
        let attr_html = "<table><tr><td>Attribute</td><td>Description</td></tr><tr><td>NDDI</td><td>Differences and Diversity</td></tr><tr><td>WI</td><td>Writing Intensive</td></tr></table>";
        let attrs = super::course_attributes(attr_html);
        assert_eq!(
            attrs,
            vec!["Differences and Diversity", "Writing Intensive"]
        );

        let no_attr_html = "<p>No Attribute information available.</p>";
        assert!(super::course_attributes(no_attr_html).is_empty());

        let fee_html = "<table><tr><td>Fee Type</td><td>Amount</td></tr><tr><td>Lab Fee</td><td>$75.00</td></tr></table>";
        let fees = super::course_fees(fee_html);
        assert_eq!(fees.len(), 1);
        assert_eq!(fees[0].description, "Lab Fee");
        assert_eq!(fees[0].amount.as_deref(), Some("$75.00"));

        let no_fee_html = "<div>No fee information available for this course.</div>";
        assert!(super::course_fees(no_fee_html).is_empty());
    }

    #[test]
    fn description_strips_html_and_empty_notices() {
        assert_eq!(
            super::description("<p>Covers <b>recursion</b> and data structures.</p>"),
            "Covers recursion and data structures."
        );
        assert_eq!(
            super::description("<p>No course description available.</p>"),
            ""
        );
        assert_eq!(super::description("   No description available   "), "");
    }

    #[test]
    fn restrictions_parses_headings_and_empty_notices() {
        let html = "<p>Must be enrolled in one of the following Levels:</p><p>Graduate</p><p>Cannot be enrolled in one of the following Colleges:</p><p>College of Professional Studies</p>";
        let r = super::enrollment_restrictions(html).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].kind, RestrictionKind::Must);
        assert_eq!(r[0].category, "Levels");
        assert_eq!(r[0].values, vec!["Graduate"]);
        assert_eq!(r[1].kind, RestrictionKind::Cannot);
        assert_eq!(r[1].category, "Colleges");
        assert_eq!(r[1].values, vec!["College of Professional Studies"]);

        let no_restrictions = "<div>No restriction information available.</div>";
        assert_eq!(
            super::enrollment_restrictions(no_restrictions),
            Some(vec![])
        );

        let not_restrictions = "<div>Welcome to Northeastern</div>";
        assert_eq!(super::enrollment_restrictions(not_restrictions), None);
    }

    #[test]
    fn catalog_page_url_builds_expected_endpoint() {
        let key = CourseKey {
            subject: "CS".into(),
            number: "2500".into(),
        };
        let url = BannerClient::catalog_page_url("202710", &key);
        assert_eq!(
            url,
            "https://bnrordsp.neu.edu/ssb-prod/bwckctlg.p_disp_course_detail?cat_term_in=202710&subj_code_in=CS&crse_numb_in=2500"
        );
    }
}
