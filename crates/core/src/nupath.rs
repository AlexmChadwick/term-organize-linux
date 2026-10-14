//! NUpath matching: at most two limited areas per course, completed courses first.
//! Also the section-attribute cache (`CourseAttributeRepository`).

use crate::index::CatalogIndex;
use crate::model::{College, CourseKey, NuPath, Term};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct NuCourse {
    pub id: String,
    pub paths: HashSet<NuPath>,
    pub is_completed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct NuRequirement {
    pub path: NuPath,
    pub count: i32,
    pub limited: bool,
}

impl NuRequirement {
    pub const ALL: [NuRequirement; 13] = [
        NuRequirement {
            path: NuPath::Nd,
            count: 1,
            limited: true,
        },
        NuRequirement {
            path: NuPath::Ei,
            count: 1,
            limited: true,
        },
        NuRequirement {
            path: NuPath::Ic,
            count: 1,
            limited: true,
        },
        NuRequirement {
            path: NuPath::Fq,
            count: 1,
            limited: true,
        },
        NuRequirement {
            path: NuPath::Si,
            count: 1,
            limited: true,
        },
        NuRequirement {
            path: NuPath::Ad,
            count: 1,
            limited: true,
        },
        NuRequirement {
            path: NuPath::Dd,
            count: 1,
            limited: true,
        },
        NuRequirement {
            path: NuPath::Er,
            count: 1,
            limited: true,
        },
        NuRequirement {
            path: NuPath::Wf,
            count: 1,
            limited: false,
        },
        NuRequirement {
            path: NuPath::Wi,
            count: 2,
            limited: false,
        },
        NuRequirement {
            path: NuPath::Wd,
            count: 1,
            limited: false,
        },
        NuRequirement {
            path: NuPath::Ex,
            count: 1,
            limited: false,
        },
        NuRequirement {
            path: NuPath::Ce,
            count: 1,
            limited: false,
        },
    ];
    pub fn title(self) -> String {
        format!("{} ({})", self.path.name(), self.path.short())
    }
}

pub const AREAS_PER_COURSE: usize = 2;

pub fn satisfied_through_coop(note: &str) -> Option<NuPath> {
    if !crate::degree_audit::is_coop_note(note) {
        return None;
    }
    let lower = note.to_ascii_lowercase();
    if lower.contains("integrating knowledge")
        || lower.contains("integration experience")
        || lower.contains("integration of experience")
    {
        return Some(NuPath::Ex);
    }
    if lower.contains("capstone") {
        return Some(NuPath::Ce);
    }
    NuPath::ALL.into_iter().find(|p| {
        let short = p.short().to_ascii_lowercase();
        lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| w == short)
    })
}

#[derive(Clone, Debug, Default)]
pub struct Matching {
    pub assigned: HashMap<NuPath, Vec<String>>,
}

impl Matching {
    pub fn new(courses: &[NuCourse], coop: &HashMap<NuPath, Vec<NuCourse>>) -> Self {
        Self::with_requirements(courses, &NuRequirement::ALL, coop)
    }

    /// `requirements` may be a subset (the optimality check uses only the areas it drew).
    pub fn with_requirements(
        courses: &[NuCourse],
        requirements: &[NuRequirement],
        coop: &HashMap<NuPath, Vec<NuCourse>>,
    ) -> Self {
        let limited: Vec<NuPath> = requirements
            .iter()
            .filter(|r| r.limited)
            .map(|r| r.path)
            .collect();
        let mut assigned = match_limited(&limited, courses);
        for req in requirements.iter().filter(|r| !r.limited) {
            let mut candidates: Vec<&NuCourse> = courses
                .iter()
                .filter(|c| c.paths.contains(&req.path))
                .collect();
            if let Some(extra) = coop.get(&req.path) {
                candidates.extend(extra.iter());
            }
            let mut ordered: Vec<&NuCourse> = candidates
                .iter()
                .copied()
                .filter(|c| c.is_completed)
                .collect();
            ordered.extend(candidates.iter().copied().filter(|c| !c.is_completed));
            let mut seen = HashSet::new();
            let ids: Vec<String> = ordered
                .into_iter()
                .filter(|c| seen.insert(c.id.clone()))
                .take(req.count as usize)
                .map(|c| c.id.clone())
                .collect();
            assigned.insert(req.path, ids);
        }
        Self { assigned }
    }
    pub fn courses_for(&self, path: NuPath) -> Vec<String> {
        self.assigned.get(&path).cloned().unwrap_or_default()
    }
}

