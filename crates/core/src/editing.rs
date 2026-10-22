//! Edits a degree plan makes when a course is dropped onto a semester.
//! Port of `DegreePlanEditing.swift`.

use crate::model::*;
use uuid::Uuid;

pub fn add(course: DegreePlanCourse, term_id: Uuid, plan: &mut DegreePlan) {
    if let Some(term) = plan.terms.iter_mut().find(|term| term.id == term_id) {
        term.courses.push(course);
    }
}

pub fn move_course(course_id: Uuid, term_id: Uuid, plan: &mut DegreePlan) {
    let Some(moved) = take(course_id, plan) else {
        return;
    };
    if let Some(term) = plan.terms.iter_mut().find(|term| term.id == term_id) {
        term.courses.push(moved);
    }
}

pub fn move_course_at(course_id: Uuid, term_id: Uuid, position: i32, plan: &mut DegreePlan) {
    let Some(destination) = plan.terms.iter().position(|term| term.id == term_id) else {
        return;
    };
    let mut index = position;
    if let Some(current) = plan.terms[destination]
        .courses
        .iter()
        .position(|course| course.id == course_id)
    {
        if (current as i32) < position {
            index -= 1;
        }
    }
    let Some(moved) = take(course_id, plan) else {
        return;
    };
    let destination = plan
        .terms
        .iter()
        .position(|term| term.id == term_id)
        .unwrap();
    let at = index.clamp(0, plan.terms[destination].courses.len() as i32) as usize;
    plan.terms[destination].courses.insert(at, moved);
}

fn take(course_id: Uuid, plan: &mut DegreePlan) -> Option<DegreePlanCourse> {
    for term in &mut plan.terms {
        if let Some(position) = term
            .courses
            .iter()
            .position(|course| course.id == course_id)
        {
            return Some(term.courses.remove(position));
        }
    }
    None
}

/// A substitution turned into the equivalence it amounts to, with what Undo puts back.
#[derive(Clone, Debug)]
pub struct Renumbering {
    pub course: CourseKey,
    pub counts_as: CourseKey,
    pub substitution: Option<RequirementSubstitution>,
    pub substitution_index: usize,
    pub equivalence_id: Option<Uuid>,
}

impl Renumbering {
    pub fn message(&self) -> String {
        format!("{} now counts as {}", self.course, self.counts_as)
    }
    pub fn can_undo(&self) -> bool {
        self.substitution.is_some() || self.equivalence_id.is_some()
    }
    pub fn undo(&self, plan_id: Uuid, data: &mut UserData) {
        let Some(plan) = data.degree_plans.iter_mut().find(|plan| plan.id == plan_id) else {
            return;
        };
        if let Some(id) = self.equivalence_id {
            plan.course_equivalences.retain(|item| item.id != id);
        }
        if let Some(substitution) = &self.substitution {
            if !plan
                .substitutions
                .iter()
                .any(|item| item.id == substitution.id)
            {
                let at = self.substitution_index.min(plan.substitutions.len());
                plan.substitutions.insert(at, substitution.clone());
            }
        }
    }
}

/// A substitution that is most likely a course under a new number.
#[derive(Clone, Debug)]
pub struct LikelyRenumbering {
    pub substitution: RequirementSubstitution,
    pub course: CourseKey,
    pub counts_as: CourseKey,
}

impl LikelyRenumbering {
    pub fn question(&self) -> String {
        format!("Is {} the new number of {}?", self.course, self.counts_as)
    }
}

/// A course in the plan, for pickers.
#[derive(Clone, Debug)]
pub struct PlanCourseChoice {
    pub key: CourseKey,
    pub title: String,
    pub credits: f64,
    pub semester: Option<String>,
    pub semester_short_name: Option<String>,
}

/// The required course and the course that stands in, when the requirement names exactly one other course.
pub fn renumbering_for(
    substitution: &RequirementSubstitution,
    requirements: &ProgramRequirements,
) -> Option<(CourseKey, CourseKey)> {
    let SubstitutionChange::CountCourse(counted) = &substitution.change else {
        return None;
    };
    let course = &counted.course;
    let rule =
        requirements.degree_rule(Some(&substitution.rule_id), Some(&substitution.rule_title))?;
    let RuleKind::Required(option) = &rule.kind else {
        return None;
    };
    if option.alternatives.len() != 1 {
        return None;
    }
    let named = option.alternatives.first()?;
    if named.len() != 1 || &named[0] == course {
        return None;
    }
    Some((course.clone(), named[0].clone()))
}

pub fn renumberable(
    plan: &DegreePlan,
    requirements: &ProgramRequirements,
) -> std::collections::HashSet<Uuid> {
    plan.substitutions
        .iter()
        .filter(|item| renumbering_for(item, requirements).is_some())
        .map(|item| item.id)
        .collect()
}

pub fn courses_to_look_up(plan: &DegreePlan, requirements: &ProgramRequirements) -> Vec<CourseKey> {
    let mut seen = std::collections::HashSet::new();
    plan.substitutions
        .iter()
        .filter_map(|item| renumbering_for(item, requirements))
        .flat_map(|(course, counts_as)| [counts_as, course])
        .filter(|key| seen.insert(key.clone()))
        .collect()
}

