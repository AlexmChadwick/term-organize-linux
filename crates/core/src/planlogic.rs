//! Prerequisite checks, credit totals, and schedule autofill.
//!
//! Port of `DegreePlanLogic.swift`. Placement tests are treated as met, because a plan
//! does not record them. A course counts as taken when an equivalent was.

use crate::equivalences::CourseEquivalences;
use crate::index::CatalogIndex;
use crate::model::*;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrereqStatus {
    Met,
    Missing(String),
    Unknown,
}

pub fn courses_before(plan: &DegreePlan, term_index: usize) -> HashSet<CourseKey> {
    let mut keys = plan.completed_keys();
    for term in plan.terms.iter().take(term_index) {
        keys.extend(term.courses.iter().filter_map(|c| c.key.clone()));
    }
    keys.into_iter().collect()
}

pub fn prerequisite_status(
    key: &CourseKey,
    term_index: usize,
    plan: &DegreePlan,
    catalog: Option<&CatalogIndex>,
    lookup: Option<&dyn Fn(&CourseKey) -> Option<Option<Requisite>>>,
    equivalences: &CourseEquivalences,
) -> PrereqStatus {
    let taken = courses_before(plan, term_index);
    let counting = equivalences.merging(&CourseEquivalences::from_declarations(
        &plan.course_equivalences,
    ));
    if let Some(listed) = catalog.and_then(|c| c.course(key)) {
        if listed.details_loaded {
            return status(listed.prereqs.as_ref(), &taken, &counting);
        }
    }
    if let Some(lookup) = lookup {
        if let Some(known) = lookup(key) {
            return status(known.as_ref(), &taken, &counting);
        }
    }
    PrereqStatus::Unknown
}

pub fn status(
    requisite: Option<&Requisite>,
    taken: &HashSet<CourseKey>,
    equivalences: &CourseEquivalences,
) -> PrereqStatus {
    let Some(requisite) = requisite else {
        return PrereqStatus::Met;
    };
    match unmet_summary(requisite, taken, equivalences) {
        None => PrereqStatus::Met,
        Some(text) => PrereqStatus::Missing(text),
    }
}

pub fn unmet_summary(
    requisite: &Requisite,
    taken: &HashSet<CourseKey>,
    equivalences: &CourseEquivalences,
) -> Option<String> {
    unmet(requisite, &equivalences.expand(taken))
}

pub fn missing_count(
    requisite: &Requisite,
    taken: &HashSet<CourseKey>,
    equivalences: &CourseEquivalences,
) -> i32 {
    count(requisite, &equivalences.expand(taken))
}

fn unmet(requisite: &Requisite, covered: &HashSet<CourseKey>) -> Option<String> {
    match requisite {
        Requisite::Course(key) => {
            if covered.contains(key) {
                None
            } else {
                Some(key.to_string())
            }
        }
        Requisite::Test { .. } => None,
        Requisite::And(items) => {
            let parts = unique(items.iter().filter_map(|i| unmet(i, covered)).collect());
            match parts.len() {
                0 | 1 => parts.into_iter().next(),
                2 | 3 => Some(
                    parts
                        .iter()
                        .map(|p| parenthesized(p))
                        .collect::<Vec<_>>()
                        .join(" and "),
                ),
                _ => Some(format!(
                    "{} and {} more",
                    parts
                        .iter()
                        .take(2)
                        .map(|p| parenthesized(p))
                        .collect::<Vec<_>>()
                        .join(", "),
                    parts.len() - 2
                )),
            }
        }
        Requisite::Or(items) => {
            if items.iter().any(|i| unmet(i, covered).is_none()) {
                return None;
            }
            let courses: Vec<CourseKey> = items
                .iter()
                .filter_map(|i| match i {
                    Requisite::Course(k) => Some(k.clone()),
                    _ => None,
                })
                .collect();
            if courses.len() == items.len() {
                let names = unique(courses.iter().map(|k| k.to_string()).collect());
                return Some(if names.len() <= 3 {
                    names.join(" or ")
                } else {
                    format!("{}, {} or {} others", names[0], names[1], names.len() - 2)
                });
            }
            let best = items.iter().min_by_key(|i| count(i, covered))?;
            let text = unmet(best, covered)?;
            Some(format!("{} or another option", parenthesized(&text)))
        }
    }
}

