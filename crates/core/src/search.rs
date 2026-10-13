//! On-device search: course codes beat titles, prefixes match, small typos are tolerated,
//! and every meaningful word has to match. Also searches instructors and descriptions.

use crate::index::{CatalogIndex, CourseStats, InstructorSummary};
use crate::model::*;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSort {
    Relevance,
    CourseNumber,
    Title,
    OpenSeats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstructionFilter {
    Any,
    InPerson,
    Online,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchFilters {
    pub subjects: HashSet<String>,
    pub nupaths: HashSet<NuPath>,
    pub levels: HashSet<i32>,
    pub min_credits: Option<f64>,
    pub max_credits: Option<f64>,
    pub campuses: HashSet<String>,
    pub class_types: HashSet<String>,
    pub honors_only: bool,
    pub open_seats_only: bool,
    pub instruction: InstructionFilter,
    pub allowed_days: Option<DaySet>,
    pub starts_after: Option<i32>,
    pub ends_before: Option<i32>,
}

impl Default for SearchFilters {
    fn default() -> Self {
        Self {
            subjects: HashSet::new(),
            nupaths: HashSet::new(),
            levels: HashSet::new(),
            min_credits: None,
            max_credits: None,
            campuses: HashSet::new(),
            class_types: HashSet::new(),
            honors_only: false,
            open_seats_only: false,
            instruction: InstructionFilter::Any,
            allowed_days: None,
            starts_after: None,
            ends_before: None,
        }
    }
}

impl SearchFilters {
    pub fn active_count(&self) -> i32 {
        let mut n = 0;
        if !self.subjects.is_empty() { n += 1; }
        if !self.nupaths.is_empty() { n += 1; }
        if !self.levels.is_empty() { n += 1; }
        if self.min_credits.is_some() || self.max_credits.is_some() { n += 1; }
        if !self.campuses.is_empty() { n += 1; }
        if !self.class_types.is_empty() { n += 1; }
        if self.honors_only { n += 1; }
        if self.open_seats_only { n += 1; }
        if self.instruction != InstructionFilter::Any { n += 1; }
        if self.allowed_days.is_some() { n += 1; }
        if self.starts_after.is_some() || self.ends_before.is_some() { n += 1; }
        n
    }
    pub fn has_section_filters(&self) -> bool {
        !self.campuses.is_empty()
            || !self.class_types.is_empty()
            || self.honors_only
            || self.open_seats_only
            || self.instruction != InstructionFilter::Any
            || self.allowed_days.is_some()
            || self.starts_after.is_some()
            || self.ends_before.is_some()
    }
    pub fn matches_course(&self, course: &Course) -> bool {
        if !self.subjects.is_empty() && !self.subjects.contains(&course.subject) {
            return false;
        }
        if !self.nupaths.is_empty() {
            let attrs: HashSet<&str> = course.attributes.iter().map(|s| s.as_str()).collect();
            if !self.nupaths.iter().all(|p| attrs.contains(p.code())) {
                return false;
            }
        }
        if !self.levels.is_empty() {
            match course.key().level() {
                Some(level) if self.levels.contains(&level) => {}
                _ => return false,
            }
        }
        if let Some(min) = self.min_credits {
            if course.max_credits < min { return false; }
        }
        if let Some(max) = self.max_credits {
            if course.min_credits > max { return false; }
        }
        true
    }
    pub fn matches_section(&self, s: &CourseSection) -> bool {
        if !self.campuses.is_empty() && !self.campuses.contains(&s.campus) { return false; }
        if !self.class_types.is_empty() && !self.class_types.contains(&s.class_type) { return false; }
        if self.honors_only && !s.honors { return false; }
        if self.open_seats_only && !s.is_open() { return false; }
        match self.instruction {
            InstructionFilter::Any => {}
            InstructionFilter::Online if !s.is_online() => return false,
            InstructionFilter::InPerson if s.is_online() => return false,
            _ => {}
        }
        let weekly = s.weekly_meetings();
        if let Some(allowed) = self.allowed_days {
            if weekly.iter().any(|m| !m.days.is_subset(allowed)) { return false; }
        }
        if let Some(after) = self.starts_after {
            if weekly.iter().any(|m| m.start.unwrap_or(0) < after) { return false; }
        }
        if let Some(before) = self.ends_before {
            if weekly.iter().any(|m| m.end.unwrap_or(0) > before) { return false; }
        }
        true
    }
}

#[derive(Clone, Debug)]
pub struct SearchQuery {
    pub text: String,
    pub filters: SearchFilters,
    pub sort: SearchSort,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self { text: String::new(), filters: SearchFilters::default(), sort: SearchSort::Relevance }
    }
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub course: Course,
    pub stats: CourseStats,
    pub score: f64,
    pub match_note: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub crn_match: Option<CourseSection>,
    pub instructors: Vec<InstructorSummary>,
    pub subjects: Vec<Subject>,
}

#[derive(Clone, Debug)]
pub struct SearchDocument {
    pub key: CourseKey,
    pub subject_lower: String,
    pub number_lower: String,
    pub title_normalized: String,
    pub title_words: Vec<String>,
    pub subject_words: Vec<String>,
    pub instructor_words: Vec<String>,
    pub instructor_names: Vec<String>,
    pub description_words: HashSet<String>,
    pub topic_words: Vec<String>,
    pub nupath_shorts: HashSet<String>,
}

impl SearchDocument {
    pub fn build(course: &Course, subject_name: &str, instructor_names: &[String], section_titles: &[String]) -> Self {
        Self {
            key: course.key(),
            subject_lower: course.subject.to_ascii_lowercase(),
            number_lower: course.number.to_ascii_lowercase(),
            title_normalized: normalize(&course.title),
            title_words: words(&course.title),
            subject_words: words(subject_name),
            instructor_names: instructor_names.to_vec(),
            instructor_words: instructor_names.iter().flat_map(|n| words(n)).collect(),
            description_words: words(&course.description).into_iter().filter(|w| w.len() >= 3).collect(),
            topic_words: section_titles.iter().flat_map(|t| words(t)).collect(),
            nupath_shorts: course.nupaths().into_iter().map(|p| p.short().to_ascii_lowercase()).collect(),
        }
    }
}

pub fn normalize(s: &str) -> String {
    let folded = s.to_lowercase();
    let mut out = String::new();
    let mut last_space = true;
    for ch in folded.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if ch == '&' {
            if !last_space { out.push(' '); }
            out.push_str("and ");
            last_space = true;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

const STOP: &[&str] = &["and", "of", "the", "to", "in", "for", "a", "an", "with", "on", "at", "intro", "introduction"];

fn alias(word: &str) -> Option<&'static str> {
    Some(match word {
        "orgo" | "ochem" => "organic chemistry",
        "linalg" => "linear algebra",
        "diffeq" | "diffeqs" => "differential equations",
        "stats" | "stat" => "statistics",
        "econ" => "economics",
        "psych" => "psychology",
        "chem" => "chemistry",
        "ml" => "machine learning",
        "ai" => "artificial intelligence",
        "hci" => "human computer interaction",
        "ood" => "object oriented",
        "algo" | "algos" => "algorithms",
        "swe" => "software engineering",
        "nlp" => "natural language processing",
        "db" | "dbs" => "databases",
        "fundies" => "program design",
        "compsci" => "computer science",
        "cs1" => "program design implementation 1",
        "calc" => "calculus",
        "bio" => "biology",
        "phys" => "physics",
        "intl" => "international",
        "mgmt" => "management",
        "acct" => "accounting",
        "mktg" => "marketing",
        "philo" => "philosophy",
        "poli" | "polisci" => "political science",
        "comm" | "comms" => "communication",
        _ => return None,
    })
}

pub fn expand_aliases(normalized: &str, subjects: &HashSet<String>) -> String {
    normalized
        .split_whitespace()
        .map(|w| {
            if subjects.contains(&w.to_ascii_uppercase()) { w.to_string() } else { alias(w).unwrap_or(w).to_string() }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn char_prefix(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

pub fn words(s: &str) -> Vec<String> {
    normalize(s).split_whitespace().map(|s| s.to_string()).collect()
}

pub fn edit_distance(a: &str, b: &str, limit: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() || b.is_empty() { return a.len().max(b.len()); }
    if a.len().abs_diff(b.len()) > limit { return limit + 1; }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        let mut row_min = curr[0];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(curr[j]);
        }
        if row_min > limit { return limit + 1; }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

pub fn search(query: &SearchQuery, index: &CatalogIndex) -> SearchResponse {
    let mut response = SearchResponse::default();
    let text = query.text.trim();
    let filters = &query.filters;
    let passes = |course: &Course| {
        if !filters.matches_course(course) { return false; }
        if !filters.has_section_filters() { return true; }
        index.sections_for(&course.key()).iter().any(|s| filters.matches_section(s))
    };
    let stats_for = |course: &Course| {
        if !filters.has_section_filters() { return index.stats_for(&course.key()); }
        make_stats(&index.sections_for(&course.key()).iter().filter(|s| filters.matches_section(s)).cloned().collect::<Vec<_>>())
    };
    if text.len() == 5 && text.chars().all(|c| c.is_ascii_digit()) {
        response.crn_match = index.section_crn(text).cloned();
    }
    if text.is_empty() {
        let mut scored = vec![];
        for course in &index.courses {
            if passes(course) {
                scored.push(SearchResult { course: course.clone(), stats: stats_for(course), score: 0.0, match_note: None });
            }
        }
        let sort = if query.sort == SearchSort::Relevance { SearchSort::CourseNumber } else { query.sort };
        response.results = sort_results(scored, sort);
        return response;
    }
    let raw = normalize(text);
    let subjects: HashSet<String> = index.subjects_by_code.keys().cloned().collect();
    let normalized = expand_aliases(&raw, &subjects);
    let tokens: Vec<String> = normalized.split_whitespace().map(|s| s.to_string()).collect();
    if tokens.is_empty() { return response; }
    response.subjects = index.active_subjects.iter().filter(|subject| {
        let name = normalize(&subject.name);
        subject.code.to_ascii_lowercase() == raw || name == raw || name == normalized || (raw.len() >= 4 && name.starts_with(&raw))
    }).cloned().collect();
    let raw_tokens: Vec<String> = raw.split_whitespace().map(|s| s.to_string()).collect();
    if raw_tokens.iter().all(|t| t.len() >= 2) && !raw_tokens.iter().all(|t| t.chars().all(|c| c.is_ascii_digit())) {
        response.instructors = index.instructors.iter().filter(|ins| {
            let words = words(&ins.display_name);
            raw_tokens.iter().all(|t| words.iter().any(|w| w.starts_with(t)))
        }).take(8).cloned().collect();
    }
    let code = parse_course_code(&raw, &index.subjects_by_code);
    let mut scored = vec![];
    for doc in &index.documents {
        let Some(course) = index.course(&doc.key) else { continue };
        let (score, note) = if let Some((subject, number)) = &code {
            if &doc.subject_lower != subject { continue; }
            if number.is_empty() {
                (600.0 - doc.key.numeric_value() as f64 / 100.0, None)
            } else if &doc.number_lower == number {
                (2000.0, None)
            } else if doc.number_lower.starts_with(number) {
                (1200.0 - (doc.number_lower.len() - number.len()) as f64, None)
            } else {
                continue;
            }
        } else if let Some(m) = score_tokens(&tokens, &normalized, doc) {
            m
        } else {
            continue;
        };
        if !passes(course) { continue; }
        scored.push(SearchResult { course: course.clone(), stats: stats_for(course), score, match_note: note });
    }
    response.results = sort_results(scored, query.sort);
    response
}

fn parse_course_code(normalized: &str, subjects: &std::collections::BTreeMap<String, Subject>) -> Option<(String, String)> {
    let compact: String = normalized.chars().filter(|c| !c.is_whitespace()).collect();
    let letters: String = compact.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let rest = &compact[letters.len()..];
    if !(2..=5).contains(&letters.len()) || !subjects.contains_key(&letters.to_ascii_uppercase()) {
        return None;
    }
    if rest.chars().any(|c| !c.is_ascii_alphanumeric()) || rest.len() > 4 {
        return None;
    }
    if rest.is_empty() {
        return if normalized == letters { Some((letters, String::new())) } else { None };
    }
    if !rest.chars().next().is_some_and(|c| c.is_ascii_digit()) { return None; }
    Some((letters, rest.to_string()))
}

fn score_tokens(tokens: &[String], phrase: &str, doc: &SearchDocument) -> Option<(f64, Option<String>)> {
    let meaningful: Vec<&String> = tokens.iter().filter(|t| !STOP.contains(&t.as_str())).collect();
    let used: Vec<&String> = if meaningful.is_empty() { tokens.iter().collect() } else { meaningful };
    let wants_intro = tokens.iter().any(|t| t == "intro" || t == "introduction" || t == "introductory");
    let mut total = 0.0;
    let mut title_hits = 0;
    let mut note = None;
    for token in used.iter().copied() {
        let mut best = 0.0f64;
        let mut best_note = None;
        if token == &doc.subject_lower { best = best.max(50.0); }
        if token == &doc.number_lower { best = best.max(60.0); }
        else if token.chars().all(|c| c.is_ascii_digit()) && doc.number_lower.starts_with(token) { best = best.max(30.0); }
        if doc.title_words.iter().any(|w| w == token) {
            best = best.max(40.0);
            title_hits += 1;
        } else if token.len() >= 2 && doc.title_words.iter().any(|w| w.starts_with(token)) {
            best = best.max(28.0);
            title_hits += 1;
        }
        if doc.subject_words.iter().any(|w| w.starts_with(token)) {
            best = best.max(if token.len() >= 3 { 20.0 } else { 8.0 });
        }
        if doc.nupath_shorts.contains(token) { best = best.max(15.0); }
        if best == 0.0 && token.len() >= 2 && doc.topic_words.iter().any(|w| w.starts_with(token)) {
            best = 24.0;
            best_note = Some("Topic match".to_string());
        }
        if best == 0.0 && token.len() >= 2 && doc.instructor_words.iter().any(|w| w.starts_with(token)) {
            best = 22.0;
            if let Some(name) = doc.instructor_names.iter().find(|n| words(n).iter().any(|w| w.starts_with(token))) {
                best_note = Some(format!("Taught by {name}"));
            }
        }
        if best == 0.0 && token.len() >= 3 && (doc.description_words.contains(token) || (token.len() >= 5 && doc.description_words.iter().any(|w| w.starts_with(token)))) {
            best = 6.0;
            best_note = Some(format!("Description mentions “{token}”"));
        }
        if best == 0.0 && token.len() >= 4 {
            let limit = if token.len() >= 7 { 2 } else { 1 };
            if doc.title_words.iter().any(|w| edit_distance(w, token, limit) <= limit || (w.chars().count() > token.chars().count() && edit_distance(char_prefix(w, token.chars().count()), token, 1) <= 1)) {
                best = 12.0;
                title_hits += 1;
            }
        }
        if best == 0.0 { return None; }
        total += best;
        if note.is_none() { note = best_note; }
    }
    if doc.title_normalized == phrase { total += 150.0; }
    else if doc.title_normalized.starts_with(phrase) { total += 80.0; }
    else if phrase.len() >= 4 && doc.title_normalized.contains(phrase) { total += 50.0; }
    let meaningful_count = tokens.iter().filter(|t| !STOP.contains(&t.as_str())).count();
    if title_hits >= meaningful_count.max(1) {
        total += 25.0;
        note = None;
    }
    if wants_intro {
        if let Some(level) = doc.key.level() {
            total += if level == 1 { 15.0 } else if level == 2 { 6.0 } else { 0.0 };
        }
    }
    total -= doc.key.numeric_value() as f64 / 10_000.0;
    Some((total, note))
}

pub fn sort_results(mut results: Vec<SearchResult>, sort: SearchSort) -> Vec<SearchResult> {
    match sort {
        SearchSort::Relevance => results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then(a.course.key().cmp(&b.course.key()))),
        SearchSort::CourseNumber => results.sort_by_key(|result| result.course.key()),
        SearchSort::Title => results.sort_by(|a, b| crate::model::natural_cmp(&a.course.title, &b.course.title)),
        SearchSort::OpenSeats => results.sort_by(|a, b| b.stats.seats_remaining.cmp(&a.stats.seats_remaining).then(a.course.key().cmp(&b.course.key()))),
    }
    results
}

pub fn make_stats(sections: &[CourseSection]) -> CourseStats {
    let mut stats = CourseStats::default();
    let mut names = Vec::new();
    for s in sections {
        stats.section_count += 1;
        if s.is_open() { stats.open_section_count += 1; }
        stats.seats_remaining += s.seats_remaining.max(0);
        stats.seat_capacity += s.seat_capacity.max(0);
        if s.waitlist_remaining > 0 { stats.waitlist_open_count += 1; }
        if !s.campus.is_empty() { stats.campuses.insert(s.campus.clone()); }
        if !s.class_type.is_empty() { stats.class_types.insert(s.class_type.clone()); }
        if s.honors { stats.has_honors = true; }
        if s.is_online() { stats.has_online = true; } else { stats.has_in_person = true; }
        stats.meeting_days = stats.meeting_days.union(s.meeting_days());
        if let Some(instructors) = &s.instructors {
            let mut list = instructors.clone();
            list.sort_by_key(|instructor| std::cmp::Reverse(instructor.primary));
            for i in list {
                let name = i.display_name();
                if !names.contains(&name) { names.push(name); }
            }
        }
    }
    stats.instructor_names = names;
    stats
}
