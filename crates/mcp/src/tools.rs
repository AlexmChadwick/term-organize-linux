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
        let mut value = json!({
            "error": self.error,
            "message": self.message,
        });
        if let Some(revision) = self.revision {
            value["revision"] = json!(revision);
        }
        value
    }

    fn rejected(message: impl Into<String>) -> Self {
        Self {
            error: "rejected",
            message: message.into(),
            revision: None,
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            error: "not_found",
            message: message.into(),
            revision: None,
        }
    }

    fn conflict(actual: u64) -> Self {
        Self {
            error: "revision_conflict",
            message: "The plan changed. Reload it and try again.".into(),
            revision: Some(actual),
        }
    }

    fn from_plan(error: PlanFileError) -> Self {
        match error {
            PlanFileError::NotFound => Self::not_found("No degree plan has that id."),
            PlanFileError::RevisionConflict { actual } => Self::conflict(actual),
            PlanFileError::Rejected(message) => Self::rejected(message),
            PlanFileError::Io(message) => Self {
                error: "io",
                message,
                revision: None,
            },
        }
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
    pub plan: PlanInput,
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
                "code": index.catalog.term.code,
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
        courses.push(course_json(&result.course));
        if courses.len() == limit {
            break;
        }
    }
    Ok(json!({
        "term": { "code": index.catalog.term.code, "name": parsed.label },
        "courses": courses,
    }))
}

pub fn get_course(args: CourseArgs) -> Result<Value, ToolError> {
    let (index, parsed) = index_for(&args.term)?;
    let course = find_course(index, &args.subject, &args.number).ok_or_else(|| {
        ToolError::rejected(format!(
            "No course {} {} in {}.",
            args.subject.trim().to_ascii_uppercase(),
            args.number.trim(),
            parsed.label
        ))
    })?;
    let sections: Vec<Value> = index
        .sections_for(&course.key())
        .iter()
        .map(section_json)
        .collect();
    let mut value = course_json(course);
    value["term"] = json!({ "code": index.catalog.term.code, "name": parsed.label });
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
        "term": { "code": index.catalog.term.code, "name": parsed.label },
        "clashes": clashes,
        "unknown_crns": unknown,
    }))
}

pub fn suggest_schedule(args: SuggestArgs) -> Result<Value, ToolError> {
    let (index, parsed) = index_for(&args.term)?;
    if args.courses.is_empty() {
        return Err(ToolError::rejected(
            "suggest_schedule needs at least one course.",
        ));
    }
    let mut inputs = Vec::new();
    for course in &args.courses {
        let found = find_course(index, &course.subject, &course.number).ok_or_else(|| {
            ToolError::rejected(format!(
                "No course {} {} in {}.",
                course.subject, course.number, parsed.label
            ))
        })?;
        let mut sections = index.sections_for(&found.key());
        if let Some(crn) = course.crn.as_deref() {
            sections.retain(|section| section.crn == crn);
            if sections.is_empty() {
                return Err(ToolError::rejected(format!(
                    "CRN {crn} is not a section of {} {} in {}.",
                    found.subject, found.number, parsed.label
                )));
            }
        }
        if sections.is_empty() {
            return Err(ToolError::rejected(format!(
                "{} {} has no sections in {}.",
                found.subject, found.number, parsed.label
            )));
        }
        inputs.push(ScheduleCourseInput::new(found.key(), true, sections));
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
        "term": { "code": index.catalog.term.code, "name": parsed.label },
        "schedules": schedules,
    }))
}

