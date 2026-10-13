//! Questions a petition still has to answer, and courses that nearly fit a range.
//! Port of `DegreePetitionQuestions.swift`.

use crate::degree_audit::{AuditStatus, CourseUse, DegreeAudit};
use crate::model::*;
use crate::CourseEquivalences;
use uuid::Uuid;

impl CourseRange {
    pub fn applying(&self, limit: Option<&CourseNumberLimit>) -> CourseRange {
        let mut range = self.clone();
        match limit {
            None | Some(CourseNumberLimit::SameAsRequirement) => return self.clone(),
            Some(CourseNumberLimit::AnyNumber) => range.lowest = 0,
            Some(CourseNumberLimit::AtLeast(n)) => range.lowest = *n,
        }
        range.label = range.described_label();
        range
    }

    pub fn reason_excluding(&self, key: &CourseKey) -> Option<String> {
        if key.subject != self.subject || self.contains(key) {
            return None;
        }
        if self.except.iter().any(|item| item == key) {
            return Some(format!("{key} is excluded"));
        }
        let digits: String = key.number.chars().take_while(|c| c.is_ascii_digit()).collect();
        let number: i32 = digits.parse().ok()?;
        if number < self.lowest {
            return Some(format!("Below {} {}", self.subject, self.lowest));
        }
        if let Some(highest) = self.highest {
            if number > highest {
                return Some(format!("Above {} {highest}", self.subject));
            }
        }
        None
    }
}

pub fn subjects_text(ranges: &[CourseRange]) -> String {
    let mut seen = std::collections::BTreeSet::new();
    let subjects: Vec<String> = ranges.iter().map(|r| r.subject.clone()).filter(|s| seen.insert(s.clone())).collect();
    join_or(&subjects)
}

pub fn minimums_text(ranges: &[CourseRange]) -> Option<String> {
    let minimums: Vec<String> = ranges.iter().filter(|r| r.lowest > 0).map(|r| format!("{} {}", r.subject, r.lowest)).collect();
    if minimums.is_empty() { None } else { Some(join_or(&minimums)) }
}

fn join_and(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => format!("{}, and {}", items[..items.len() - 1].join(", "), items.last().unwrap()),
    }
}

fn join_or(items: &[String]) -> String {
    join_and(items).replace(" and ", " or ")
}

impl ProgramRequirements {
    pub fn degree_rule(&self, id: Option<&str>, name: Option<&str>) -> Option<&RequirementRule> {
        if let Some(id) = id {
            if let Some(rule) = self.rule(id) {
                return Some(rule);
            }
        }
        let name = name?.to_ascii_lowercase();
        let matches: Vec<&RequirementRule> = self
            .all_rules()
            .into_iter()
            .filter(|rule| {
                self.degree_short_name(&rule.id).is_some_and(|text| text.to_ascii_lowercase() == name)
                    || self.title_of_rule(&rule.id).is_some_and(|text| text.to_ascii_lowercase() == name)
            })
            .collect();
        if matches.len() == 1 { matches.into_iter().next() } else { None }
    }
}

impl RequirementReplacement {
    pub fn like_ranges<'a>(&self, requirements: &'a ProgramRequirements) -> Vec<&'a CourseRange> {
        let Some(rule) = requirements.degree_rule(self.like_rule_id.as_deref(), self.like_rule_title.as_deref()) else { return vec![] };
        match &rule.kind {
            RuleKind::Choose { ranges, .. } => ranges.iter().collect(),
            _ => vec![],
        }
    }

    pub fn asks_for_course_numbers(&self, requirements: &ProgramRequirements) -> bool {
        self.like_ranges(requirements).iter().any(|range| range.lowest > 0)
    }
}