fn match_limited(areas: &[NuPath], courses: &[NuCourse]) -> HashMap<NuPath, Vec<String>> {
    let mut load: Vec<Vec<NuPath>> = vec![vec![]; courses.len()];
    let mut owner: HashMap<NuPath, usize> = HashMap::new();
    fn augment(
        area: NuPath,
        courses: &[NuCourse],
        load: &mut [Vec<NuPath>],
        owner: &mut HashMap<NuPath, usize>,
        allowed: &dyn Fn(usize) -> bool,
        visited: &mut HashSet<usize>,
    ) -> bool {
        for index in 0..courses.len() {
            if !allowed(index) || !courses[index].paths.contains(&area) || !visited.insert(index) {
                continue;
            }
            if load[index].len() < AREAS_PER_COURSE {
                load[index].push(area);
                owner.insert(area, index);
                return true;
            }
            let others = load[index].clone();
            for other in others {
                if augment(other, courses, load, owner, allowed, visited) {
                    load[index].retain(|p| *p != other);
                    load[index].push(area);
                    owner.insert(area, index);
                    return true;
                }
            }
        }
        false
    }
    for completed_only in [true, false] {
        for area in areas {
            if owner.contains_key(area) {
                continue;
            }
            let mut visited = HashSet::new();
            let allowed = |index: usize| {
                if completed_only {
                    courses[index].is_completed
                } else {
                    true
                }
            };
            let _ = augment(
                *area,
                courses,
                &mut load,
                &mut owner,
                &allowed,
                &mut visited,
            );
        }
    }
    owner
        .into_iter()
        .map(|(path, index)| (path, vec![courses[index].id.clone()]))
        .collect()
}

/// Banner (or a test double) answering section attributes for a term.
pub trait AttributeSource {
    fn terms(&self) -> Result<Vec<Term>, String>;
    fn section_attributes(
        &self,
        term: &str,
        courses: &[CourseKey],
    ) -> Result<HashMap<CourseKey, HashSet<String>>, String>;
}

/// Attributes of a course's sections, or why they weren't found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttributeLookup {
    Found {
        codes: HashSet<String>,
        term: String,
    },
    NotFound,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttributeEntry {
    codes: Option<Vec<String>>,
    term: Option<String>,
    checked_at: f64,
}

#[derive(Serialize, Deserialize)]
struct AttributeFile {
    version: i32,
    entries: BTreeMap<String, AttributeEntry>,
}

/// Finds NUpath and other attributes from loaded catalogs, then recent Banner terms.
/// Found answers last a month; courses with no sections last a week. Failures are not cached.
pub struct AttributeRepository<F> {
    pub fetcher: F,
    file: PathBuf,
    now: f64,
    window_months: i32,
    found_lifetime: f64,
    not_found_lifetime: f64,
    entries: HashMap<String, AttributeEntry>,
    loaded: bool,
    dirty: bool,
}

impl<F: AttributeSource> AttributeRepository<F> {
    pub const VERSION: i32 = 1;
    const BATCH: usize = 10;

    pub fn new(fetcher: F, directory: impl AsRef<Path>) -> Self {
        Self {
            fetcher,
            file: directory.as_ref().join("course-attributes.json"),
            now: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
            window_months: 36,
            found_lifetime: 30.0 * 24.0 * 3600.0,
            not_found_lifetime: 7.0 * 24.0 * 3600.0,
            entries: HashMap::new(),
            loaded: false,
            dirty: false,
        }
    }

