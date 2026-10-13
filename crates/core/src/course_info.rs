//! Catalog entries for courses that are not in the loaded term.
//! Port of `CourseInfo.swift` and `CourseInfoRepository.swift`.

use crate::index::CatalogIndex;
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseInfo {
    pub key: CourseKey,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_credits: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_credits: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<Requisite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corequisites: Option<Requisite>,
    #[serde(default)]
    pub equivalents: Vec<CourseKey>,
    pub source_term: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_retired: Option<bool>,
    pub fetched_at: f64,
}

impl CourseInfo {
    pub fn new(key: CourseKey, title: impl Into<String>, source_term: impl Into<String>, fetched_at: f64) -> Self {
        Self {
            key,
            title: title.into(),
            min_credits: None,
            max_credits: None,
            prerequisites: None,
            corequisites: None,
            equivalents: vec![],
            source_term: source_term.into(),
            is_retired: None,
            fetched_at,
        }
    }

    pub fn with_equivalents(mut self, equivalents: Vec<CourseKey>) -> Self {
        let mut equivalents: Vec<CourseKey> = equivalents.into_iter().filter(|key| key != &self.key).collect();
        equivalents.sort();
        equivalents.dedup();
        self.equivalents = equivalents;
        self
    }

    pub fn from_loaded(key: &CourseKey, indexes: &[CatalogIndex]) -> Option<Self> {
        for (position, index) in indexes.iter().enumerate() {
            let Some(course) = index.course(key) else { continue };
            if !course.details_loaded {
                continue;
            }
            return Some(Self {
                key: course.key(),
                title: course.title.clone(),
                min_credits: Some(course.min_credits),
                max_credits: Some(course.max_credits),
                prerequisites: course.prereqs.clone(),
                corequisites: course.coreqs.clone(),
                equivalents: vec![],
                source_term: index.catalog.term.code.clone(),
                is_retired: if position == 0 { Some(false) } else { None },
                fetched_at: index.catalog.generated_at,
            });
        }
        None
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    Found(CourseInfo),
    NotFound,
    Failed,
}

pub trait CourseInfoSource: Send + Sync {
    fn terms(&self) -> Result<Vec<Term>, String>;
    fn catalog_entry(&self, key: &CourseKey, term: &str) -> Result<Option<CourseInfo>, String>;
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Missing {
    key: CourseKey,
    checked_at: f64,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: i32,
    found: Vec<CourseInfo>,
    missing: Vec<Missing>,
}

pub struct CourseInfoRepository<F> {
    pub fetcher: F,
    pub file: PathBuf,
    pub now: f64,
    pub max_terms_back: usize,
    pub not_found_lifetime: f64,
    pub found_lifetime: f64,
    pub concurrency: usize,
    found: HashMap<CourseKey, CourseInfo>,
    missing: HashMap<CourseKey, f64>,
    loaded: bool,
    dirty: bool,
    term_cache: Option<(Vec<String>, f64)>,
}

impl<F: CourseInfoSource> CourseInfoRepository<F> {
    pub const VERSION: i32 = 1;

    pub fn new(fetcher: F, file: PathBuf, now: f64) -> Self {
        Self {
            fetcher,
            file,
            now,
            max_terms_back: 8,
            not_found_lifetime: 7.0 * 24.0 * 3600.0,
            found_lifetime: 180.0 * 24.0 * 3600.0,
            concurrency: 2,
            found: HashMap::new(),
            missing: HashMap::new(),
            loaded: false,
            dirty: false,
            term_cache: None,
        }
    }

    pub fn catalog_terms(terms: &[Term], max_back: usize) -> Vec<String> {
        let mut neu: Vec<&Term> = terms.iter().filter(|term| term.college() == College::Neu).collect();
        neu.sort_by(|a, b| b.code.cmp(&a.code));
        let Some(newest) = neu.first() else { return vec![] };
        let mut codes = vec![newest.code.clone()];
        for term in neu.into_iter().skip(1) {
            if term.season() != Season::Fall && term.season() != Season::Spring {
                continue;
            }
            if codes.len() > max_back {
                break;
            }
            codes.push(term.code.clone());
        }
        codes
    }

    pub fn cached_lookup(&mut self, key: &CourseKey) -> Option<Lookup> {
        self.load();
        if let Some(info) = self.found.get(key) {
            if self.is_fresh(info) {
                return Some(Lookup::Found(info.clone()));
            }
        }
        if let Some(checked) = self.missing.get(key) {
            if self.now - checked < self.not_found_lifetime {
                return Some(Lookup::NotFound);
            }
        }
        None
    }

    pub fn lookup_one(&mut self, key: &CourseKey, indexes: &[CatalogIndex]) -> Lookup {
        self.lookup(std::slice::from_ref(key), indexes).remove(key).unwrap_or(Lookup::Failed)
    }

