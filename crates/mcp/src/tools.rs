// SPDX-License-Identifier: AGPL-3.0-only
//! Catalog and degree-plan tools. Write tools edit the same `user.json` the app loads.

use crate::catalogs::catalogs;
use crate::present::{course_json, section_json};
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use termorganize_core::index::CatalogIndex;
use termorganize_core::model::{
    Course, CourseKey, DaySet, DegreePlan, DegreePlanCourse, DegreePlanTerm, PreferenceStrength,
    ScheduleFilters, SchedulePreferences, TimeLimit,
};
use termorganize_core::planfile::{self, PlanFileError};
use termorganize_core::planlogic;
use termorganize_core::reorganize::{self, PlanSlot, SlotCourse};
use termorganize_core::schedule::{self, Limits, ScheduleCourseInput, ScheduleSort};
use termorganize_core::search::{self, SearchFilters, SearchQuery, SearchSort};
use termorganize_core::store::plan_revision;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ToolError {
    pub error: &'static str,
    pub message: String,
    pub revision: Option<u64>,
}

impl ToolError {
    pub fn to_value(&self) -> Value {
        json!({
            "error": {
                "code": self.error,
                "message": nonempty(&self.message),
            }
        })
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            error: "invalid_argument",
            message: nonempty(&message.into()),
            revision: None,
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            error: "not_found",
            message: nonempty(&message.into()),
            revision: None,
        }
    }

    fn conflict(actual: u64) -> Self {
        Self {
            error: "revision_conflict",
            message: format!("The plan changed (revision {actual}). Reload it and try again."),
            revision: Some(actual),
        }
    }

    pub fn read_only(message: impl Into<String>) -> Self {
        Self {
            error: "read_only",
            message: nonempty(&message.into()),
            revision: None,
        }
    }

    pub fn unknown_tool(message: impl Into<String>) -> Self {
        Self {
            error: "unknown_tool",
            message: nonempty(&message.into()),
            revision: None,
        }
    }

    fn from_plan(error: PlanFileError) -> Self {
        match error {
            PlanFileError::NotFound => Self::not_found("No degree plan has that id."),
            PlanFileError::RevisionConflict { actual } => Self::conflict(actual),
            PlanFileError::Rejected(message) => Self::invalid(message),
            PlanFileError::Io(message) => Self {
                error: "storage_error",
                message: nonempty(&message),
                revision: None,
            },
        }
    }
}

