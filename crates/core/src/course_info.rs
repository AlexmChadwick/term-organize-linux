//! Catalog entries for courses that are not in the loaded term.
//! Port of `CourseInfo.swift` and `CourseInfoRepository.swift`.

use crate::equivalences::CourseEquivalences;
use crate::index::CatalogIndex;
use crate::model::*;
use crate::offering::{OfferingHistory, OfferingRepository, OfferingSource};
use crate::suggestions::{self, EquivalenceSuggestion};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
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
    pub fn new(
        key: CourseKey,
        title: impl Into<String>,
        source_term: impl Into<String>,
        fetched_at: f64,
    ) -> Self {
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
        self.equivalents = normalize_equivalents(&self.key, equivalents);
        self
    }

    /// Catalog entry. Equivalents are sorted, deduped, and never include the course itself.
    pub fn entry(
        key: CourseKey,
        title: impl Into<String>,
        min_credits: Option<f64>,
        max_credits: Option<f64>,
        prerequisites: Option<Requisite>,
        equivalents: Vec<CourseKey>,
        source_term: impl Into<String>,
        fetched_at: f64,
    ) -> Self {
        let mut info = Self::new(key, title, source_term, fetched_at);
        info.min_credits = min_credits;
        info.max_credits = max_credits;
        info.prerequisites = prerequisites;
        info.with_equivalents(equivalents)
    }

    /// `4 credits`, `1–4 credits`; none when the credit hours are unknown.
    pub fn credits_label(&self) -> Option<String> {
        let low = self.min_credits.or(self.max_credits)?;
        let high = self.max_credits.unwrap_or(low);
        Some(credit_label(low, high))
    }

    pub fn from_loaded(key: &CourseKey, indexes: &[CatalogIndex]) -> Option<Self> {
        for (position, index) in indexes.iter().enumerate() {
            let Some(course) = index.course(key) else {
                continue;
            };
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
        let mut neu: Vec<&Term> = terms
            .iter()
            .filter(|term| term.college() == College::Neu)
            .collect();
        neu.sort_by(|a, b| b.code.cmp(&a.code));
        let Some(newest) = neu.first() else {
            return vec![];
        };
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
        self.lookup(std::slice::from_ref(key), indexes)
            .remove(key)
            .unwrap_or(Lookup::Failed)
    }

    pub fn lookup(
        &mut self,
        keys: &[CourseKey],
        indexes: &[CatalogIndex],
    ) -> HashMap<CourseKey, Lookup> {
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
            let answer = if result == Lookup::Failed {
                self.stale_or_failed(&key)
            } else {
                result
            };
            results.insert(key, answer);
        }
        self.save();
        results
    }

    fn fetch_parallel(
        &mut self,
        pending: Vec<CourseKey>,
        terms: &[String],
    ) -> HashMap<CourseKey, Lookup> {
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
                scope.spawn(move || loop {
                    let key = { jobs.lock().ok().and_then(|mut jobs| jobs.pop()) };
                    let Some(key) = key else { break };
                    let result = search_one(fetcher, &key, &terms, now);
                    if let Ok(mut slot) = found_out.lock() {
                        slot.insert(key, result);
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
        self.found
            .get(key)
            .cloned()
            .map(Lookup::Found)
            .unwrap_or(Lookup::Failed)
    }

    fn is_fresh(&self, info: &CourseInfo) -> bool {
        self.now - info.fetched_at < self.found_lifetime
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Ok(text) = std::fs::read_to_string(&self.file) else {
            return;
        };
        let Ok(stored) = serde_json::from_str::<Stored>(&text) else {
            return;
        };
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
        let found: Vec<CourseInfo> = self
            .found
            .values()
            .filter(|info| self.is_fresh(info))
            .cloned()
            .collect();
        let missing = self
            .missing
            .iter()
            .filter(|(_, at)| self.now - **at < self.not_found_lifetime)
            .map(|(key, at)| Missing {
                key: key.clone(),
                checked_at: *at,
            })
            .collect();
        let stored = Stored {
            version: Self::VERSION,
            found,
            missing,
        };
        let Ok(text) = serde_json::to_string(&stored) else {
            return;
        };
        if let Some(parent) = self.file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.file, text);
    }
}

fn search_one<F: CourseInfoSource>(
    fetcher: &F,
    key: &CourseKey,
    terms: &[String],
    now: f64,
) -> Lookup {
    for (position, term) in terms.iter().enumerate() {
        match fetcher.catalog_entry(key, term) {
            Ok(Some(mut info)) => {
                info.is_retired = Some(position > 0);
                info.fetched_at = if info.fetched_at == 0.0 {
                    now
                } else {
                    info.fetched_at
                };
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
    keys.iter()
        .filter(|key| seen.insert((*key).clone()))
        .cloned()
        .collect()
}

fn normalize_equivalents(key: &CourseKey, equivalents: Vec<CourseKey>) -> Vec<CourseKey> {
    let mut equivalents: Vec<CourseKey> =
        equivalents.into_iter().filter(|item| item != key).collect();
    equivalents.sort();
    equivalents.dedup();
    equivalents
}

fn nonempty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// What a screen has learned about a course. Synchronous port of `CourseInfoStore`.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum StoreState {
    Unknown,
    Found(CourseInfo),
    NotFound,
    Failed,
}

pub struct CourseInfoStore<F, O> {
    pub repository: CourseInfoRepository<F>,
    pub offering: OfferingRepository<O>,
    pub now: f64,
    infos: HashMap<CourseKey, CourseInfo>,
    not_found: HashSet<CourseKey>,
    failed_at: HashMap<CourseKey, f64>,
    derived: CourseEquivalences,
    indexes: Vec<CatalogIndex>,
    index_stamp: BTreeMap<String, f64>,
    pub version: u64,
    retry_delay: f64,
}

impl<F, O> CourseInfoStore<F, O> {
    pub fn new(
        repository: CourseInfoRepository<F>,
        offering: OfferingRepository<O>,
        now: f64,
    ) -> Self {
        Self {
            repository,
            offering,
            now,
            infos: HashMap::new(),
            not_found: HashSet::new(),
            failed_at: HashMap::new(),
            derived: CourseEquivalences::none(),
            indexes: Vec::new(),
            index_stamp: BTreeMap::new(),
            version: 0,
            retry_delay: 60.0,
        }
    }

    pub fn info(&self, key: &CourseKey) -> Option<&CourseInfo> {
        self.infos.get(key)
    }

    pub fn state(&self, key: &CourseKey) -> StoreState {
        if let Some(info) = self.infos.get(key) {
            return StoreState::Found(info.clone());
        }
        if self.not_found.contains(key) {
            return StoreState::NotFound;
        }
        if self.failed_at.contains_key(key) {
            return StoreState::Failed;
        }
        StoreState::Unknown
    }

    /// `None` when the course is not known, `Some(None)` when it has no prerequisites.
    pub fn requisite_lookup(&self) -> HashMap<CourseKey, Option<Requisite>> {
        self.infos
            .iter()
            .map(|(key, info)| (key.clone(), info.prerequisites.clone()))
            .collect()
    }

    pub fn equivalences(&self, plan: Option<&DegreePlan>) -> CourseEquivalences {
        match plan {
            None => self.derived.clone(),
            Some(plan) => self.derived.merging(&CourseEquivalences::from_declarations(
                &plan.course_equivalences,
            )),
        }
    }

    pub fn equivalence_suggestions(
        &self,
        key: &CourseKey,
        plan: Option<&DegreePlan>,
        limit: usize,
        catalog_titles: &HashMap<CourseKey, String>,
        also_comparing: &[CourseKey],
    ) -> Vec<EquivalenceSuggestion> {
        let Some(newest) = self.indexes.first() else {
            return Vec::new();
        };
        if newest.course(key).is_some() {
            return Vec::new();
        }
        match self.state(key) {
            StoreState::Found(info) if info.is_retired == Some(false) => return Vec::new(),
            StoreState::Found(_) | StoreState::NotFound => {}
            StoreState::Unknown | StoreState::Failed => return Vec::new(),
        }
        let prerequisites: Vec<(CourseKey, Requisite)> = newest
            .courses
            .iter()
            .filter_map(|course| {
                course
                    .prereqs
                    .clone()
                    .map(|requisite| (course.key(), requisite))
            })
            .collect();
        let mut candidates = Vec::new();
        if let Some(plan) = plan {
            candidates.extend(
                plan.completed
                    .iter()
                    .filter_map(|course| course.key.clone()),
            );
            candidates.extend(
                plan.terms
                    .iter()
                    .flat_map(|term| term.courses.iter().filter_map(|course| course.key.clone())),
            );
        }
        candidates.extend(also_comparing.iter().cloned());
        let known = self.equivalences(plan);
        suggestions::suggestions(
            key,
            &prerequisites,
            |course| newest.course(course).is_some(),
            &known,
            2,
            0.5,
            limit,
            |course| self.title_of(course, catalog_titles, plan),
            &candidates,
            crate::titles::MINIMUM_SIMILARITY,
        )
    }

    fn title_of(
        &self,
        key: &CourseKey,
        catalog_titles: &HashMap<CourseKey, String>,
        plan: Option<&DegreePlan>,
    ) -> Option<String> {
        if let Some(title) = catalog_titles.get(key).and_then(|text| nonempty(text)) {
            return Some(title);
        }
        if let Some(title) = self.infos.get(key).and_then(|info| nonempty(&info.title)) {
            return Some(title);
        }
        for index in &self.indexes {
            if let Some(title) = index.course(key).and_then(|course| nonempty(&course.title)) {
                return Some(title);
            }
        }
        let plan = plan?;
        for course in plan
            .completed
            .iter()
            .chain(plan.terms.iter().flat_map(|term| term.courses.iter()))
        {
            if course.key.as_ref() == Some(key) {
                if let Some(title) = nonempty(&course.title) {
                    return Some(title);
                }
            }
        }
        None
    }

    /// True when `derived` changed.
    fn rebuild_equivalences(&mut self) -> bool {
        let groups: Vec<Vec<CourseKey>> = self
            .infos
            .values()
            .filter(|info| !info.equivalents.is_empty())
            .map(|info| {
                let mut group = vec![info.key.clone()];
                group.extend(info.equivalents.iter().cloned());
                group
            })
            .collect();
        let mut result = CourseEquivalences::from_groups(&groups);
        for index in &self.indexes {
            result = result.merging(&CourseEquivalences::from_cross_listed(index));
        }
        if result == self.derived {
            return false;
        }
        self.derived = result;
        true
    }
}

impl<F: CourseInfoSource, O> CourseInfoStore<F, O> {
    pub fn load(&mut self, keys: &[CourseKey], indexes: &[CatalogIndex]) {
        self.repository.now = self.now;
        self.indexes = indexes.to_vec();
        let mut seen = HashSet::new();
        let wanted: Vec<CourseKey> = keys
            .iter()
            .filter(|key| seen.insert((*key).clone()))
            .cloned()
            .collect();

        let mut changed = false;
        for key in &wanted {
            let Some(mut info) = CourseInfo::from_loaded(key, indexes) else {
                continue;
            };
            if let Some(previous) = self.infos.get(key) {
                if info.source_term.as_str() < previous.source_term.as_str() {
                    continue;
                }
                info.equivalents = previous.equivalents.clone();
                if info == *previous {
                    continue;
                }
            }
            self.infos.insert(key.clone(), info);
            self.not_found.remove(key);
            changed = true;
        }
        let mut stamp = BTreeMap::new();
        for index in indexes {
            stamp
                .entry(index.catalog.term.code.clone())
                .or_insert(index.catalog.sections_updated_at);
        }
        let indexes_changed = stamp != self.index_stamp;
        self.index_stamp = stamp;
        if (changed || indexes_changed) && self.rebuild_equivalences() {
            changed = true;
        }
        if changed {
            self.version += 1;
        }

        let pending: Vec<CourseKey> = wanted
            .into_iter()
            .filter(|key| {
                !self.infos.contains_key(key)
                    && !self.not_found.contains(key)
                    && self
                        .failed_at
                        .get(key)
                        .is_none_or(|at| self.now - *at > self.retry_delay)
            })
            .collect();
        if pending.is_empty() {
            return;
        }
        let results = self.repository.lookup(&pending, indexes);
        for key in pending {
            let result = results.get(&key).cloned().unwrap_or(Lookup::Failed);
            self.receive(key, result);
        }
    }

    fn receive(&mut self, key: CourseKey, result: Lookup) {
        match result {
            Lookup::Found(info) => {
                self.infos.insert(key.clone(), info);
                self.not_found.remove(&key);
                self.failed_at.remove(&key);
                self.rebuild_equivalences();
            }
            Lookup::NotFound => {
                self.failed_at.remove(&key);
                self.not_found.insert(key);
            }
            Lookup::Failed => {
                self.failed_at.insert(key, self.now);
            }
        }
        self.version += 1;
    }
}

impl<F, O: OfferingSource> CourseInfoStore<F, O> {
    pub fn offering_history(
        &mut self,
        keys: &[CourseKey],
        indexes: Option<&[CatalogIndex]>,
    ) -> OfferingHistory {
        self.offering.now = self.now;
        if let Some(indexes) = indexes {
            self.offering.history(keys, indexes)
        } else {
            let indexes = self.indexes.clone();
            self.offering.history(keys, &indexes)
        }
    }
}
