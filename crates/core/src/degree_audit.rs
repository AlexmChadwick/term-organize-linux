//! Degree audit: match a plan's courses to catalog requirements.
//!
//! Port of `DegreeAudit.swift`. Each course counts toward one requirement. Substitutions apply
//! first, then required courses, choice lists, ranges, rebalancing, an exact search when a
//! requirement is still short of something the plan can meet, surplus-hour trimming, and
//! general electives.

use crate::model::{
    apply_number_limit, CourseEquivalence, CourseKey, CourseNumberLimit, CourseOption, CourseRange,
    DegreePlan, DegreePlanCourse, ProgramRequirements, RequirementReplacement, RequirementRule,
    RequirementSection, RuleKind, SubstitutionChange,
};
use crate::nupath::{self, NuCourse};
use crate::NuPath;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuditStatus {
    Missing = 0,
    Partial = 1,
    Planned = 2,
    Complete = 3,
    Waived = 4,
}

impl AuditStatus {
    pub fn is_satisfied(self) -> bool {
        self >= Self::Planned
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CourseUse {
    pub key: CourseKey,
    pub title: String,
    pub credits: f64,
    pub semester: Option<String>,
    pub order: i32,
    pub by_substitution: bool,
    pub satisfies: Option<CourseKey>,
    pub instance: i32,
}

impl CourseUse {
    pub fn id(&self) -> String {
        if self.instance == 0 {
            self.key.to_string()
        } else {
            format!("{} ({})", self.key, self.instance + 1)
        }
    }
    pub fn is_completed(&self) -> bool {
        self.semester.is_none()
    }
    pub fn counts_as_label(&self) -> Option<String> {
        self.satisfies.as_ref().map(|k| format!("counts as {k}"))
    }
}

#[derive(Clone, Debug)]
pub struct RuleResult {
    pub rule: RequirementRule,
    pub effective_kind: Option<RuleKind>,
    pub status: AuditStatus,
    pub used: Vec<CourseUse>,
    pub remaining_count: Option<i32>,
    pub remaining_hours: Option<f64>,
    pub substitutions: Vec<crate::model::RequirementSubstitution>,
}

impl RuleResult {
    pub fn id(&self) -> &str {
        &self.rule.id
    }
    pub fn use_for(&self, required: &CourseKey) -> Option<&CourseUse> {
        self.used
            .iter()
            .find(|u| &u.key == required && u.satisfies.is_none())
            .or_else(|| {
                self.used
                    .iter()
                    .find(|u| u.satisfies.as_ref() == Some(required))
            })
    }
}

#[derive(Clone, Debug)]
pub struct GroupResult {
    pub group_id: String,
    pub group_title: Option<String>,
    pub rules: Vec<RuleResult>,
}

impl GroupResult {
    pub fn status(&self) -> AuditStatus {
        self.rules
            .iter()
            .map(|r| r.status)
            .min()
            .unwrap_or(AuditStatus::Complete)
    }
}

#[derive(Clone, Debug)]
pub struct SectionResult {
    pub section: RequirementSection,
    pub groups: Vec<GroupResult>,
}

impl SectionResult {
    pub fn rules(&self) -> Vec<&RuleResult> {
        self.groups.iter().flat_map(|g| g.rules.iter()).collect()
    }
    pub fn status(&self) -> AuditStatus {
        self.rules()
            .into_iter()
            .map(|r| r.status)
            .min()
            .unwrap_or(AuditStatus::Complete)
    }
}

#[derive(Clone, Debug)]
pub enum NoteKind {
    MajorHours { required: f64 },
    Coop,
    Nupath,
    Universitywide,
    TotalHours { required: f64 },
}

#[derive(Clone, Debug)]
pub struct NoteItem {
    pub id: String,
    pub title: String,
    pub status: Option<AuditStatus>,
    pub detail: String,
    pub courses: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct NoteCheck {
    pub section_id: String,
    pub note: String,
    pub kind: NoteKind,
    pub status: AuditStatus,
    pub detail: String,
    pub items: Vec<NoteItem>,
    pub counts: bool,
}

impl NoteCheck {
    pub fn id(&self) -> String {
        format!("{}|{}", self.section_id, self.note)
    }
    pub fn title(&self) -> &'static str {
        match self.kind {
            NoteKind::MajorHours { .. } => "Semester hours in the major",
            NoteKind::Coop => "Co-op",
            NoteKind::Nupath => "NUpath requirements",
            NoteKind::Universitywide => "Universitywide requirements",
            NoteKind::TotalHours { .. } => "Total semester hours",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DegreeCourseAttributes {
    pub nupaths: HashMap<CourseKey, HashSet<NuPath>>,
    pub loading: HashSet<CourseKey>,
}

impl DegreeCourseAttributes {
    pub fn none() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.nupaths.is_empty() && self.loading.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct DegreeAudit {
    pub requirements: ProgramRequirements,
    pub sections: Vec<SectionResult>,
    pub general_electives: Vec<CourseUse>,
    pub unused: Vec<CourseUse>,
    pub completed_hours: f64,
    pub planned_hours: f64,
    pub required_hours: Option<f64>,
    pub unmatched_substitutions: Vec<crate::model::RequirementSubstitution>,
    pub placements: HashMap<uuid::Uuid, String>,
    pub needs_concentration: bool,
    pub note_checks: Vec<NoteCheck>,
    pub attributes: DegreeCourseAttributes,
}

impl DegreeAudit {
    pub fn run(
        plan: &DegreePlan,
        requirements: &ProgramRequirements,
        equivalences: &crate::CourseEquivalences,
        attributes: &DegreeCourseAttributes,
    ) -> Self {
        let merged = equivalences.merging(&crate::CourseEquivalences::from_declarations(
            &plan.course_equivalences,
        ));
        let mut engine = AuditEngine::new(plan, requirements, &merged, attributes);
        let mut audit = engine.run();
        audit.attributes = attributes.clone();
        audit
    }

    pub fn fewest_credits_choice(uses: &[CourseUse], need: f64) -> Option<HashSet<String>> {
        fewest_credits(uses, need)
    }

    pub fn rule_results(&self) -> Vec<&RuleResult> {
        self.sections
            .iter()
            .flat_map(|s| s.groups.iter().flat_map(|g| g.rules.iter()))
            .collect()
    }
    pub fn result_for(&self, rule_id: &str) -> Option<&RuleResult> {
        self.rule_results()
            .into_iter()
            .find(|r| r.rule.id == rule_id)
    }
    pub fn counted_checks(&self) -> Vec<&NoteCheck> {
        self.note_checks.iter().filter(|c| c.counts).collect()
    }
    pub fn satisfied_count(&self) -> usize {
        self.rule_results()
            .iter()
            .filter(|r| r.status.is_satisfied())
            .count()
            + self
                .counted_checks()
                .iter()
                .filter(|c| c.status.is_satisfied())
                .count()
    }
    pub fn complete_count(&self) -> usize {
        self.rule_results()
            .iter()
            .filter(|r| r.status == AuditStatus::Complete || r.status == AuditStatus::Waived)
            .count()
            + self
                .counted_checks()
                .iter()
                .filter(|c| c.status == AuditStatus::Complete)
                .count()
    }
    pub fn total_count(&self) -> usize {
        self.rule_results().len() + self.counted_checks().len()
    }
    pub fn note_check(&self, section_id: &str, note: &str) -> Option<&NoteCheck> {
        self.note_checks
            .iter()
            .find(|c| c.section_id == section_id && c.note == note)
    }
    pub fn rule_counting(&self, key: &CourseKey) -> Option<&RuleResult> {
        self.rule_results()
            .into_iter()
            .find(|r| r.used.iter().any(|u| &u.key == key))
    }
}

pub fn is_coop_note(note: &str) -> bool {
    regex::Regex::new(r"(?i)satisfied\s+(?:through|by|with)\s+(?:a\s+)?co-?op")
        .unwrap()
        .is_match(note)
}

pub fn unchecked_reason(note: &str) -> Option<&'static str> {
    if regex::Regex::new(r"(?i)\bGPA\b|grade\s+point|minimum\s+grade|grade\s+of\s+[A-D]")
        .unwrap()
        .is_match(note)
    {
        return Some("Not checked: plans don’t include grades");
    }
    if regex::Regex::new(
        r"(?i)residen|(?:at|from)\s+Northeastern|Northeastern\s+University\s+semester\s+hours",
    )
    .unwrap()
    .is_match(note)
    {
        return Some("Not checked: a plan doesn’t say where credits were earned");
    }
    if regex::Regex::new(r"(?i)(?:pick|choose|select|complete)\s+(?:one|a)\s+(?:of\s+the\s+following\s+)?concentrations?").unwrap().is_match(note) {
        return Some("Checked through the concentration you choose");
    }
    None
}

pub fn counts_separately(title: &str, credits: f64, other_title: &str, other_credits: f64) -> bool {
    if credits <= 0.0 || other_credits <= 0.0 {
        return false;
    }
    let first = normalized_title(title);
    let second = normalized_title(other_title);
    if !first.is_empty() && !second.is_empty() && first != second {
        return true;
    }
    first.contains("topic") || second.contains("topic")
}

fn normalized_title(title: &str) -> String {
    title
        .to_ascii_lowercase()
        .replace('&', " and ")
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn hours_text(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded:.1}")
    }
}

fn gpa_text(value: f64) -> String {
    let text = format!("{value:.3}");
    let trimmed = text.trim_end_matches('0');
    let trimmed = trimmed.trim_end_matches('.');
    if trimmed.contains('.') {
        trimmed.to_string()
    } else {
        format!("{trimmed}.0")
    }
}

fn english_list(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        n => {
            let mut s = items[..n - 1].join(", ");
            s.push_str(", and ");
            s.push_str(&items[n - 1]);
            s
        }
    }
}

#[derive(Clone, Debug)]
enum Effective {
    Waived,
    Kind { kind: RuleKind, any_course: bool },
}

#[derive(Clone)]
struct AuditEngine {
    requirements: ProgramRequirements,
    equivalences: crate::CourseEquivalences,
    attributes: DegreeCourseAttributes,
    sections: Vec<RequirementSection>,
    rules: Vec<RequirementRule>,
    substitutions_by_rule: HashMap<String, Vec<crate::model::RequirementSubstitution>>,
    unplaced: Vec<crate::model::RequirementSubstitution>,
    rules_by_name: HashMap<String, HashSet<String>>,
    pool: HashMap<String, CourseUse>,
    instances: HashMap<CourseKey, Vec<String>>,
    available: HashSet<String>,
    used: HashMap<String, Vec<CourseUse>>,
    options_counted: HashMap<String, i32>,
    effective: HashMap<String, Effective>,
    demand: HashMap<CourseKey, i32>,
    singles: HashMap<String, HashSet<String>>,
    unmatched: Vec<crate::model::RequirementSubstitution>,
    concentration_id: Option<String>,
    concentrations_nonempty: bool,
}

impl AuditEngine {
    fn new(
        plan: &DegreePlan,
        requirements: &ProgramRequirements,
        equivalences: &crate::CourseEquivalences,
        attributes: &DegreeCourseAttributes,
    ) -> Self {
        let sections: Vec<RequirementSection> = requirements
            .applicable_sections(plan.concentration_id.as_deref())
            .into_iter()
            .cloned()
            .collect();
        let rules: Vec<RequirementRule> = sections
            .iter()
            .flat_map(|s| s.groups.iter().flat_map(|g| g.rules.clone()))
            .collect();
        let mut rules_by_name: HashMap<String, HashSet<String>> = HashMap::new();
        for rule in &rules {
            for name in [
                requirements.degree_short_name(&rule.id),
                requirements.title_of_rule(&rule.id),
            ]
            .into_iter()
            .flatten()
            {
                rules_by_name
                    .entry(name.to_ascii_lowercase())
                    .or_default()
                    .insert(rule.id.clone());
            }
        }
        let ids: HashSet<String> = rules.iter().map(|r| r.id.clone()).collect();
        let mut by_rule: HashMap<String, Vec<_>> = HashMap::new();
        let mut unplaced = vec![];
        for substitution in &plan.substitutions {
            if ids.contains(&substitution.rule_id) {
                by_rule
                    .entry(substitution.rule_id.clone())
                    .or_default()
                    .push(substitution.clone());
            } else if rules_by_name
                .get(&substitution.rule_title.to_ascii_lowercase())
                .is_some_and(|n| n.len() == 1)
            {
                let id = rules_by_name[&substitution.rule_title.to_ascii_lowercase()]
                    .iter()
                    .next()
                    .unwrap()
                    .clone();
                by_rule.entry(id).or_default().push(substitution.clone());
            } else {
                unplaced.push(substitution.clone());
            }
        }
        let mut engine = Self {
            requirements: requirements.clone(),
            equivalences: equivalences.clone(),
            attributes: attributes.clone(),
            sections,
            rules,
            substitutions_by_rule: by_rule,
            unplaced,
            rules_by_name,
            pool: HashMap::new(),
            instances: HashMap::new(),
            available: HashSet::new(),
            used: HashMap::new(),
            options_counted: HashMap::new(),
            effective: HashMap::new(),
            demand: HashMap::new(),
            singles: HashMap::new(),
            unmatched: vec![],
            concentration_id: plan.concentration_id.clone(),
            concentrations_nonempty: !requirements.concentrations().is_empty(),
        };
        let mut order = 0i32;
        let mut offer = |course: &DegreePlanCourse, semester: Option<String>| {
            let current = order;
            order += 1;
            let Some(key) = course.key.clone() else {
                return;
            };
            let mut use_ = CourseUse {
                key: key.clone(),
                title: course.title.clone(),
                credits: course.credits,
                semester,
                order: current,
                by_substitution: false,
                satisfies: None,
                instance: 0,
            };
            let earlier: Vec<CourseUse> = engine
                .instances
                .get(&key)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| engine.pool.get(id).cloned())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(repeated) = earlier
                .iter()
                .find(|e| !counts_separately(&use_.title, use_.credits, &e.title, e.credits))
            {
                let better = if repeated.is_completed() == use_.is_completed() {
                    use_.credits > repeated.credits
                } else {
                    use_.is_completed()
                };
                if better {
                    use_.instance = repeated.instance;
                    engine.pool.insert(use_.id(), use_);
                }
                return;
            }
            use_.instance = earlier.len() as i32;
            let id = use_.id();
            engine.instances.entry(key).or_default().push(id.clone());
            engine.pool.insert(id, use_);
        };
        for course in &plan.completed {
            offer(course, None);
        }
        for term in &plan.terms {
            for course in &term.courses {
                offer(course, Some(term.name()));
            }
        }
        engine.available = engine.pool.keys().cloned().collect();
        engine
    }

    fn run(&mut self) -> DegreeAudit {
        self.resolve_substitutions();
        self.apply_course_substitutions();
        let mut required: Vec<(usize, String, CourseOption)> = vec![];
        for (index, rule) in self.rules.iter().enumerate() {
            if let Some(Effective::Kind {
                kind: RuleKind::Required(option),
                ..
            }) = self.effective.get(&rule.id)
            {
                required.push((index, rule.id.clone(), option.clone()));
            }
        }
        required.sort_by(|a, b| {
            a.2.alternatives
                .len()
                .cmp(&b.2.alternatives.len())
                .then(a.0.cmp(&b.0))
        });
        for (_, id, option) in required {
            if let Some(rule) = self.rules.iter().find(|r| r.id == id).cloned() {
                self.fill_required(&rule, &option);
            }
        }
        let mut choices: Vec<(usize, String, usize, bool, bool)> = vec![];
        for (index, rule) in self.rules.iter().enumerate() {
            if let Some(Effective::Kind {
                kind:
                    RuleKind::Choose {
                        options,
                        ranges,
                        count,
                        ..
                    },
                any_course,
            }) = self.effective.get(&rule.id)
            {
                choices.push((
                    index,
                    rule.id.clone(),
                    options.len(),
                    !ranges.is_empty() || *any_course,
                    count.is_some(),
                ));
            }
        }
        let mut plain: Vec<_> = choices.iter().filter(|c| !c.3).cloned().collect();
        plain.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)));
        for (_, id, _, _, _) in plain {
            if let Some(rule) = self.rules.iter().find(|r| r.id == id).cloned() {
                self.fill_choice(&rule);
            }
        }
        for (_, id, _, ranged, counts_courses) in &choices {
            if *ranged && *counts_courses {
                if let Some(rule) = self.rules.iter().find(|r| r.id == *id).cloned() {
                    self.fill_choice(&rule);
                }
            }
        }
        for (_, id, _, ranged, counts_courses) in &choices {
            if *ranged && !*counts_courses {
                if let Some(rule) = self.rules.iter().find(|r| r.id == *id).cloned() {
                    self.fill_choice(&rule);
                }
            }
        }
        self.rebalance();
        if self.ensure_best_matching() {
            for rule in self.rules.clone() {
                if self.is_met(&rule.id) {
                    continue;
                }
                match self.effective.get(&rule.id).cloned() {
                    Some(Effective::Kind {
                        kind: RuleKind::Required(option),
                        ..
                    }) => self.fill_required(&rule, &option),
                    Some(Effective::Kind {
                        kind: RuleKind::Choose { .. },
                        ..
                    }) => self.fill_choice(&rule),
                    _ => {}
                }
            }
        }
        self.trim_surplus_hours();
        let mut general_electives = vec![];
        for rule in self.rules.clone() {
            let Some(Effective::Kind {
                kind: RuleKind::GeneralElectives { hours },
                ..
            }) = self.effective.get(&rule.id).cloned()
            else {
                continue;
            };
            let counted: f64 = self
                .used
                .get(&rule.id)
                .map(|u| u.iter().map(|c| c.credits).sum())
                .unwrap_or(0.0);
            let candidates: Vec<CourseUse> = self
                .remaining()
                .into_iter()
                .filter(|u| u.credits > 0.0)
                .collect();
            let chosen = fewest_credits(&candidates, hours - counted)
                .unwrap_or_else(|| candidates.iter().map(|c| c.id()).collect());
            for use_ in &candidates {
                if chosen.contains(&use_.id()) {
                    self.consume(&use_.id(), &rule.id, false, None);
                }
            }
            general_electives.extend(self.used.get(&rule.id).cloned().unwrap_or_default());
        }
        let results = self
            .sections
            .iter()
            .map(|section| SectionResult {
                section: section.clone(),
                groups: section
                    .groups
                    .iter()
                    .map(|group| GroupResult {
                        group_id: group.id.clone(),
                        group_title: group.title.clone(),
                        rules: group
                            .rules
                            .iter()
                            .map(|rule| self.result_for(rule))
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        let courses: Vec<CourseUse> = self.pool.values().cloned().collect();
        let concentration_chosen = self.concentration_id.as_ref().is_some_and(|id| {
            self.requirements
                .concentrations()
                .iter()
                .any(|s| &s.id == id)
        });
        let mut placements = HashMap::new();
        for (rule_id, subs) in &self.substitutions_by_rule {
            for sub in subs {
                placements.insert(sub.id, rule_id.clone());
            }
        }
        DegreeAudit {
            requirements: self.requirements.clone(),
            sections: results,
            general_electives,
            unused: self.remaining(),
            completed_hours: courses
                .iter()
                .filter(|c| c.is_completed())
                .map(|c| c.credits.max(0.0))
                .sum(),
            planned_hours: courses
                .iter()
                .filter(|c| !c.is_completed())
                .map(|c| c.credits.max(0.0))
                .sum(),
            required_hours: self.requirements.total_hours,
            unmatched_substitutions: self.unmatched.clone(),
            placements,
            needs_concentration: self.concentrations_nonempty && !concentration_chosen,
            note_checks: self.note_checks(),
            attributes: self.attributes.clone(),
        }
    }

    fn resolve_substitutions(&mut self) {
        self.unmatched = self.unplaced.clone();
        for rule in &self.rules {
            let subs = self
                .substitutions_by_rule
                .get(&rule.id)
                .cloned()
                .unwrap_or_default();
            if subs
                .iter()
                .any(|s| matches!(s.change, SubstitutionChange::Waive))
            {
                self.effective.insert(rule.id.clone(), Effective::Waived);
            } else if let Some(replacement) = subs.iter().rev().find_map(|s| match &s.change {
                SubstitutionChange::Replace(r) => Some(r.clone()),
                _ => None,
            }) {
                self.effective
                    .insert(rule.id.clone(), self.replacement_kind(&replacement));
            } else {
                self.effective.insert(
                    rule.id.clone(),
                    Effective::Kind {
                        kind: rule.kind.clone(),
                        any_course: false,
                    },
                );
            }
        }
        for rule in &self.rules {
            let Some(Effective::Kind { kind, .. }) = self.effective.get(&rule.id) else {
                continue;
            };
            let listed = match kind {
                RuleKind::Required(option) => option.all_courses(),
                RuleKind::Choose { options, .. } => {
                    options.iter().flat_map(|o| o.all_courses()).collect()
                }
                RuleKind::GeneralElectives { .. } => vec![],
            };
            for key in listed {
                *self.demand.entry(key).or_default() += 1;
            }
        }
    }

    fn replacement_kind(&self, replacement: &RequirementReplacement) -> Effective {
        let mut options: Vec<CourseOption> = replacement
            .courses
            .iter()
            .map(|c| CourseOption {
                alternatives: vec![vec![c.clone()]],
            })
            .collect();
        let mut ranges = vec![];
        let mut any_course = false;
        let named = replacement
            .like_rule_title
            .as_ref()
            .and_then(|t| self.rules_by_name.get(&t.to_ascii_lowercase()))
            .filter(|s| s.len() == 1)
            .and_then(|s| s.iter().next().cloned());
        let like_by_id = replacement
            .like_rule_id
            .as_ref()
            .and_then(|id| self.requirements.rule(id).cloned());
        if like_by_id.is_none()
            && named.is_none()
            && replacement
                .like_rule_title
                .as_ref()
                .is_some_and(|t| t.to_ascii_lowercase().starts_with("general electives"))
        {
            any_course = true;
        }
        if let Some(like) = like_by_id.or_else(|| {
            named
                .as_ref()
                .and_then(|id| self.requirements.rule(id).cloned())
        }) {
            match like.kind {
                RuleKind::Required(option) => options.insert(0, option),
                RuleKind::Choose {
                    options: like_options,
                    ranges: like_ranges,
                    ..
                } => {
                    let mut combined = like_options;
                    combined.extend(options);
                    options = combined;
                    ranges = like_ranges
                        .iter()
                        .map(|r| apply_number_limit(r, replacement.course_numbers.as_ref()))
                        .collect();
                }
                RuleKind::GeneralElectives { .. } => any_course = true,
            }
        }
        let count = replacement.count.or(if replacement.hours.is_none() {
            Some(1)
        } else {
            None
        });
        Effective::Kind {
            kind: RuleKind::Choose {
                count,
                hours: replacement.hours,
                options,
                ranges,
            },
            any_course,
        }
    }

    fn apply_course_substitutions(&mut self) {
        for rule in self.rules.clone() {
            let Some(Effective::Kind { .. }) = self.effective.get(&rule.id) else {
                continue;
            };
            let subs = self
                .substitutions_by_rule
                .get(&rule.id)
                .cloned()
                .unwrap_or_default();
            for substitution in subs {
                let SubstitutionChange::CountCourse(key) = &substitution.change else {
                    continue;
                };
                let Some(taken) = self.stand_in(key, &HashSet::new()) else {
                    self.unmatched.push(substitution);
                    continue;
                };
                self.consume(&taken, &rule.id, true, Some(key.clone()));
                *self.options_counted.entry(rule.id.clone()).or_default() += 1;
            }
        }
    }

    fn fill_required(&mut self, rule: &RequirementRule, option: &CourseOption) {
        if !self
            .used
            .get(&rule.id)
            .map(|u| u.is_empty())
            .unwrap_or(true)
        {
            return;
        }
        let complete: Vec<Vec<CourseKey>> = option
            .alternatives
            .iter()
            .filter(|a| self.is_complete(a))
            .cloned()
            .collect();
        if let Some(best) = complete
            .iter()
            .min_by(|a, b| self.score(a).cmp(&self.score(b)))
        {
            for (required, taken) in self.assign(best) {
                self.consume(&taken, &rule.id, false, Some(required));
            }
            return;
        }
        let partial = option
            .alternatives
            .iter()
            .map(|a| self.assign(a))
            .reduce(|best, pairs| {
                if pairs.len() > best.len() {
                    pairs
                } else {
                    best
                }
            });
        for (required, taken) in partial.unwrap_or_default() {
            self.consume(&taken, &rule.id, false, Some(required));
        }
    }

    fn fill_choice(&mut self, rule: &RequirementRule) {
        let Some(Effective::Kind {
            kind:
                RuleKind::Choose {
                    count,
                    hours,
                    options,
                    ranges,
                },
            any_course,
        }) = self.effective.get(&rule.id).cloned()
        else {
            return;
        };
        let mut candidates = vec![];
        for (index, option) in options.iter().enumerate() {
            let complete: Vec<&Vec<CourseKey>> = option
                .alternatives
                .iter()
                .filter(|a| self.is_complete(a))
                .collect();
            let Some(best) = complete
                .iter()
                .copied()
                .min_by(|a, b| self.score(a).cmp(&self.score(b)))
            else {
                continue;
            };
            let s = self.score(best);
            candidates.push((best.clone(), (s.0, s.1, index as i32)));
        }
        candidates.sort_by_key(|c| c.1);
        for (alternative, _) in candidates {
            if self.is_satisfied(&rule.id, count, hours) {
                break;
            }
            if !self.is_complete(&alternative) {
                continue;
            }
            let pairs = self.assign(&alternative);
            let single = pairs.len() == 1;
            let first = pairs.first().map(|p| p.1.clone());
            for (required, taken) in pairs {
                self.consume(&taken, &rule.id, false, Some(required));
            }
            if single {
                if let Some(id) = first {
                    self.singles.entry(rule.id.clone()).or_default().insert(id);
                }
            }
            *self.options_counted.entry(rule.id.clone()).or_default() += 1;
        }
        if self.is_satisfied(&rule.id, count, hours) || (ranges.is_empty() && !any_course) {
            return;
        }
        let matches: Vec<(CourseUse, Option<CourseKey>)> = self
            .remaining()
            .into_iter()
            .filter_map(|use_| {
                if use_.credits <= 0.0 || (count.is_some() && use_.credits < 2.0) {
                    return None;
                }
                if any_course {
                    return Some((use_, None));
                }
                let in_range = self.course_in_range(&use_.key, &ranges)?;
                let via = if in_range == use_.key {
                    None
                } else {
                    Some(in_range)
                };
                Some((use_, via))
            })
            .collect();
        for (use_, via) in matches {
            if self.is_satisfied(&rule.id, count, hours) {
                break;
            }
            let id = use_.id();
            self.consume(&id, &rule.id, false, via);
            *self.options_counted.entry(rule.id.clone()).or_default() += 1;
            self.singles.entry(rule.id.clone()).or_default().insert(id);
        }
    }

    fn rebalance(&mut self) {
        let ids: Vec<String> = self.rules.iter().map(|r| r.id.clone()).collect();
        for id in ids {
            let mut budget = 2000i32;
            while !self.choice_is_satisfied(&id) {
                let mut moved = false;
                for steps in 0..=3 {
                    let avoid = HashSet::from([id.clone()]);
                    if let Some(next) = self.adding(&id, 0.0, steps, &avoid, &mut budget) {
                        *self = next;
                        moved = true;
                        break;
                    }
                }
                if !moved {
                    break;
                }
            }
        }
    }

    fn adding(
        &self,
        rule_id: &str,
        min_credits: f64,
        steps: i32,
        avoiding: &HashSet<String>,
        budget: &mut i32,
    ) -> Option<Self> {
        for use_ in self.remaining() {
            if use_.credits < min_credits {
                continue;
            }
            let Some(named) = self.single_match(&use_, rule_id) else {
                continue;
            };
            let mut next = self.clone();
            next.place(&use_.id(), rule_id, &named);
            return Some(next);
        }
        if steps <= 0 {
            return None;
        }
        for donor in &self.rules {
            if avoiding.contains(&donor.id) || !self.choice_is_satisfied(&donor.id) {
                continue;
            }
            let uses = self.used.get(&donor.id).cloned().unwrap_or_default();
            for use_ in uses {
                if use_.credits < min_credits
                    || !self
                        .singles
                        .get(&donor.id)
                        .is_some_and(|s| s.contains(&use_.id()))
                {
                    continue;
                }
                if *budget <= 0 {
                    return None;
                }
                *budget -= 1;
                let Some(named) = self.single_match(&use_, rule_id) else {
                    continue;
                };
                let mut next = self.clone();
                next.release(&use_.id(), &donor.id);
                next.place(&use_.id(), rule_id, &named);
                if next.choice_is_satisfied(&donor.id) {
                    return Some(next);
                }
                let mut avoid2 = avoiding.clone();
                avoid2.insert(rule_id.to_string());
                avoid2.insert(donor.id.clone());
                let short = next.shortfall(&donor.id);
                if let Some(refilled) = next.adding(&donor.id, short, steps - 1, &avoid2, budget) {
                    if refilled.choice_is_satisfied(&donor.id) {
                        return Some(refilled);
                    }
                }
            }
        }
        for donor in &self.rules {
            if avoiding.contains(&donor.id) {
                continue;
            }
            let Some(Effective::Kind {
                kind: RuleKind::Required(option),
                ..
            }) = self.effective.get(&donor.id)
            else {
                continue;
            };
            if option.alternatives.len() <= 1 {
                continue;
            }
            let current = self.used.get(&donor.id).cloned().unwrap_or_default();
            if current.is_empty() || current.iter().any(|u| u.by_substitution) {
                continue;
            }
            for use_ in &current {
                if use_.credits < min_credits {
                    continue;
                }
                if *budget <= 0 {
                    return None;
                }
                *budget -= 1;
                let Some(named) = self.single_match(use_, rule_id) else {
                    continue;
                };
                let mut next = self.clone();
                next.release_all(&donor.id);
                next.available.remove(&use_.id());
                let complete: Vec<Vec<CourseKey>> = option
                    .alternatives
                    .iter()
                    .filter(|a| next.is_complete(a))
                    .cloned()
                    .collect();
                let Some(best) = complete
                    .iter()
                    .min_by(|a, b| next.score(a).cmp(&next.score(b)))
                    .cloned()
                else {
                    continue;
                };
                let pairs = next.assign(&best);
                next.available.insert(use_.id());
                for (required, taken) in pairs {
                    next.consume(&taken, &donor.id, false, Some(required));
                }
                next.place(&use_.id(), rule_id, &named);
                return Some(next);
            }
        }
        None
    }

    fn single_match(&self, use_: &CourseUse, rule_id: &str) -> Option<CourseKey> {
        let Effective::Kind {
            kind:
                RuleKind::Choose {
                    count,
                    options,
                    ranges,
                    ..
                },
            any_course,
        } = self.effective.get(rule_id)?
        else {
            return None;
        };
        if count.is_none() && use_.credits <= 0.0 {
            return None;
        }
        let counted: HashSet<CourseKey> = self
            .used
            .get(rule_id)
            .map(|u| {
                u.iter()
                    .map(|c| c.satisfies.clone().unwrap_or_else(|| c.key.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for option in options {
            if option.all_courses().iter().any(|c| counted.contains(c)) {
                continue;
            }
            for alternative in &option.alternatives {
                if alternative.len() == 1
                    && self.equivalences.are_equivalent(&alternative[0], &use_.key)
                {
                    return Some(alternative[0].clone());
                }
            }
        }
        if use_.credits <= 0.0 || (count.is_some() && use_.credits < 2.0) {
            return None;
        }
        if *any_course {
            return Some(use_.key.clone());
        }
        if ranges.is_empty() {
            None
        } else {
            self.course_in_range(&use_.key, ranges)
        }
    }

    fn choice_is_satisfied(&self, rule_id: &str) -> bool {
        match self.effective.get(rule_id) {
            Some(Effective::Kind {
                kind: RuleKind::Choose { count, hours, .. },
                ..
            }) => self.is_satisfied(rule_id, *count, *hours),
            _ => true,
        }
    }

    fn shortfall(&self, rule_id: &str) -> f64 {
        match self.effective.get(rule_id) {
            Some(Effective::Kind {
                kind:
                    RuleKind::Choose {
                        count: None,
                        hours: Some(hours),
                        ..
                    },
                ..
            }) => {
                let done: f64 = self
                    .used
                    .get(rule_id)
                    .map(|u| u.iter().map(|c| c.credits).sum())
                    .unwrap_or(0.0);
                (hours - done).max(0.0)
            }
            _ => 0.0,
        }
    }

    fn place(&mut self, id: &str, rule_id: &str, named: &CourseKey) {
        self.consume(id, rule_id, false, Some(named.clone()));
        *self.options_counted.entry(rule_id.to_string()).or_default() += 1;
        self.singles
            .entry(rule_id.to_string())
            .or_default()
            .insert(id.to_string());
    }

    fn release(&mut self, id: &str, rule_id: &str) {
        let Some(list) = self.used.get_mut(rule_id) else {
            return;
        };
        let Some(index) = list.iter().position(|u| u.id() == id) else {
            return;
        };
        list.remove(index);
        if let Some(n) = self.options_counted.get_mut(rule_id) {
            *n -= 1;
        }
        if let Some(set) = self.singles.get_mut(rule_id) {
            set.remove(id);
        }
        self.available.insert(id.to_string());
    }

    fn release_all(&mut self, rule_id: &str) {
        for use_ in self.used.get(rule_id).cloned().unwrap_or_default() {
            self.available.insert(use_.id());
        }
        self.used.insert(rule_id.to_string(), vec![]);
        self.options_counted.insert(rule_id.to_string(), 0);
        self.singles.insert(rule_id.to_string(), HashSet::new());
    }

    fn is_satisfied(&self, rule_id: &str, count: Option<i32>, hours: Option<f64>) -> bool {
        let counted = self.options_counted.get(rule_id).copied().unwrap_or(0);
        if let Some(count) = count {
            return counted >= count;
        }
        if let Some(hours) = hours {
            let done: f64 = self
                .used
                .get(rule_id)
                .map(|u| u.iter().map(|c| c.credits).sum())
                .unwrap_or(0.0);
            return done >= hours;
        }
        counted >= 1
    }

    fn score(&self, alternative: &[CourseKey]) -> (i32, i32) {
        let taken: Vec<CourseUse> = self
            .assign(alternative)
            .into_iter()
            .filter_map(|(_, id)| self.pool.get(&id).cloned())
            .collect();
        let planned = taken.iter().filter(|u| !u.is_completed()).count() as i32;
        let contested = taken.iter().map(|u| self.contest(&u.key)).sum();
        (planned, contested)
    }

    fn contest(&self, key: &CourseKey) -> i32 {
        self.equivalences
            .group_of(key)
            .iter()
            .map(|k| self.demand.get(k).copied().unwrap_or(0))
            .sum()
    }

    fn consume(
        &mut self,
        id: &str,
        rule_id: &str,
        by_substitution: bool,
        required: Option<CourseKey>,
    ) {
        if !self.available.contains(id) {
            return;
        }
        let Some(mut use_) = self.pool.get(id).cloned() else {
            return;
        };
        use_.by_substitution = by_substitution;
        use_.satisfies = required.filter(|k| k != &use_.key);
        self.available.remove(id);
        self.used
            .entry(rule_id.to_string())
            .or_default()
            .append(&mut vec![use_]);
    }

    fn stand_in(&self, required: &CourseKey, claimed: &HashSet<String>) -> Option<String> {
        let free = |id: &String| self.available.contains(id) && !claimed.contains(id);
        if let Some(own) = self
            .instances
            .get(required)
            .map(|ids| {
                ids.iter()
                    .filter(|id| free(id))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .and_then(|ids| self.best_id(&ids))
        {
            return Some(own);
        }
        if self.equivalences.is_empty() {
            return None;
        }
        let ids: Vec<String> = self
            .equivalences
            .equivalents(required)
            .iter()
            .flat_map(|k| self.instances.get(k).cloned().unwrap_or_default())
            .filter(|id| free(id))
            .collect();
        self.best_id(&ids)
    }

    fn best_id(&self, ids: &[String]) -> Option<String> {
        ids.iter().min_by(|a, b| self.comes_first(a, b)).cloned()
    }

    fn comes_first(&self, a: &str, b: &str) -> std::cmp::Ordering {
        let rank = |id: &str| {
            let use_ = self.pool.get(id);
            (
                if use_.is_some_and(|u| u.is_completed()) {
                    0
                } else {
                    1
                },
                use_.map(|u| u.order).unwrap_or(i32::MAX),
                id.to_string(),
            )
        };
        rank(a).cmp(&rank(b))
    }

    fn assign(&self, alternative: &[CourseKey]) -> Vec<(CourseKey, String)> {
        let mut named = HashSet::new();
        let mut claimed = HashSet::new();
        let mut matches = vec![];
        for required in alternative {
            if !named.insert(required.clone()) {
                continue;
            }
            let Some(taken) = self.stand_in(required, &claimed) else {
                continue;
            };
            claimed.insert(taken.clone());
            matches.push((required.clone(), taken));
        }
        matches
    }

    fn is_complete(&self, alternative: &[CourseKey]) -> bool {
        let unique: HashSet<&CourseKey> = alternative.iter().collect();
        self.assign(alternative).len() == unique.len()
    }

    fn course_in_range(&self, key: &CourseKey, ranges: &[CourseRange]) -> Option<CourseKey> {
        if ranges.iter().any(|r| r.contains(key)) {
            return Some(key.clone());
        }
        self.equivalences
            .equivalents(key)
            .into_iter()
            .find(|eq| ranges.iter().any(|r| r.contains(eq)))
    }

    fn remaining(&self) -> Vec<CourseUse> {
        let mut uses: Vec<CourseUse> = self
            .available
            .iter()
            .filter_map(|id| self.pool.get(id).cloned())
            .collect();
        uses.sort_by_key(|u| (if u.is_completed() { 0 } else { 1 }, u.order));
        uses
    }

    fn result_for(&self, rule: &RequirementRule) -> RuleResult {
        let uses = self.used.get(&rule.id).cloned().unwrap_or_default();
        let substitutions = self
            .substitutions_by_rule
            .get(&rule.id)
            .cloned()
            .unwrap_or_default();
        let Some(Effective::Kind { kind, .. }) = self.effective.get(&rule.id).cloned() else {
            return RuleResult {
                rule: rule.clone(),
                effective_kind: None,
                status: AuditStatus::Waived,
                used: vec![],
                remaining_count: None,
                remaining_hours: None,
                substitutions,
            };
        };
        let hours_done: f64 = uses.iter().map(|u| u.credits).sum();
        let (satisfied, remaining_count, remaining_hours) = match &kind {
            RuleKind::Required(option) => {
                let keys: HashSet<CourseKey> = uses
                    .iter()
                    .map(|u| u.satisfies.clone().unwrap_or_else(|| u.key.clone()))
                    .collect();
                let ok = uses.iter().any(|u| u.by_substitution)
                    || option
                        .alternatives
                        .iter()
                        .any(|alt| alt.iter().all(|k| keys.contains(k)));
                (ok, None, None)
            }
            RuleKind::Choose { count, hours, .. } => {
                let ok = self.is_satisfied(&rule.id, *count, *hours);
                if let Some(count) = count {
                    (
                        ok,
                        Some(
                            (count - self.options_counted.get(&rule.id).copied().unwrap_or(0))
                                .max(0),
                        ),
                        None,
                    )
                } else if let Some(hours) = hours {
                    (ok, None, Some((hours - hours_done).max(0.0)))
                } else {
                    (ok, Some(if ok { 0 } else { 1 }), None)
                }
            }
            RuleKind::GeneralElectives { hours } => (
                hours_done >= *hours,
                None,
                Some((hours - hours_done).max(0.0)),
            ),
        };
        let status = if satisfied {
            if uses.iter().all(|u| u.is_completed()) {
                AuditStatus::Complete
            } else {
                AuditStatus::Planned
            }
        } else if uses.is_empty() {
            AuditStatus::Missing
        } else {
            AuditStatus::Partial
        };
        RuleResult {
            rule: rule.clone(),
            effective_kind: Some(kind),
            status,
            used: uses,
            remaining_count,
            remaining_hours,
            substitutions,
        }
    }

    fn trim_surplus_hours(&mut self) {
        for rule in self.rules.clone() {
            let Some(Effective::Kind {
                kind:
                    RuleKind::Choose {
                        count: None,
                        hours: Some(hours),
                        ..
                    },
                ..
            }) = self.effective.get(&rule.id).cloned()
            else {
                continue;
            };
            if !self.choice_is_satisfied(&rule.id) {
                continue;
            }
            let uses = self.used.get(&rule.id).cloned().unwrap_or_default();
            let movable: Vec<CourseUse> = uses
                .iter()
                .filter(|u| {
                    !u.by_substitution
                        && self
                            .singles
                            .get(&rule.id)
                            .is_some_and(|s| s.contains(&u.id()))
                })
                .cloned()
                .collect();
            let movable_ids: HashSet<String> = movable.iter().map(|u| u.id()).collect();
            let fixed: f64 = uses
                .iter()
                .filter(|u| !movable_ids.contains(&u.id()))
                .map(|u| u.credits)
                .sum();
            let unused: Vec<(CourseUse, CourseKey)> = self
                .remaining()
                .into_iter()
                .filter_map(|u| self.single_match(&u, &rule.id).map(|n| (u, n)))
                .collect();
            let mut pool = movable.clone();
            pool.extend(unused.iter().map(|(u, _)| u.clone()));
            let Some(keep) = fewest_credits(&pool, hours - fixed) else {
                continue;
            };
            for use_ in &movable {
                if !keep.contains(&use_.id()) {
                    self.release(&use_.id(), &rule.id);
                }
            }
            for (use_, named) in &unused {
                if keep.contains(&use_.id()) {
                    self.place(&use_.id(), &rule.id, named);
                }
            }
        }
    }

    fn note_checks(&self) -> Vec<NoteCheck> {
        let mut checks = vec![];
        for section in &self.sections {
            let mut notes = section.notes.clone();
            for group in &section.groups {
                if group.rules.is_empty() {
                    notes.extend(group.notes.clone());
                }
            }
            for note in notes {
                if let Some(hours) = major_hours(&note) {
                    checks.push(self.major_hours_check(&section.id, &note, hours));
                } else if is_coop_note(&note) {
                    checks.push(self.coop_check(&section.id, &note));
                } else if is_nupath_note(&note) {
                    if let Some(check) = self.nupath_check(&section.id, &note) {
                        checks.push(check);
                    }
                } else if is_universitywide_note(&note) {
                    if let Some(check) = self.universitywide_check(&section.id, &note) {
                        checks.push(check);
                    }
                } else if let Some(hours) = total_hours_note(&note) {
                    checks.push(self.total_hours_check(&section.id, &note, hours));
                }
            }
        }
        checks
    }

    fn major_hours_check(&self, section_id: &str, note: &str, required: f64) -> NoteCheck {
        let mut counted: HashMap<String, CourseUse> = HashMap::new();
        let mut ranges = vec![];
        for section in &self.sections {
            for group in &section.groups {
                let is_writing = section.title.to_ascii_lowercase().contains("writing")
                    || group
                        .title
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains("writing");
                for rule in &group.rules {
                    let Some(Effective::Kind { kind, .. }) = self.effective.get(&rule.id) else {
                        continue;
                    };
                    if matches!(kind, RuleKind::GeneralElectives { .. }) {
                        continue;
                    }
                    if let RuleKind::Choose {
                        ranges: rule_ranges,
                        ..
                    } = kind
                    {
                        ranges.extend(rule_ranges.clone());
                    }
                    if is_writing {
                        continue;
                    }
                    for use_ in self.used.get(&rule.id).cloned().unwrap_or_default() {
                        counted.insert(use_.id(), use_);
                    }
                }
            }
        }
        for use_ in self.pool.values() {
            if counted.contains_key(&use_.id()) {
                continue;
            }
            if self.course_in_range(&use_.key, &ranges).is_some() {
                counted.insert(use_.id(), use_.clone());
            }
        }
        let completed: f64 = counted
            .values()
            .filter(|u| u.is_completed())
            .map(|u| u.credits.max(0.0))
            .sum();
        let planned: f64 = counted
            .values()
            .filter(|u| !u.is_completed())
            .map(|u| u.credits.max(0.0))
            .sum();
        let (status, detail) = hours_status(completed, planned, required);
        NoteCheck {
            section_id: section_id.into(),
            note: note.into(),
            kind: NoteKind::MajorHours { required },
            status,
            detail,
            items: vec![],
            counts: true,
        }
    }

    fn coop_check(&self, section_id: &str, note: &str) -> NoteCheck {
        let mut coops: Vec<CourseUse> =
            self.pool.values().filter(|u| is_coop(u)).cloned().collect();
        coops.sort_by_key(|u| u.order);
        let (mut status, mut detail) = if let Some(done) = coops.iter().find(|u| u.is_completed()) {
            (AuditStatus::Complete, format!("{} completed", done.key))
        } else if let Some(next) = coops.first() {
            let when = next
                .semester
                .as_ref()
                .map(|s| format!(" · {s}"))
                .unwrap_or_default();
            (AuditStatus::Planned, format!("{} planned{when}", next.key))
        } else {
            (AuditStatus::Missing, "No co-op in the plan".into())
        };
        if coops.is_empty() {
            if let Some(path) = nupath::satisfied_through_coop(note) {
                let mut courses: Vec<CourseUse> = self
                    .pool
                    .values()
                    .filter(|u| self.nupaths_of(u).is_some_and(|p| p.contains(&path)))
                    .cloned()
                    .collect();
                courses.sort_by_key(|u| (if u.is_completed() { 0 } else { 1 }, u.order));
                if let Some(course) = courses.first() {
                    status = if course.is_completed() {
                        AuditStatus::Complete
                    } else {
                        AuditStatus::Planned
                    };
                    let word = if course.is_completed() {
                        "completed"
                    } else {
                        "planned"
                    };
                    detail = format!(
                        "No co-op in the plan · {} ({}) {word}",
                        course.key,
                        path.short()
                    );
                }
            }
        }
        NoteCheck {
            section_id: section_id.into(),
            note: note.into(),
            kind: NoteKind::Coop,
            status,
            detail,
            items: vec![],
            counts: true,
        }
    }

    fn nupaths_of(&self, use_: &CourseUse) -> Option<HashSet<NuPath>> {
        if let Some(paths) = self.attributes.nupaths.get(&use_.key) {
            return Some(paths.clone());
        }
        if is_coop(use_) {
            return Some(HashSet::new());
        }
        for other in self.equivalences.group_of(&use_.key) {
            if other != use_.key {
                if let Some(paths) = self.attributes.nupaths.get(&other) {
                    return Some(paths.clone());
                }
            }
        }
        None
    }

    fn nupath_check(&self, section_id: &str, note: &str) -> Option<NoteCheck> {
        if self.attributes.is_empty() {
            return None;
        }
        let mut uses: Vec<CourseUse> = self.pool.values().cloned().collect();
        uses.sort_by_key(|u| u.order);
        let mut courses = vec![];
        let mut unknown = vec![];
        for use_ in &uses {
            match self.nupaths_of(use_) {
                Some(paths) if !paths.is_empty() => courses.push(NuCourse {
                    id: use_.id(),
                    paths,
                    is_completed: use_.is_completed(),
                }),
                Some(_) => {}
                None => unknown.push(use_.clone()),
            }
        }
        let coops: Vec<NuCourse> = uses
            .iter()
            .filter(|u| is_coop(u))
            .map(|u| NuCourse {
                id: u.id(),
                paths: HashSet::new(),
                is_completed: u.is_completed(),
            })
            .collect();
        let mut coop_counts: HashMap<NuPath, Vec<NuCourse>> = HashMap::new();
        if !coops.is_empty() {
            for section in &self.requirements.sections {
                for note in section
                    .notes
                    .iter()
                    .chain(section.groups.iter().flat_map(|g| g.notes.iter()))
                {
                    if let Some(path) = nupath::satisfied_through_coop(note) {
                        coop_counts.insert(path, coops.clone());
                    }
                }
            }
        }
        let matching = nupath::Matching::new(&courses, &coop_counts);
        let is_loading = unknown
            .iter()
            .any(|u| self.attributes.loading.contains(&u.key));
        let mut items = vec![];
        for requirement in nupath::NuRequirement::ALL {
            let chosen: Vec<CourseUse> = matching
                .courses_for(requirement.path)
                .into_iter()
                .filter_map(|id| self.pool.get(&id).cloned())
                .collect();
            let (status, detail) = if chosen.len() >= requirement.count as usize {
                let status = if chosen.iter().all(|u| u.is_completed()) {
                    AuditStatus::Complete
                } else {
                    AuditStatus::Planned
                };
                (Some(status), describe(&chosen))
            } else if unknown.is_empty() {
                let status = if chosen.is_empty() {
                    AuditStatus::Missing
                } else {
                    AuditStatus::Partial
                };
                let detail = if chosen.is_empty() {
                    if requirement.count == 1 {
                        "No course in the plan has it".into()
                    } else {
                        format!("Needs {} courses", requirement.count)
                    }
                } else {
                    format!(
                        "{} of {} · {}",
                        chosen.len(),
                        requirement.count,
                        describe(&chosen)
                    )
                };
                (Some(status), detail)
            } else {
                let found = if chosen.is_empty() {
                    String::new()
                } else {
                    format!(
                        "{} of {} · {}",
                        chosen.len(),
                        requirement.count,
                        describe(&chosen)
                    )
                };
                let detail = if found.is_empty() {
                    if is_loading {
                        "Looking up the plan’s courses…".into()
                    } else {
                        "Not found among the courses with NUpath data".into()
                    }
                } else {
                    found
                };
                (None, detail)
            };
            items.push(NoteItem {
                id: requirement.path.code().into(),
                title: requirement.title(),
                status,
                detail,
                courses: chosen.iter().map(|c| c.id()).collect(),
            });
        }
        let met: Vec<&NoteItem> = items
            .iter()
            .filter(|i| i.status.is_some_and(|s| s.is_satisfied()))
            .collect();
        let open: Vec<&NoteItem> = items
            .iter()
            .filter(|i| !i.status.is_some_and(|s| s.is_satisfied()))
            .collect();
        let status = if open.is_empty() {
            if met.iter().all(|i| i.status == Some(AuditStatus::Complete)) {
                AuditStatus::Complete
            } else {
                AuditStatus::Planned
            }
        } else if met.is_empty() && items.iter().all(|i| i.courses.is_empty()) {
            AuditStatus::Missing
        } else {
            AuditStatus::Partial
        };
        let mut detail = format!("{} of {} met", met.len(), items.len());
        let missing: Vec<&NoteItem> = items
            .iter()
            .filter(|i| {
                i.status == Some(AuditStatus::Missing) || i.status == Some(AuditStatus::Partial)
            })
            .collect();
        if !missing.is_empty() && missing.len() <= 3 {
            let names = missing
                .iter()
                .map(|i| {
                    NuPath::ALL
                        .into_iter()
                        .find(|p| p.code() == i.id)
                        .map(|p| p.short().to_string())
                        .unwrap_or_else(|| i.title.clone())
                })
                .collect::<Vec<_>>()
                .join(", ");
            detail.push_str(" · missing ");
            detail.push_str(&names);
        }
        if !unknown.is_empty() && items.iter().any(|i| i.status.is_none()) {
            if is_loading {
                detail.push_str(&format!(
                    " · looking up {}",
                    if unknown.len() == 1 {
                        "1 course".into()
                    } else {
                        format!("{} courses", unknown.len())
                    }
                ));
            } else {
                let names = unknown
                    .iter()
                    .take(3)
                    .map(|u| u.key.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let extra = if unknown.len() > 3 {
                    format!(" and {} more", unknown.len() - 3)
                } else {
                    String::new()
                };
                detail.push_str(&format!(" · no NUpath data for {names}{extra}"));
            }
        }
        let counts = !items.iter().any(|i| i.status.is_none());
        Some(NoteCheck {
            section_id: section_id.into(),
            note: note.into(),
            kind: NoteKind::Nupath,
            status,
            detail,
            items,
            counts,
        })
    }

    fn universitywide_check(&self, section_id: &str, note: &str) -> Option<NoteCheck> {
        let minimums = self.requirements.universitywide.clone()?;
        let completed: f64 = self
            .pool
            .values()
            .filter(|u| u.is_completed())
            .map(|u| u.credits.max(0.0))
            .sum();
        let planned: f64 = self
            .pool
            .values()
            .filter(|u| !u.is_completed())
            .map(|u| u.credits.max(0.0))
            .sum();
        let mut items = vec![];
        if let Some(total) = minimums.total_hours {
            let (status, detail) = hours_status(completed, planned, total);
            items.push(NoteItem {
                id: "total-hours".into(),
                title: format!("{} total semester hours", hours_text(total)),
                status: Some(status),
                detail,
                courses: vec![],
            });
        }
        if minimums.requires_experiential_learning {
            let mut coops: Vec<CourseUse> =
                self.pool.values().filter(|u| is_coop(u)).cloned().collect();
            coops.sort_by_key(|u| u.order);
            let coop = coops.iter().find(|u| u.is_completed()).or(coops.first());
            items.push(NoteItem {
                id: "experiential".into(),
                title: "Experiential learning".into(),
                status: coop.map(|u| {
                    if u.is_completed() {
                        AuditStatus::Complete
                    } else {
                        AuditStatus::Planned
                    }
                }),
                detail: coop
                    .map(|u| describe(std::slice::from_ref(u)))
                    .unwrap_or_else(|| "Not checked: only co-ops are recognized".into()),
                courses: coop.map(|u| vec![u.id()]).unwrap_or_default(),
            });
        }
        if let Some(gpa) = minimums.gpa {
            items.push(NoteItem {
                id: "gpa".into(),
                title: format!("{} GPA", gpa_text(gpa)),
                status: None,
                detail: "Not checked: plans don’t include grades".into(),
                courses: vec![],
            });
        }
        if let Some(residency) = minimums.residency_hours {
            items.push(NoteItem {
                id: "residency".into(),
                title: format!("{} semester hours at Northeastern", hours_text(residency)),
                status: None,
                detail: "Not checked: a plan doesn’t say where credits were earned".into(),
                courses: vec![],
            });
        }
        let checked: Vec<&NoteItem> = items.iter().filter(|i| i.status.is_some()).collect();
        let status = checked.iter().filter_map(|i| i.status).min()?;
        let names = [
            ("experiential", "experiential learning"),
            ("gpa", "GPA"),
            ("residency", "residency"),
        ];
        let unchecked: Vec<String> = items
            .iter()
            .filter(|i| i.status.is_none())
            .filter_map(|i| {
                names
                    .iter()
                    .find(|(id, _)| *id == i.id)
                    .map(|(_, n)| (*n).to_string())
            })
            .collect();
        let mut detail = checked
            .iter()
            .map(|i| i.detail.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        if !unchecked.is_empty() {
            detail.push_str(" · ");
            detail.push_str(&english_list(&unchecked));
            detail.push_str(" not checked");
        }
        Some(NoteCheck {
            section_id: section_id.into(),
            note: note.into(),
            kind: NoteKind::Universitywide,
            status,
            detail,
            items,
            counts: true,
        })
    }

    fn total_hours_check(&self, section_id: &str, note: &str, required: f64) -> NoteCheck {
        let completed: f64 = self
            .pool
            .values()
            .filter(|u| u.is_completed())
            .map(|u| u.credits.max(0.0))
            .sum();
        let planned: f64 = self
            .pool
            .values()
            .filter(|u| !u.is_completed())
            .map(|u| u.credits.max(0.0))
            .sum();
        let (status, detail) = hours_status(completed, planned, required);
        NoteCheck {
            section_id: section_id.into(),
            note: note.into(),
            kind: NoteKind::TotalHours { required },
            status,
            detail,
            items: vec![],
            counts: true,
        }
    }

    fn is_met(&self, rule_id: &str) -> bool {
        match self.effective.get(rule_id) {
            Some(Effective::Kind {
                kind: RuleKind::Required(option),
                ..
            }) => {
                let uses = self.used.get(rule_id).cloned().unwrap_or_default();
                let keys: HashSet<CourseKey> = uses
                    .iter()
                    .map(|u| u.satisfies.clone().unwrap_or_else(|| u.key.clone()))
                    .collect();
                uses.iter().any(|u| u.by_substitution)
                    || option
                        .alternatives
                        .iter()
                        .any(|alt| alt.iter().all(|k| keys.contains(k)))
            }
            Some(Effective::Kind {
                kind: RuleKind::Choose { count, hours, .. },
                ..
            }) => self.is_satisfied(rule_id, *count, *hours),
            Some(Effective::Kind {
                kind: RuleKind::GeneralElectives { hours },
                ..
            }) => self
                .used
                .get(rule_id)
                .map(|u| u.iter().map(|c| c.credits).sum::<f64>() >= *hours)
                .unwrap_or(false),
            _ => true,
        }
    }

    fn ensure_best_matching(&mut self) -> bool {
        let pinned: HashSet<String> = self
            .used
            .values()
            .flat_map(|u| u.iter().filter(|c| c.by_substitution).map(|c| c.id()))
            .collect();
        let mut free: Vec<String> = self
            .pool
            .keys()
            .filter(|id| !pinned.contains(*id))
            .cloned()
            .collect();
        free.sort_by(|a, b| self.comes_first(a, b));
        if free.is_empty() || free.len() > 128 {
            return false;
        }
        let index: HashMap<String, usize> = free
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();
        let credits: Vec<f64> = free
            .iter()
            .map(|id| self.pool.get(id).map(|u| u.credits.max(0.0)).unwrap_or(0.0))
            .collect();
        let mut all = CourseBits::default();
        for i in 0..free.len() {
            all = all.union(CourseBits::one(i));
        }
        let mut needs = vec![];
        let mut electives = vec![];
        for rule in &self.rules {
            match self.effective.get(&rule.id).cloned() {
                Some(Effective::Kind {
                    kind: RuleKind::GeneralElectives { hours },
                    ..
                }) => {
                    let counted: f64 = self
                        .used
                        .get(&rule.id)
                        .map(|u| {
                            u.iter()
                                .filter(|c| c.by_substitution)
                                .map(|c| c.credits)
                                .sum()
                        })
                        .unwrap_or(0.0);
                    if counted < hours {
                        electives.push(hours - counted);
                    }
                }
                Some(Effective::Kind { kind, any_course }) => {
                    if let Some(need) = self.need_for(&rule.id, &kind, any_course, &free, &index) {
                        needs.push(need);
                    }
                }
                _ => {}
            }
        }
        let scale = self.rules.len() as i32 + 1;
        let electives_met = |available: CourseBits| -> i32 {
            if electives.is_empty() {
                return 0;
            }
            let mut left = 0.0;
            for (i, credit) in credits.iter().enumerate() {
                if available.has(i) {
                    left += credit;
                }
            }
            let mut met = 0;
            for hours in &electives {
                if left >= *hours {
                    met += 1;
                    left -= hours;
                }
            }
            met
        };
        let mut now = CourseBits::default();
        for id in &self.available {
            if let Some(i) = index.get(id) {
                now = now.union(CourseBits::one(*i));
            }
        }
        let current_specific = needs.iter().filter(|n| self.is_met(&n.rule_id)).count() as i32;
        let current_electives = electives_met(now);
        let current = current_specific * scale + current_electives;
        let may_improve = needs
            .iter()
            .any(|n| !self.is_met(&n.rule_id) && can_still_meet(n, all, &credits))
            || electives_met(all) > electives_met(now);
        if !may_improve {
            return false;
        }
        let mut order: Vec<usize> = (0..needs.len()).collect();
        order.sort_by_key(|i| (fills_for(&needs[*i], all, 400).len(), *i));
        let mut state = SearchState {
            nodes: 0,
            best: None,
            best_score: current,
            best_specific: current_specific,
            best_electives: current_electives,
            chosen: HashMap::new(),
            visited: HashMap::new(),
        };
        visit(
            0,
            all,
            0,
            &order,
            &needs,
            &credits,
            &electives,
            scale,
            &mut state,
            &electives_met,
        );
        let Some(best) = state.best else { return false };
        let meets_everything = state.best_specific == needs.len() as i32
            && state.best_electives == electives.len() as i32;
        if state.best_specific <= current_specific && !meets_everything {
            return false;
        }
        self.adopt(&best, &needs, &free);
        true
    }

    fn need_for(
        &self,
        rule_id: &str,
        kind: &RuleKind,
        any_course: bool,
        free: &[String],
        index: &HashMap<String, usize>,
    ) -> Option<Need> {
        let pinned = self
            .used
            .get(rule_id)
            .map(|u| {
                u.iter()
                    .filter(|c| c.by_substitution)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        match kind {
            RuleKind::Required(option) => {
                if !pinned.is_empty() {
                    return None;
                }
                let mut units = vec![];
                for alternative in &option.alternatives {
                    for assignment in self.assignments_of(alternative, free) {
                        units.push(self.unit(&assignment, 0, index));
                    }
                }
                units.sort_by(preferred_unit);
                Some(Need {
                    rule_id: rule_id.into(),
                    amount: NeedAmount::Alternative,
                    units,
                })
            }
            RuleKind::Choose {
                count,
                hours,
                options,
                ranges,
            } => {
                let counted: HashSet<CourseKey> = pinned
                    .iter()
                    .map(|u| u.satisfies.clone().unwrap_or_else(|| u.key.clone()))
                    .collect();
                let mut units = vec![];
                for (number, option) in options.iter().enumerate() {
                    if option.all_courses().iter().any(|c| counted.contains(c)) {
                        continue;
                    }
                    for alternative in &option.alternatives {
                        for assignment in self.assignments_of(alternative, free) {
                            units.push(self.unit(&assignment, number as i32, index));
                        }
                    }
                }
                if !ranges.is_empty() || any_course {
                    let mut tag = options.len() as i32;
                    for id in free {
                        let Some(use_) = self.pool.get(id) else {
                            continue;
                        };
                        if use_.credits <= 0.0 || (count.is_some() && use_.credits < 2.0) {
                            continue;
                        }
                        let named = if any_course {
                            Some(use_.key.clone())
                        } else {
                            self.course_in_range(&use_.key, ranges)
                        };
                        let Some(named) = named else { continue };
                        units.push(self.unit(&[(named, id.clone())], tag, index));
                        tag += 1;
                    }
                }
                let amount = if let Some(count) = count {
                    if *count - pinned.len() as i32 <= 0 {
                        return None;
                    }
                    NeedAmount::Count(*count - pinned.len() as i32)
                } else if let Some(hours) = hours {
                    let left = *hours - pinned.iter().map(|u| u.credits).sum::<f64>();
                    if left <= 0.0 {
                        return None;
                    }
                    units.retain(|u| u.credits > 0.0);
                    NeedAmount::Hours(left)
                } else {
                    if !pinned.is_empty() {
                        return None;
                    }
                    NeedAmount::Count(1)
                };
                units.sort_by(preferred_unit);
                Some(Need {
                    rule_id: rule_id.into(),
                    amount,
                    units,
                })
            }
            RuleKind::GeneralElectives { .. } => None,
        }
    }

    fn assignments_of(
        &self,
        alternative: &[CourseKey],
        free: &[String],
    ) -> Vec<Vec<(CourseKey, String)>> {
        let free_set: HashSet<&str> = free.iter().map(|s| s.as_str()).collect();
        let mut seen = HashSet::new();
        let named: Vec<CourseKey> = alternative
            .iter()
            .filter(|k| seen.insert((*k).clone()))
            .cloned()
            .collect();
        let mut results = vec![];
        let mut current = vec![];
        let mut claimed = HashSet::new();
        fn extend(
            engine: &AuditEngine,
            position: usize,
            named: &[CourseKey],
            free_set: &HashSet<&str>,
            results: &mut Vec<Vec<(CourseKey, String)>>,
            current: &mut Vec<(CourseKey, String)>,
            claimed: &mut HashSet<String>,
        ) {
            if results.len() >= 4 {
                return;
            }
            if position >= named.len() {
                results.push(current.clone());
                return;
            }
            let key = &named[position];
            let mut candidates = engine.instances.get(key).cloned().unwrap_or_default();
            for eq in engine.equivalences.equivalents(key) {
                candidates.extend(engine.instances.get(&eq).cloned().unwrap_or_default());
            }
            for id in candidates {
                if !free_set.contains(id.as_str()) || claimed.contains(&id) {
                    continue;
                }
                current.push((key.clone(), id.clone()));
                claimed.insert(id.clone());
                extend(
                    engine,
                    position + 1,
                    named,
                    free_set,
                    results,
                    current,
                    claimed,
                );
                claimed.remove(&id);
                current.pop();
            }
        }
        extend(
            self,
            0,
            &named,
            &free_set,
            &mut results,
            &mut current,
            &mut claimed,
        );
        results
    }

    fn unit(
        &self,
        pairs: &[(CourseKey, String)],
        option: i32,
        index: &HashMap<String, usize>,
    ) -> Unit {
        let uses: Vec<CourseUse> = pairs
            .iter()
            .filter_map(|(_, id)| self.pool.get(id).cloned())
            .collect();
        let mut bits = CourseBits::default();
        for (_, id) in pairs {
            if let Some(i) = index.get(id) {
                bits = bits.union(CourseBits::one(*i));
            }
        }
        Unit {
            courses: pairs.iter().map(|(_, id)| id.clone()).collect(),
            named: pairs.iter().map(|(k, _)| k.clone()).collect(),
            bits,
            option,
            credits: uses.iter().map(|u| u.credits.max(0.0)).sum(),
            is_completed: uses.iter().all(|u| u.is_completed()),
            order: uses.iter().map(|u| u.order).min().unwrap_or(0),
        }
    }

    fn adopt(&mut self, fills: &HashMap<usize, Vec<usize>>, needs: &[Need], free: &[String]) {
        for need in needs {
            let pinned: Vec<CourseUse> = self
                .used
                .get(&need.rule_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|u| u.by_substitution)
                .collect();
            let count = pinned.len() as i32;
            self.used.insert(need.rule_id.clone(), pinned);
            self.options_counted.insert(need.rule_id.clone(), count);
            self.singles.insert(need.rule_id.clone(), HashSet::new());
        }
        self.available = free.iter().cloned().collect();
        for (n, chosen) in fills {
            let rule_id = &needs[*n].rule_id;
            for position in chosen {
                let unit = &needs[*n].units[*position];
                for (named, id) in unit.named.iter().zip(unit.courses.iter()) {
                    self.consume(id, rule_id, false, Some(named.clone()));
                }
                *self.options_counted.entry(rule_id.clone()).or_default() += 1;
                if unit.courses.len() == 1 {
                    self.singles
                        .entry(rule_id.clone())
                        .or_default()
                        .insert(unit.courses[0].clone());
                }
            }
        }
    }
}

fn is_coop(use_: &CourseUse) -> bool {
    use_.key.subject == "COOP"
        || regex::Regex::new(r"(?i)co-?op\s+(?:work\s+)?experience|cooperative\s+education")
            .unwrap()
            .is_match(&use_.title)
}

fn is_nupath_note(note: &str) -> bool {
    regex::Regex::new(r"(?i)complete\s+(?:the\s+)?NUpath\s+requirements")
        .unwrap()
        .is_match(note)
}
fn is_universitywide_note(note: &str) -> bool {
    regex::Regex::new(r"(?i)complete\s+(?:the\s+)?university-?\s?wide\s+requirements")
        .unwrap()
        .is_match(note)
}
fn total_hours_note(note: &str) -> Option<f64> {
    let re =
        regex::Regex::new(r"(?i)(\d+(?:\.\d+)?)\s+total\s+semester\s+hours\s+required").unwrap();
    re.captures(note)?.get(1)?.as_str().parse().ok()
}
fn major_hours(note: &str) -> Option<f64> {
    let re = regex::Regex::new(
        r"(?i)(\d+(?:\.\d+)?)\s+(?:semester\s+)?(?:credit\s+)?hours\s+(?:in|within)\s+the\s+major",
    )
    .unwrap();
    re.captures(note)?.get(1)?.as_str().parse().ok()
}

fn hours_status(completed: f64, planned: f64, required: f64) -> (AuditStatus, String) {
    let status = if completed >= required {
        AuditStatus::Complete
    } else if completed + planned >= required {
        AuditStatus::Planned
    } else if completed + planned > 0.0 {
        AuditStatus::Partial
    } else {
        AuditStatus::Missing
    };
    let mut detail = format!(
        "{} of {} semester hours",
        hours_text(completed + planned),
        hours_text(required)
    );
    if planned > 0.0 {
        detail.push_str(&format!(" · {} planned", hours_text(planned)));
    }
    (status, detail)
}

fn describe(uses: &[CourseUse]) -> String {
    uses.iter()
        .map(|use_| {
            if use_.is_completed() {
                format!("{} completed", use_.key)
            } else {
                let when = use_
                    .semester
                    .as_ref()
                    .map(|s| format!(" · {s}"))
                    .unwrap_or_default();
                format!("{} planned{when}", use_.key)
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn fewest_credits(uses: &[CourseUse], need: f64) -> Option<HashSet<String>> {
    if need <= 0.0 {
        return Some(HashSet::new());
    }
    let items: Vec<&CourseUse> = uses.iter().filter(|u| u.credits > 0.0).collect();
    let weights: Vec<usize> = items
        .iter()
        .map(|u| ((u.credits * 2.0).round() as usize).max(1))
        .collect();
    let target = (need * 2.0).ceil() as usize;
    if weights.iter().sum::<usize>() < target {
        return None;
    }
    let largest = *weights.iter().max()?;
    let cap = target + largest;
    #[derive(Clone)]
    struct Pick {
        completed: i32,
        count: i32,
        order: i32,
        items: Vec<usize>,
    }
    impl Pick {
        fn better(&self, other: &Pick) -> bool {
            (-self.completed, self.count, self.order) < (-other.completed, other.count, other.order)
        }
    }
    let mut best: Vec<Option<Pick>> = vec![None; cap + 1];
    best[0] = Some(Pick {
        completed: 0,
        count: 0,
        order: 0,
        items: vec![],
    });
    for (index, item) in items.iter().enumerate() {
        let weight = weights[index];
        if weight > cap {
            continue;
        }
        for sum in (weight..=cap).rev() {
            let Some(prev) = best[sum - weight].clone() else {
                continue;
            };
            let mut pick = prev;
            pick.completed += if item.is_completed() { 1 } else { 0 };
            pick.count += 1;
            pick.order += item.order;
            pick.items.push(index);
            if best[sum]
                .as_ref()
                .is_some_and(|current| !pick.better(current))
            {
                continue;
            }
            best[sum] = Some(pick);
        }
    }
    let pick = (target..=cap).find_map(|s| best[s].clone())?;
    Some(pick.items.into_iter().map(|i| items[i].id()).collect())
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
struct CourseBits {
    low: u64,
    high: u64,
}

impl CourseBits {
    fn one(index: usize) -> Self {
        if index < 64 {
            Self {
                low: 1 << index,
                high: 0,
            }
        } else if index < 128 {
            Self {
                low: 0,
                high: 1 << (index - 64),
            }
        } else {
            Self::default()
        }
    }
    fn contains(self, other: Self) -> bool {
        self.low & other.low == other.low && self.high & other.high == other.high
    }
    fn intersects(self, other: Self) -> bool {
        self.low & other.low != 0 || self.high & other.high != 0
    }
    fn has(self, index: usize) -> bool {
        self.contains(Self::one(index)) && index < 128
    }
    fn union(self, other: Self) -> Self {
        Self {
            low: self.low | other.low,
            high: self.high | other.high,
        }
    }
    fn subtracting(self, other: Self) -> Self {
        Self {
            low: self.low & !other.low,
            high: self.high & !other.high,
        }
    }
}

#[derive(Clone)]
struct Unit {
    courses: Vec<String>,
    named: Vec<CourseKey>,
    bits: CourseBits,
    option: i32,
    credits: f64,
    is_completed: bool,
    order: i32,
}

#[derive(Clone)]
enum NeedAmount {
    Alternative,
    Count(i32),
    Hours(f64),
}

struct Need {
    rule_id: String,
    amount: NeedAmount,
    units: Vec<Unit>,
}

fn preferred_unit(a: &Unit, b: &Unit) -> std::cmp::Ordering {
    (if a.is_completed { 0 } else { 1 }, a.courses.len(), a.order).cmp(&(
        if b.is_completed { 0 } else { 1 },
        b.courses.len(),
        b.order,
    ))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SearchKey {
    depth: usize,
    low: u64,
    high: u64,
}

struct SearchState {
    nodes: i32,
    best: Option<HashMap<usize, Vec<usize>>>,
    best_score: i32,
    best_specific: i32,
    best_electives: i32,
    chosen: HashMap<usize, Vec<usize>>,
    visited: HashMap<SearchKey, i32>,
}

fn can_still_meet(need: &Need, available: CourseBits, credits: &[f64]) -> bool {
    match need.amount {
        NeedAmount::Alternative => need.units.iter().any(|u| available.contains(u.bits)),
        NeedAmount::Count(wanted) => {
            let mut options = HashSet::new();
            for unit in &need.units {
                if available.contains(unit.bits) {
                    options.insert(unit.option);
                    if options.len() as i32 >= wanted {
                        return true;
                    }
                }
            }
            false
        }
        NeedAmount::Hours(wanted) => {
            let mut covered = CourseBits::default();
            for unit in &need.units {
                if available.contains(unit.bits) {
                    covered = covered.union(unit.bits);
                }
            }
            let sum: f64 = credits
                .iter()
                .enumerate()
                .filter(|(i, _)| covered.has(*i))
                .map(|(_, c)| c)
                .sum();
            sum >= wanted
        }
    }
}

fn fills_for(need: &Need, available: CourseBits, limit: usize) -> Vec<Vec<usize>> {
    let usable: Vec<usize> = need
        .units
        .iter()
        .enumerate()
        .filter(|(_, u)| available.contains(u.bits))
        .map(|(i, _)| i)
        .collect();
    match &need.amount {
        NeedAmount::Alternative => usable.into_iter().take(limit).map(|i| vec![i]).collect(),
        NeedAmount::Count(wanted) => {
            let wanted = *wanted as usize;
            let mut result = vec![];
            let mut chosen = vec![];
            fn pick(
                start: usize,
                taken: CourseBits,
                options: &HashSet<i32>,
                usable: &[usize],
                need: &Need,
                wanted: usize,
                limit: usize,
                result: &mut Vec<Vec<usize>>,
                chosen: &mut Vec<usize>,
            ) {
                if chosen.len() == wanted {
                    result.push(chosen.clone());
                    return;
                }
                if usable.len().saturating_sub(start) < wanted.saturating_sub(chosen.len()) {
                    return;
                }
                for position in start..usable.len() {
                    if result.len() >= limit {
                        return;
                    }
                    let unit = &need.units[usable[position]];
                    if taken.intersects(unit.bits) || options.contains(&unit.option) {
                        continue;
                    }
                    chosen.push(usable[position]);
                    let mut next_options = options.clone();
                    next_options.insert(unit.option);
                    pick(
                        position + 1,
                        taken.union(unit.bits),
                        &next_options,
                        usable,
                        need,
                        wanted,
                        limit,
                        result,
                        chosen,
                    );
                    chosen.pop();
                }
            }
            pick(
                0,
                CourseBits::default(),
                &HashSet::new(),
                &usable,
                need,
                wanted,
                limit,
                &mut result,
                &mut chosen,
            );
            result
        }
        NeedAmount::Hours(wanted) => {
            let wanted = *wanted;
            let mut later = vec![0.0; usable.len() + 1];
            for position in (0..usable.len()).rev() {
                later[position] = later[position + 1] + need.units[usable[position]].credits;
            }
            let mut result = vec![];
            let mut chosen = vec![];
            fn pick(
                start: usize,
                taken: CourseBits,
                options: &HashSet<i32>,
                credits: f64,
                usable: &[usize],
                need: &Need,
                wanted: f64,
                later: &[f64],
                limit: usize,
                result: &mut Vec<Vec<usize>>,
                chosen: &mut Vec<usize>,
            ) {
                if credits >= wanted {
                    if chosen
                        .iter()
                        .all(|i| credits - need.units[*i].credits < wanted)
                    {
                        result.push(chosen.clone());
                    }
                    return;
                }
                if start >= later.len() || credits + later[start] < wanted {
                    return;
                }
                for position in start..usable.len() {
                    if result.len() >= limit {
                        return;
                    }
                    let unit = &need.units[usable[position]];
                    if taken.intersects(unit.bits) || options.contains(&unit.option) {
                        continue;
                    }
                    chosen.push(usable[position]);
                    let mut next_options = options.clone();
                    next_options.insert(unit.option);
                    pick(
                        position + 1,
                        taken.union(unit.bits),
                        &next_options,
                        credits + unit.credits,
                        usable,
                        need,
                        wanted,
                        later,
                        limit,
                        result,
                        chosen,
                    );
                    chosen.pop();
                }
            }
            pick(
                0,
                CourseBits::default(),
                &HashSet::new(),
                0.0,
                &usable,
                need,
                wanted,
                &later,
                limit,
                &mut result,
                &mut chosen,
            );
            result
        }
    }
}

fn visit(
    depth: usize,
    available: CourseBits,
    met: i32,
    order: &[usize],
    needs: &[Need],
    credits: &[f64],
    electives: &[f64],
    scale: i32,
    state: &mut SearchState,
    electives_met: &dyn Fn(CourseBits) -> i32,
) {
    state.nodes += 1;
    if state.nodes > 20_000 {
        return;
    }
    let key = SearchKey {
        depth,
        low: available.low,
        high: available.high,
    };
    if state
        .visited
        .get(&key)
        .is_some_and(|earlier| *earlier >= met)
    {
        return;
    }
    state.visited.insert(key, met);
    let mut optimistic = met;
    for later in order.iter().skip(depth) {
        if can_still_meet(&needs[*later], available, credits) {
            optimistic += 1;
        }
    }
    let electives_at_best = electives_met(available);
    if optimistic * scale + electives_at_best <= state.best_score {
        return;
    }
    if depth >= order.len() {
        state.best_score = optimistic * scale + electives_at_best;
        state.best_specific = optimistic;
        state.best_electives = electives_at_best;
        state.best = Some(state.chosen.clone());
        let _ = electives;
        return;
    }
    let n = order[depth];
    for fill in fills_for(&needs[n], available, 400) {
        let taken = fill.iter().fold(CourseBits::default(), |bits, i| {
            bits.union(needs[n].units[*i].bits)
        });
        state.chosen.insert(n, fill);
        visit(
            depth + 1,
            available.subtracting(taken),
            met + 1,
            order,
            needs,
            credits,
            electives,
            scale,
            state,
            electives_met,
        );
        state.chosen.remove(&n);
        if state.nodes > 20_000 {
            return;
        }
    }
    visit(
        depth + 1,
        available,
        met,
        order,
        needs,
        credits,
        electives,
        scale,
        state,
        electives_met,
    );
}

/// A course declared equivalent, used when building groups from a plan plus catalog pairs.
pub fn declared_pairs(items: &[CourseEquivalence]) -> Vec<(CourseKey, CourseKey)> {
    items
        .iter()
        .map(|e| (e.course.clone(), e.counts_as.clone()))
        .collect()
}

/// Which requirements a page shows. Remaining is not yet complete; Done is complete or waived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequirementsFilter {
    All,
    Remaining,
    Done,
}

impl RequirementsFilter {
    pub fn includes_status(self, status: AuditStatus) -> bool {
        match self {
            Self::All => true,
            Self::Remaining => status <= AuditStatus::Planned,
            Self::Done => status >= AuditStatus::Complete,
        }
    }

    /// A check that does not count yet shows in All only.
    pub fn includes(self, check: &NoteCheck) -> bool {
        if check.counts {
            self.includes_status(check.status)
        } else {
            self == Self::All
        }
    }
}

/// One item of a requirement's list, in catalog order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListEntry {
    pub id: i32,
    pub alternatives: Vec<Vec<CourseKey>>,
    pub category: Option<String>,
}

impl ListEntry {
    pub fn courses(&self) -> Vec<CourseKey> {
        self.alternatives.iter().flatten().cloned().collect()
    }
}

/// A catalog sentence and the check the audit made of it.
#[derive(Clone, Debug)]
pub struct NoteEntry {
    pub section_id: String,
    pub text: String,
    pub check: Option<NoteCheck>,
}

/// How a course stands in a requirement row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CourseStanding {
    Planned(String),
    NotInPlan,
    Completed(Option<String>),
    InProgress(String),
    NotNeeded,
}

#[derive(Clone, Debug)]
pub struct CourseRowInfo {
    pub status: CourseStanding,
    pub is_counted_here: bool,
    pub by_substitution: bool,
    pub caption: Option<String>,
}

pub fn has_header_row(result: &RuleResult) -> bool {
    match result.effective_kind.as_ref() {
        None => true,
        Some(RuleKind::Required(_)) => is_replaced(result),
        Some(_) => true,
    }
}

pub fn is_replaced(result: &RuleResult) -> bool {
    result
        .substitutions
        .iter()
        .any(|item| matches!(item.change, SubstitutionChange::Replace(_)))
}

pub fn is_shown(use_: &CourseUse, listed: &HashSet<CourseKey>) -> bool {
    listed.contains(&use_.key)
        || use_
            .satisfies
            .as_ref()
            .is_some_and(|key| listed.contains(key))
}

pub fn alternative_entries(option: &CourseOption) -> Vec<ListEntry> {
    option
        .alternatives
        .iter()
        .enumerate()
        .map(|(index, alternative)| ListEntry {
            id: index as i32,
            alternatives: vec![alternative.clone()],
            category: None,
        })
        .collect()
}

pub fn option_entries(
    options: &[CourseOption],
    rule: &RequirementRule,
    subject_name: impl Fn(&str) -> Option<String>,
) -> Vec<ListEntry> {
    let starts = category_starts(options, rule, subject_name);
    options
        .iter()
        .enumerate()
        .map(|(index, option)| ListEntry {
            id: index as i32,
            alternatives: option.alternatives.clone(),
            category: starts.get(&index).cloned(),
        })
        .collect()
}

pub fn listing(rule: &RequirementRule) -> (Vec<CourseOption>, Vec<CourseRange>) {
    match &rule.kind {
        RuleKind::Required(option) => (vec![option.clone()], vec![]),
        RuleKind::Choose {
            options, ranges, ..
        } => (options.clone(), ranges.clone()),
        RuleKind::GeneralElectives { .. } => (vec![], vec![]),
    }
}

/// The relevant entries and the first `extra` others, in catalog order.
pub fn collapsed_entries(
    entries: &[ListEntry],
    extra: i32,
    is_relevant: impl Fn(&ListEntry) -> bool,
) -> Vec<ListEntry> {
    let mut others = 0;
    entries
        .iter()
        .filter(|entry| {
            if is_relevant(entry) {
                return true;
            }
            if others >= extra {
                return false;
            }
            others += 1;
            true
        })
        .cloned()
        .collect()
}

pub fn note_entries(
    section: &RequirementSection,
    audit: &DegreeAudit,
    filter: RequirementsFilter,
    states_total_hours: bool,
) -> Vec<NoteEntry> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (note, shows_unchecked) in section.notes.iter().map(|note| (note, true)).chain(
        section
            .groups
            .iter()
            .filter(|group| group.rules.is_empty())
            .flat_map(|group| group.notes.iter().map(|note| (note, false))),
    ) {
        let id = format!("{}|{note}", section.id);
        if !seen.insert(id) {
            continue;
        }
        if let Some(check) = audit.note_check(&section.id, note) {
            if filter.includes(check) {
                out.push(NoteEntry {
                    section_id: section.id.clone(),
                    text: note.clone(),
                    check: Some(check.clone()),
                });
            }
            continue;
        }
        let restated =
            states_total_hours && note.to_ascii_lowercase().contains("total semester hours");
        if filter == RequirementsFilter::All && shows_unchecked && !restated {
            out.push(NoteEntry {
                section_id: section.id.clone(),
                text: note.clone(),
                check: None,
            });
        }
    }
    out
}

pub fn split_unlisted(
    result: &RuleResult,
    listed: &HashSet<CourseKey>,
) -> (Vec<CourseUse>, Vec<CourseUse>) {
    let unlisted: Vec<CourseUse> = result
        .used
        .iter()
        .filter(|use_| !is_shown(use_, listed))
        .cloned()
        .collect();
    let substitutes = unlisted
        .iter()
        .filter(|use_| use_.by_substitution)
        .cloned()
        .collect();
    let others = unlisted
        .into_iter()
        .filter(|use_| !use_.by_substitution)
        .collect();
    (substitutes, others)
}

pub fn requirement_name(result: &RuleResult, requirements: &ProgramRequirements) -> String {
    requirements
        .degree_short_name(&result.rule.id)
        .unwrap_or_else(|| result.rule.display_label())
}

pub fn counting_note(key: &CourseKey, result: &RuleResult) -> Option<String> {
    result
        .substitutions
        .iter()
        .find(
            |item| matches!(&item.change, SubstitutionChange::CountCourse(course) if course == key),
        )
        .and_then(substitution_caption)
}

pub fn counting_caption(
    key: &CourseKey,
    result: &RuleResult,
    requirements: &ProgramRequirements,
) -> String {
    let mut parts = vec![format!(
        "Counts for {} by substitution",
        requirement_name(result, requirements)
    )];
    if let Some(note) = counting_note(key, result) {
        parts.push(note);
    }
    parts.join(" · ")
}

pub fn row_caption(
    info: &CourseRowInfo,
    key: &CourseKey,
    result: &RuleResult,
    requirements: &ProgramRequirements,
    is_child: bool,
) -> Option<String> {
    if !info.by_substitution {
        return info.caption.clone();
    }
    if is_child {
        counting_note(key, result)
    } else {
        Some(counting_caption(key, result, requirements))
    }
}

fn substitution_caption(item: &crate::model::RequirementSubstitution) -> Option<String> {
    let note = item.note.trim();
    if note.is_empty() {
        None
    } else {
        Some(note.to_string())
    }
}

fn category_starts(
    options: &[CourseOption],
    rule: &RequirementRule,
    subject_name: impl Fn(&str) -> Option<String>,
) -> HashMap<usize, String> {
    if let RuleKind::Choose { options: own, .. } = &rule.kind {
        if own == options {
            if let Some(categories) = &rule.option_categories {
                if categories.len() == options.len() {
                    let mut starts = HashMap::new();
                    for (index, category) in categories.iter().enumerate() {
                        if let Some(category) = category {
                            if index == 0
                                || categories[index - 1].as_deref() != Some(category.as_str())
                            {
                                starts.insert(index, category.clone());
                            }
                        }
                    }
                    return starts;
                }
            }
        }
    }
    let categories: Vec<&str> = rule
        .notes
        .iter()
        .filter(|note| degree_is_category(note))
        .map(|note| note.as_str())
        .collect();
    let Some(first) = categories.first() else {
        return HashMap::new();
    };
    if options.is_empty() {
        return HashMap::new();
    }
    let subjects: Vec<String> = options
        .iter()
        .map(|option| {
            option
                .all_courses()
                .first()
                .map(|key| key.subject.clone())
                .unwrap_or_default()
        })
        .collect();
    let runs: Vec<usize> = subjects
        .iter()
        .enumerate()
        .filter(|(index, subject)| *index == 0 || subjects[*index - 1] != **subject)
        .map(|(index, _)| index)
        .collect();
    if runs.len() == categories.len() {
        return runs
            .into_iter()
            .zip(categories)
            .map(|(index, name)| (index, name.to_string()))
            .collect();
    }
    let mut starts = HashMap::new();
    starts.insert(0, (*first).to_string());
    let mut current = 0usize;
    for start in runs.into_iter().skip(1) {
        if current + 1 >= categories.len() {
            break;
        }
        let name = subject_name(&subjects[start]).unwrap_or_else(|| subjects[start].clone());
        if shares_word(&name, categories[current + 1]) {
            current += 1;
            starts.insert(start, categories[current].to_string());
        }
    }
    if current == categories.len() - 1 {
        starts
    } else {
        HashMap::new()
    }
}

fn degree_is_category(note: &str) -> bool {
    note.chars().count() <= 48 && !note.contains('.') && !note.contains(':')
}

fn shares_word(left: &str, right: &str) -> bool {
    fn words(text: &str) -> Vec<String> {
        text.split(|c: char| !c.is_alphabetic())
            .map(|word| word.to_ascii_lowercase())
            .filter(|word| word.chars().count() >= 4)
            .collect()
    }
    let right = words(right);
    words(left).iter().any(|word| {
        right
            .iter()
            .any(|other| word.starts_with(other.as_str()) || other.starts_with(word.as_str()))
    })
}

/// Suppress unused import warning if a limit variant is only constructed via serde.
#[allow(dead_code)]
fn _limit(limit: CourseNumberLimit) -> CourseNumberLimit {
    limit
}
