//! Ways to move a course out of a semester that does not teach it.
//! Port of `DegreePlanReordering.swift`.

use crate::model::*;
use crate::offering::OfferingHistory;
use crate::planlogic::{self, status, PrereqStatus};
use crate::CourseEquivalences;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    pub course: CourseKey,
    pub from: Uuid,
    pub to: Uuid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub id: String,
    pub moves: Vec<Move>,
    pub title: String,
    pub reasons: Vec<String>,
    pub adds_semester: Option<DegreePlanTerm>,
}

pub const MAX_SUGGESTIONS: usize = 5;

#[derive(Clone)]
enum Need {
    Unknown,
    Nothing,
    Needs(Requisite),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    Met,
    Missing,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    moves: i32,
    target: i32,
    uncertainty: i32,
    order: i32,
    detail: i32,
}

struct Candidate {
    suggestion: Suggestion,
    rank: Rank,
}

pub fn suggestions(
    plan: &DegreePlan,
    term_id: Uuid,
    blocking: &[CourseKey],
    history: &OfferingHistory,
    prerequisites: impl Fn(&CourseKey) -> Option<Option<Requisite>>,
    equivalences: &CourseEquivalences,
    max_credits: f64,
) -> Vec<Suggestion> {
    let mut plan = plan.clone();
    plan.sort_terms();
    let Some(origin) = plan.terms.iter().position(|term| term.id == term_id) else { return vec![] };
    let mut requirements = HashMap::new();
    for key in plan.terms.iter().flat_map(|term| term.courses.iter().filter_map(|c| c.key.clone())) {
        requirements.entry(key.clone()).or_insert_with(|| match prerequisites(&key) {
            None => Need::Unknown,
            Some(None) => Need::Nothing,
            Some(Some(requisite)) => Need::Needs(requisite),
        });
    }
    let equivalences = equivalences.merging(&CourseEquivalences::from_declarations(&plan.course_equivalences));
    let before = evaluate(&plan, &requirements, &equivalences);
    let engine = Engine { plan, origin, history, requirements, equivalences, max_credits, before };
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let blocking_set: HashSet<CourseKey> = blocking.iter().cloned().collect();
    for (order, key) in blocking.iter().enumerate() {
        if !seen.insert(key.clone()) {
            continue;
        }
        let Some(course) = engine.plan.terms[origin].courses.iter().find(|c| c.key.as_ref() == Some(key)).cloned() else { continue };
        candidates.extend(engine.candidates(&course, order as i32, &blocking_set));
    }
    candidates.sort_by_key(|candidate| candidate.rank);
    candidates.into_iter().take(MAX_SUGGESTIONS).map(|c| c.suggestion).collect()
}

pub fn apply(suggestion: &Suggestion, plan: &mut DegreePlan) {
    let mut updated = plan.clone();
    let mut semester_id: HashMap<Uuid, Uuid> = HashMap::new();
    if let Some(semester) = &suggestion.adds_semester {
        if let Some(existing) = updated.terms.iter().find(|term| term.season == semester.season && term.year == semester.year) {
            semester_id.insert(semester.id, existing.id);
        } else {
            updated.terms.push(semester.clone());
            updated.sort_terms();
        }
    }
    for move_ in &suggestion.moves {
        let to = semester_id.get(&move_.to).copied().unwrap_or(move_.to);
        if !perform_move(&move_.course, move_.from, to, &mut updated) {
            return;
        }
    }
    *plan = updated;
}

fn perform_move(course: &CourseKey, from: Uuid, to: Uuid, plan: &mut DegreePlan) -> bool {
    let Some(source) = plan.terms.iter().position(|term| term.id == from) else { return false };
    if !plan.terms.iter().any(|term| term.id == to) {
        return false;
    }
    let Some(position) = plan.terms[source].courses.iter().position(|item| item.key.as_ref() == Some(course)) else { return false };
    let moved = plan.terms[source].courses.remove(position);
    let target = plan.terms.iter().position(|term| term.id == to).unwrap();
    plan.terms[target].courses.push(moved);
    true
}