    pub fn lookup(
        &mut self,
        keys: &[CourseKey],
        indexes: &[CatalogIndex],
    ) -> HashMap<CourseKey, AttributeLookup> {
        self.load();
        let mut seen = HashSet::new();
        let mut remaining = Vec::new();
        for key in keys {
            if seen.insert(key.clone()) {
                remaining.push(key.clone());
            }
        }
        let mut results = HashMap::new();
        remaining.retain(|key| {
            if let Some(found) = from_loaded(key, indexes) {
                results.insert(key.clone(), found);
                return false;
            }
            if let Some(cached) = self.cached(key) {
                results.insert(key.clone(), cached);
                return false;
            }
            true
        });
        if remaining.is_empty() {
            return results;
        }
        let mut terms = self.fetcher.terms().unwrap_or_default();
        terms.retain(|term| term.college() == College::Neu);
        for index in indexes {
            if index.catalog.term.college() == College::Neu
                && !terms
                    .iter()
                    .any(|term| term.code == index.catalog.term.code)
            {
                terms.push(index.catalog.term.clone());
            }
        }
        let window = attribute_window(&terms, self.window_months);
        if window.is_empty() {
            for key in remaining {
                results.insert(key, AttributeLookup::Failed);
            }
            return results;
        }
        let loaded: HashSet<String> = indexes
            .iter()
            .map(|index| index.catalog.term.code.clone())
            .collect();
        for term in window.iter().filter(|term| !loaded.contains(&term.code)) {
            let mut start = 0;
            while start < remaining.len() {
                let end = (start + Self::BATCH).min(remaining.len());
                let batch = remaining[start..end].to_vec();
                start = end;
                match self.fetcher.section_attributes(&term.code, &batch) {
                    Ok(found) => {
                        for (key, codes) in found {
                            results.insert(
                                key.clone(),
                                AttributeLookup::Found {
                                    codes: codes.clone(),
                                    term: term.code.clone(),
                                },
                            );
                            self.store_found(&key, &codes, &term.code);
                        }
                    }
                    Err(_) => {
                        for key in &remaining {
                            results
                                .entry(key.clone())
                                .or_insert(AttributeLookup::Failed);
                        }
                        self.save();
                        return results;
                    }
                }
            }
            remaining.retain(|key| !results.contains_key(key));
            if remaining.is_empty() {
                break;
            }
        }
        for key in &remaining {
            if !results.contains_key(key) {
                results.insert(key.clone(), AttributeLookup::NotFound);
                self.store_missing(key);
            }
        }
        self.save();
        results
    }

    fn cached(&self, key: &CourseKey) -> Option<AttributeLookup> {
        let entry = self.entries.get(&key.to_string())?;
        let age = self.now - entry.checked_at;
        match (&entry.codes, &entry.term) {
            (Some(codes), Some(term)) if age < self.found_lifetime => {
                Some(AttributeLookup::Found {
                    codes: codes.iter().cloned().collect(),
                    term: term.clone(),
                })
            }
            (None, None) if age < self.not_found_lifetime => Some(AttributeLookup::NotFound),
            _ => None,
        }
    }

    fn store_found(&mut self, key: &CourseKey, codes: &HashSet<String>, term: &str) {
        let mut listed: Vec<String> = codes.iter().cloned().collect();
        listed.sort();
        self.entries.insert(
            key.to_string(),
            AttributeEntry {
                codes: Some(listed),
                term: Some(term.to_string()),
                checked_at: self.now,
            },
        );
        self.dirty = true;
    }

    fn store_missing(&mut self, key: &CourseKey) {
        self.entries.insert(
            key.to_string(),
            AttributeEntry {
                codes: None,
                term: None,
                checked_at: self.now,
            },
        );
        self.dirty = true;
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Ok(bytes) = std::fs::read(&self.file) else {
            return;
        };
        let Ok(stored) = serde_json::from_slice::<AttributeFile>(&bytes) else {
            return;
        };
        if stored.version != Self::VERSION {
            return;
        }
        self.entries = stored.entries.into_iter().collect();
    }

    fn save(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let stored = AttributeFile {
            version: Self::VERSION,
            entries: self
                .entries
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };
        let Ok(bytes) = serde_json::to_vec(&stored) else {
            return;
        };
        if let Some(parent) = self.file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.file, bytes);
    }
}

/// Newest loaded term that has sections of `key`, using the course's attribute codes.
pub fn from_loaded(key: &CourseKey, indexes: &[CatalogIndex]) -> Option<AttributeLookup> {
    for index in indexes {
        if index.sections_for(key).is_empty() {
            continue;
        }
        let codes = index
            .course(key)
            .map(|course| course.attributes.iter().cloned().collect())
            .unwrap_or_default();
        return Some(AttributeLookup::Found {
            codes,
            term: index.catalog.term.code.clone(),
        });
    }
    None
}

fn attribute_window(terms: &[Term], months: i32) -> Vec<Term> {
    let mut sorted = terms.to_vec();
    sorted.sort_by(|a, b| b.code.cmp(&a.code));
    let Some(newest) = sorted.first() else {
        return vec![];
    };
    let oldest = crate::offering::month_index(&newest.code) - months;
    sorted
        .into_iter()
        .filter(|term| crate::offering::month_index(&term.code) >= oldest)
        .collect()
}