    pub fn lookup(&mut self, keys: &[CourseKey], indexes: &[CatalogIndex]) -> HashMap<CourseKey, Lookup> {
        self.load();
        let mut results = HashMap::new();
        let mut pending = Vec::new();
        for key in unique(keys) {
            if let Some(info) = CourseInfo::from_loaded(&key, indexes) {
                results.insert(key, Lookup::Found(info));
            } else if let Some(known) = self.cached_lookup(&key) {
                results.insert(key, known);
            } else {
                pending.push(key);
            }
        }
        if pending.is_empty() {
            return results;
        }
        let terms = match self.catalog_term_codes() {
            Ok(terms) => terms,
            Err(_) => {
                for key in pending {
                    results.insert(key.clone(), self.stale_or_failed(&key));
                }
                return results;
            }
        };
        let fetched = self.fetch_parallel(pending, &terms);
        for (key, result) in fetched {
            let answer = if result == Lookup::Failed { self.stale_or_failed(&key) } else { result };
            results.insert(key, answer);
        }
        self.save();
        results
    }

    fn fetch_parallel(&mut self, pending: Vec<CourseKey>, terms: &[String]) -> HashMap<CourseKey, Lookup> {
        let limit = self.concurrency.max(1).min(pending.len().max(1));
        let jobs = Arc::new(Mutex::new(pending));
        let terms = terms.to_vec();
        let now = self.now;
        let found_out = Arc::new(Mutex::new(HashMap::new()));
        let fetcher = &self.fetcher;
        std::thread::scope(|scope| {
            for _ in 0..limit {
                let jobs = Arc::clone(&jobs);
                let terms = terms.clone();
                let found_out = Arc::clone(&found_out);
                scope.spawn(move || {
                    loop {
                        let key = { jobs.lock().ok().and_then(|mut jobs| jobs.pop()) };
                        let Some(key) = key else { break };
                        let result = search_one(fetcher, &key, &terms, now);
                        if let Ok(mut slot) = found_out.lock() {
                            slot.insert(key, result);
                        }
                    }
                });
            }
        });
        let fetched = found_out.lock().map(|map| map.clone()).unwrap_or_default();
        for (key, result) in &fetched {
            match result {
                Lookup::Found(info) => {
                    self.found.insert(key.clone(), info.clone());
                    self.missing.remove(key);
                    self.dirty = true;
                }
                Lookup::NotFound => {
                    self.missing.insert(key.clone(), self.now);
                    self.dirty = true;
                }
                Lookup::Failed => {}
            }
        }
        fetched
    }

    fn catalog_term_codes(&mut self) -> Result<Vec<String>, String> {
        if let Some((codes, at)) = &self.term_cache {
            if self.now - at < 3600.0 {
                return Ok(codes.clone());
            }
        }
        let codes = Self::catalog_terms(&self.fetcher.terms()?, self.max_terms_back);
        if codes.is_empty() {
            return Err("no terms".into());
        }
        self.term_cache = Some((codes.clone(), self.now));
        Ok(codes)
    }

    fn stale_or_failed(&self, key: &CourseKey) -> Lookup {
        self.found.get(key).cloned().map(Lookup::Found).unwrap_or(Lookup::Failed)
    }

    fn is_fresh(&self, info: &CourseInfo) -> bool {
        self.now - info.fetched_at < self.found_lifetime
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
        for info in stored.found {
            self.found.insert(info.key.clone(), info);
        }
        for entry in stored.missing {
            self.missing.insert(entry.key, entry.checked_at);
        }
    }

    fn save(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let found: Vec<CourseInfo> = self.found.values().filter(|info| self.is_fresh(info)).cloned().collect();
        let missing = self
            .missing
            .iter()
            .filter(|(_, at)| self.now - **at < self.not_found_lifetime)
            .map(|(key, at)| Missing { key: key.clone(), checked_at: *at })
            .collect();
        let stored = Stored { version: Self::VERSION, found, missing };
        let Ok(text) = serde_json::to_string(&stored) else { return };
        if let Some(parent) = self.file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.file, text);
    }
}

fn search_one<F: CourseInfoSource>(fetcher: &F, key: &CourseKey, terms: &[String], now: f64) -> Lookup {
    for (position, term) in terms.iter().enumerate() {
        match fetcher.catalog_entry(key, term) {
            Ok(Some(mut info)) => {
                info.is_retired = Some(position > 0);
                info.fetched_at = if info.fetched_at == 0.0 { now } else { info.fetched_at };
                return Lookup::Found(info);
            }
            Ok(None) => {}
            Err(_) => return Lookup::Failed,
        }
    }
    Lookup::NotFound
}

fn unique(keys: &[CourseKey]) -> Vec<CourseKey> {
    let mut seen = HashSet::new();
    keys.iter().filter(|key| seen.insert((*key).clone())).cloned().collect()
}
