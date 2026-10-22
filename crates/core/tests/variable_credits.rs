//! Variable-credit courses keep Banner's low and high. Each degree-plan row
//! and each schedule row stores its own chosen value. Hours, sync, and a
//! catalog refresh use that value: a missing choice becomes the default, and
//! a saved choice is clamped into a new range.

use std::collections::{HashMap, HashSet};
use termorganize_core::catalog::{self, BannerSection};
use termorganize_core::degree_audit::{DegreeAudit, DegreeCourseAttributes};
use termorganize_core::index::CatalogIndex;
use termorganize_core::model::*;
use termorganize_core::planlogic::{self, CreditEdit};
use termorganize_core::schedule::ResolvedPlan;
use termorganize_core::CourseEquivalences;
use uuid::Uuid;

fn readings(low: f64, high: f64, reported: Option<f64>) -> Course {
    Course {
        subject: "CS".into(),
        number: "8982".into(),
        title: "Readings".into(),
        description: String::new(),
        min_credits: low,
        max_credits: high,
        credit_hours: reported,
        attributes: vec![],
        prereqs: None,
        coreqs: None,
        postreqs: vec![],
        special_topics: false,
        college: None,
        department: None,
        details_loaded: true,
    }
}

fn fixed(number: &str, credits: f64) -> Course {
    Course {
        number: number.into(),
        title: "Algorithms".into(),
        min_credits: credits,
        max_credits: credits,
        credit_hours: Some(credits),
        ..readings(credits, credits, Some(credits))
    }
}

fn key() -> CourseKey {
    CourseKey::parse("CS 8982").unwrap()
}

fn degree_row(credits: f64, crn: Option<&str>) -> DegreePlanCourse {
    let mut course = DegreePlanCourse::catalog(key(), "Readings", credits);
    course.crn = crn.map(str::to_string);
    course
}

fn empty_requirements() -> ProgramRequirements {
    ProgramRequirements {
        year: CatalogYear::new(2025),
        program: CatalogProgram {
            title: "Test".into(),
            path: "test".into(),
        },
        sections: vec![],
        total_hours: None,
        fetched_at: 0.0,
        course_titles: std::collections::BTreeMap::new(),
        universitywide: None,
    }
}

fn plan_with(courses: Vec<DegreePlanCourse>) -> DegreePlan {
    let mut term = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    term.courses = courses;
    DegreePlan {
        name: "Plan".into(),
        terms: vec![term],
        ..DegreePlan::default()
    }
}

fn audit(plan: &DegreePlan) -> DegreeAudit {
    DegreeAudit::run(
        plan,
        &empty_requirements(),
        &CourseEquivalences::none(),
        &DegreeCourseAttributes::none(),
    )
}

fn counted_ids(audit: &DegreeAudit) -> Vec<String> {
    let mut ids: Vec<String> = audit
        .rule_results()
        .into_iter()
        .flat_map(|result| result.used.iter())
        .chain(audit.unused.iter())
        .map(|use_| use_.id())
        .collect();
    ids.sort();
    ids
}

fn schedule_row(credits: Option<f64>, pin: Option<&str>) -> PlanCourse {
    PlanCourse {
        key: key(),
        is_required: true,
        excluded_crns: HashSet::new(),
        color_index: 0,
        pinned_crn: pin.map(str::to_string),
        degree_row_id: None,
        credits,
    }
}

fn schedule(courses: Vec<PlanCourse>, primary: bool) -> SchedulePlan {
    SchedulePlan {
        id: Uuid::new_v4(),
        name: "Fall plan".into(),
        term_code: "202710".into(),
        courses,
        selected_crns: vec![],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: primary,
        created_at: 0.0,
        updated_at: 0.0,
    }
}

fn banner() -> Vec<BannerSection> {
    serde_json::from_str(
        r#"[
            {
                "courseReferenceNumber": "11111",
                "subject": "CS",
                "courseNumber": "8982",
                "courseTitle": "Readings",
                "creditHours": 4,
                "creditHourLow": 1,
                "creditHourHigh": 8,
                "creditHourIndicator": "TO"
            },
            {
                "courseReferenceNumber": "22222",
                "subject": "CS",
                "courseNumber": "8982",
                "courseTitle": "Readings",
                "creditHours": 4
            },
            {
                "courseReferenceNumber": "33333",
                "subject": "CS",
                "courseNumber": "2500",
                "courseTitle": "Fundamentals",
                "creditHours": 4
            }
        ]"#,
    )
    .unwrap()
}

