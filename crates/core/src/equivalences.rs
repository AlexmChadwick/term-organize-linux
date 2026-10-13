//! Symmetric, transitive groups of interchangeable courses.

use crate::model::{CourseEquivalence, CourseKey, Requisite};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CourseEquivalences {
    pub groups: Vec<Vec<CourseKey>>,
    index: BTreeMap<CourseKey, usize>,
}

impl CourseEquivalences {
    pub fn none() -> Self { Self::default() }
    pub fn from_groups(input: &[Vec<CourseKey>]) -> Self {
        let mut parent: BTreeMap<CourseKey, CourseKey> = BTreeMap::new();
        fn add(parent: &mut BTreeMap<CourseKey, CourseKey>, key: CourseKey) {
            parent.entry(key.clone()).or_insert(key);
        }
        fn find(parent: &BTreeMap<CourseKey, CourseKey>, key: &CourseKey) -> CourseKey {
            let mut k = key.clone();
            while parent.get(&k).is_some_and(|p| p != &k) {
                k = parent[&k].clone();
            }
            k
        }
        fn union(parent: &mut BTreeMap<CourseKey, CourseKey>, a: &CourseKey, b: &CourseKey) {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra != rb { parent.insert(rb, ra); }
        }
        for group in input {
            if let Some(first) = group.first() {
                add(&mut parent, first.clone());
                for other in group.iter().skip(1) {
                    add(&mut parent, other.clone());
                    union(&mut parent, first, other);
                }
            }
        }
        // Path compression for find after unions.
        let keys: Vec<_> = parent.keys().cloned().collect();
        let mut buckets: BTreeMap<CourseKey, Vec<CourseKey>> = BTreeMap::new();
        for key in keys {
            let root = find(&parent, &key);
            buckets.entry(root).or_default().push(key);
        }
        let mut built: Vec<Vec<CourseKey>> = buckets.into_values().filter(|g| g.len() > 1).map(|mut g| { g.sort(); g }).collect();
        built.sort_by(|a, b| a[0].cmp(&b[0]));
        let mut index = BTreeMap::new();
        for (i, group) in built.iter().enumerate() {
            for key in group { index.insert(key.clone(), i); }
        }
        Self { groups: built, index }
    }
    pub fn from_pairs(pairs: &[(CourseKey, CourseKey)]) -> Self {
        Self::from_groups(&pairs.iter().map(|(a, b)| vec![a.clone(), b.clone()]).collect::<Vec<_>>())
    }
    pub fn from_declarations(items: &[CourseEquivalence]) -> Self {
        Self::from_pairs(&items.iter().map(|e| (e.course.clone(), e.counts_as.clone())).collect::<Vec<_>>())
    }
    pub fn is_empty(&self) -> bool { self.groups.is_empty() }
    pub fn equivalents(&self, key: &CourseKey) -> BTreeSet<CourseKey> {
        let Some(i) = self.index.get(key) else { return BTreeSet::new() };
        self.groups[*i].iter().filter(|k| *k != key).cloned().collect()
    }
    pub fn group_of(&self, key: &CourseKey) -> Vec<CourseKey> {
        if let Some(i) = self.index.get(key) {
            self.groups[*i].clone()
        } else {
            vec![key.clone()]
        }
    }
    pub fn are_equivalent(&self, a: &CourseKey, b: &CourseKey) -> bool {
        a == b || self.index.get(a).is_some_and(|i| self.index.get(b) == Some(i))
    }
    pub fn expand(&self, taken: &HashSet<CourseKey>) -> HashSet<CourseKey> {
        let mut out = taken.clone();
        for key in taken {
            out.extend(self.group_of(key));
        }
        out
    }
    pub fn merging(&self, other: &CourseEquivalences) -> Self {
        let mut groups = self.groups.clone();
        groups.extend(other.groups.clone());
        Self::from_groups(&groups)
    }
    pub fn satisfier<'a>(&self, required: &CourseKey, taken: &'a HashSet<CourseKey>) -> Option<&'a CourseKey> {
        if taken.contains(required) { return taken.get(required); }
        taken.iter().find(|k| self.are_equivalent(required, k))
    }
}

pub fn requisite_satisfied(req: &Requisite, taken: &HashSet<CourseKey>, equivalences: &CourseEquivalences, assume_tests: bool) -> bool {
    let expanded = equivalences.expand(taken);
    req.is_satisfied(&expanded, assume_tests)
}

/// Same number and the same title, not special topics, not the same subject.
/// A cross-list is an exact title match after collapsing whitespace, which is how
/// `CourseEquivalences(crossListedIn:)` decides two sections are one course.
pub fn cross_list_equivalent(a_subject: &str, a_number: &str, a_title: &str, a_topics: bool, b_subject: &str, b_number: &str, b_title: &str, b_topics: bool) -> bool {
    if a_topics || b_topics || a_number != b_number || a_subject == b_subject {
        return false;
    }
    let norm = |title: &str| title.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    let (a, b) = (norm(a_title), norm(b_title));
    !a.is_empty() && a == b
}

pub fn titles_alike(a: &str, b: &str) -> bool {
    crate::titles::similarity(a, b).is_some()
}

impl CourseEquivalences {
    /// Cross-listed sections that are the same course under more than one subject.
    pub fn from_cross_listed(index: &crate::index::CatalogIndex) -> Self {
        let mut pairs = Vec::new();
        for section in &index.catalog.sections {
            if section.cross_list_crns.is_empty() {
                continue;
            }
            let key = section.course_key();
            if index.course(&key).is_some_and(|course| course.special_topics) {
                continue;
            }
            for crn in &section.cross_list_crns {
                let Some(other) = index.section_crn(crn) else { continue };
                let other_key = other.course_key();
                if other_key.number != key.number || other_key.subject == key.subject {
                    continue;
                }
                if index.course(&other_key).is_some_and(|course| course.special_topics) {
                    continue;
                }
                let title = index.course(&key).map(|c| c.title.as_str()).unwrap_or(section.title.as_str());
                let other_title = index.course(&other_key).map(|c| c.title.as_str()).unwrap_or(other.title.as_str());
                if cross_list_equivalent(&key.subject, &key.number, title, false, &other_key.subject, &other_key.number, other_title, false) {
                    pairs.push((key.clone(), other_key));
                }
            }
        }
        Self::from_pairs(&pairs)
    }
}