fn evaluate(plan: &DegreePlan, requirements: &HashMap<CourseKey, Need>, equivalences: &CourseEquivalences) -> HashMap<Uuid, Check> {
    let mut taken: HashSet<CourseKey> = plan.completed_keys().into_iter().collect();
    let mut checks = HashMap::new();
    for term in &plan.terms {
        for course in &term.courses {
            let Some(key) = &course.key else { continue };
            checks.insert(course.id, check(key, &taken, requirements, equivalences));
        }
        taken.extend(term.courses.iter().filter_map(|c| c.key.clone()));
    }
    checks
}

fn check(key: &CourseKey, taken: &HashSet<CourseKey>, requirements: &HashMap<CourseKey, Need>, equivalences: &CourseEquivalences) -> Check {
    match requirements.get(key).unwrap_or(&Need::Unknown) {
        Need::Unknown => Check::Unknown,
        Need::Nothing => Check::Met,
        Need::Needs(requisite) => {
            if matches!(status(Some(requisite), taken, equivalences), PrereqStatus::Met) { Check::Met } else { Check::Missing }
        }
    }
}

struct Engine<'a> {
    plan: DegreePlan,
    origin: usize,
    history: &'a OfferingHistory,
    requirements: HashMap<CourseKey, Need>,
    equivalences: CourseEquivalences,
    max_credits: f64,
    before: HashMap<Uuid, Check>,
}

impl<'a> Engine<'a> {
    fn checks_after(&self, changed: &DegreePlan, moved: &HashSet<Uuid>) -> Option<HashMap<Uuid, Check>> {
        let after = evaluate(changed, &self.requirements, &self.equivalences);
        for (id, check) in &after {
            if moved.contains(id) {
                if *check == Check::Missing {
                    return None;
                }
            } else if self.before.get(id) == Some(&Check::Met) && *check == Check::Missing {
                return None;
            }
        }
        Some(after)
    }

    fn credits_label(&self, credits: f64) -> String {
        format!("{} {}", crate::model::credit_number(credits), if (credits - 1.0).abs() < 1e-9 { "credit" } else { "credits" })
    }

    fn uncertainty(&self, key: &CourseKey, season: PlanSeason) -> Option<i32> {
        match self.history.is_likely_offered(key, season) {
            Some(false) => None,
            Some(true) => Some(0),
            None => Some(1),
        }
    }

    fn offering_reason(&self, key: &CourseKey, season: PlanSeason, named: bool) -> String {
        let text = self.history.summary_in(key, season);
        if named { format!("{key}: {}", lower_first(&text)) } else { text }
    }

    fn prerequisite_reason(&self, key: &CourseKey, check: Check, was_met: bool, named: bool) -> String {
        let text = match (self.requirements.get(key).unwrap_or(&Need::Unknown), check) {
            (_, Check::Unknown) => "Prerequisites couldn’t be checked".to_string(),
            (Need::Nothing, _) => "No prerequisites".into(),
            (_, Check::Met) => if was_met { "Prerequisites still met".into() } else { "Prerequisites met by then".into() },
            (_, Check::Missing) => "Prerequisites not met".into(),
        };
        if named { format!("{key}: {}", lower_first(&text)) } else { text }
    }

    fn candidates(&self, course: &DegreePlanCourse, order: i32, blocking: &HashSet<CourseKey>) -> Vec<Candidate> {
        let Some(key) = &course.key else { return vec![] };
        let mut found = Vec::new();
        if self.origin + 1 < self.plan.terms.len() {
            for target in (self.origin + 1)..self.plan.terms.len() {
                if let Some(single) = self.single_move(course, key, target, order) {
                    found.push(single);
                }
                for (position, partner) in self.plan.terms[target].courses.iter().enumerate() {
                    let Some(partner_key) = &partner.key else { continue };
                    if blocking.contains(partner_key) || partner_key == key {
                        continue;
                    }
                    if let Some(swap) = self.swap(course, key, partner, partner_key, target, order, position as i32) {
                        found.push(swap);
                    }
                }
            }
        }
        if found.is_empty() {
            if let Some(appended) = self.appended_semester(course, key, order) {
                found.push(appended);
            }
        }
        found
    }