#[test]
fn banner_keeps_the_low_and_high_and_a_later_fixed_section_does_not_erase_them() {
    let sections = banner();
    assert_eq!(sections[0].credit_hour_indicator.as_deref(), Some("TO"));
    assert!(sections[2].credit_hour_high.is_none());

    let (courses, _) = catalog::stub_courses(&sections);
    let readings = courses
        .iter()
        .find(|course| course.number == "8982")
        .unwrap();
    assert!(readings.is_variable_credit());
    assert!((readings.min_credits - 1.0).abs() < 1e-9);
    assert!((readings.max_credits - 8.0).abs() < 1e-9);
    assert_eq!(readings.credit_hours, Some(4.0));
    let fixed = courses
        .iter()
        .find(|course| course.number == "2500")
        .unwrap();
    assert!(!fixed.is_variable_credit());
    assert!((fixed.min_credits - 4.0).abs() < 1e-9);
    assert!((fixed.max_credits - 4.0).abs() < 1e-9);

    let built = catalog::make_sections(&sections, &HashMap::new());
    let variable = built.iter().find(|section| section.crn == "11111").unwrap();
    assert_eq!(variable.credit_hour_low, Some(1.0));
    assert_eq!(variable.credit_hour_high, Some(8.0));
    assert_eq!(variable.credit_hours, Some(4.0));
    let plain = built.iter().find(|section| section.crn == "22222").unwrap();
    assert!(plain.credit_hour_high.is_none());
}

#[test]
fn default_credits_uses_reported_hours_when_they_sit_in_the_range() {
    let reported = readings(1.0, 8.0, Some(4.0));
    assert!((reported.default_credits() - 4.0).abs() < 1e-9);
    assert_eq!(reported.credits_choice_label(4.0), "Credits: 4 (1 to 8)");

    let older_catalog = readings(1.0, 8.0, None);
    assert!((older_catalog.default_credits() - 4.0).abs() < 1e-9);

    let practicum = readings(2.0, 4.0, Some(4.0));
    assert!((practicum.default_credits() - 4.0).abs() < 1e-9);

    let above_the_report = readings(6.0, 8.0, Some(4.0));
    assert!((above_the_report.default_credits() - 6.0).abs() < 1e-9);

    let only = fixed("2500", 4.0);
    assert!((only.default_credits() - 4.0).abs() < 1e-9);
    assert!(!only.is_variable_credit());
}

#[test]
fn clamp_keeps_a_whole_number_inside_the_range() {
    let course = readings(1.0, 8.0, Some(4.0));
    assert!((course.clamp_credits(0.0) - 1.0).abs() < 1e-9);
    assert!((course.clamp_credits(9.0) - 8.0).abs() < 1e-9);
    assert!((course.clamp_credits(4.6) - 5.0).abs() < 1e-9);
    assert!((course.clamp_credits(4.0) - 4.0).abs() < 1e-9);
    assert!((fixed("2500", 4.0).clamp_credits(9.0) - 4.0).abs() < 1e-9);
}

#[test]
fn two_copies_with_different_credits_both_count() {
    let plan = plan_with(vec![degree_row(1.0, None), degree_row(8.0, None)]);
    let result = audit(&plan);
    assert!((result.planned_hours - 9.0).abs() < 1e-9);
    assert!((plan.terms[0].credits() - 9.0).abs() < 1e-9);
    assert_eq!(counted_ids(&result), ["CS 8982", "CS 8982 (2)"]);

    let distinct = plan_with(vec![
        degree_row(1.0, Some("11111")),
        degree_row(8.0, Some("22222")),
    ]);
    let result = audit(&distinct);
    assert!((result.planned_hours - 9.0).abs() < 1e-9);
    assert!(planlogic::duplicate_keys(&distinct).is_empty());
}

#[test]
fn schedule_and_degree_rows_keep_each_copy_in_sync() {
    let degree = plan_with(vec![degree_row(1.0, None), degree_row(8.0, Some("22222"))]);
    let term = degree.terms[0].clone();
    let first = term.courses[0].id;
    let second = term.courses[1].id;
    let mut data = UserData::default();
    data.degree_plans.push(degree);
    let opened = planlogic::schedule_for_semester(&mut data, &term, "202710").unwrap();
    assert!(opened.is_new);
    assert_eq!(opened.plan.courses.len(), 2);
    assert_eq!(opened.plan.courses[0].credits, Some(1.0));
    assert_eq!(opened.plan.courses[1].credits, Some(8.0));
    assert_eq!(opened.plan.courses[0].degree_row_id, Some(first));
    assert_eq!(opened.plan.courses[1].degree_row_id, Some(second));
    let again = planlogic::schedule_for_semester(&mut data, &term, "202710").unwrap();
    assert!(!again.is_new);
    assert_eq!(data.plans.len(), 1);
    assert_eq!(again.plan.courses.len(), 2);

    let schedule_id = data.plans[0].id;
    planlogic::set_chosen_credits(
        &mut data,
        CreditEdit::ScheduleRow {
            plan: schedule_id,
            index: 1,
        },
        6.0,
    );
    assert_eq!(data.plans[0].courses[1].credits, Some(6.0));
    assert!((data.degree_plans[0].terms[0].courses[1].credits - 6.0).abs() < 1e-9);
    assert!((data.degree_plans[0].terms[0].courses[0].credits - 1.0).abs() < 1e-9);

    planlogic::set_chosen_credits(&mut data, CreditEdit::DegreeRow(first), 2.0);
    assert!((data.degree_plans[0].terms[0].courses[0].credits - 2.0).abs() < 1e-9);
    assert_eq!(data.plans[0].courses[0].credits, Some(2.0));
    assert_eq!(data.plans[0].courses[1].credits, Some(6.0));

    planlogic::set_chosen_credits(
        &mut data,
        CreditEdit::ScheduleRow {
            plan: schedule_id,
            index: 1,
        },
        7.4,
    );
    assert_eq!(data.plans[0].courses[1].credits, Some(7.0));
    assert!((data.degree_plans[0].terms[0].courses[1].credits - 7.0).abs() < 1e-9);
    assert_eq!(data.degree_plans[0].terms[0].courses.len(), 2);
    assert_eq!(data.plans[0].courses.len(), 2);
}