pub fn reorganize_plan(args: ReorganizeArgs) -> Result<Value, ToolError> {
    let slots: Vec<PlanSlot> = args
        .plan
        .terms
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
        .collect();
    let outcome = reorganize::reorganize(&slots, catalogs());
    Ok(json!({
        "plan": {
            "terms": outcome.terms.iter().map(|term| json!({
                "term": term.term,
                "courses": term.courses.iter().map(|course| json!({
                    "subject": course.subject,
                    "number": course.number,
                    "crn": course.crn,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        },
        "moves": outcome.moves.iter().map(move_json).collect::<Vec<_>>(),
        "notes": outcome.notes,
    }))
}

pub fn list_plans(dir: &Path) -> Value {
    let data = planfile::read_user(dir);
    let plans: Vec<Value> = data
        .degree_plans
        .iter()
        .map(|plan| {
            json!({
                "id": plan.id.to_string(),
                "name": plan.name,
                "terms": plan.terms.iter().map(|term| json!({
                    "name": term.name(),
                    "code": term_code(term),
                })).collect::<Vec<_>>(),
                "updated_at": plan.updated_at,
                "revision": plan_revision(&data, plan.id),
            })
        })
        .collect();
    json!({ "plans": plans })
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
        return Err(ToolError::rejected("A plan needs a name."));
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
    let credits = course.min_credits;
    let key = course.key();
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        let term = term_on_plan(plan, parsed.season, parsed.year)?;
        if term
            .courses
            .iter()
            .any(|row| row.key.as_ref() == Some(&key))
        {
            return Err("That course is already on this term.".into());
        }
        let mut row = DegreePlanCourse::catalog(key.clone(), title.clone(), credits);
        row.crn = crn.clone();
        term.courses.push(row);
        Ok(())
    })
    .map_err(ToolError::from_plan)?;
    plan_value(dir, id)
}

pub fn remove_course(dir: &Path, args: RemoveCourseArgs) -> Result<Value, ToolError> {
    let id = parse_id(&args.plan_id)?;
    let parsed = parse_label(&args.term)?;
    let subject = args.subject.trim().to_ascii_uppercase();
    let number = args.number.trim().to_string();
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        let term = term_on_plan(plan, parsed.season, parsed.year)?;
        let before = term.courses.len();
        term.courses.retain(|course| {
            !course.key.as_ref().is_some_and(|key| {
                key.subject.eq_ignore_ascii_case(&subject)
                    && key.number.eq_ignore_ascii_case(&number)
            })
        });
        if term.courses.len() == before {
            return Err(format!("{subject} {number} is not on {}.", term.name()));
        }
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
        return Err(ToolError::rejected("from_term and to_term are the same."));
    }
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        let from_index = plan
            .terms
            .iter()
            .position(|term| term.season == from.season && term.year == from.year)
            .ok_or_else(|| format!("{} is not on this plan.", from.label))?;
        let position = plan.terms[from_index]
            .courses
            .iter()
            .position(|row| row.key.as_ref() == Some(&key))
            .ok_or_else(|| format!("{} {} is not on {}.", key.subject, key.number, from.label))?;
        let mut row = plan.terms[from_index].courses.remove(position);
        let to_index = plan
            .terms
            .iter()
            .position(|term| term.season == to.season && term.year == to.year)
            .ok_or_else(|| format!("{} is not on this plan.", to.label))?;
        if plan.terms[to_index]
            .courses
            .iter()
            .any(|existing| existing.key.as_ref() == Some(&key))
        {
            return Err(format!(
                "{} {} is already on {}.",
                key.subject, key.number, to.label
            ));
        }
        if !crn_belongs(dest_index, &key, row.crn.as_deref()) {
            row.crn = None;
        }
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
        .ok_or_else(|| ToolError::rejected("A CRN is required."))?;
    let key = course.key();
    planfile::update_plan(dir, id, args.expected_revision, |plan| {
        let term = term_on_plan(plan, parsed.season, parsed.year)?;
        let row = term
            .courses
            .iter_mut()
            .find(|row| row.key.as_ref() == Some(&key))
            .ok_or_else(|| format!("{} {} is not on {}.", key.subject, key.number, parsed.label))?;
        row.crn = Some(crn.clone());
        Ok(())
    })
    .map_err(ToolError::from_plan)?;
    plan_value(dir, id)
}

pub fn apply_reorganize(dir: &Path, args: RevisionArgs) -> Result<Value, ToolError> {
    let id = parse_id(&args.plan_id)?;
    let data = planfile::read_user(dir);
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
    value["notes"] = json!(result.notes);
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
        "from": item.from,
        "to": item.to,
        "reason": item.reason,
    })
}

fn plan_value(dir: &Path, id: Uuid) -> Result<Value, ToolError> {
    let data = planfile::read_user(dir);
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
        "name": term.name(),
        "code": index.map(|catalog| catalog.catalog.term.code.clone()),
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
    json!({
        "subject": key.subject,
        "number": key.number,
        "title": course.title,
        "credits": course.credits,
        "crn": course.crn,
        "section": section,
        "offered": offered,
    })
}

fn validation(plan: &DegreePlan) -> Value {
    let mut clashes = Vec::new();
    let mut unoffered = Vec::new();
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
                unoffered.push(json!({
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
    json!({ "clashes": clashes, "unoffered": unoffered })
}

fn term_code(term: &DegreePlanTerm) -> Option<String> {
    reorganize::catalog_for_term(catalogs(), term.season, term.year)
        .map(|index| index.catalog.term.code.clone())
}

fn index_for(term: &str) -> Result<(&'static CatalogIndex, reorganize::ParsedTerm), ToolError> {
    let parsed = parse_label(term)?;
    let index =
        reorganize::catalog_for_term(catalogs(), parsed.season, parsed.year).ok_or_else(|| {
            ToolError::rejected(format!(
                "No bundled catalog for {}. This server reads Fall 2026 and Spring 2027 only.",
                parsed.label
            ))
        })?;
    Ok((index, parsed))
}

fn parse_label(term: &str) -> Result<reorganize::ParsedTerm, ToolError> {
    reorganize::parse_term_label(term)
        .ok_or_else(|| ToolError::rejected(format!("Unrecognized term \"{}\".", term.trim())))
}

fn parse_id(raw: &str) -> Result<Uuid, ToolError> {
    Uuid::parse_str(raw.trim()).map_err(|_| ToolError::rejected("plan_id is not a UUID."))
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
        ToolError::rejected(format!(
            "No course {} {} in {label}.",
            subject.trim().to_ascii_uppercase(),
            number.trim()
        ))
    })?;
    if index.sections_for(&course.key()).is_empty() {
        return Err(ToolError::rejected(format!(
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
        return Err(ToolError::rejected(format!(
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
            .ok_or_else(|| ToolError::rejected(format!("Unrecognized day off \"{name}\".")))?;
        days.0 |= day.0;
    }
    if !days.is_empty() {
        filters.free_days = days;
        preferences.days_off = days;
        preferences.days_off_strength = PreferenceStrength::Required;
    }
    Ok((filters, preferences))
}

fn parse_clock(raw: &str) -> Result<i32, ToolError> {
    let text = raw.trim().to_ascii_lowercase().replace('.', "");
    let (clock, afternoon) = if let Some(rest) = text.strip_suffix("am") {
        (rest.trim(), Some(false))
    } else if let Some(rest) = text.strip_suffix("pm") {
        (rest.trim(), Some(true))
    } else {
        (text.as_str(), None)
    };
    let mut parts = clock.split(':');
    let hour: i32 = parts
        .next()
        .unwrap_or("")
        .trim()
        .parse()
        .map_err(|_| ToolError::rejected(format!("Unrecognized time \"{raw}\".")))?;
    let minute: i32 = parts.next().unwrap_or("0").trim().parse().unwrap_or(-1);
    if parts.next().is_some() || !(0..60).contains(&minute) {
        return Err(ToolError::rejected(format!("Unrecognized time \"{raw}\".")));
    }
    let hour = match afternoon {
        Some(false) => {
            if hour == 12 {
                0
            } else {
                hour
            }
        }
        Some(true) => {
            if hour == 12 {
                12
            } else {
                hour + 12
            }
        }
        None => hour,
    };
    if !(0..24).contains(&hour) {
        return Err(ToolError::rejected(format!("Unrecognized time \"{raw}\".")));
    }
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
