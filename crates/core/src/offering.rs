//! Which recent terms a course had sections in. Port of `OfferingHistory.swift`
//! and `OfferingHistoryRepository.swift`.

use crate::calendar::approximate_window;
use crate::index::CatalogIndex;
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OfferingHistory {
    pub checked_terms: Vec<String>,
    pub offered: BTreeMap<CourseKey, BTreeSet<String>>,
    pub checked_keys: Option<BTreeSet<CourseKey>>,
}

impl OfferingHistory {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_checked(&self, key: &CourseKey) -> bool {
        !self.checked_terms.is_empty() && self.checked_keys.as_ref().map(|keys| keys.contains(key)).unwrap_or(true)
    }

    pub fn offered_terms(&self, key: &CourseKey) -> Vec<String> {
        let Some(terms) = self.offered.get(key) else { return vec![] };
        self.checked_terms.iter().filter(|code| terms.contains(*code)).cloned().collect()
    }

    pub fn seasons(&self, key: &CourseKey) -> BTreeSet<PlanSeason> {
        self.offered_terms(key).into_iter().map(|code| plan_season(term_season(&code))).collect()
    }

    pub fn is_likely_offered(&self, key: &CourseKey, season: PlanSeason) -> Option<bool> {
        if !self.is_checked(key) {
            return None;
        }
        let relevant: Vec<&String> = self.checked_terms.iter().filter(|code| season.matches_term_season(term_season(code))).collect();
        if relevant.is_empty() {
            return None;
        }
        let terms = self.offered.get(key);
        Some(relevant.iter().any(|code| terms.is_some_and(|set| set.contains(code.as_str()))))
    }

    pub fn summary(&self, key: &CourseKey) -> String {
        if !self.is_checked(key) {
            return "Offering history unknown".into();
        }
        let terms = self.offered_terms(key);
        let Some(latest) = terms.first() else {
            return not_offered(self.checked_years());
        };
        let regular: Vec<Family> = Family::ALL.into_iter().filter(|family| self.is_regular(*family, key)).collect();
        if regular.is_empty() {
            return format!("Last offered {}", label_of_term(latest));
        }
        let others: Vec<Family> = Family::ALL
            .into_iter()
            .filter(|family| !regular.contains(family) && terms.iter().any(|code| Family::of_season(term_season(code)) == Some(*family)))
            .collect();
        let regularly = format!("Offered every {}", list(&regular.iter().map(|f| f.name().to_string()).collect::<Vec<_>>()));
        if others.is_empty() {
            regularly
        } else {
            format!("{regularly}, and sometimes in the {}", list(&others.iter().map(|f| f.name().to_string()).collect::<Vec<_>>()))
        }
    }

    pub fn summary_in(&self, key: &CourseKey, season: PlanSeason) -> String {
        if self.is_likely_offered(key, season).is_none() {
            return "Offering history unknown".into();
        }
        let family = Family::of_plan(season);
        let relevant: Vec<&String> = self.checked_terms.iter().filter(|code| season.matches_term_season(term_season(code))).collect();
        let terms = self.offered.get(key);
        let latest = relevant.iter().find(|code| terms.is_some_and(|set| set.contains(code.as_str())));
        let Some(latest) = latest else {
            let years = self.checked_years();
            let span = if years == 1 { "the last year".to_string() } else { format!("the last {years} years") };
            let name = family.map(|f| f.name().to_string()).unwrap_or_else(|| season.short_name().to_ascii_lowercase());
            return format!("Not offered in a {name} term in {span}");
        };
        if let Some(family) = family {
            if self.is_regular(family, key) {
                return format!("Offered every {}", family.name());
            }
        }
        format!("Offered in {}", label_of_term(latest))
    }

    pub fn merging(&self, other: &OfferingHistory) -> OfferingHistory {
        let mut terms: Vec<String> = self.checked_terms.iter().chain(other.checked_terms.iter()).cloned().collect();
        terms.sort();
        terms.dedup();
        terms.reverse();
        let mut offered = self.offered.clone();
        for (key, codes) in &other.offered {
            offered.entry(key.clone()).or_default().extend(codes.iter().cloned());
        }
        let checked_keys = match (&self.checked_keys, &other.checked_keys) {
            (Some(a), Some(b)) => Some(a.union(b).cloned().collect()),
            _ => None,
        };
        OfferingHistory { checked_terms: terms, offered, checked_keys }
    }