    fn single_move(&self, course: &DegreePlanCourse, key: &CourseKey, target: usize, order: i32) -> Option<Candidate> {
        let semester = &self.plan.terms[target];
        let offering = self.uncertainty(key, semester.season)?;
        let total = semester.credits() + course.credits;
        if total > self.max_credits {
            return None;
        }
        let move_ = Move { course: key.clone(), from: self.plan.terms[self.origin].id, to: semester.id };
        let mut changed = self.plan.clone();
        if !perform_move(key, move_.from, move_.to, &mut changed) {
            return None;
        }
        let after = self.checks_after(&changed, &HashSet::from([course.id]))?;
        let check = after.get(&course.id).copied().unwrap_or(Check::Unknown);
        let reasons = vec![
            self.offering_reason(key, semester.season, false),
            self.prerequisite_reason(key, check, self.before.get(&course.id) == Some(&Check::Met), false),
            format!("{} would have {}", semester.short_name(), self.credits_label(total)),
        ];
        Some(Candidate {
            suggestion: Suggestion {
                id: format!("move-{key}-{}", move_.to.as_hyphenated()),
                moves: vec![move_],
                title: format!("Move {key} to {}", semester.short_name()),
                reasons,
                adds_semester: None,
            },
            rank: Rank { moves: 1, target: target as i32, uncertainty: offering + i32::from(check == Check::Unknown), order, detail: 0 },
        })
    }

    fn swap(&self, course: &DegreePlanCourse, key: &CourseKey, partner: &DegreePlanCourse, partner_key: &CourseKey, target: usize, order: i32, detail: i32) -> Option<Candidate> {
        let semester = &self.plan.terms[target];
        let home = &self.plan.terms[self.origin];
        let course_offering = self.uncertainty(key, semester.season)?;
        let partner_offering = self.uncertainty(partner_key, home.season)?;
        let new_home = home.credits() - course.credits + partner.credits;
        let new_target = semester.credits() - partner.credits + course.credits;
        if new_home > self.max_credits.max(home.credits()) || new_target > self.max_credits.max(semester.credits()) {
            return None;
        }
        let moves = vec![
            Move { course: key.clone(), from: home.id, to: semester.id },
            Move { course: partner_key.clone(), from: semester.id, to: home.id },
        ];
        let mut changed = self.plan.clone();
        for move_ in &moves {
            if !perform_move(&move_.course, move_.from, move_.to, &mut changed) {
                return None;
            }
        }
        let after = self.checks_after(&changed, &HashSet::from([course.id, partner.id]))?;
        let course_check = after.get(&course.id).copied().unwrap_or(Check::Unknown);
        let partner_check = after.get(&partner.id).copied().unwrap_or(Check::Unknown);
        let reasons = vec![
            self.offering_reason(key, semester.season, true),
            self.offering_reason(partner_key, home.season, true),
            self.prerequisite_reason(key, course_check, self.before.get(&course.id) == Some(&Check::Met), true),
            self.prerequisite_reason(partner_key, partner_check, self.before.get(&partner.id) == Some(&Check::Met), true),
            format!("{} would have {}", home.short_name(), self.credits_label(new_home)),
            format!("{} would have {}", semester.short_name(), self.credits_label(new_target)),
        ];
        let unsure = course_offering + partner_offering + i32::from(course_check == Check::Unknown) + i32::from(partner_check == Check::Unknown);
        Some(Candidate {
            suggestion: Suggestion {
                id: format!("swap-{key}-{}-{partner_key}", semester.id.as_hyphenated()),
                moves,
                title: format!("Move {key} to {} and {partner_key} to {}", semester.short_name(), home.short_name()),
                reasons,
                adds_semester: None,
            },
            rank: Rank { moves: 2, target: target as i32, uncertainty: unsure, order, detail },
        })
    }

