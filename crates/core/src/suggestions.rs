//! Guesses which current course a retired course became.
//! Port of `EquivalenceSuggestions.swift`.

use crate::model::{CourseKey, Requisite};
use crate::titles::{self, Title};
use crate::CourseEquivalences;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub struct EquivalenceSuggestion {
    pub course: CourseKey,
    pub candidate: CourseKey,
    pub together: i32,
    pub total: i32,
    pub similar_title: Option<String>,
    pub title_score: Option<f64>,
}

impl EquivalenceSuggestion {
    pub fn id(&self) -> String {
        format!("{}|{}", self.course, self.candidate)
    }
    /// The share of the retired course's dependents that list the candidate beside it.
    /// 0 when the suggestion rests on the titles alone.
    pub fn confidence(&self) -> f64 {
        if self.total > 0 {
            self.together as f64 / self.total as f64
        } else {
            0.0
        }
    }
    pub fn evidence(&self) -> String {
        let listing = if self.together > 0 {
            Some(if self.total == 1 {
                format!(
                    "Listed together in the prerequisites of the 1 course that names {}",
                    self.course
                )
            } else {
                format!(
                    "Listed together in {} of the {} prerequisites that name {}",
                    self.together, self.total, self.course
                )
            })
        } else {
            None
        };
        match (listing, &self.similar_title) {
            (Some(listing), Some(_)) => format!("{listing} · similar title"),
            (Some(listing), None) => listing,
            (None, Some(title)) => format!("Similar title: {title}"),
            (None, None) => String::new(),
        }
    }
    pub fn is_listed_together(&self) -> bool {
        self.together > 0
    }
}

pub fn suggestions(
    retired: &CourseKey,
    prerequisites: &[(CourseKey, Requisite)],
    is_current: impl Fn(&CourseKey) -> bool,
    known: &CourseEquivalences,
    minimum_together: i32,
    minimum_share: f64,
    limit: usize,
    title: impl Fn(&CourseKey) -> Option<String>,
    title_candidates: &[CourseKey],
    minimum_title: f64,
) -> Vec<EquivalenceSuggestion> {
    if limit == 0 {
        return vec![];
    }
    let (listed, dependents) = listed_together(
        retired,
        prerequisites,
        &is_current,
        known,
        minimum_together,
        minimum_share,
    );
    let similar = similar_titles(retired, title_candidates, known, &title, minimum_title);
    let mut by_prerequisites: Vec<EquivalenceSuggestion> = listed
        .into_iter()
        .map(|mut suggestion| {
            if let Some((title, score)) = similar.get(&suggestion.candidate) {
                suggestion.similar_title = Some(title.clone());
                suggestion.title_score = Some(*score);
            }
            suggestion
        })
        .collect();
    by_prerequisites.sort_by(|a, b| {
        b.together
            .cmp(&a.together)
            .then(a.candidate.cmp(&b.candidate))
    });
    let listed_keys: HashSet<CourseKey> = by_prerequisites
        .iter()
        .map(|s| s.candidate.clone())
        .collect();
    let mut by_title: Vec<EquivalenceSuggestion> = similar
        .iter()
        .filter(|(key, _)| !listed_keys.contains(*key))
        .map(|(key, (title, score))| EquivalenceSuggestion {
            course: retired.clone(),
            candidate: key.clone(),
            together: 0,
            total: dependents,
            similar_title: Some(title.clone()),
            title_score: Some(*score),
        })
        .collect();
    by_title.sort_by(|a, b| {
        b.title_score
            .unwrap_or(0.0)
            .partial_cmp(&a.title_score.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.candidate.cmp(&b.candidate))
    });
    let free = limit.saturating_sub(by_prerequisites.len());
    let reserved = usize::from(limit >= 2 && !by_prerequisites.is_empty());
    let title_slots = by_title.len().min(free.max(reserved));
    let mut out: Vec<_> = by_prerequisites
        .into_iter()
        .take(limit - title_slots)
        .collect();
    out.extend(by_title.into_iter().take(title_slots));
    out
}

fn listed_together(
    retired: &CourseKey,
    prerequisites: &[(CourseKey, Requisite)],
    is_current: &impl Fn(&CourseKey) -> bool,
    known: &CourseEquivalences,
    minimum_together: i32,
    minimum_share: f64,
) -> (Vec<EquivalenceSuggestion>, i32) {
    let mut dependents = HashSet::new();
    let mut together: HashMap<CourseKey, HashSet<CourseKey>> = HashMap::new();
    for (course, requisite) in prerequisites {
        if course == retired {
            continue;
        }
        let mut mentions = false;
        if let Some(flat) = requisite.simplified() {
            visit(&flat, retired, course, &mut mentions, &mut together);
        }
        if mentions {
            dependents.insert(course.clone());
        }
    }
    let total = dependents.len() as i32;
    if total == 0 {
        return (vec![], 0);
    }
    let found = together
        .into_iter()
        .filter_map(|(candidate, courses)| {
            if !is_current(&candidate) || known.are_equivalent(retired, &candidate) {
                return None;
            }
            let count = courses.len() as i32;
            if count < minimum_together || (count as f64) / (total as f64) < minimum_share {
                return None;
            }
            Some(EquivalenceSuggestion {
                course: retired.clone(),
                candidate,
                together: count,
                total,
                similar_title: None,
                title_score: None,
            })
        })
        .collect();
    (found, total)
}

fn visit(
    node: &Requisite,
    retired: &CourseKey,
    course: &CourseKey,
    mentions: &mut bool,
    together: &mut HashMap<CourseKey, HashSet<CourseKey>>,
) {
    match node {
        Requisite::Course(key) => {
            if key == retired {
                *mentions = true;
            }
        }
        Requisite::Test { .. } => {}
        Requisite::And(items) => items
            .iter()
            .for_each(|item| visit(item, retired, course, mentions, together)),
        Requisite::Or(items) => {
            let alternatives: Vec<CourseKey> = items
                .iter()
                .filter_map(|item| match item {
                    Requisite::Course(key) => Some(key.clone()),
                    _ => None,
                })
                .collect();
            if alternatives.iter().any(|key| key == retired) {
                for other in alternatives.iter().filter(|key| *key != retired) {
                    together
                        .entry(other.clone())
                        .or_default()
                        .insert(course.clone());
                }
            }
            items
                .iter()
                .for_each(|item| visit(item, retired, course, mentions, together));
        }
    }
}

fn similar_titles(
    retired: &CourseKey,
    candidates: &[CourseKey],
    known: &CourseEquivalences,
    title: &impl Fn(&CourseKey) -> Option<String>,
    minimum: f64,
) -> HashMap<CourseKey, (String, f64)> {
    let mut found = HashMap::new();
    if candidates.is_empty() {
        return found;
    }
    let Some(retired_text) = title(retired) else {
        return found;
    };
    let retired_title = Title::parse(&retired_text);
    if !retired_title.is_distinctive() {
        return found;
    }
    for candidate in candidates {
        if candidate == retired
            || found.contains_key(candidate)
            || known.are_equivalent(retired, candidate)
        {
            continue;
        }
        let Some(text) = title(candidate)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if let Some(score) =
            titles::similarity_titles(&retired_title, &Title::parse(&text), minimum)
        {
            found.insert(candidate.clone(), (text, score));
        }
    }
    found
}