    fn is_regular(&self, family: Family, key: &CourseKey) -> bool {
        let offered = self.offered.get(key);
        let mut periods: BTreeMap<i32, bool> = BTreeMap::new();
        for code in &self.checked_terms {
            if Family::of_season(term_season(code)) != Some(family) {
                continue;
            }
            let year = Term { code: code.clone(), banner_description: String::new() }.calendar_year();
            let hit = offered.is_some_and(|set| set.contains(code));
            let slot = periods.entry(year).or_insert(false);
            *slot = *slot || hit;
        }
        periods.len() >= 2 && periods.values().all(|offered| *offered)
    }

    fn checked_years(&self) -> i32 {
        let (Some(newest), Some(oldest)) = (self.checked_terms.first(), self.checked_terms.last()) else { return 1 };
        let months = month_index(newest) - month_index(oldest);
        ((months as f64 / 12.0).round() as i32).max(1)
    }
}

fn not_offered(years: i32) -> String {
    if years == 1 { "Not offered in the last year".into() } else { format!("Not offered in the last {years} years") }
}

fn list(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => format!("{} and {}", items[..items.len() - 1].join(", "), items[items.len() - 1]),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Fall,
    Spring,
    Summer,
}

impl Family {
    const ALL: [Family; 3] = [Family::Fall, Family::Spring, Family::Summer];
    fn name(self) -> &'static str {
        match self {
            Self::Fall => "fall",
            Self::Spring => "spring",
            Self::Summer => "summer",
        }
    }
    fn of_season(season: Season) -> Option<Self> {
        match season {
            Season::Fall => Some(Self::Fall),
            Season::Spring => Some(Self::Spring),
            Season::Summer1 | Season::SummerFull | Season::Summer2 => Some(Self::Summer),
            Season::Winter => None,
        }
    }
    fn of_plan(season: PlanSeason) -> Option<Self> {
        match season {
            PlanSeason::Fall => Some(Self::Fall),
            PlanSeason::Spring => Some(Self::Spring),
            PlanSeason::SummerA | PlanSeason::SummerFull | PlanSeason::SummerB => Some(Self::Summer),
            PlanSeason::Winter => None,
        }
    }
}

fn term_season(code: &str) -> Season {
    Term { code: code.to_string(), banner_description: String::new() }.season()
}

pub fn plan_season(season: Season) -> PlanSeason {
    match season {
        Season::Winter => PlanSeason::Winter,
        Season::Spring => PlanSeason::Spring,
        Season::Summer1 => PlanSeason::SummerA,
        Season::SummerFull => PlanSeason::SummerFull,
        Season::Summer2 => PlanSeason::SummerB,
        Season::Fall => PlanSeason::Fall,
    }
}

pub fn month_index(code: &str) -> i32 {
    let term = Term { code: code.to_string(), banner_description: String::new() };
    term.calendar_year() * 12 + plan_season(term.season()).start_month() - 1
}

