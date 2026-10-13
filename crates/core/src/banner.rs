//! Banner HTML parsers and a small client for nubanner.neu.edu.
//!
//! Portions derived from SearchNEU (AGPL-3.0), packages/scraper.

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
    let mut nodes = vec![Node { is_and: false, items: vec![] }];
    let mut stack = vec![0usize];
    for cells in rows {
        if cells.len() < 6 {
            continue;
        }
        let cell = |i: usize| cells.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
        let and_or = cell(0).to_ascii_lowercase();
        let current = *stack.last().unwrap();
        if and_or == "and" {
            nodes[current].is_and = true;
        } else if and_or == "or" {
            nodes[current].is_and = false;
        }
        if !cell(1).is_empty() {
            nodes.push(Node { is_and: false, items: vec![] });
            let child = nodes.len() - 1;
            nodes[current].items.push(Item::Node(child));
            stack.push(child);
        }
        let current = *stack.last().unwrap();
        if !cell(2).is_empty() {
            let score = cell(3).parse().ok();
            nodes[current].items.push(Item::Leaf(Requisite::Test { name: cell(2), score }));
        }
        if !cell(4).is_empty() && !cell(5).is_empty() {
            let key = CourseKey { subject: subjects.code(&cell(4)), number: cell(5) };
            nodes[current].items.push(Item::Leaf(Requisite::Course(key)));
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
            Some(Requisite::Course(CourseKey { subject: subjects.code(subject.trim()), number: number.trim().to_string() }))
        })
        .collect();
    match items.len() {
        0 => None,
        1 => Some(items.into_iter().next().unwrap()),
        _ => Some(Requisite::And(items)),
    }
}

pub fn description(html: &str) -> String {
    let text = plain_text(html);
    let lowered = text.to_ascii_lowercase();
    if lowered.starts_with("no course description") || lowered.contains("no information available") {
        String::new()
    } else {
        text
    }
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
    let lines: Vec<String> = plain_text(html).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    fn value(lines: &[String], label: &str) -> Option<String> {
        let i = lines.iter().position(|l| l.starts_with(label))?;
        let inline = lines[i][label.len()..].trim().to_string();
        if !inline.is_empty() {
            return Some(inline);
        }
        let next = lines.get(i + 1)?;
        if next.ends_with(':') { None } else { Some(next.clone()) }
    }
    fn strip_code(s: Option<String>) -> Option<String> {
        let s = s?;
        let mut parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() > 1 {
            if let Some(last) = parts.last() {
                if last.len() <= 4 && last.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
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
    let nums: Vec<f64> = Regex::new(r"\d+(?:\.\d+)?").unwrap().find_iter(text).filter_map(|m| m.as_str().parse().ok()).collect();
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
                    push_unique(&mut keys, CourseKey { subject: subjects.code(&cells[i]), number: cells[i + 1].clone() });
                    break;
                }
            }
        }
    }
    if keys.is_empty() {
        for cap in Regex::new(r"\b([A-Z]{2,6})\s?(\d{3,4}[A-Z]?)\b").unwrap().captures_iter(&text) {
            push_unique(&mut keys, CourseKey { subject: cap[1].to_string(), number: cap[2].to_string() });
        }
    }
    keys
}