    fn appended_semester(&self, course: &DegreePlanCourse, key: &CourseKey, order: i32) -> Option<Candidate> {
        if course.credits > self.max_credits {
            return None;
        }
        let home = self.plan.terms[self.origin].clone();
        for upcoming in planlogic::next_semesters(&self.plan, 8, 2026, 10) {
            if upcoming.sort_key() <= home.sort_key() {
                continue;
            }
            let Some(offering) = self.uncertainty(key, upcoming.season) else { continue };
            let mut semester = DegreePlanTerm::new(upcoming.season, upcoming.year);
            semester.id = stable_id(&format!("degree-plan-term-{}-{}", season_raw(semester.season), semester.year));
            let mut changed = self.plan.clone();
            changed.terms.push(semester.clone());
            changed.sort_terms();
            let move_ = Move { course: key.clone(), from: home.id, to: semester.id };
            if !perform_move(key, move_.from, move_.to, &mut changed) {
                continue;
            }
            let Some(after) = self.checks_after(&changed, &HashSet::from([course.id])) else { continue };
            let check = after.get(&course.id).copied().unwrap_or(Check::Unknown);
            let reasons = vec![
                self.offering_reason(key, semester.season, false),
                self.prerequisite_reason(key, check, self.before.get(&course.id) == Some(&Check::Met), false),
                format!("Adds {} to your plan, with {}", semester.short_name(), self.credits_label(course.credits)),
            ];
            return Some(Candidate {
                suggestion: Suggestion {
                    id: format!("move-{key}-{}", semester.id.as_hyphenated()),
                    moves: vec![move_],
                    title: format!("Move {key} to a new {} semester", semester.short_name()),
                    reasons,
                    adds_semester: Some(semester),
                },
                rank: Rank { moves: 1, target: self.plan.terms.len() as i32, uncertainty: offering + i32::from(check == Check::Unknown), order, detail: 0 },
            });
        }
        None
    }
}

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn season_raw(season: PlanSeason) -> &'static str {
    match season {
        PlanSeason::Winter => "winter",
        PlanSeason::Spring => "spring",
        PlanSeason::SummerA => "summerA",
        PlanSeason::SummerFull => "summerFull",
        PlanSeason::SummerB => "summerB",
        PlanSeason::Fall => "fall",
    }
}

pub fn stable_id(text: &str) -> Uuid {
    let mut first: u64 = 0xcbf2_9ce4_8422_2325;
    let mut second: u64 = 0x8422_2325_cbf2_9ce4;
    for byte in text.bytes() {
        first ^= u64::from(byte);
        first = first.wrapping_mul(0x0000_0100_0000_01b3);
        second = second.wrapping_add(u64::from(byte));
        second = second.wrapping_mul(0x0000_0100_0000_01b3) ^ first;
    }
    let hex = format!("{first:016x}{second:016x}");
    let formatted = format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32]);
    Uuid::parse_str(&formatted).unwrap_or_else(|_| Uuid::nil())
}

pub fn schedule_option(suggestion: &Suggestion, origin: Uuid) -> (Vec<CourseKey>, Vec<CourseKey>, bool, String) {
    let away = suggestion.moves.iter().filter(|m| m.from == origin).map(|m| m.course.clone()).collect();
    let into = suggestion.moves.iter().filter(|m| m.to == origin).map(|m| m.course.clone()).collect();
    let applied = if let Some(rest) = suggestion.title.strip_prefix("Move ") {
        format!("Moved {rest}")
    } else {
        "Updated your degree plan".into()
    };
    (away, into, suggestion.adds_semester.is_some(), applied)
}