pub fn label_of_term(code: &str) -> String {
    let term = Term { code: code.to_string(), banner_description: String::new() };
    format!("{} {}", term.season().display_name(), term.calendar_year())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfferingJson {
    #[serde(default)]
    checked_terms: Vec<String>,
    #[serde(default)]
    offered: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checked_keys: Option<Vec<String>>,
}

impl Serialize for OfferingHistory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut offered = BTreeMap::new();
        for (key, codes) in &self.offered {
            let mut list: Vec<String> = codes.iter().cloned().collect();
            list.sort_by(|a, b| b.cmp(a));
            offered.insert(key.to_string(), list);
        }
        OfferingJson {
            checked_terms: self.checked_terms.clone(),
            offered,
            checked_keys: self.checked_keys.as_ref().map(|keys| keys.iter().map(|k| k.to_string()).collect()),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OfferingHistory {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = OfferingJson::deserialize(deserializer)?;
        let mut offered = BTreeMap::new();
        for (code, terms) in raw.offered {
            if let Some(key) = CourseKey::parse(&code) {
                offered.insert(key, terms.into_iter().collect());
            }
        }
        let checked_keys = raw.checked_keys.map(|keys| keys.into_iter().filter_map(|code| CourseKey::parse(&code)).collect());
        Ok(OfferingHistory { checked_terms: raw.checked_terms, offered, checked_keys })
    }
}

pub trait OfferingSource {
    fn terms(&self) -> Result<Vec<Term>, String>;
    fn offered_courses(&self, term: &str, courses: &[CourseKey]) -> Result<HashSet<CourseKey>, String>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    fetched_at: f64,
    offered: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    version: i32,
    terms: BTreeMap<String, BTreeMap<String, Entry>>,
}

pub struct OfferingRepository<F> {
    pub fetcher: F,
    pub file: PathBuf,
    pub now: f64,
    pub window_months: i32,
    pub current_term_lifetime: f64,
    cache: BTreeMap<String, BTreeMap<String, Entry>>,
    loaded: bool,
    dirty: bool,
}

impl<F: OfferingSource> OfferingRepository<F> {
    pub const BATCH: usize = 10;
    pub const VERSION: i32 = 2;

    pub fn new(fetcher: F, file: PathBuf, now: f64) -> Self {
        Self {
            fetcher,
            file,
            now,
            window_months: 24,
            current_term_lifetime: 7.0 * 24.0 * 3600.0,
            cache: BTreeMap::new(),
            loaded: false,
            dirty: false,
        }
    }

    pub fn window(terms: &[Term], months: i32) -> Vec<Term> {
        let mut sorted = terms.to_vec();
        sorted.sort_by(|a, b| b.code.cmp(&a.code));
        let Some(newest) = sorted.first() else { return vec![] };
        let oldest = month_index(&newest.code) - months;
        sorted.into_iter().filter(|term| month_index(&term.code) >= oldest).collect()
    }

    pub fn history(&mut self, keys: &[CourseKey], indexes: &[CatalogIndex]) -> OfferingHistory {
        self.load();
        let mut seen = HashSet::new();
        let keys: Vec<CourseKey> = keys.iter().filter(|key| seen.insert((*key).clone())).cloned().collect();
        if keys.is_empty() {
            return OfferingHistory::empty();
        }
        let mut terms = self.fetcher.terms().unwrap_or_default().into_iter().filter(|term| term.college() == College::Neu).collect::<Vec<_>>();
        for index in indexes {
            if index.catalog.term.college() == College::Neu && !terms.iter().any(|term| term.code == index.catalog.term.code) {
                terms.push(index.catalog.term.clone());
            }
        }
        let window = Self::window(&terms, self.window_months);
        let Some(newest) = window.first().map(|term| term.code.clone()) else {
            return OfferingHistory { checked_terms: vec![], offered: BTreeMap::new(), checked_keys: Some(keys.into_iter().collect()) };
        };
        let mut answers: HashMap<String, HashSet<CourseKey>> = HashMap::new();
        for term in &window {
            if let Some(index) = indexes.iter().find(|index| index.catalog.term.code == term.code) {
                answers.insert(term.code.clone(), keys.iter().filter(|key| !index.sections_for(key).is_empty()).cloned().collect());
                continue;
            }
            if let Some(offered) = self.offered(&keys, &term.code, term.code == newest) {
                answers.insert(term.code.clone(), offered);
            }
        }
        self.save();
        let checked: Vec<String> = window.iter().map(|term| term.code.clone()).filter(|code| answers.contains_key(code)).collect();
        let mut offered: BTreeMap<CourseKey, BTreeSet<String>> = BTreeMap::new();
        for code in &checked {
            for key in answers.get(code).into_iter().flatten() {
                offered.entry(key.clone()).or_default().insert(code.clone());
            }
        }
        OfferingHistory { checked_terms: checked, offered, checked_keys: Some(keys.into_iter().collect()) }
    }

    fn offered(&mut self, keys: &[CourseKey], term: &str, newest: bool) -> Option<HashSet<CourseKey>> {
        let missing: Vec<CourseKey> = keys.iter().filter(|key| !self.is_fresh(term, key, newest)).cloned().collect();
        let mut start = 0;
        while start < missing.len() {
            let end = (start + Self::BATCH).min(missing.len());
            let batch = &missing[start..end];
            start = end;
            match self.fetcher.offered_courses(term, batch) {
                Ok(found) => {
                    let stamp = self.now;
                    for key in batch {
                        self.cache.entry(term.to_string()).or_default().insert(key.to_string(), Entry { fetched_at: stamp, offered: found.contains(key) });
                    }
                    self.dirty = true;
                }
                Err(_) => return None,
            }
        }
        Some(keys.iter().filter(|key| self.cache.get(term).and_then(|row| row.get(&key.to_string())).is_some_and(|entry| entry.offered)).cloned().collect())
    }

    fn is_fresh(&self, term: &str, key: &CourseKey, newest: bool) -> bool {
        let Some(entry) = self.cache.get(term).and_then(|row| row.get(&key.to_string())) else { return false };
        if newest {
            return self.now - entry.fetched_at < self.current_term_lifetime;
        }
        match approximate_window(term) {
            Some((start, _)) => entry.fetched_at >= start,
            None => true,
        }
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Ok(text) = std::fs::read_to_string(&self.file) else { return };
        let Ok(stored) = serde_json::from_str::<Stored>(&text) else { return };
        if stored.version != Self::VERSION {
            return;
        }
        self.cache = stored.terms;
    }

    fn save(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let stored = Stored { version: Self::VERSION, terms: self.cache.clone() };
        let Ok(text) = serde_json::to_string(&stored) else { return };
        if let Some(parent) = self.file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.file, text);
    }
}