impl DegreePlan {
    pub fn set_course_numbers(&mut self, limit: Option<CourseNumberLimit>, id: Uuid) {
        let Some(index) = self.substitutions.iter().position(|item| item.id == id) else { return };
        let SubstitutionChange::Replace(mut replacement) = self.substitutions[index].change.clone() else { return };
        replacement.course_numbers = limit;
        self.substitutions[index].change = SubstitutionChange::Replace(replacement);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CourseNumberChoice {
    SameAsRequirement,
    AnyNumber,
    AtLeast,
}

impl CourseNumberChoice {
    pub fn from_limit(limit: Option<&CourseNumberLimit>) -> Option<Self> {
        match limit {
            Some(CourseNumberLimit::SameAsRequirement) => Some(Self::SameAsRequirement),
            Some(CourseNumberLimit::AnyNumber) => Some(Self::AnyNumber),
            Some(CourseNumberLimit::AtLeast(_)) => Some(Self::AtLeast),
            None => None,
        }
    }
    pub fn limit(self, minimum: i32) -> Option<CourseNumberLimit> {
        match self {
            Self::SameAsRequirement => Some(CourseNumberLimit::SameAsRequirement),
            Self::AnyNumber => Some(CourseNumberLimit::AnyNumber),
            Self::AtLeast => Self::is_valid_minimum(minimum).then_some(CourseNumberLimit::AtLeast(minimum)),
        }
    }
    pub fn is_valid_minimum(minimum: i32) -> bool {
        (1..=9999).contains(&minimum)
    }
    pub fn suggested_minimum(ranges: &[CourseRange], saved: Option<&CourseNumberLimit>) -> i32 {
        if let Some(CourseNumberLimit::AtLeast(minimum)) = saved {
            return *minimum;
        }
        let lowest = ranges.iter().map(|r| r.lowest).filter(|n| *n > 0).min().unwrap_or(2000);
        (lowest / 1000 * 1000).max(1000)
    }
    pub fn title(self, like_title: &str) -> String {
        match self {
            Self::SameAsRequirement => format!("Same as {like_title}"),
            Self::AnyNumber => "Any course number".into(),
            Self::AtLeast => "A different minimum".into(),
        }
    }
    pub fn detail(self, ranges: &[CourseRange], minimum: i32) -> String {
        match self {
            Self::SameAsRequirement => ranges.iter().map(|r| r.label.clone()).collect::<Vec<_>>().join(" · "),
            Self::AnyNumber => format!("Any {} course", subjects_text(ranges)),
            Self::AtLeast => {
                if Self::is_valid_minimum(minimum) {
                    ranges.iter().map(|r| r.applying(Some(&CourseNumberLimit::AtLeast(minimum))).label).collect::<Vec<_>>().join(" · ")
                } else {
                    "Enter a course number, like 2000".into()
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct DegreePetitionQuestion {
    pub substitution: RequirementSubstitution,
    pub rule_id: String,
    pub rule_title: String,
    pub like_title: String,
    pub ranges: Vec<CourseRange>,
    pub courses: Vec<CourseKey>,
    pub satisfied: usize,
    pub satisfied_without_minimum: usize,
    pub total: usize,
}

impl DegreePetitionQuestion {
    pub fn id(&self) -> Uuid {
        self.substitution.id
    }
    pub fn question(&self) -> String {
        format!("Does your {} petition accept any {} course?", self.rule_title, subjects_text(&self.ranges))
    }
    pub fn matters(&self) -> bool {
        self.satisfied_without_minimum > self.satisfied
    }
    pub fn explanation(&self) -> String {
        let mut parts = Vec::new();
        if let Some(minimums) = minimums_text(&self.ranges) {
            parts.push(format!("{} start at {minimums}.", self.like_title));
        }
        if !self.matters() {
            parts.push("Your petition decides whether that minimum applies here.".into());
            return parts.join(" ");
        }
        let gained = self.satisfied_without_minimum - self.satisfied;
        let effect = if self.satisfied_without_minimum >= self.total {
            "every requirement is met".to_string()
        } else if gained == 1 {
            "1 more requirement is met".into()
        } else {
            format!("{gained} more requirements are met")
        };
        if self.courses.is_empty() {
            parts.push(format!("With any number, {effect}."));
        } else {
            let course_list = join_and(&self.courses.iter().map(|k| k.to_string()).collect::<Vec<_>>());
            let verb = if self.courses.len() == 1 { "counts" } else { "count" };
            parts.push(format!("With any number, {course_list} {verb} and {effect}."));
        }
        parts.join(" ")
    }
}

pub fn questions(plan: &DegreePlan, requirements: &ProgramRequirements, equivalences: &CourseEquivalences, audit: Option<&DegreeAudit>) -> Vec<DegreePetitionQuestion> {
    let candidates: Vec<&RequirementSubstitution> = plan.substitutions.iter().filter(|item| is_unanswered(item, requirements)).collect();
    if candidates.is_empty() {
        return vec![];
    }
    let owned;
    let current = if let Some(audit) = audit {
        audit
    } else {
        owned = DegreeAudit::run(plan, requirements, equivalences, &crate::degree_audit::DegreeCourseAttributes::none());
        &owned
    };
    candidates
        .into_iter()
        .filter_map(|item| question_about(item, plan, requirements, equivalences, current))
        .filter(|item| item.matters())
        .collect()
}

pub fn question_about(
    substitution: &RequirementSubstitution,
    plan: &DegreePlan,
    requirements: &ProgramRequirements,
    equivalences: &CourseEquivalences,
    audit: &DegreeAudit,
) -> Option<DegreePetitionQuestion> {
    let rule_id = audit.placements.get(&substitution.id).cloned().unwrap_or_else(|| substitution.rule_id.clone());
    if !is_unanswered(substitution, requirements) {
        return None;
    }
    let SubstitutionChange::Replace(replacement) = &substitution.change else { return None };
    audit.result_for(&rule_id)?;
    let mut relaxed = plan.clone();
    relaxed.set_course_numbers(Some(CourseNumberLimit::AnyNumber), substitution.id);
    let without = DegreeAudit::run(&relaxed, requirements, equivalences, &audit.attributes);
    let ranges: Vec<CourseRange> = replacement.like_ranges(requirements).into_iter().cloned().collect();
    let newly = without
        .result_for(&rule_id)
        .map(|result| result.used.iter().map(|use_| use_.key.clone()).filter(|key| !ranges.iter().any(|range| range.contains(key)) && !replacement.courses.contains(key)).collect())
        .unwrap_or_default();
    let like_title = replacement
        .like_rule_title
        .clone()
        .or_else(|| replacement.like_rule_id.as_ref().and_then(|id| requirements.title_of_rule(id)))
        .unwrap_or_else(|| "That requirement".into());
    Some(DegreePetitionQuestion {
        substitution: substitution.clone(),
        rule_id: rule_id.clone(),
        rule_title: requirements.title_of_rule(&rule_id).unwrap_or_else(|| substitution.rule_title.clone()),
        like_title,
        ranges,
        courses: newly,
        satisfied: audit.satisfied_count(),
        satisfied_without_minimum: without.satisfied_count(),
        total: audit.total_count(),
    })
}

pub fn is_unanswered(substitution: &RequirementSubstitution, requirements: &ProgramRequirements) -> bool {
    let SubstitutionChange::Replace(replacement) = &substitution.change else { return false };
    replacement.course_numbers.is_none() && replacement.asks_for_course_numbers(requirements)
}

#[derive(Clone, Debug, PartialEq)]
pub struct NearMiss {
    pub course: CourseUse,
    pub range: CourseRange,
    pub reason: String,
}

impl DegreeAudit {
    pub fn near_misses(&self, rule_id: &str) -> Vec<NearMiss> {
        let Some(result) = self.result_for(rule_id) else { return vec![] };
        if result.status.is_satisfied() {
            return vec![];
        }
        let Some(RuleKind::Choose { count, ranges, .. }) = &result.effective_kind else { return vec![] };
        if ranges.is_empty() {
            return vec![];
        }
        let mut candidates: Vec<&CourseUse> = self.general_electives.iter().chain(self.unused.iter()).filter(|use_| use_.credits > 0.0 && (count.is_none() || use_.credits >= 2.0)).collect();
        candidates.sort_by_key(|use_| (!use_.is_completed(), use_.order));
        candidates
            .into_iter()
            .filter_map(|use_| {
                if ranges.iter().any(|range| range.contains(&use_.key)) {
                    return None;
                }
                for range in ranges {
                    if let Some(reason) = range.reason_excluding(&use_.key) {
                        return Some(NearMiss { course: use_.clone(), range: range.clone(), reason });
                    }
                }
                None
            })
            .collect()
    }
}

pub fn status_label(status: AuditStatus) -> &'static str {
    match status {
        AuditStatus::Missing => "Missing",
        AuditStatus::Partial => "Partial",
        AuditStatus::Planned => "Planned",
        AuditStatus::Complete => "Complete",
        AuditStatus::Waived => "Waived",
    }
}