fn nonempty(message: &str) -> String {
    let message = message.trim();
    if message.is_empty() {
        "The request could not be completed.".to_string()
    } else {
        message.to_string()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    pub term: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub number: Option<String>,
    #[serde(default)]
    pub attribute: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Keep courses whose loaded details list no prerequisites.
    /// A course whose details were not loaded is unknown and is left out.
    #[serde(default)]
    pub no_prerequisites: bool,
    /// Keep courses whose loaded details list no corequisites.
    /// Unknown details are left out.
    #[serde(default)]
    pub no_corequisites: bool,
    /// `all`, `undergrad` (number below 5000), or `grad` (5000 and up).
    #[serde(default)]
    pub level: SearchLevel,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SearchLevel {
    #[default]
    All,
    Undergrad,
    Grad,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CourseArgs {
    pub term: String,
    pub subject: String,
    pub number: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckArgs {
    pub term: String,
    pub crns: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CourseRef {
    pub subject: String,
    pub number: String,
    #[serde(default)]
    pub crn: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Constraints {
    #[serde(default)]
    pub no_classes_before: Option<String>,
    #[serde(default)]
    pub no_classes_after: Option<String>,
    #[serde(default)]
    pub days_off: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestArgs {
    pub term: String,
    pub courses: Vec<CourseRef>,
    #[serde(default)]
    pub constraints: Option<Constraints>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TermInput {
    pub term: String,
    #[serde(default)]
    pub courses: Vec<CourseRef>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanInput {
    pub terms: Vec<TermInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReorganizeArgs {
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub plan: Option<PlanInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreatePlanArgs {
    pub name: String,
    #[serde(default)]
    pub terms: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanIdArgs {
    pub plan_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameArgs {
    pub plan_id: String,
    pub name: String,
    pub expected_revision: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddCourseArgs {
    pub plan_id: String,
    pub term: String,
    pub subject: String,
    pub number: String,
    #[serde(default)]
    pub crn: Option<String>,
    /// Chosen credits for a variable-credit course. Omitted uses the catalog default.
    #[serde(default)]
    pub credits: Option<f64>,
    pub expected_revision: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetCreditsArgs {
    pub plan_id: String,
    pub term: String,
    pub subject: String,
    pub number: String,
    #[serde(default)]
    pub crn: Option<String>,
    pub credits: f64,
    pub expected_revision: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveCourseArgs {
    pub plan_id: String,
    pub term: String,
    pub subject: String,
    pub number: String,
    pub expected_revision: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveCourseArgs {
    pub plan_id: String,
    pub subject: String,
    pub number: String,
    pub from_term: String,
    pub to_term: String,
    pub expected_revision: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetSectionArgs {
    pub plan_id: String,
    pub term: String,
    pub subject: String,
    pub number: String,
    pub crn: String,
    pub expected_revision: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RevisionArgs {
    pub plan_id: String,
    pub expected_revision: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteArgs {
    pub plan_id: String,
    pub expected_revision: u64,
    #[serde(default)]
    pub confirm: bool,
}

pub fn list_terms() -> Value {
    let terms: Vec<Value> = catalogs()
        .iter()
        .map(|index| {
            json!({
                "term": index.catalog.term.code,
                "name": index.catalog.term.name(),
            })
        })
        .collect();
    json!({ "terms": terms })
}

pub fn search_courses(args: SearchArgs) -> Result<Value, ToolError> {
    let (index, parsed) = index_for(&args.term)?;
    let mut filters = SearchFilters::default();
    if let Some(subject) = args.subject.as_deref() {
        filters.subjects.insert(subject.trim().to_ascii_uppercase());
    }
    filters.no_prerequisites = args.no_prerequisites;
    filters.no_corequisites = args.no_corequisites;
    filters.course_level = match args.level {
        SearchLevel::All => termorganize_core::search::CourseLevel::All,
        SearchLevel::Undergrad => termorganize_core::search::CourseLevel::Undergrad,
        SearchLevel::Grad => termorganize_core::search::CourseLevel::Grad,
    };
    let limit = args.limit.unwrap_or(25).clamp(1, 100) as usize;
    let text = args.query.unwrap_or_default();
    let response = search::search(
        &SearchQuery {
            text,
            filters,
            sort: SearchSort::Relevance,
        },
        index,
    );
    let number = args.number.as_deref().map(str::trim);
    let attribute = args
        .attribute
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase());
    let mut courses = Vec::new();
    for result in response.results {
        if let Some(number) = number {
            if !result.course.number.eq_ignore_ascii_case(number) {
                continue;
            }
        }
        if let Some(attribute) = attribute.as_deref() {
            if !matches_attribute(&result.course, attribute) {
                continue;
            }
        }
        let sections = index.sections_for(&result.course.key());
        courses.push(course_json(&result.course, &sections));
        if courses.len() == limit {
            break;
        }
    }
    Ok(json!({
        "term": { "term": index.catalog.term.code, "name": parsed.label },
        "courses": courses,
    }))
}

pub fn get_course(args: CourseArgs) -> Result<Value, ToolError> {
    let (index, parsed) = index_for(&args.term)?;
    let course = find_course(index, &args.subject, &args.number).ok_or_else(|| {
        ToolError::not_found(format!(
            "No course {} {} in {}.",
            args.subject.trim().to_ascii_uppercase(),
            args.number.trim(),
            parsed.label
        ))
    })?;
    let listed = index.sections_for(&course.key());
    let sections: Vec<Value> = listed.iter().map(section_json).collect();
    let mut value = course_json(course, &listed);
    value["term"] = json!({ "term": index.catalog.term.code, "name": parsed.label });
    value["sections"] = json!(sections);
    Ok(value)
}

pub fn check_schedule(args: CheckArgs) -> Result<Value, ToolError> {
    let (index, parsed) = index_for(&args.term)?;
    let mut unknown = Vec::new();
    let mut sections = Vec::new();
    for crn in &args.crns {
        match index.section_crn(crn.trim()) {
            Some(section) => sections.push(section),
            None => unknown.push(crn.trim().to_string()),
        }
    }
    let mut clashes = Vec::new();
    for left in 0..sections.len() {
        for right in (left + 1)..sections.len() {
            if sections[left].conflicts_with(sections[right]) {
                clashes.push(json!({
                    "first": sections[left].crn,
                    "second": sections[right].crn,
                    "reason": "The meetings overlap.",
                }));
            }
        }
    }
    Ok(json!({
        "term": { "term": index.catalog.term.code, "name": parsed.label },
        "clashes": clashes,
        "unknown_crns": unknown,
    }))
}

pub fn suggest_schedule(args: SuggestArgs) -> Result<Value, ToolError> {
    let (index, parsed) = index_for(&args.term)?;
    if args.courses.is_empty() {
        return Err(ToolError::invalid(
            "suggest_schedule needs at least one course.",
        ));
    }
    let mut chosen: Vec<(CourseKey, Option<String>)> = Vec::new();
    for course in &args.courses {
        let found = find_course(index, &course.subject, &course.number).ok_or_else(|| {
            ToolError::not_found(format!(
                "No course {} {} in {}.",
                course.subject, course.number, parsed.label
            ))
        })?;
        let crn = course
            .crn
            .as_deref()
            .map(str::trim)
            .filter(|crn| !crn.is_empty())
            .map(str::to_string);
        let existing: Vec<Option<&str>> = chosen
            .iter()
            .filter(|(key, _)| key == &found.key())
            .map(|(_, crn)| crn.as_deref())
            .collect();
        if !planlogic::repeated_course_allowed(existing.iter().copied(), crn.as_deref()) {
            if crn.as_ref().is_some_and(|value| {
                existing
                    .iter()
                    .any(|have| planlogic::same_chosen_crn(*have, Some(value)))
            }) {
                return Err(ToolError::invalid(format!(
                    "CRN {} is already in this request.",
                    crn.unwrap_or_default()
                )));
            }
            return Err(ToolError::invalid(format!(
                "{} {} is already in this request. Another copy needs its own section CRN.",
                found.subject, found.number
            )));
        }
        if let Some(crn) = crn.as_deref() {
            if !crn_belongs(index, &found.key(), Some(crn)) {
                return Err(ToolError::not_found(format!(
                    "CRN {crn} is not a section of {} {} in {}.",
                    found.subject, found.number, parsed.label
                )));
            }
        }
        chosen.push((found.key(), crn));
    }
    let claimed: std::collections::HashSet<&str> = chosen
        .iter()
        .filter_map(|(_, crn)| crn.as_deref())
        .collect();
    let mut inputs = Vec::new();
    for (key, crn) in &chosen {
        let mut sections = index.sections_for(key);
        if let Some(crn) = crn {
            sections.retain(|section| section.crn == *crn);
        } else {
            sections.retain(|section| !claimed.contains(section.crn.as_str()));
        }
        if sections.is_empty() {
            return Err(ToolError::not_found(format!(
                "{} {} has no sections in {}.",
                key.subject, key.number, parsed.label
            )));
        }
        inputs.push(ScheduleCourseInput::new(key.clone(), true, sections));
    }
    let (filters, preferences) = constraints(args.constraints.as_ref())?;
    let generated = schedule::generate(
        &inputs,
        &filters,
        &preferences,
        ScheduleSort::Best,
        &Limits {
            max_results: 8,
            max_exhaustive_leaves: 80_000,
            max_nodes: 400_000,
            time_budget: 0.5,
            sample_leaves: 20_000,
            sample_time_budget: 0.2,
            allow_sampling: true,
        },
    );
    let schedules: Vec<Value> = generated
        .schedules
        .iter()
        .map(|schedule| {
            json!({
                "crns": schedule.crns,
                "sections": schedule.sections.iter().map(|section| {
                    let mut value = section_json(section);
                    value["subject"] = json!(section.subject);
                    value["number"] = json!(section.number);
                    value
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({
        "term": { "term": index.catalog.term.code, "name": parsed.label },
        "schedules": schedules,
    }))
}

pub fn reorganize_plan(dir: &Path, args: ReorganizeArgs) -> Result<Value, ToolError> {
    if args.plan_id.is_some() && args.plan.is_some() {
        return Err(ToolError::invalid("Pass plan_id or plan, not both."));
    }
    let slots = if let Some(plan_id) = args.plan_id.as_deref() {
        let id = parse_id(plan_id)?;
        let data = planfile::read_user(dir).map_err(ToolError::from_plan)?;
        let plan = data
            .degree_plans
            .iter()
            .find(|plan| plan.id == id)
            .ok_or_else(|| ToolError::not_found("No degree plan has that id."))?;
        slots_from_degree(plan)
    } else if let Some(plan) = args.plan {
        slots_from_input(plan)
    } else {
        return Err(ToolError::invalid("reorganize_plan needs plan_id or plan."));
    };
    outcome_json(&reorganize::reorganize(&slots, catalogs()))
}

fn slots_from_input(plan: PlanInput) -> Vec<PlanSlot> {
    plan.terms
        .into_iter()
        .map(|term| PlanSlot {
            term: term.term,
            courses: term
                .courses
                .into_iter()
                .map(|course| SlotCourse {
                    subject: course.subject,
                    number: course.number,
                    crn: course.crn,
                })
                .collect(),
        })
        .collect()
}

fn slots_from_degree(plan: &DegreePlan) -> Vec<PlanSlot> {
    plan.terms
        .iter()
        .map(|term| PlanSlot {
            term: term_code(term).unwrap_or_else(|| term.name()),
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
        .collect()
}

fn outcome_json(outcome: &reorganize::ReorganizeOutcome) -> Result<Value, ToolError> {
    Ok(json!({
        "moves": outcome.moves.iter().map(move_json).collect::<Vec<_>>(),
        "unresolved": outcome.unresolved.iter().map(|item| json!({
            "subject": item.subject,
            "number": item.number,
            "reason": item.reason,
        })).collect::<Vec<_>>(),
        "plan": {
            "terms": outcome.terms.iter().map(|term| json!({
                "term": term.term,
                "courses": term.courses.iter().map(|course| {
                    let mut row = json!({
                        "subject": course.subject,
                        "number": course.number,
                    });
                    if let Some(crn) = &course.crn {
                        row["crn"] = json!(crn);
                    }
                    row
                }).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        },
    }))
}

pub fn list_plans(dir: &Path) -> Result<Value, ToolError> {
    let data = planfile::read_user(dir).map_err(ToolError::from_plan)?;
    let plans: Vec<Value> = data
        .degree_plans
        .iter()
        .map(|plan| {
            json!({
                "id": plan.id.to_string(),
                "name": plan.name,
                "terms": plan.terms.iter().map(|term| json!({
                    "term": term_code(term).unwrap_or_else(|| term.name()),
                    "name": term.name(),
                })).collect::<Vec<_>>(),
                "updated_at": plan.updated_at,
                "revision": plan_revision(&data, plan.id),
            })
        })
        .collect();
    Ok(json!({ "plans": plans }))
}

pub fn get_plan(dir: &Path, plan_id: &str) -> Result<Value, ToolError> {
    let id = parse_id(plan_id)?;
    plan_value(dir, id)
}

pub fn create_plan(dir: &Path, args: CreatePlanArgs) -> Result<Value, ToolError> {
    let mut terms = Vec::new();
    for name in args.terms.unwrap_or_default() {
        let parsed = parse_label(&name)?;
        terms.push(DegreePlanTerm::new(parsed.season, parsed.year));
    }
    let (plan, _) = planfile::create_plan(dir, &args.name, terms).map_err(ToolError::from_plan)?;
    plan_value(dir, plan.id)
}

pub fn rename_plan(dir: &Path, args: RenameArgs) -> Result<Value, ToolError> {
    let id = parse_id(&args.plan_id)?;
    let name = args.name.trim().to_string();
    if name.is_empty() {
        return Err(ToolError::invalid("A plan needs a name."));
    }
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        plan.name = name.clone();
        Ok(())
    })
    .map_err(ToolError::from_plan)?;
    plan_value(dir, id)
}

pub fn add_course(dir: &Path, args: AddCourseArgs) -> Result<Value, ToolError> {
    let id = parse_id(&args.plan_id)?;
    let (index, parsed) = index_for(&args.term)?;
    let course = require_offered(index, &parsed.label, &args.subject, &args.number)?;
    let crn = checked_crn(index, course, args.crn.as_deref(), &parsed.label)?;
    let title = course.title.clone();
    let credits = match args.credits {
        Some(value) => chosen_credits(course, value)?,
        None => course.default_credits(),
    };
    let key = course.key();
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        if !plan
            .terms
            .iter()
            .any(|term| term.season == parsed.season && term.year == parsed.year)
        {
            return Err(format!("{} is not on this plan.", parsed.label));
        }
        let term = term_on_plan(plan, parsed.season, parsed.year)?;
        let mut row = DegreePlanCourse::catalog(key.clone(), title.clone(), credits);
        row.crn = crn.clone();
        term.courses.push(row);
        Ok(())
    })
    .map_err(ToolError::from_plan)?;
    plan_value(dir, id)
}

pub fn set_credits(dir: &Path, args: SetCreditsArgs) -> Result<Value, ToolError> {
    let id = parse_id(&args.plan_id)?;
    let (index, parsed) = index_for(&args.term)?;
    let course = require_offered(index, &parsed.label, &args.subject, &args.number)?;
    let chosen = chosen_credits(course, args.credits)?;
    let key = course.key();
    let crn = args
        .crn
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let row_id = std::cell::Cell::new(None);
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        let term = term_on_plan(plan, parsed.season, parsed.year)?;
        let matches: Vec<usize> = term
            .courses
            .iter()
            .enumerate()
            .filter(|(_, row)| row.key.as_ref() == Some(&key))
            .map(|(index, _)| index)
            .collect();
        if matches.is_empty() {
            return Err(format!(
                "{} {} is not on {}.",
                key.subject,
                key.number,
                term.name()
            ));
        }
        let index = if matches.len() == 1 && crn.is_none() {
            matches[0]
        } else if matches.len() > 1 && crn.is_none() {
            return Err(format!(
                "{} {} is on {} more than once, each with its own section.",
                key.subject,
                key.number,
                term.name()
            ));
        } else {
            let crn = crn.unwrap_or("");
            let Some(found) = matches
                .iter()
                .copied()
                .find(|index| term.courses[*index].crn.as_deref() == Some(crn))
            else {
                return Err(format!(
                    "{} {} with CRN {crn} is not on {}.",
                    key.subject,
                    key.number,
                    term.name()
                ));
            };
            found
        };
        term.courses[index].credits = chosen;
        row_id.set(Some(term.courses[index].id));
        Ok(())
    })
    .map_err(ToolError::from_plan)?;
    if let Some(row) = row_id.get() {
        if let Ok(mut data) = planfile::read_user(dir) {
            let mut linked = false;
            for schedule in &mut data.plans {
                for course in &mut schedule.courses {
                    if course.degree_row_id == Some(row) {
                        course.credits = Some(chosen);
                        linked = true;
                    }
                }
            }
            if linked {
                let _ = termorganize_core::store::save_user(dir, &data);
            }
        }
    }
    plan_value(dir, id)
}

fn chosen_credits(course: &Course, value: f64) -> Result<f64, ToolError> {
    use termorganize_core::model::credit_number;
    if !course.is_variable_credit() {
        return Err(ToolError::invalid(format!(
            "{} {} has fixed credit.",
            course.subject, course.number
        )));
    }
    let low = credit_number(course.min_credits);
    let high = credit_number(course.max_credits);
    if (value - value.round()).abs() > 1e-6
        || value + 1e-9 < course.min_credits
        || value > course.max_credits + 1e-9
    {
        return Err(ToolError::invalid(format!(
            "Credits must be a whole number from {low} to {high}."
        )));
    }
    Ok(value.round())
}

pub fn remove_course(dir: &Path, args: RemoveCourseArgs) -> Result<Value, ToolError> {
    let id = parse_id(&args.plan_id)?;
    let parsed = parse_label(&args.term)?;
    let subject = args.subject.trim().to_ascii_uppercase();
    let number = args.number.trim().to_string();
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        let term = term_on_plan(plan, parsed.season, parsed.year)?;
        let matches: Vec<usize> = term
            .courses
            .iter()
            .enumerate()
            .filter(|(_, course)| {
                course.key.as_ref().is_some_and(|key| {
                    key.subject.eq_ignore_ascii_case(&subject)
                        && key.number.eq_ignore_ascii_case(&number)
                })
            })
            .map(|(index, _)| index)
            .collect();
        if matches.is_empty() {
            return Err(format!("{subject} {number} is not on {}.", term.name()));
        }
        if matches.len() > 1 {
            return Err(format!(
                "{subject} {number} is on {} more than once, each with its own section.",
                term.name()
            ));
        }
        term.courses.remove(matches[0]);
        Ok(())
    })
    .map_err(ToolError::from_plan)?;
    plan_value(dir, id)
}

pub fn move_course(dir: &Path, args: MoveCourseArgs) -> Result<Value, ToolError> {
    let id = parse_id(&args.plan_id)?;
    let from = parse_label(&args.from_term)?;
    let to = parse_label(&args.to_term)?;
    let (dest_index, _) = index_for(&args.to_term)?;
    let course = require_offered(dest_index, &to.label, &args.subject, &args.number)?;
    let key = course.key();
    if from.season == to.season && from.year == to.year {
        return Err(ToolError::invalid("from_term and to_term are the same."));
    }
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        let from_index = plan
            .terms
            .iter()
            .position(|term| term.season == from.season && term.year == from.year)
            .ok_or_else(|| format!("{} is not on this plan.", from.label))?;
        let matches: Vec<usize> = plan.terms[from_index]
            .courses
            .iter()
            .enumerate()
            .filter(|(_, row)| row.key.as_ref() == Some(&key))
            .map(|(index, _)| index)
            .collect();
        if matches.is_empty() {
            return Err(format!(
                "{} {} is not on {}.",
                key.subject, key.number, from.label
            ));
        }
        if matches.len() > 1 {
            return Err(format!(
                "{} {} is on {} more than once, each with its own section.",
                key.subject, key.number, from.label
            ));
        }
        let row = plan.terms[from_index].courses.remove(matches[0]);
        let to_index = plan
            .terms
            .iter()
            .position(|term| term.season == to.season && term.year == to.year)
            .ok_or_else(|| format!("{} is not on this plan.", to.label))?;
        plan.terms[to_index].courses.push(row);
        Ok(())
    })
    .map_err(ToolError::from_plan)?;
    plan_value(dir, id)
}

pub fn set_section(dir: &Path, args: SetSectionArgs) -> Result<Value, ToolError> {
    let id = parse_id(&args.plan_id)?;
    let (index, parsed) = index_for(&args.term)?;
    let course = require_offered(index, &parsed.label, &args.subject, &args.number)?;
    let crn = checked_crn(index, course, Some(&args.crn), &parsed.label)?
        .ok_or_else(|| ToolError::invalid("A CRN is required."))?;
    let key = course.key();
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        let target = plan
            .terms
            .iter()
            .position(|term| term.season == parsed.season && term.year == parsed.year)
            .ok_or_else(|| format!("{} is not on this plan.", parsed.label))?;
        let on_term: Vec<usize> = plan.terms[target]
            .courses
            .iter()
            .enumerate()
            .filter(|(_, row)| row.key.as_ref() == Some(&key))
            .map(|(index, _)| index)
            .collect();
        if on_term.is_empty() {
            return Err(format!(
                "{} {} is not on {}.",
                key.subject, key.number, parsed.label
            ));
        }
        if on_term
            .iter()
            .any(|index| plan.terms[target].courses[*index].crn.as_deref() == Some(crn.as_str()))
        {
            return Ok(());
        }
        let used = plan
            .completed
            .iter()
            .any(|row| row.crn.as_deref() == Some(crn.as_str()))
            || plan.terms.iter().any(|term| {
                term.courses
                    .iter()
                    .any(|row| row.crn.as_deref() == Some(crn.as_str()))
            });
        if used {
            return Err("That section is already on this plan.".into());
        }
        if on_term.len() == 1 {
            plan.terms[target].courses[on_term[0]].crn = Some(crn.clone());
            return Ok(());
        }
        let unpinned: Vec<usize> = on_term
            .iter()
            .copied()
            .filter(|index| {
                planlogic::chosen_crn(plan.terms[target].courses[*index].crn.as_deref()).is_none()
            })
            .collect();
        if unpinned.len() == 1 {
            plan.terms[target].courses[unpinned[0]].crn = Some(crn.clone());
            return Ok(());
        }
        Err(format!(
            "{} {} is on {} more than once, and each copy already has a CRN.",
            key.subject, key.number, parsed.label
        ))
    })
    .map_err(ToolError::from_plan)?;
    plan_value(dir, id)
}

pub fn apply_reorganize(dir: &Path, args: RevisionArgs) -> Result<Value, ToolError> {
    let id = parse_id(&args.plan_id)?;
    let data = planfile::read_user(dir).map_err(ToolError::from_plan)?;
    let plan = data
        .degree_plans
        .iter()
        .find(|plan| plan.id == id)
        .cloned()
        .ok_or_else(|| ToolError::not_found("No degree plan has that id."))?;
    let actual = plan_revision(&data, id);
    if actual != args.expected_revision {
        return Err(ToolError::conflict(actual));
    }
    let result = reorganize::reorganize_degree_plan(&plan, catalogs());
    let moves: Vec<Value> = result.moves.iter().map(move_json).collect();
    if !result.moves.is_empty() {
        let replacement = result.plan;
        planfile::update_plan(dir, id, args.expected_revision, |plan| {
            let updated_at = plan.updated_at.clone();
            let created_at = plan.created_at.clone();
            *plan = replacement.clone();
            plan.updated_at = updated_at;
            plan.created_at = created_at;
            Ok(())
        })
        .map_err(ToolError::from_plan)?;
    }
    let mut value = plan_value(dir, id)?;
    value["moves"] = json!(moves);
    value["unresolved"] = json!(result
        .unresolved
        .iter()
        .map(|item| json!({
            "subject": item.subject,
            "number": item.number,
            "reason": item.reason,
        }))
        .collect::<Vec<_>>());
    Ok(value)
}

pub fn delete_plan(dir: &Path, args: DeleteArgs) -> Result<Value, ToolError> {
    if !args.confirm {
        return Err(ToolError {
            error: "confirm_required",
            message: "delete_plan refuses to run unless confirm is true.".into(),
            revision: None,
        });
    }
    let id = parse_id(&args.plan_id)?;
    planfile::delete_plan(dir, id, args.expected_revision).map_err(ToolError::from_plan)?;
    Ok(json!({ "deleted": true, "plan_id": id.to_string() }))
}

fn move_json(item: &reorganize::CourseMove) -> Value {
    json!({
        "subject": item.subject,
        "number": item.number,
        "from_term": item.from,
        "to_term": item.to,
        "reason": item.reason,
    })
}

fn plan_value(dir: &Path, id: Uuid) -> Result<Value, ToolError> {
    let data = planfile::read_user(dir).map_err(ToolError::from_plan)?;
    let plan = data
        .degree_plans
        .iter()
        .find(|plan| plan.id == id)
        .ok_or_else(|| ToolError::not_found("No degree plan has that id."))?;
    Ok(json!({
        "id": plan.id.to_string(),
        "name": plan.name,
        "revision": plan_revision(&data, id),
        "updated_at": plan.updated_at,
        "terms": plan.terms.iter().map(term_json).collect::<Vec<_>>(),
        "validation": validation(plan),
    }))
}

fn term_json(term: &DegreePlanTerm) -> Value {
    let index = reorganize::catalog_for_term(catalogs(), term.season, term.year);
    json!({
        "term": index
            .map(|catalog| catalog.catalog.term.code.clone())
            .unwrap_or_else(|| term.name()),
        "name": term.name(),
        "courses": term.courses.iter().map(|course| course_row(course, index)).collect::<Vec<_>>(),
    })
}

fn course_row(course: &DegreePlanCourse, index: Option<&CatalogIndex>) -> Value {
    let Some(key) = &course.key else {
        return json!({
            "label": course.label,
            "title": course.title,
            "credits": course.credits,
            "crn": Value::Null,
            "section": Value::Null,
            "offered": Value::Null,
        });
    };
    let offered = index
        .map(|catalog| !catalog.sections_for(key).is_empty())
        .unwrap_or(false);
    let section = course.crn.as_deref().and_then(|crn| {
        index
            .and_then(|catalog| catalog.section_crn(crn))
            .filter(|section| section.course_key() == *key)
            .map(section_json)
    });
    let listed = index.and_then(|catalog| catalog.course(key));
    json!({
        "subject": key.subject,
        "number": key.number,
        "title": course.title,
        "credits": course.credits,
        "credit_low": listed.map(|item| item.min_credits),
        "credit_high": listed.map(|item| item.max_credits),
        "crn": course.crn,
        "section": section,
        "offered": offered,
    })
}

fn validation(plan: &DegreePlan) -> Value {
    let mut clashes = Vec::new();
    let mut not_offered = Vec::new();
    for term in &plan.terms {
        let Some(index) = reorganize::catalog_for_term(catalogs(), term.season, term.year) else {
            continue;
        };
        let mut chosen: Vec<&termorganize_core::model::CourseSection> = Vec::new();
        for course in &term.courses {
            let Some(key) = &course.key else {
                continue;
            };
            if index.sections_for(key).is_empty() {
                not_offered.push(json!({
                    "term": term.name(),
                    "subject": key.subject,
                    "number": key.number,
                }));
            }
            let Some(crn) = course.crn.as_deref() else {
                continue;
            };
            let Some(section) = index
                .section_crn(crn)
                .filter(|section| section.course_key() == *key)
            else {
                continue;
            };
            for other in &chosen {
                if section.conflicts_with(other) {
                    clashes.push(json!({
                        "term": term.name(),
                        "first": { "subject": other.subject, "number": other.number, "crn": other.crn },
                        "second": { "subject": section.subject, "number": section.number, "crn": section.crn },
                        "reason": "The meetings overlap.",
                    }));
                }
            }
            chosen.push(section);
        }
    }
    let mut duplicates: Vec<String> = planlogic::duplicate_keys(plan)
        .into_iter()
        .map(|key| planlogic::duplicate_copy_notice(&key))
        .collect();
    duplicates.sort();
    json!({ "clashes": clashes, "not_offered": not_offered, "duplicates": duplicates })
}

fn term_code(term: &DegreePlanTerm) -> Option<String> {
    reorganize::catalog_for_term(catalogs(), term.season, term.year)
        .map(|index| index.catalog.term.code.clone())
}

fn index_for(term: &str) -> Result<(&'static CatalogIndex, reorganize::ParsedTerm), ToolError> {
    let parsed = parse_label(term)?;
    let index =
        reorganize::catalog_for_term(catalogs(), parsed.season, parsed.year).ok_or_else(|| {
            ToolError::not_found(format!(
                "No bundled catalog for {}. This server reads Fall 2026 and Spring 2027 only.",
                parsed.label
            ))
        })?;
    Ok((index, parsed))
}

fn parse_label(term: &str) -> Result<reorganize::ParsedTerm, ToolError> {
    reorganize::parse_term_label(term)
        .ok_or_else(|| ToolError::invalid(format!("Unrecognized term \"{}\".", term.trim())))
}

fn parse_id(raw: &str) -> Result<Uuid, ToolError> {
    Uuid::parse_str(raw.trim()).map_err(|_| ToolError::invalid("plan_id is not a UUID."))
}

fn find_course<'a>(index: &'a CatalogIndex, subject: &str, number: &str) -> Option<&'a Course> {
    let subject = subject.trim();
    let number = number.trim();
    index.courses.iter().find(|course| {
        course.subject.eq_ignore_ascii_case(subject) && course.number.eq_ignore_ascii_case(number)
    })
}

fn require_offered<'a>(
    index: &'a CatalogIndex,
    label: &str,
    subject: &str,
    number: &str,
) -> Result<&'a Course, ToolError> {
    let course = find_course(index, subject, number).ok_or_else(|| {
        ToolError::not_found(format!(
            "No course {} {} in {label}.",
            subject.trim().to_ascii_uppercase(),
            number.trim()
        ))
    })?;
    if index.sections_for(&course.key()).is_empty() {
        return Err(ToolError::not_found(format!(
            "{} {} has no sections in {label}.",
            course.subject, course.number
        )));
    }
    Ok(course)
}

fn checked_crn(
    index: &CatalogIndex,
    course: &Course,
    crn: Option<&str>,
    label: &str,
) -> Result<Option<String>, ToolError> {
    let Some(crn) = crn.map(str::trim).filter(|crn| !crn.is_empty()) else {
        return Ok(None);
    };
    if !crn_belongs(index, &course.key(), Some(crn)) {
        return Err(ToolError::not_found(format!(
            "CRN {crn} is not a section of {} {} in {label}.",
            course.subject, course.number
        )));
    }
    Ok(Some(crn.to_string()))
}

fn crn_belongs(index: &CatalogIndex, key: &CourseKey, crn: Option<&str>) -> bool {
    let Some(crn) = crn else {
        return true;
    };
    index
        .section_crn(crn)
        .is_some_and(|section| section.course_key() == *key)
}

fn term_on_plan(
    plan: &mut DegreePlan,
    season: termorganize_core::model::PlanSeason,
    year: i32,
) -> Result<&mut DegreePlanTerm, String> {
    let label = format!("{} {year}", season.export_name());
    plan.terms
        .iter_mut()
        .find(|term| term.season == season && term.year == year)
        .ok_or_else(|| format!("{label} is not on this plan."))
}

fn matches_attribute(course: &Course, attribute: &str) -> bool {
    course.attributes.iter().any(|code| {
        code.eq_ignore_ascii_case(attribute) || code.to_ascii_lowercase().contains(attribute)
    })
}

fn constraints(
    raw: Option<&Constraints>,
) -> Result<(ScheduleFilters, SchedulePreferences), ToolError> {
    let mut filters = ScheduleFilters::default();
    let mut preferences = SchedulePreferences::default();
    let Some(raw) = raw else {
        return Ok((filters, preferences));
    };
    if let Some(text) = raw.no_classes_before.as_deref() {
        let minutes = parse_clock(text)?;
        filters.earliest_start = Some(minutes);
        preferences.earliest_start = Some(TimeLimit {
            minutes,
            strength: PreferenceStrength::Required,
        });
    }
    if let Some(text) = raw.no_classes_after.as_deref() {
        let minutes = parse_clock(text)?;
        filters.latest_end = Some(minutes);
        preferences.latest_end = Some(TimeLimit {
            minutes,
            strength: PreferenceStrength::Required,
        });
    }
    let mut days = DaySet(0);
    for name in &raw.days_off {
        let day = parse_day(name)
            .ok_or_else(|| ToolError::invalid(format!("Unrecognized day off \"{name}\".")))?;
        days.0 |= day.0;
    }
    if !days.is_empty() {
        filters.free_days = days;
        preferences.days_off = days;
        preferences.days_off_strength = PreferenceStrength::Required;
    }
    Ok((filters, preferences))
}

/// `HH:MM` on a 24-hour clock, or an explicit 12-hour time with `am` or `pm`.
/// `8am` and `8:00 AM` are 08:00. A bare `8`, or any other shape, is rejected.
fn parse_clock(raw: &str) -> Result<i32, ToolError> {
    let invalid = || ToolError::invalid(format!("Unrecognized time \"{raw}\"."));
    let text = raw.trim().to_ascii_lowercase().replace('.', "");
    if text.is_empty() {
        return Err(invalid());
    }
    let (clock, afternoon) = if let Some(rest) = text.strip_suffix("am") {
        (rest.trim(), Some(false))
    } else if let Some(rest) = text.strip_suffix("pm") {
        (rest.trim(), Some(true))
    } else {
        (text.as_str(), None)
    };
    if clock.is_empty()
        || clock
            .chars()
            .any(|character| !character.is_ascii_digit() && character != ':')
    {
        return Err(invalid());
    }
    let mut parts = clock.split(':');
    let hour_text = parts.next().unwrap_or("");
    let minute_text = match (parts.next(), afternoon) {
        (Some(minute), _) => minute,
        (None, Some(_)) => "00",
        (None, None) => return Err(invalid()),
    };
    if parts.next().is_some()
        || hour_text.is_empty()
        || minute_text.len() != 2
        || !minute_text
            .chars()
            .all(|character| character.is_ascii_digit())
        || !hour_text
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err(invalid());
    }
    let hour: i32 = hour_text.parse().map_err(|_| invalid())?;
    let minute: i32 = minute_text.parse().map_err(|_| invalid())?;
    if !(0..60).contains(&minute) {
        return Err(invalid());
    }
    let hour = match afternoon {
        Some(false) => match hour {
            12 => 0,
            1..=11 => hour,
            _ => return Err(invalid()),
        },
        Some(true) => match hour {
            12 => 12,
            1..=11 => hour + 12,
            _ => return Err(invalid()),
        },
        None if (0..24).contains(&hour) => hour,
        None => return Err(invalid()),
    };
    Ok(hour * 60 + minute)
}

fn parse_day(raw: &str) -> Option<DaySet> {
    let index = match raw.trim().to_ascii_lowercase().as_str() {
        "u" | "su" | "sun" | "sunday" => 0,
        "m" | "mo" | "mon" | "monday" => 1,
        "t" | "tu" | "tue" | "tues" | "tuesday" => 2,
        "w" | "we" | "wed" | "wednesday" => 3,
        "r" | "th" | "thu" | "thur" | "thurs" | "thursday" => 4,
        "f" | "fr" | "fri" | "friday" => 5,
        "s" | "sa" | "sat" | "saturday" => 6,
        _ => return None,
    };
    Some(DaySet(1 << index))
}

#[cfg(test)]
mod clock_tests {
    use super::parse_clock;

    #[test]
    fn eight_am_is_explicit_and_other_shapes_are_rejected() {
        assert_eq!(parse_clock("8am").unwrap(), 8 * 60);
        assert_eq!(parse_clock("8:00 AM").unwrap(), 8 * 60);
        assert_eq!(parse_clock("08:00").unwrap(), 8 * 60);
        assert_eq!(parse_clock("17:00").unwrap(), 17 * 60);
        assert_eq!(parse_clock("9:00 PM").unwrap(), 21 * 60);
        for raw in ["8", "eight", "8:00:00", "25:00", ""] {
            let error = parse_clock(raw).unwrap_err();
            assert_eq!(error.error, "invalid_argument");
            assert!(!error.message.is_empty(), "{raw}");
        }
    }
}