fn count(requisite: &Requisite, covered: &HashSet<CourseKey>) -> i32 {
    match requisite {
        Requisite::Course(key) => {
            if covered.contains(key) {
                0
            } else {
                1
            }
        }
        Requisite::Test { .. } => 0,
        Requisite::And(items) => items.iter().map(|i| count(i, covered)).sum(),
        Requisite::Or(items) => items.iter().map(|i| count(i, covered)).min().unwrap_or(0),
    }
}

fn parenthesized(text: &str) -> String {
    if text.contains(" or ") || text.contains(" and ") || text.ends_with(" more") {
        format!("({text})")
    } else {
        text.to_string()
    }
}

fn unique(texts: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    texts
        .into_iter()
        .filter(|t| seen.insert(t.clone()))
        .collect()
}

pub fn duplicate_keys(plan: &DegreePlan) -> HashSet<CourseKey> {
    let planned: Vec<&DegreePlanCourse> =
        plan.terms.iter().flat_map(|t| t.courses.iter()).collect();
    let mut duplicates = HashSet::new();
    for (index, course) in planned.iter().enumerate() {
        let Some(key) = &course.key else { continue };
        let repeats_planned = planned[..index]
            .iter()
            .any(|other| other.key.as_ref() == Some(key) && !counts_separately(other, course));
        let repeats_completed = plan
            .completed
            .iter()
            .any(|other| other.key.as_ref() == Some(key) && !counts_separately(other, course));
        if repeats_planned || repeats_completed {
            duplicates.insert(key.clone());
        }
    }
    duplicates
}

