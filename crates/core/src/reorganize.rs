// SPDX-License-Identifier: AGPL-3.0-only
//! Whole-plan course moves across terms that are already on the plan.
//!
//! Unofficial. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.
//!
//! [`crate::reorder::suggestions`] moves blocking courses out of one semester and can append a
//! semester. This pass does not. It only moves a course into another term the plan already has,
//! using the catalogs handed to it (for the MCP server, the bundled Fall 2026 and Spring 2027
//! snapshots) as the offering history.

use crate::index::CatalogIndex;
use crate::model::{
    CourseKey, DegreePlan, DegreePlanCourse, PlanSeason, Requisite, ScheduleFilters,
    SchedulePreferences, Term,
};
use crate::offering::{self, OfferingHistory};
use crate::schedule::{self, Limits, ScheduleCourseInput, ScheduleSort};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_MOVES: usize = 32;
pub const NOTE: &str = "Courses move only among terms already on the plan, so the plan does not gain a semester after the last one. Offering checks use the catalogs passed in (the bundled snapshots for the MCP server). This whole-plan pass is separate from the one-semester reorder. A chosen CRN stays on its course. A pinned course moves only when the destination catalog has that same CRN; otherwise it stays and is unresolved.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedTerm {
    pub season: PlanSeason,
    pub year: i32,
    pub label: String,
    pub code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotCourse {
    pub subject: String,
    pub number: String,
    pub crn: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanSlot {
    pub term: String,
    pub courses: Vec<SlotCourse>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CourseMove {
    pub subject: String,
    pub number: String,
    pub from: String,
    pub to: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnresolvedCourse {
    pub subject: String,
    pub number: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReorganizeOutcome {
    pub terms: Vec<PlanSlot>,
    pub moves: Vec<CourseMove>,
    pub unresolved: Vec<UnresolvedCourse>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DegreeReorganize {
    pub plan: DegreePlan,
    pub moves: Vec<CourseMove>,
    pub unresolved: Vec<UnresolvedCourse>,
    pub notes: Vec<String>,
}

#[derive(Clone)]
struct WorkTerm {
    label: String,
    season: PlanSeason,
    year: i32,
    sort: i32,
    exact: Option<usize>,
    courses: Vec<SlotCourse>,
}

/// `Fall 2026`, `Spring 2027`, or a six-digit Banner code such as `202710`.
pub fn parse_term_label(raw: &str) -> Option<ParsedTerm> {
    let raw = raw.trim();
    if raw.len() == 6 && raw.chars().all(|c| c.is_ascii_digit()) {
        let term = Term {
            code: raw.to_string(),
            banner_description: String::new(),
        };
        let season = offering::plan_season(term.season());
        let year = term.calendar_year();
        return Some(ParsedTerm {
            season,
            year,
            label: format!("{} {year}", season.export_name()),
            code: Some(raw.to_string()),
        });
    }
    let mut parts: Vec<&str> = raw.split_whitespace().collect();
    let year = parts.pop()?.parse().ok()?;
    let season = PlanSeason::parse_export(&parts.join(" "))?;
    Some(ParsedTerm {
        season,
        year,
        label: format!("{} {year}", season.export_name()),
        code: None,
    })
}

pub fn catalog_for_term(
    catalogs: &[CatalogIndex],
    season: PlanSeason,
    year: i32,
) -> Option<&CatalogIndex> {
    catalogs.iter().find(|index| {
        let term = &index.catalog.term;
        term.calendar_year() == year && season.matches_term_season(term.season())
    })
}

pub fn reorganize(plan: &[PlanSlot], catalogs: &[CatalogIndex]) -> ReorganizeOutcome {
    let history = history_from(catalogs);
    let mut terms = Vec::new();
    let mut notes = vec![NOTE.to_string()];
    for slot in plan {
        let Some(parsed) = parse_term_label(&slot.term) else {
            notes.push(format!(
                "Left \"{}\" unchanged because the term name was not recognized.",
                slot.term
            ));
            terms.push(WorkTerm {
                label: slot.term.clone(),
                season: PlanSeason::Fall,
                year: 0,
                sort: i32::MAX,
                exact: None,
                courses: slot.courses.clone(),
            });
            continue;
        };
        let exact = catalog_for_term(catalogs, parsed.season, parsed.year)
            .and_then(|found| catalogs.iter().position(|index| std::ptr::eq(index, found)));
        terms.push(WorkTerm {
            label: slot.term.clone(),
            season: parsed.season,
            year: parsed.year,
            sort: parsed.year * 12 + parsed.season.start_month(),
            exact,
            courses: slot.courses.clone(),
        });
    }

    let mut moves = Vec::new();
    let mut unresolved = Vec::new();
    move_unoffered(
        &mut terms,
        catalogs,
        &history,
        &mut moves,
        &mut unresolved,
        &mut notes,
    );
    move_clashes(
        &mut terms,
        catalogs,
        &history,
        &mut moves,
        &mut unresolved,
        &mut notes,
    );

    ReorganizeOutcome {
        terms: terms
            .into_iter()
            .map(|term| PlanSlot {
                term: term.label,
                courses: term.courses,
            })
            .collect(),
        moves,
        unresolved,
        notes,
    }
}

pub fn reorganize_degree_plan(plan: &DegreePlan, catalogs: &[CatalogIndex]) -> DegreeReorganize {
    let slots: Vec<PlanSlot> = plan
        .terms
        .iter()
        .map(|term| PlanSlot {
            term: term.name(),
            courses: term
                .courses
                .iter()
                .filter_map(|course| {
                    let key = course.key.clone()?;
                    Some(SlotCourse {
                        subject: key.subject,
                        number: key.number,
                        crn: course.crn.clone(),
                    })
                })
                .collect(),
        })
        .collect();
    let outcome = reorganize(&slots, catalogs);
    if outcome.moves.is_empty() {
        return DegreeReorganize {
            plan: plan.clone(),
            moves: Vec::new(),
            unresolved: outcome.unresolved,
            notes: outcome.notes,
        };
    }
    let mut pool: Vec<DegreePlanCourse> = plan
        .terms
        .iter()
        .flat_map(|term| term.courses.iter().filter(|course| course.key.is_some()))
        .cloned()
        .collect();
    let mut next = plan.clone();
    for (index, slot) in outcome.terms.iter().enumerate() {
        let mut rebuilt: Vec<DegreePlanCourse> = plan.terms[index]
            .courses
            .iter()
            .filter(|course| course.key.is_none())
            .cloned()
            .collect();
        for slot_course in &slot.courses {
            if let Some(course) = take_matching_course(&mut pool, slot_course) {
                rebuilt.push(course);
            }
        }
        next.terms[index].courses = rebuilt;
    }
    DegreeReorganize {
        plan: next,
        moves: outcome.moves,
        unresolved: outcome.unresolved,
        notes: outcome.notes,
    }
}

fn history_from(catalogs: &[CatalogIndex]) -> OfferingHistory {
    let mut checked_terms: Vec<String> = catalogs
        .iter()
        .map(|index| index.catalog.term.code.clone())
        .collect();
    checked_terms.sort();
    checked_terms.dedup();
    let mut offered: BTreeMap<CourseKey, BTreeSet<String>> = BTreeMap::new();
    let mut keys = BTreeSet::new();
    for index in catalogs {
        let code = index.catalog.term.code.clone();
        for course in &index.catalog.courses {
            keys.insert(course.key());
        }
        for section in &index.catalog.sections {
            let key = section.course_key();
            keys.insert(key.clone());
            offered.entry(key).or_default().insert(code.clone());
        }
    }
    OfferingHistory {
        checked_terms,
        offered,
        checked_keys: Some(keys),
    }
}

fn course_key(course: &SlotCourse) -> CourseKey {
    CourseKey {
        subject: course.subject.trim().to_ascii_uppercase(),
        number: course.number.trim().to_string(),
    }
}

/// Pull the degree-plan row for this slot without giving it another row's CRN.
fn take_matching_course(
    pool: &mut Vec<DegreePlanCourse>,
    slot: &SlotCourse,
) -> Option<DegreePlanCourse> {
    let same_key = |course: &DegreePlanCourse| {
        course.key.as_ref().is_some_and(|key| {
            key.subject.eq_ignore_ascii_case(&slot.subject)
                && key.number.eq_ignore_ascii_case(&slot.number)
        })
    };
    let pinned = crate::planlogic::chosen_crn(slot.crn.as_deref());
    let position = if let Some(crn) = pinned {
        pool.iter()
            .position(|course| same_key(course) && course.crn.as_deref() == Some(crn))
    } else {
        pool.iter().position(|course| {
            same_key(course) && crate::planlogic::chosen_crn(course.crn.as_deref()).is_none()
        })
    };
    let position = position.or_else(|| pool.iter().position(same_key))?;
    Some(pool.remove(position))
}

fn is_offered(
    key: &CourseKey,
    term: &WorkTerm,
    catalogs: &[CatalogIndex],
    history: &OfferingHistory,
) -> bool {
    if term.year == 0 {
        return true;
    }
    if let Some(index) = term.exact.map(|index| &catalogs[index]) {
        return !index.sections_for(key).is_empty();
    }
    history.is_likely_offered(key, term.season).unwrap_or(true)
}

fn move_unoffered(
    terms: &mut [WorkTerm],
    catalogs: &[CatalogIndex],
    history: &OfferingHistory,
    moves: &mut Vec<CourseMove>,
    unresolved: &mut Vec<UnresolvedCourse>,
    notes: &mut Vec<String>,
) {
    for index in 0..terms.len() {
        let mut course_index = 0;
        while course_index < terms[index].courses.len() && moves.len() < MAX_MOVES {
            let key = course_key(&terms[index].courses[course_index]);
            if is_offered(&key, &terms[index], catalogs, history) {
                course_index += 1;
                continue;
            }
            let reason = format!(
                "{} {} has no sections in {}.",
                key.subject, key.number, terms[index].label
            );
            if let Err(blocked) = relocate(
                terms,
                catalogs,
                history,
                index,
                course_index,
                &reason,
                moves,
            ) {
                unresolved.push(UnresolvedCourse {
                    subject: key.subject,
                    number: key.number,
                    reason: blocked.clone(),
                });
                notes.push(blocked);
                course_index += 1;
                continue;
            }
        }
    }
}

fn move_clashes(
    terms: &mut [WorkTerm],
    catalogs: &[CatalogIndex],
    history: &OfferingHistory,
    moves: &mut Vec<CourseMove>,
    unresolved: &mut Vec<UnresolvedCourse>,
    notes: &mut Vec<String>,
) {
    for _ in 0..MAX_MOVES {
        if moves.len() >= MAX_MOVES {
            break;
        }
        if let Some((term_index, course_index, reason)) = first_crn_clash(terms, catalogs) {
            if let Err(blocked) = relocate(
                terms,
                catalogs,
                history,
                term_index,
                course_index,
                &reason,
                moves,
            ) {
                let key = course_key(&terms[term_index].courses[course_index]);
                unresolved.push(UnresolvedCourse {
                    subject: key.subject,
                    number: key.number,
                    reason: blocked.clone(),
                });
                notes.push(blocked);
                break;
            }
            continue;
        }
        if let Some((term_index, course_index, reason)) = first_generated_clash(terms, catalogs) {
            if let Err(blocked) = relocate(
                terms,
                catalogs,
                history,
                term_index,
                course_index,
                &reason,
                moves,
            ) {
                let key = course_key(&terms[term_index].courses[course_index]);
                unresolved.push(UnresolvedCourse {
                    subject: key.subject,
                    number: key.number,
                    reason: blocked.clone(),
                });
                notes.push(blocked);
                break;
            }
            continue;
        }
        break;
    }
}

fn first_crn_clash(
    terms: &[WorkTerm],
    catalogs: &[CatalogIndex],
) -> Option<(usize, usize, String)> {
    for (term_index, term) in terms.iter().enumerate() {
        let Some(catalog) = term.exact.map(|index| &catalogs[index]) else {
            continue;
        };
        for left in 0..term.courses.len() {
            for right in (left + 1)..term.courses.len() {
                if sections_conflict(catalog, &term.courses[left], &term.courses[right]) {
                    let course = &term.courses[right];
                    return Some((
                        term_index,
                        right,
                        format!(
                            "{} {} overlaps {} {} in {}.",
                            course.subject,
                            course.number,
                            term.courses[left].subject,
                            term.courses[left].number,
                            term.label
                        ),
                    ));
                }
            }
        }
    }
    None
}

fn sections_conflict(catalog: &CatalogIndex, left: &SlotCourse, right: &SlotCourse) -> bool {
    let (Some(left_crn), Some(right_crn)) = (left.crn.as_deref(), right.crn.as_deref()) else {
        return false;
    };
    let (Some(left_section), Some(right_section)) = (
        catalog.section_crn(left_crn),
        catalog.section_crn(right_crn),
    ) else {
        return false;
    };
    left_section.conflicts_with(right_section)
}

fn first_generated_clash(
    terms: &[WorkTerm],
    catalogs: &[CatalogIndex],
) -> Option<(usize, usize, String)> {
    for (term_index, term) in terms.iter().enumerate() {
        if term.courses.len() < 2 || term.courses.len() > 6 {
            continue;
        }
        if term.courses.iter().any(|course| course.crn.is_some()) {
            continue;
        }
        let Some(catalog) = term.exact.map(|index| &catalogs[index]) else {
            continue;
        };
        let inputs: Vec<ScheduleCourseInput> = term
            .courses
            .iter()
            .map(|course| {
                let key = course_key(course);
                ScheduleCourseInput::new(key.clone(), true, catalog.sections_for(&key))
            })
            .filter(|input| !input.sections.is_empty())
            .collect();
        if inputs.len() != term.courses.len() {
            continue;
        }
        let generated = schedule::generate(
            &inputs,
            &ScheduleFilters::default(),
            &SchedulePreferences::default(),
            ScheduleSort::Best,
            &Limits::blocker_probe(),
        );
        if !generated.schedules.is_empty() {
            continue;
        }
        let Some(pair) = generated.diagnostics.conflicting_pairs.first() else {
            continue;
        };
        let position = term
            .courses
            .iter()
            .position(|course| course_key(course) == pair.second)
            .or_else(|| {
                term.courses
                    .iter()
                    .position(|course| course_key(course) == pair.first)
            });
        let Some(position) = position else {
            continue;
        };
        let course = &term.courses[position];
        return Some((
            term_index,
            position,
            format!(
                "{} {} clashes with another course in {}.",
                course.subject, course.number, term.label
            ),
        ));
    }
    None
}

fn relocate(
    terms: &mut [WorkTerm],
    catalogs: &[CatalogIndex],
    history: &OfferingHistory,
    from: usize,
    course_index: usize,
    reason: &str,
    moves: &mut Vec<CourseMove>,
) -> Result<(), String> {
    if moves.len() >= MAX_MOVES || course_index >= terms[from].courses.len() {
        return Err("No further courses can move.".into());
    }
    let course = terms[from].courses[course_index].clone();
    let key = course_key(&course);
    let incoming = course.crn.as_deref();
    let dest = destination(terms, catalogs, history, from, &key, incoming)?;
    if crate::planlogic::chosen_crn(incoming).is_some()
        && crn_for_destination(&course, &key, &terms[dest], catalogs).is_none()
    {
        return Err(format!(
            "{} {} stays pinned to CRN {} because {} does not offer that section.",
            key.subject,
            key.number,
            incoming.unwrap_or("").trim(),
            terms[dest].label
        ));
    }
    terms[from].courses.remove(course_index);
    terms[dest].courses.push(course);
    moves.push(CourseMove {
        subject: key.subject,
        number: key.number,
        from: terms[from].label.clone(),
        to: terms[dest].label.clone(),
        reason: reason.to_string(),
    });
    Ok(())
}

fn destination(
    terms: &[WorkTerm],
    catalogs: &[CatalogIndex],
    history: &OfferingHistory,
    from: usize,
    key: &CourseKey,
    incoming: Option<&str>,
) -> Result<usize, String> {
    let origin = terms[from].sort;
    let mut candidates: Vec<usize> = (0..terms.len()).filter(|index| *index != from).collect();
    candidates.sort_by_key(|index| {
        let delta = terms[*index].sort - origin;
        if delta > 0 {
            (0, delta)
        } else {
            (1, -delta)
        }
    });
    let mut blocked: Option<String> = None;
    for index in candidates {
        if !is_offered(key, &terms[index], catalogs, history) {
            continue;
        }
        let existing: Vec<Option<&str>> = terms[index]
            .courses
            .iter()
            .filter(|course| course_key(course) == *key)
            .map(|course| course.crn.as_deref())
            .collect();
        if !crate::planlogic::repeated_course_allowed(existing, incoming) {
            continue;
        }
        if let Some(reason) = prerequisite_block(terms, catalogs, from, index, key, incoming) {
            if blocked.is_none() {
                blocked = Some(reason);
            }
            continue;
        }
        return Ok(index);
    }
    Err(blocked.unwrap_or_else(|| {
        format!(
            "{} {} stays in {} because no other term on the plan offers it.",
            key.subject, key.number, terms[from].label
        )
    }))
}

/// Prerequisite order is kept only among courses on the plan. A prerequisite
/// that is not on the plan (completed, transferred, or AP) does not block a
/// move. A planned prerequisite must stay in a strictly earlier term, and a
/// later planned course that requires the moved course must stay later.
/// Corequisites are not part of this check.
fn prerequisite_block(
    terms: &[WorkTerm],
    catalogs: &[CatalogIndex],
    from: usize,
    dest: usize,
    key: &CourseKey,
    incoming: Option<&str>,
) -> Option<String> {
    let simulated = simulate_move(terms, from, dest, key, incoming);
    if move_keeps_order(&simulated, catalogs, key) {
        None
    } else {
        Some(format!(
            "Moving {} {} would put it ahead of a prerequisite that is also in this plan.",
            key.subject, key.number
        ))
    }
}

fn simulate_move(
    terms: &[WorkTerm],
    from: usize,
    dest: usize,
    key: &CourseKey,
    incoming: Option<&str>,
) -> Vec<WorkTerm> {
    let mut terms = terms.to_vec();
    let Some(position) = terms[from].courses.iter().position(|course| {
        course_key(course) == *key
            && match crate::planlogic::chosen_crn(incoming) {
                Some(crn) => course.crn.as_deref() == Some(crn),
                None => crate::planlogic::chosen_crn(course.crn.as_deref()).is_none(),
            }
    }) else {
        return terms;
    };
    let course = terms[from].courses.remove(position);
    terms[dest].courses.push(course);
    terms
}

fn move_keeps_order(terms: &[WorkTerm], catalogs: &[CatalogIndex], moved: &CourseKey) -> bool {
    let placed = placement(terms);
    if let Some(Some(requisite)) = known_prereq(catalogs, moved) {
        let course_sort = placed.get(moved).copied().unwrap_or(0);
        if !prerequisite_holds(requisite, course_sort, &placed) {
            return false;
        }
    }
    for (other, sort) in &placed {
        if other == moved {
            continue;
        }
        let Some(Some(requisite)) = known_prereq(catalogs, other) else {
            continue;
        };
        if requisite_references(requisite, moved) && !prerequisite_holds(requisite, *sort, &placed)
        {
            return false;
        }
    }
    true
}

fn placement(terms: &[WorkTerm]) -> HashMap<CourseKey, i32> {
    let mut placed = HashMap::new();
    for term in terms {
        for course in &term.courses {
            placed.entry(course_key(course)).or_insert(term.sort);
        }
    }
    placed
}

/// An off-plan course prerequisite holds. A course that is on the plan holds
/// only when its term is strictly earlier.
fn prerequisite_holds(
    requisite: &Requisite,
    course_sort: i32,
    placed: &HashMap<CourseKey, i32>,
) -> bool {
    match requisite {
        Requisite::Course(key) => match placed.get(key) {
            None => true,
            Some(earlier) => *earlier < course_sort,
        },
        Requisite::Test { .. } => true,
        Requisite::And(items) => items
            .iter()
            .all(|item| prerequisite_holds(item, course_sort, placed)),
        Requisite::Or(items) => {
            items.is_empty()
                || items
                    .iter()
                    .any(|item| prerequisite_holds(item, course_sort, placed))
        }
    }
}

fn requisite_references(requisite: &Requisite, key: &CourseKey) -> bool {
    match requisite {
        Requisite::Course(course) => course == key,
        Requisite::Test { .. } => false,
        Requisite::And(items) | Requisite::Or(items) => {
            items.iter().any(|item| requisite_references(item, key))
        }
    }
}

fn known_prereq<'a>(
    catalogs: &'a [CatalogIndex],
    key: &CourseKey,
) -> Option<Option<&'a Requisite>> {
    for index in catalogs {
        if let Some(course) = index.course(key) {
            if course.details_loaded {
                return Some(course.prereqs.as_ref());
            }
        }
    }
    None
}

fn crn_for_destination(
    course: &SlotCourse,
    key: &CourseKey,
    dest: &WorkTerm,
    catalogs: &[CatalogIndex],
) -> Option<String> {
    let crn = course.crn.clone()?;
    let index = dest.exact.map(|index| &catalogs[index])?;
    let section = index.section_crn(&crn)?;
    if section.course_key() == *key {
        Some(crn)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TermCatalog;
    use serde_json::json;

    fn catalog(
        code: &str,
        description: &str,
        rows: &[(&str, &str, &str, i32, i32)],
    ) -> CatalogIndex {
        let courses: Vec<_> = rows
            .iter()
            .map(|(subject, number, _, _, _)| {
                json!({
                    "subject": subject,
                    "number": number,
                    "title": format!("{subject} {number}"),
                    "minCredits": 4,
                    "maxCredits": 4
                })
            })
            .collect();
        let sections: Vec<_> = rows
            .iter()
            .map(|(subject, number, crn, start, end)| {
                json!({
                    "crn": crn,
                    "subject": subject,
                    "number": number,
                    "meetings": [{ "days": 2, "start": start, "end": end }]
                })
            })
            .collect();
        let value = json!({
            "version": 1,
            "term": { "code": code, "bannerDescription": description },
            "courses": courses,
            "sections": sections,
            "subjects": [],
            "attributes": [],
            "campuses": []
        });
        let catalog: TermCatalog = serde_json::from_value(value).unwrap();
        CatalogIndex::build(catalog)
    }

    #[test]
    fn moves_an_unoffered_course_into_an_existing_later_term() {
        let fall = catalog(
            "202710",
            "Fall 2026 Semester",
            &[("CS", "2500", "10001", 540, 600)],
        );
        let spring = catalog(
            "202730",
            "Spring 2027 Semester",
            &[
                ("CS", "2500", "20001", 540, 600),
                ("CS", "2510", "20002", 660, 720),
            ],
        );
        let outcome = reorganize(
            &[
                PlanSlot {
                    term: "Fall 2026".into(),
                    courses: vec![SlotCourse {
                        subject: "CS".into(),
                        number: "2510".into(),
                        crn: None,
                    }],
                },
                PlanSlot {
                    term: "Spring 2027".into(),
                    courses: vec![],
                },
            ],
            &[fall, spring],
        );
        assert_eq!(outcome.terms.len(), 2, "the pass does not add a semester");
        assert!(outcome.terms[0].courses.is_empty());
        assert_eq!(outcome.terms[1].courses[0].number, "2510");
        assert_eq!(outcome.moves.len(), 1);
        assert_eq!(outcome.moves[0].from, "Fall 2026");
        assert_eq!(outcome.moves[0].to, "Spring 2027");
    }

    #[test]
    fn leaves_the_course_when_the_plan_has_no_term_that_offers_it() {
        let fall = catalog(
            "202710",
            "Fall 2026 Semester",
            &[("CS", "2500", "10001", 540, 600)],
        );
        let spring = catalog(
            "202730",
            "Spring 2027 Semester",
            &[("CS", "2510", "20002", 660, 720)],
        );
        let outcome = reorganize(
            &[PlanSlot {
                term: "Fall 2026".into(),
                courses: vec![SlotCourse {
                    subject: "CS".into(),
                    number: "2510".into(),
                    crn: None,
                }],
            }],
            &[fall, spring],
        );
        assert_eq!(outcome.terms.len(), 1);
        assert!(outcome.moves.is_empty());
        assert_eq!(outcome.terms[0].courses[0].number, "2510");
    }

    #[test]
    fn moves_a_clashing_section_into_the_other_term() {
        let rows = &[
            ("CS", "2500", "10001", 540, 600),
            ("CS", "2510", "10002", 570, 630),
        ];
        let fall = catalog("202710", "Fall 2026 Semester", rows);
        let spring = catalog(
            "202730",
            "Spring 2027 Semester",
            &[
                ("CS", "2500", "20001", 540, 600),
                ("CS", "2510", "20002", 540, 600),
            ],
        );
        let outcome = reorganize(
            &[
                PlanSlot {
                    term: "202710".into(),
                    courses: vec![
                        SlotCourse {
                            subject: "CS".into(),
                            number: "2500".into(),
                            crn: Some("10001".into()),
                        },
                        SlotCourse {
                            subject: "CS".into(),
                            number: "2510".into(),
                            crn: Some("10002".into()),
                        },
                    ],
                },
                PlanSlot {
                    term: "202730".into(),
                    courses: vec![],
                },
            ],
            &[fall, spring],
        );
        assert!(outcome.moves.is_empty());
        assert_eq!(outcome.terms[0].courses.len(), 2);
        assert!(outcome.terms[1].courses.is_empty());
        assert_eq!(outcome.terms[0].courses[0].crn.as_deref(), Some("10001"));
        assert_eq!(outcome.terms[0].courses[1].crn.as_deref(), Some("10002"));
        assert_eq!(outcome.unresolved.len(), 1);
        assert_eq!(outcome.unresolved[0].number, "2510");
        assert!(outcome.unresolved[0].reason.contains("pinned to CRN 10002"));
    }

    #[test]
    fn moves_an_unpinned_clash_into_the_other_term() {
        let rows = &[
            ("CS", "2500", "10001", 540, 600),
            ("CS", "2510", "10002", 570, 630),
        ];
        let fall = catalog("202710", "Fall 2026 Semester", rows);
        let spring = catalog(
            "202730",
            "Spring 2027 Semester",
            &[
                ("CS", "2500", "20001", 540, 600),
                ("CS", "2510", "20002", 540, 600),
            ],
        );
        let outcome = reorganize(
            &[
                PlanSlot {
                    term: "202710".into(),
                    courses: vec![
                        SlotCourse {
                            subject: "CS".into(),
                            number: "2500".into(),
                            crn: None,
                        },
                        SlotCourse {
                            subject: "CS".into(),
                            number: "2510".into(),
                            crn: None,
                        },
                    ],
                },
                PlanSlot {
                    term: "202730".into(),
                    courses: vec![],
                },
            ],
            &[fall, spring],
        );
        assert_eq!(outcome.terms[0].courses.len(), 1);
        assert_eq!(outcome.terms[1].courses.len(), 1);
        assert!(outcome.terms[1].courses[0].crn.is_none());
        assert_eq!(outcome.moves.len(), 1);
        assert_eq!(outcome.moves[0].to, "202730");
    }

    #[test]
    fn distinct_topic_crns_stay_on_their_own_rows() {
        let fall = catalog(
            "202710",
            "Fall 2026 Semester",
            &[
                ("CS", "5976", "44444", 540, 600),
                ("CS", "5976", "55555", 780, 840),
            ],
        );
        let spring = catalog(
            "202730",
            "Spring 2027 Semester",
            &[("CS", "2510", "20002", 660, 720)],
        );
        let mut networks = DegreePlanCourse::catalog(
            CourseKey {
                subject: "CS".into(),
                number: "5976".into(),
            },
            "Networks",
            4.0,
        );
        networks.crn = Some("44444".into());
        let mut graphics = DegreePlanCourse::catalog(
            CourseKey {
                subject: "CS".into(),
                number: "5976".into(),
            },
            "Graphics",
            4.0,
        );
        graphics.crn = Some("55555".into());
        let fundamentals = DegreePlanCourse::catalog(
            CourseKey {
                subject: "CS".into(),
                number: "2510".into(),
            },
            "Fundamentals 2",
            4.0,
        );
        let mut plan = DegreePlan {
            terms: vec![
                crate::model::DegreePlanTerm {
                    courses: vec![networks, graphics, fundamentals],
                    ..crate::model::DegreePlanTerm::new(PlanSeason::Fall, 2026)
                },
                crate::model::DegreePlanTerm::new(PlanSeason::Spring, 2027),
            ],
            ..DegreePlan::default()
        };
        plan.sort_terms();
        let result = reorganize_degree_plan(&plan, &[fall, spring]);
        assert_eq!(result.moves.len(), 1);
        assert_eq!(result.moves[0].number, "2510");
        let fall = result
            .plan
            .terms
            .iter()
            .find(|term| term.season == PlanSeason::Fall)
            .unwrap();
        assert_eq!(fall.courses.len(), 2);
        assert_eq!(fall.courses[0].title, "Networks");
        assert_eq!(fall.courses[0].crn.as_deref(), Some("44444"));
        assert_eq!(fall.courses[1].title, "Graphics");
        assert_eq!(fall.courses[1].crn.as_deref(), Some("55555"));
    }

    #[test]
    fn degree_plan_keeps_course_identity_when_it_moves() {
        let fall = catalog(
            "202710",
            "Fall 2026 Semester",
            &[("CS", "2500", "10001", 540, 600)],
        );
        let spring = catalog(
            "202730",
            "Spring 2027 Semester",
            &[
                ("CS", "2500", "20001", 540, 600),
                ("CS", "2510", "20002", 660, 720),
            ],
        );
        let course = DegreePlanCourse::catalog(
            CourseKey {
                subject: "CS".into(),
                number: "2510".into(),
            },
            "Fundamentals 2",
            4.0,
        );
        let id = course.id;
        let mut plan = DegreePlan {
            terms: vec![
                crate::model::DegreePlanTerm {
                    courses: vec![course],
                    ..crate::model::DegreePlanTerm::new(PlanSeason::Fall, 2026)
                },
                crate::model::DegreePlanTerm::new(PlanSeason::Spring, 2027),
            ],
            ..DegreePlan::default()
        };
        plan.sort_terms();
        let result = reorganize_degree_plan(&plan, &[fall, spring]);
        assert_eq!(result.moves.len(), 1);
        let spring = result
            .plan
            .terms
            .iter()
            .find(|term| term.season == PlanSeason::Spring)
            .unwrap();
        assert_eq!(spring.courses[0].id, id);
        assert_eq!(spring.courses[0].title, "Fundamentals 2");
    }

    fn catalog_requiring(
        code: &str,
        description: &str,
        rows: &[(&str, &str, &str, i32, i32, Option<(&str, &str)>)],
    ) -> CatalogIndex {
        let courses: Vec<_> = rows
            .iter()
            .map(|(subject, number, _, _, _, prereq)| {
                let mut course = json!({
                    "subject": subject,
                    "number": number,
                    "title": format!("{subject} {number}"),
                    "minCredits": 4,
                    "maxCredits": 4,
                    "detailsLoaded": true
                });
                if let Some((ps, pn)) = prereq {
                    course["prereqs"] = json!({ "s": ps, "n": pn });
                }
                course
            })
            .collect();
        let sections: Vec<_> = rows
            .iter()
            .map(|(subject, number, crn, start, end, _)| {
                json!({
                    "crn": crn,
                    "subject": subject,
                    "number": number,
                    "meetings": [{ "days": 2, "start": start, "end": end }]
                })
            })
            .collect();
        let value = json!({
            "version": 1,
            "term": { "code": code, "bannerDescription": description },
            "courses": courses,
            "sections": sections,
            "subjects": [],
            "attributes": [],
            "campuses": []
        });
        CatalogIndex::build(serde_json::from_value::<TermCatalog>(value).unwrap())
    }

    #[test]
    fn an_off_plan_prerequisite_does_not_block_the_move() {
        let fall = catalog_requiring("202710", "Fall 2026 Semester", &[]);
        let spring = catalog_requiring(
            "202730",
            "Spring 2027 Semester",
            &[("CS", "4500", "20001", 540, 600, Some(("CS", "3500")))],
        );
        let outcome = reorganize(
            &[
                PlanSlot {
                    term: "202710".into(),
                    courses: vec![SlotCourse {
                        subject: "CS".into(),
                        number: "4500".into(),
                        crn: None,
                    }],
                },
                PlanSlot {
                    term: "202730".into(),
                    courses: vec![],
                },
            ],
            &[fall, spring],
        );
        assert!(outcome.unresolved.is_empty());
        assert_eq!(outcome.moves.len(), 1);
        assert_eq!(outcome.moves[0].number, "4500");
        assert_eq!(outcome.moves[0].to, "202730");
    }

    #[test]
    fn a_planned_prerequisite_must_stay_in_an_earlier_term() {
        let fall = catalog_requiring("202710", "Fall 2026 Semester", &[]);
        let spring = catalog_requiring(
            "202730",
            "Spring 2027 Semester",
            &[
                ("CS", "3500", "20001", 540, 600, None),
                ("CS", "4500", "20002", 660, 720, Some(("CS", "3500"))),
            ],
        );
        let outcome = reorganize(
            &[
                PlanSlot {
                    term: "202710".into(),
                    courses: vec![SlotCourse {
                        subject: "CS".into(),
                        number: "4500".into(),
                        crn: None,
                    }],
                },
                PlanSlot {
                    term: "202730".into(),
                    courses: vec![SlotCourse {
                        subject: "CS".into(),
                        number: "3500".into(),
                        crn: None,
                    }],
                },
            ],
            &[fall, spring],
        );
        assert!(outcome.moves.is_empty());
        assert_eq!(outcome.unresolved.len(), 1);
        assert_eq!(outcome.unresolved[0].number, "4500");
        assert!(outcome.unresolved[0]
            .reason
            .contains("prerequisite that is also in this plan"));
    }
}
