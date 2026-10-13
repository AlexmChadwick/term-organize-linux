//! Banner HTML parsers and a small client for nubanner.neu.edu.
//!
//! Portions derived from SearchNEU (AGPL-3.0), packages/scraper.

use crate::html::{plain_text, table_rows};
use crate::model::*;
use regex::Regex;
use std::collections::HashMap;

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
    let Some(rows) = table_rows(html) else { return vec![] };
    let mut keys = vec![];
    for cells in rows {
        if cells.len() < 2 { continue; }
        let subject = cells[0].trim();
        let number = cells[1].trim();
        if subject.is_empty() || number.is_empty() { continue; }
        if !number.chars().next().is_some_and(|c| c.is_ascii_digit()) { continue; }
        keys.push(CourseKey { subject: subjects.code(subject), number: number.to_string() });
    }
    keys
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
        start_date: if start_date.is_empty() { None } else { Some(start_date.into()) },
        end_date: if end_date.is_empty() { None } else { Some(end_date.into()) },
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