fn counts_separately(a: &DegreePlanCourse, b: &DegreePlanCourse) -> bool {
    crate::degree_audit::counts_separately(&a.title, a.credits, &b.title, b.credits)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleSuggestion {
    pub plan_name: String,
    pub semester: Option<String>,
    pub title: String,
    pub detail: String,
    pub is_exact_term: bool,
    pub courses: Vec<CourseKey>,
    pub unavailable: Vec<CourseKey>,
}

pub fn month_index_of(term: &Term) -> i32 {
    let season = match term.season() {
        Season::Winter => PlanSeason::Winter,
        Season::Spring => PlanSeason::Spring,
        Season::Summer1 => PlanSeason::SummerA,
        Season::SummerFull => PlanSeason::SummerFull,
        Season::Summer2 => PlanSeason::SummerB,
        Season::Fall => PlanSeason::Fall,
    };
    term.calendar_year() * 12 + season.start_month() - 1
}

pub fn schedule_suggestions(
    term: &Term,
    degree_plans: &[DegreePlan],
    schedule_plans: &[SchedulePlan],
    excluding_plan: Option<Uuid>,
    already: &HashSet<CourseKey>,
    index: Option<&CatalogIndex>,
) -> Vec<ScheduleSuggestion> {
    let target = month_index_of(term);
    let mut plans: Vec<&DegreePlan> = degree_plans.iter().collect();
    plans.sort_by(|a, b| {
        let ap = if a.is_primary { 0 } else { 1 };
        let bp = if b.is_primary { 0 } else { 1 };
        ap.cmp(&bp).then(b.updated_at.cmp(&a.updated_at))
    });
    let mut exact = vec![];
    let mut closest = vec![];
    for plan in plans {
        let done = plan.completed_keys();
        let mut candidates = vec![];
        for plan_term in &plan.terms {
            let mut seen = HashSet::new();
            let keys: Vec<CourseKey> = plan_term
                .courses
                .iter()
                .filter_map(|c| c.key.clone())
                .filter(|k| !done.contains(k) && !already.contains(k) && seen.insert(k.clone()))
                .collect();
            if keys.is_empty() {
                continue;
            }
            let distance = (plan_term.month_index() - target).abs();
            candidates.push((plan_term, keys, distance));
        }
        let Some((plan_term, keys, distance)) = candidates
            .into_iter()
            .min_by(|a, b| a.2.cmp(&b.2).then(b.0.sort_key().cmp(&a.0.sort_key())))
        else {
            continue;
        };
        let (offered, unavailable) = split_offered(&keys, term, index);
        if offered.is_empty() && unavailable.is_empty() {
            continue;
        }
        let is_exact = plan_term.matches_term(term);
        let suggestion = ScheduleSuggestion {
            plan_name: plan.name.clone(),
            semester: Some(plan_term.short_name()),
            title: format!("From “{}” · {}", plan.name, plan_term.short_name()),
            detail: if is_exact {
                "Your plan for this term".into()
            } else if distance == 0 {
                "Planned for this time".into()
            } else {
                "Closest semester you planned".into()
            },
            is_exact_term: is_exact,
            courses: offered,
            unavailable,
        };
        if is_exact {
            exact.push(suggestion);
        } else {
            closest.push(suggestion);
        }
    }
    let mut copies = vec![];
    let mut schedules: Vec<&SchedulePlan> = schedule_plans
        .iter()
        .filter(|p| {
            p.term_code == term.code && Some(p.id) != excluding_plan && !p.courses.is_empty()
        })
        .collect();
    schedules.sort_by(|a, b| {
        b.updated_at
            .partial_cmp(&a.updated_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for plan in schedules.into_iter().take(2) {
        let keys: Vec<CourseKey> = plan
            .courses
            .iter()
            .map(|c| c.key.clone())
            .filter(|k| !already.contains(k))
            .collect();
        if keys.is_empty() {
            continue;
        }
        copies.push(ScheduleSuggestion {
            plan_name: plan.name.clone(),
            semester: None,
            title: format!("Copy “{}”", plan.name),
            detail: if plan.is_primary {
                "Your schedule for this term".into()
            } else {
                "Another schedule for this term".into()
            },
            is_exact_term: true,
            courses: keys,
            unavailable: vec![],
        });
    }
    exact.extend(copies);
    exact.extend(closest);
    exact
}

fn split_offered(
    keys: &[CourseKey],
    term: &Term,
    index: Option<&CatalogIndex>,
) -> (Vec<CourseKey>, Vec<CourseKey>) {
    let Some(index) = index else {
        return (keys.to_vec(), vec![]);
    };
    if index.catalog.term.code != term.code {
        return (keys.to_vec(), vec![]);
    }
    let mut offered = vec![];
    let mut unavailable = vec![];
    for key in keys {
        if index.sections_for(key).is_empty() {
            unavailable.push(key.clone());
        } else {
            offered.push(key.clone());
        }
    }
    (offered, unavailable)
}

pub fn new_degree_plan_draft(
    name: &str,
    source: Option<&DegreePlan>,
    schedule_plans: &[SchedulePlan],
    now_year: i32,
    now_month: u32,
) -> DegreePlan {
    let mut draft = DegreePlan {
        name: name.into(),
        ..DegreePlan::default()
    };
    if let Some(source) = source {
        draft.program = source.program.clone();
        draft.student_name = source.student_name.clone();
        draft.student_id = source.student_id.clone();
        draft.catalog_year = source.catalog_year;
        draft.catalog_program_path = source.catalog_program_path.clone();
        draft.catalog_program_title = source.catalog_program_title.clone();
        draft.concentration_id = source.concentration_id.clone();
        draft.substitutions = source
            .substitutions
            .iter()
            .cloned()
            .map(|mut s| {
                s.id = Uuid::new_v4();
                s
            })
            .collect();
        draft.course_equivalences = source
            .course_equivalences
            .iter()
            .cloned()
            .map(|mut e| {
                e.id = Uuid::new_v4();
                e
            })
            .collect();
        draft.completed = source
            .completed
            .iter()
            .cloned()
            .map(|mut c| {
                c.id = Uuid::new_v4();
                c
            })
            .collect();
        draft.terms = source
            .terms
            .iter()
            .cloned()
            .map(|mut term| {
                term.id = Uuid::new_v4();
                term.courses = term
                    .courses
                    .into_iter()
                    .map(|mut c| {
                        c.id = Uuid::new_v4();
                        c
                    })
                    .collect();
                term
            })
            .collect();
    }
    let done = draft.completed_keys();
    for plan in schedule_plans
        .iter()
        .filter(|p| p.is_primary && !p.courses.is_empty())
    {
        let term = Term {
            code: plan.term_code.clone(),
            banner_description: String::new(),
        };
        if term_has_ended(&term, now_year, now_month) {
            continue;
        }
        let season = match term.season() {
            Season::Winter => PlanSeason::Winter,
            Season::Spring => PlanSeason::Spring,
            Season::Summer1 => PlanSeason::SummerA,
            Season::SummerFull => PlanSeason::SummerFull,
            Season::Summer2 => PlanSeason::SummerB,
            Season::Fall => PlanSeason::Fall,
        };
        let year = term.calendar_year();
        let position = if let Some(i) = draft
            .terms
            .iter()
            .position(|t| t.season == season && t.year == year)
        {
            i
        } else {
            draft.terms.push(DegreePlanTerm::new(season, year));
            draft.terms.len() - 1
        };
        let present: HashSet<CourseKey> = draft.terms[position]
            .courses
            .iter()
            .filter_map(|c| c.key.clone())
            .collect();
        for course in &plan.courses {
            if present.contains(&course.key) || done.contains(&course.key) {
                continue;
            }
            draft.terms[position]
                .courses
                .push(DegreePlanCourse::catalog(
                    course.key.clone(),
                    course.key.to_string(),
                    4.0,
                ));
        }
    }
    draft.sort_terms();
    draft
}

fn term_has_ended(term: &Term, year: i32, month: u32) -> bool {
    let end = match term.season() {
        Season::Fall => (term.calendar_year(), 12),
        Season::Winter => (term.calendar_year(), 3),
        Season::Spring => (term.calendar_year(), 5),
        Season::Summer1 => (term.calendar_year(), 7),
        Season::SummerFull => (term.calendar_year(), 8),
        Season::Summer2 => (term.calendar_year(), 8),
    };
    (year, month) > end
}

fn credits_phrase(value: f64) -> String {
    let number = credit_number(value);
    if (value - 1.0).abs() < 1e-9 {
        format!("{number} credit")
    } else {
        format!("{number} credits")
    }
}

fn courses_phrase(count: usize) -> String {
    if count == 1 {
        "1 course".into()
    } else {
        format!("{count} courses")
    }
}

pub fn completed_line(plan: &DegreePlan) -> String {
    if plan.completed.is_empty() {
        "No completed courses yet".into()
    } else {
        format!("{} completed", credits_phrase(plan.completed_credits()))
    }
}

/// Banner code of a semester at Northeastern (`202730` for Spring 2027).
pub fn banner_code(term: &DegreePlanTerm) -> String {
    match term.season {
        PlanSeason::Fall => format!("{}10", term.year + 1),
        PlanSeason::Winter => format!("{}20", term.year),
        PlanSeason::Spring => format!("{}30", term.year),
        PlanSeason::SummerA => format!("{}40", term.year),
        PlanSeason::SummerFull => format!("{}50", term.year),
        PlanSeason::SummerB => format!("{}60", term.year),
    }
}

pub fn semester_has_ended(term: &DegreePlanTerm, now: f64) -> bool {
    crate::calendar::approximate_window(&banner_code(term)).is_some_and(|(_, end)| end < now)
}

pub fn next_summary(plan: &DegreePlan, now: f64) -> Option<String> {
    let next = plan
        .terms
        .iter()
        .find(|term| !semester_has_ended(term, now) && !term.courses.is_empty())?;
    Some(format!(
        "{} · {} · {}",
        next.name(),
        credits_phrase(next.credits()),
        courses_phrase(next.courses.len())
    ))
}

pub fn next_line(plan: &DegreePlan, now: f64) -> String {
    if let Some(summary) = next_summary(plan, now) {
        return format!("Next: {summary}");
    }
    if plan.terms.is_empty() {
        return "No semesters planned yet".into();
    }
    if plan.terms.iter().all(|term| semester_has_ended(term, now)) {
        return "No upcoming semesters".into();
    }
    "No courses planned yet".into()
}

pub fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// A schedule opened for one semester of a degree plan.
#[derive(Clone, Debug)]
pub struct SemesterSchedule {
    pub plan: SchedulePlan,
    pub is_new: bool,
    pub course_count: usize,
}

pub fn schedule_for_semester(
    data: &mut UserData,
    semester: &DegreePlanTerm,
    term_code: &str,
) -> Option<SemesterSchedule> {
    let mut seen = HashSet::new();
    let keys: Vec<CourseKey> = semester
        .courses
        .iter()
        .filter_map(|course| course.key.clone())
        .filter(|key| seen.insert(key.clone()))
        .collect();
    if keys.is_empty() {
        return None;
    }
    let wanted: HashSet<CourseKey> = keys.iter().cloned().collect();
    if let Some(existing) = data.plans.iter().find(|plan| {
        plan.term_code == term_code
            && plan
                .courses
                .iter()
                .map(|course| course.key.clone())
                .collect::<HashSet<_>>()
                == wanted
    }) {
        return Some(SemesterSchedule {
            plan: existing.clone(),
            is_new: false,
            course_count: keys.len(),
        });
    }
    let names: Vec<String> = data
        .plans
        .iter()
        .filter(|plan| plan.term_code == term_code)
        .map(|plan| plan.name.clone())
        .collect();
    let schedule = SchedulePlan {
        id: Uuid::new_v4(),
        name: unique_schedule_name(&format!("{} plan", semester.name()), &names),
        term_code: term_code.to_string(),
        courses: keys
            .iter()
            .enumerate()
            .map(|(index, key)| PlanCourse {
                key: key.clone(),
                is_required: true,
                excluded_crns: HashSet::new(),
                color_index: index as i32,
            })
            .collect(),
        selected_crns: vec![],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: false,
        created_at: 0.0,
        updated_at: 0.0,
    };
    let id = schedule.id;
    data.add_schedule(schedule);
    let stored = data.plans.iter().find(|plan| plan.id == id).cloned()?;
    Some(SemesterSchedule {
        plan: stored,
        is_new: true,
        course_count: keys.len(),
    })
}

fn unique_schedule_name(base: &str, existing: &[String]) -> String {
    let taken: HashSet<String> = existing
        .iter()
        .map(|name| name.trim().to_ascii_lowercase())
        .collect();
    let wanted = base.trim();
    if !taken.contains(&wanted.to_ascii_lowercase()) {
        return wanted.to_string();
    }
    let mut number = 2;
    while taken.contains(&format!("{wanted} {number}").to_ascii_lowercase()) {
        number += 1;
    }
    format!("{wanted} {number}")
}

pub fn next_semesters(
    plan: &DegreePlan,
    count: usize,
    year: i32,
    month: u32,
) -> Vec<DegreePlanTerm> {
    let order = [
        PlanSeason::Spring,
        PlanSeason::SummerA,
        PlanSeason::SummerB,
        PlanSeason::Fall,
    ];
    let mut year = year;
    let mut season_index = if month < 5 {
        0
    } else if month < 7 {
        1
    } else if month < 9 {
        2
    } else {
        3
    };
    if let Some(last) = plan.terms.last() {
        if let Some(i) = order.iter().position(|s| *s == last.season) {
            year = last.year;
            season_index = i + 1;
        }
    }
    let existing: HashSet<(PlanSeason, i32)> =
        plan.terms.iter().map(|t| (t.season, t.year)).collect();
    let mut result = vec![];
    while result.len() < count {
        if season_index >= order.len() {
            season_index = 0;
            year += 1;
        }
        let candidate = DegreePlanTerm::new(order[season_index], year);
        if !existing.contains(&(candidate.season, candidate.year)) {
            result.push(candidate);
        }
        season_index += 1;
    }
    result
}