pub fn convert_renumbering(
    substitution: &RequirementSubstitution,
    plan_id: Uuid,
    requirements: &ProgramRequirements,
    data: &mut UserData,
) -> Option<Renumbering> {
    let plan = data.degree_plans.iter().find(|plan| plan.id == plan_id)?;
    let index = plan
        .substitutions
        .iter()
        .position(|item| item.id == substitution.id)?;
    let (course, counts_as) = renumbering_for(&plan.substitutions[index], requirements)?;
    let stored = plan.substitutions[index].clone();
    let known: std::collections::HashSet<Uuid> = plan
        .course_equivalences
        .iter()
        .map(|item| item.id)
        .collect();
    let plan = data
        .degree_plans
        .iter_mut()
        .find(|plan| plan.id == plan_id)?;
    plan.substitutions.retain(|item| item.id != stored.id);
    let _added = declare_equivalence(plan, &course, &counts_as, "Renumbered");
    let declared = plan
        .course_equivalences
        .iter()
        .find(|item| !known.contains(&item.id))
        .map(|item| item.id);
    Some(Renumbering {
        course,
        counts_as,
        substitution: Some(stored),
        substitution_index: index,
        equivalence_id: declared,
    })
}

pub fn declare_renumbering(
    course: CourseKey,
    counts_as: CourseKey,
    plan_id: Uuid,
    data: &mut UserData,
) -> Option<Renumbering> {
    if course == counts_as || !data.degree_plans.iter().any(|plan| plan.id == plan_id) {
        return None;
    }
    let plan = data
        .degree_plans
        .iter_mut()
        .find(|plan| plan.id == plan_id)?;
    let added = declare_equivalence(plan, &course, &counts_as, "Renumbered");
    let equivalence_id = if added {
        plan.course_equivalences.last().map(|item| item.id)
    } else {
        None
    };
    Some(Renumbering {
        course,
        counts_as,
        substitution: None,
        substitution_index: 0,
        equivalence_id,
    })
}

/// Declares that taking `course` counts as `counts_as`. False when they are the same or already equivalent.
pub fn declare_equivalence(
    plan: &mut DegreePlan,
    course: &CourseKey,
    counts_as: &CourseKey,
    note: &str,
) -> bool {
    if course == counts_as {
        return false;
    }
    if crate::CourseEquivalences::from_declarations(&plan.course_equivalences)
        .are_equivalent(course, counts_as)
    {
        return false;
    }
    plan.course_equivalences.push(CourseEquivalence {
        id: Uuid::new_v4(),
        course: course.clone(),
        counts_as: counts_as.clone(),
        note: note.to_string(),
    });
    true
}

pub fn likely_renumberings(
    plan: &DegreePlan,
    requirements: &ProgramRequirements,
    is_retired: impl Fn(&CourseKey) -> Option<bool>,
) -> Vec<LikelyRenumbering> {
    let equivalences = crate::CourseEquivalences::from_declarations(&plan.course_equivalences);
    plan.substitutions
        .iter()
        .filter_map(|substitution| {
            let (course, counts_as) = renumbering_for(substitution, requirements)?;
            if is_retired(&counts_as) != Some(true) || is_retired(&course) == Some(true) {
                return None;
            }
            if equivalences.are_equivalent(&course, &counts_as) {
                return None;
            }
            Some(LikelyRenumbering {
                substitution: substitution.clone(),
                course,
                counts_as,
            })
        })
        .collect()
}

pub fn likely_renumbering_map(
    plan: &DegreePlan,
    requirements: &ProgramRequirements,
    dismissed: &std::collections::HashSet<String>,
    is_retired: impl Fn(&CourseKey) -> Option<bool>,
) -> std::collections::HashMap<Uuid, LikelyRenumbering> {
    likely_renumberings(plan, requirements, is_retired)
        .into_iter()
        .filter(|item| !dismissed.contains(&pair_id(&item.course, &item.counts_as)))
        .map(|item| (item.substitution.id, item))
        .collect()
}

pub fn pair_id(a: &CourseKey, b: &CourseKey) -> String {
    if a < b {
        format!("{a}|{b}")
    } else {
        format!("{b}|{a}")
    }
}

pub fn matching_choices(choices: &[PlanCourseChoice], query: &str) -> Vec<PlanCourseChoice> {
    let query = query.trim();
    if query.is_empty() {
        return choices.to_vec();
    }
    let compact = query.replace(' ', "");
    let query_lower = query.to_ascii_lowercase();
    let compact_lower = compact.to_ascii_lowercase();
    choices
        .iter()
        .filter(|choice| {
            choice
                .key
                .to_string()
                .to_ascii_lowercase()
                .contains(&query_lower)
                || format!("{}{}", choice.key.subject, choice.key.number)
                    .to_ascii_lowercase()
                    .contains(&compact_lower)
                || choice.title.to_ascii_lowercase().contains(&query_lower)
        })
        .cloned()
        .collect()
}