fn coerce_section_ints(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else { return };
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
    (3..=5).contains(&text.len()) && text.chars().next().is_some_and(|c| c.is_ascii_digit()) && text.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn catalog_lists(items: &[(String, String)], course: &CourseKey) -> bool {
    let wanted = format!("{}{}", course.subject, course.number).chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_uppercase();
    items.iter().any(|(code, _)| code.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_uppercase() == wanted)
}

#[derive(Clone, Debug)]
pub struct BannerClient {
    pub base: String,
}

impl Default for BannerClient {
    fn default() -> Self {
        Self { base: "https://nubanner.neu.edu".into() }
    }
}

impl BannerClient {
    pub fn get(&self, path: &str) -> Result<String, String> {
        let url = if path.starts_with("http") { path.to_string() } else { format!("{}{}", self.base, path) };
        ureq::get(&url)
            .timeout(std::time::Duration::from_secs(20))
            .call()
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())
    }

    /// Open a Banner search session and read current seats for one course.
    /// Falls back to the bundled snapshot when Banner is unreachable.
    pub fn live_seats(&self, term: &str, subject: &str, number: &str) -> Result<Vec<LiveSeat>, String> {
        let base = "https://nubanner.neu.edu/StudentRegistrationSsb/ssb/";
        let opened = ureq::post(&format!("{base}term/search?mode=search"))
            .set("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
            .timeout(std::time::Duration::from_secs(20))
            .send_string(&format!("term={term}&studyPath=&studyPathText=&startDatepicker=&endDatepicker="))
            .map_err(|e| e.to_string())?;
        let cookie = opened
            .all("set-cookie")
            .into_iter()
            .filter_map(|header| header.split(';').next().map(str::trim))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        let _ = opened.into_string();
        if cookie.is_empty() {
            return Err("Banner did not start a search session".into());
        }
        let url = format!(
            "{base}searchResults/searchResults?txt_term={term}&txt_subject={subject}&txt_courseNumber={number}&pageOffset=0&pageMaxSize=500&sortColumn=subjectDescription&sortDirection=asc"
        );
        let body = ureq::get(&url)
            .set("Cookie", &cookie)
            .timeout(std::time::Duration::from_secs(20))
            .call()
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())?;
        parse_live_seats(&body, subject, number)
    }

    fn ssb(&self) -> String {
        if self.base.contains("StudentRegistrationSsb") {
            let base = self.base.trim_end_matches('/');
            format!("{base}/")
        } else {
            format!("{}/StudentRegistrationSsb/ssb/", self.base.trim_end_matches('/'))
        }
    }

    pub fn fetch_terms(&self, max: i32) -> Result<Vec<Term>, String> {
        let url = format!("{}classSearch/getTerms?offset=1&max={max}&searchTerm=", self.ssb());
        let body = self.get(&url)?;
        let rows: Vec<serde_json::Value> = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let code = row.get("code").and_then(|v| v.as_str())?.to_string();
                let description = crate::html::decode_entities(row.get("description").and_then(|v| v.as_str()).unwrap_or(""));
                Some(Term { code, banner_description: description })
            })
            .collect())
    }

    pub fn open_session(&self, term: &str) -> Result<String, String> {
        let opened = ureq::post(&format!("{}term/search?mode=search", self.ssb()))
            .set("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
            .timeout(std::time::Duration::from_secs(30))
            .send_string(&format!("term={term}&studyPath=&studyPathText=&startDatepicker=&endDatepicker="))
            .map_err(|e| e.to_string())?;
        let cookie = opened
            .all("set-cookie")
            .into_iter()
            .filter_map(|header| header.split(';').next().map(str::trim))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        let _ = opened.into_string();
        if cookie.is_empty() { Err("Banner did not start a search session".into()) } else { Ok(cookie) }
    }

    pub fn search_page(&self, cookie: &str, term: &str, subject: Option<&str>, number: Option<&str>, offset: i32, page_size: i32) -> Result<serde_json::Value, String> {
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
        let body = ureq::get(&url).set("Cookie", cookie).timeout(std::time::Duration::from_secs(40)).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
        serde_json::from_str(&body).map_err(|e| e.to_string())
    }

    pub fn fetch_all_sections(&self, term: &str) -> Result<Vec<crate::catalog::BannerSection>, String> {
        let cookie = self.open_session(term)?;
        let probe = self.search_page(&cookie, term, None, None, 0, 1)?;
        let total = probe.get("totalCount").and_then(|v| v.as_i64()).unwrap_or(0);
        if total <= 0 {
            return Ok(vec![]);
        }
        let page_size = 500i64;
        let pages = (total + page_size - 1) / page_size;
        let mut all = Vec::new();
        let mut seen = HashSet::new();
        for page in 0..pages {
            let value = self.search_page(&cookie, term, None, None, (page * page_size) as i32, page_size as i32)?;
            let rows = value.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default();
            for mut row in rows {
                coerce_section_ints(&mut row);
                let section: crate::catalog::BannerSection = serde_json::from_value(row).map_err(|e| e.to_string())?;
                if seen.insert(section.course_reference_number.clone()) {
                    all.push(section);
                }
            }
        }
        Ok(all)
    }

    pub fn fetch_offered(&self, term: &str, courses: &[CourseKey]) -> Result<HashSet<CourseKey>, String> {
        if courses.is_empty() {
            return Ok(HashSet::new());
        }
        let cookie = self.open_session(term)?;
        let mut offered = HashSet::new();
        for (index, key) in courses.iter().enumerate() {
            if index > 0 {
                let _ = ureq::post(&format!("{}classSearch/resetDataForm", self.ssb())).set("Cookie", &cookie).call().map(|r| r.into_string());
            }
            let page = self.search_page(&cookie, term, Some(&key.subject), Some(&key.number), 0, 20)?;
            let rows = page.get("data").and_then(|d| d.as_array());
            let hit = rows.is_some_and(|rows| {
                rows.iter().any(|row| row.get("subject").and_then(|v| v.as_str()) == Some(key.subject.as_str()) && row.get("courseNumber").and_then(|v| v.as_str()) == Some(key.number.as_str()))
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
        let rows: Vec<serde_json::Value> = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let items: Vec<(String, String)> = rows
            .into_iter()
            .filter_map(|row| Some((row.get("code")?.as_str()?.to_string(), row.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string())))
            .collect();
        Ok(catalog_lists(&items, course))
    }

    pub fn fetch_subjects(&self, term: &str) -> Result<Vec<Subject>, String> {
        self.code_list(&format!("{}classSearch/get_subject?searchTerm=&term={term}&offset=1&max=900", self.ssb()))
            .map(|rows| rows.into_iter().map(|(code, name)| Subject { code, name }).collect())
    }

    pub fn fetch_campuses(&self, term: &str) -> Result<Vec<Campus>, String> {
        self.code_list(&format!("{}classSearch/get_campus?searchTerm=&term={term}&offset=1&max=200", self.ssb()))
            .map(|rows| rows.into_iter().map(|(code, name)| Campus { code, name }).collect())
    }

    pub fn fetch_attributes(&self, term: &str) -> Result<Vec<AttributeInfo>, String> {
        self.code_list(&format!("{}classSearch/get_attribute?searchTerm=&term={term}&offset=1&max=500", self.ssb()))
            .map(|rows| rows.into_iter().map(|(code, name)| AttributeInfo { code, name }).collect())
    }

    fn code_list(&self, url: &str) -> Result<Vec<(String, String)>, String> {
        let body = self.get(url)?;
        let rows: Vec<serde_json::Value> = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some((
                    row.get("code")?.as_str()?.to_string(),
                    crate::html::decode_entities(row.get("description").and_then(|v| v.as_str()).unwrap_or("")).trim().to_string(),
                ))
            })
            .collect())
    }

    fn post_course_form(&self, path: &str, term: &str, course: &CourseKey) -> Result<String, String> {
        let url = format!("{}{path}", self.ssb());
        let body = format!("term={term}&subjectCode={}&courseNumber={}", course.subject, course.number);
        ureq::post(&url)
            .set("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
            .timeout(std::time::Duration::from_secs(25))
            .send_string(&body)
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())
    }

    /// Catalog entry for one course in one term. `None` when that catalog does not list it.
    pub fn fetch_course_info(&self, course: &CourseKey, term: &str, subjects: &SubjectLookup, now: f64) -> Result<Option<crate::course_info::CourseInfo>, String> {
        if !self.catalog_lists_course(course, term)? {
            return Ok(None);
        }
        let details_html = self.post_course_form("courseSearchResults/getCourseCatalogDetails", term, course)?;
        let prereq_html = self.post_course_form("courseSearchResults/getPrerequisites", term, course)?;
        let coreq_html = self.post_course_form("courseSearchResults/getCorequisites", term, course)?;
        let exclusion_html = self.post_course_form("courseSearchResults/getCourseMutuallyExclusions", term, course)?;
        let details = catalog_details(&details_html);
        Ok(Some(crate::course_info::CourseInfo {
            key: course.clone(),
            title: details.title.unwrap_or_default(),
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
    fn offered_courses(&self, term: &str, courses: &[CourseKey]) -> Result<HashSet<CourseKey>, String> {
        self.fetch_offered(term, courses)
    }
}

impl crate::course_info::CourseInfoSource for BannerClient {
    fn terms(&self) -> Result<Vec<Term>, String> {
        self.fetch_terms(120)
    }
    fn catalog_entry(&self, key: &CourseKey, term: &str) -> Result<Option<crate::course_info::CourseInfo>, String> {
        let subjects = self.fetch_subjects(term).unwrap_or_default();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
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
        let crn = item.get("courseReferenceNumber").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
        let waitlist_remaining = item.get("waitAvailable").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let seat_capacity = item.get("maximumEnrollment").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        seats.push(LiveSeat { crn, seats_remaining, waitlist_remaining, seat_capacity });
    }
    Ok(seats)
}

/// `09/09/2026` becomes `2026-09-09`. An ISO date is kept. Empty stays absent.
fn banner_slash_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.len() == 3 && parts[0].len() <= 2 && parts[1].len() <= 2 && parts[2].len() == 4 {
        return Some(format!("{:0>4}-{:0>2}-{:0>2}", parts[2], parts[0], parts[1]));
    }
    Some(raw.to_string())
}

/// Meeting from Banner's compact day string and HHMM times.
pub fn meeting_from_banner(days: &str, start_hhmm: i32, end_hhmm: i32, building: &str, room: &str, start_date: &str, end_date: &str) -> Meeting {
    let mut set = DaySet(0);
    for ch in days.chars() {
        let bit = match ch {
            'U' => 0, 'M' => 1, 'T' => 2, 'W' => 3, 'R' => 4, 'F' => 5, 'S' => 6, _ => continue,
        };
        set.0 |= 1 << bit;
    }
    Meeting {
        days: set,
        start: Some(minutes_from_hhmm(start_hhmm)),
        end: Some(minutes_from_hhmm(end_hhmm)),
        building_code: if building.is_empty() { None } else { Some(building.into()) },
        building: None,
        room: if room.is_empty() { None } else { Some(room.into()) },
        campus_code: None,
        kind: "Class".into(),
        start_date: banner_slash_date(start_date),
        end_date: banner_slash_date(end_date),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_live_seats;

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
        let seats = super::BannerClient::default().live_seats("202710", "CS", "3000").expect("banner");
        assert!(!seats.is_empty(), "expected Fall 2026 CS 3000 sections");
    }

    #[test]
    fn null_data_is_an_empty_list() {
        let seats = parse_live_seats(r#"{"success":true,"totalCount":0,"data":null}"#, "CS", "3000").unwrap();
        assert!(seats.is_empty());
    }
}