#[test]
fn a_schedule_draft_copies_each_rows_credits_and_does_not_merge() {
    let rows = vec![schedule_row(Some(2.0), None), schedule_row(Some(6.0), None)];
    let draft = planlogic::new_degree_plan_draft("Plan", None, &[schedule(rows, true)], 2026, 10);
    assert_eq!(draft.terms.len(), 1);
    assert_eq!(draft.terms[0].courses.len(), 2);
    assert!((draft.terms[0].courses[0].credits - 2.0).abs() < 1e-9);
    assert!((draft.terms[0].courses[1].credits - 6.0).abs() < 1e-9);
    assert!(draft.terms[0].courses[0].crn.is_none());
    assert!(draft.terms[0].courses[1].crn.is_none());
}

#[test]
fn schedule_totals_use_the_chosen_value() {
    let catalog = TermCatalog {
        version: 1,
        term: Term {
            code: "202710".into(),
            banner_description: "Fall 2026".into(),
        },
        generated_at: 0.0,
        sections_updated_at: 0.0,
        courses: vec![readings(1.0, 8.0, Some(4.0))],
        sections: vec![],
        subjects: vec![],
        attributes: vec![],
        campuses: vec![],
    };
    let index = CatalogIndex::build(catalog);
    let plan = schedule(
        vec![
            schedule_row(Some(8.0), None),
            schedule_row(Some(8.0), None),
            schedule_row(None, None),
        ],
        true,
    );
    let resolved = ResolvedPlan::resolve(&plan, Some(&index));
    assert!((resolved.credits_min - 20.0).abs() < 1e-9);
    assert!((resolved.credits_max - 20.0).abs() < 1e-9);
    assert!(resolved.is_over_credit_limit());
}

#[test]
fn old_files_take_the_default_and_a_refresh_clamps_a_saved_choice() {
    let text = r#"{
        "version": 1,
        "plans": [{
            "name": "Fall plan",
            "termCode": "202710",
            "courses": [{
                "key": {"subject": "CS", "number": "8982"},
                "isRequired": true
            }, {
                "key": {"subject": "CS", "number": "8982"},
                "isRequired": true,
                "credits": 8
            }, {
                "key": {"subject": "CS", "number": "8982"},
                "isRequired": true,
                "credits": 3
            }]
        }],
        "degreePlans": [{
            "name": "Plan",
            "terms": [{
                "season": "fall",
                "year": 2026,
                "courses": [
                    {"label": "CS 8982", "title": "Readings", "credits": 3, "key": {"subject": "CS", "number": "8982"}},
                    {"label": "CS 8982", "title": "Readings", "credits": 8, "key": {"subject": "CS", "number": "8982"}},
                    {"label": "CS 3000", "title": "Algorithms", "credits": 0, "key": {"subject": "CS", "number": "3000"}}
                ]
            }]
        }]
    }"#;
    let mut data = UserData::from_json_lossy(text);
    assert_eq!(data.version, 1);
    assert!(data.plans[0].courses[0].credits.is_none());
    assert_eq!(data.plans[0].courses[1].credits, Some(8.0));

    let widened = readings(2.0, 6.0, Some(4.0));
    let algorithms = fixed("3000", 4.0);
    planlogic::clamp_saved_credits(&mut data, &[widened, algorithms]);

    assert_eq!(data.plans[0].courses[0].credits, Some(4.0));
    assert_eq!(data.plans[0].courses[1].credits, Some(6.0));
    assert_eq!(data.plans[0].courses[2].credits, Some(3.0));
    let rows = &data.degree_plans[0].terms[0].courses;
    assert!((rows[0].credits - 3.0).abs() < 1e-9);
    assert!((rows[1].credits - 6.0).abs() < 1e-9);
    assert!((rows[2].credits - 0.0).abs() < 1e-9);
}
