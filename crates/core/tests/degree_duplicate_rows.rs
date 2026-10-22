//! A degree plan may list a course more than once. Every copy with credit counts.
//! A warning is for a real repeat: two copies with no distinct sections, or the
//! same CRN twice. A title containing "topic", or two different CRNs, is not a
//! warning. Substitutions name one row. Schedule sync copies one row at a time
//! and does not invent a CRN.

use std::collections::{HashMap, HashSet};
use termorganize_core::degree_audit::{DegreeAudit, DegreeCourseAttributes};
use termorganize_core::model::*;
use termorganize_core::planlogic::{
    self, already_in_plan_notice, duplicate_copy_notice, duplicate_plan_notice,
};
use termorganize_core::CourseEquivalences;
use uuid::Uuid;

fn key() -> CourseKey {
    CourseKey::parse("CS 4973").unwrap()
}

fn seminar(title: &str, crn: Option<&str>) -> DegreePlanCourse {
    let mut course = DegreePlanCourse::catalog(key(), title, 4.0);
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

fn rule(id: &str) -> RequirementRule {
    RequirementRule {
        id: id.into(),
        kind: RuleKind::Required(CourseOption {
            alternatives: vec![vec![CourseKey::parse("CS 9999").unwrap()]],
        }),
        label: id.into(),
        hours: None,
        notes: vec![],
        option_categories: None,
    }
}

fn requirements_of(rules: Vec<RequirementRule>) -> ProgramRequirements {
    ProgramRequirements {
        sections: vec![RequirementSection {
            id: "major".into(),
            title: "Major".into(),
            is_concentration: false,
            groups: rules
                .into_iter()
                .map(|rule| RequirementGroup {
                    id: format!("group-{}", rule.id),
                    title: Some(rule.id.clone()),
                    rules: vec![rule],
                    notes: vec![],
                })
                .collect(),
            notes: vec![],
        }],
        ..empty_requirements()
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

fn schedule_row(code: &str, pin: Option<&str>) -> PlanCourse {
    PlanCourse {
        key: CourseKey::parse(code).unwrap(),
        is_required: true,
        excluded_crns: HashSet::new(),
        color_index: 0,
        pinned_crn: pin.map(str::to_string),
        degree_row_id: None,
        credits: None,
    }
}

#[test]
fn two_unpinned_seminars_count_and_warn() {
    let plan = plan_with(vec![seminar("Seminar", None), seminar("Seminar", None)]);
    let audit = audit(&plan);
    assert!((audit.planned_hours - 8.0).abs() < 1e-9);
    assert_eq!(counted_ids(&audit), ["CS 4973", "CS 4973 (2)"]);
    assert_eq!(planlogic::duplicate_keys(&plan), HashSet::from([key()]));
    assert_eq!(
        duplicate_plan_notice(1),
        "1 course appears more than once in this plan. Each copy still counts."
    );
    assert_eq!(
        duplicate_plan_notice(2),
        "2 courses appear more than once in this plan. Each copy still counts."
    );
    assert_eq!(
        already_in_plan_notice("Fall 2026"),
        "Already in Fall 2026. Adding it again keeps both copies."
    );
    assert_eq!(
        duplicate_copy_notice(&key()),
        "CS 4973 appears more than once. Each copy still counts."
    );
}

#[test]
fn the_same_crn_twice_counts_and_warns() {
    let plan = plan_with(vec![
        seminar("Seminar", Some("11111")),
        seminar("Seminar", Some("11111")),
    ]);
    let audit = audit(&plan);
    assert!((audit.planned_hours - 8.0).abs() < 1e-9);
    assert_eq!(counted_ids(&audit), ["CS 4973", "CS 4973 (2)"]);
    assert_eq!(planlogic::duplicate_keys(&plan), HashSet::from([key()]));
}

#[test]
fn different_crns_count_without_a_warning() {
    let plan = plan_with(vec![
        seminar("Seminar", Some("11111")),
        seminar("Seminar", Some("22222")),
    ]);
    let audit = audit(&plan);
    assert!((audit.planned_hours - 8.0).abs() < 1e-9);
    assert_eq!(counted_ids(&audit), ["CS 4973", "CS 4973 (2)"]);
    assert!(planlogic::duplicate_keys(&plan).is_empty());
}

#[test]
fn a_topic_title_is_not_a_warning_and_still_counts() {
    let plan = plan_with(vec![
        seminar("Topics in Computer Science", None),
        seminar("Topics in Computer Science", None),
    ]);
    let audit = audit(&plan);
    assert!((audit.planned_hours - 8.0).abs() < 1e-9);
    assert!(planlogic::duplicate_keys(&plan).is_empty());
}

#[test]
fn completed_copies_warn_each_other() {
    let mut plan = DegreePlan {
        name: "Done".into(),
        completed: vec![seminar("Seminar", None), seminar("Seminar", None)],
        ..DegreePlan::default()
    };
    assert_eq!(planlogic::duplicate_keys(&plan), HashSet::from([key()]));
    let done = audit(&plan);
    assert!((done.completed_hours - 8.0).abs() < 1e-9);

    plan.completed.pop();
    let mut term = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    term.courses = vec![seminar("Seminar", None)];
    plan.terms = vec![term];
    assert_eq!(planlogic::duplicate_keys(&plan), HashSet::from([key()]));

    plan.completed.clear();
    assert!(planlogic::duplicate_keys(&plan).is_empty());
}

#[test]
fn a_substitution_names_one_row() {
    let first = seminar("Seminar", None);
    let second = seminar("Seminar", None);
    let first_id = first.id;
    let second_id = second.id;
    let mut plan = plan_with(vec![first, second]);
    let left = Uuid::new_v4();
    let right = Uuid::new_v4();
    plan.substitutions = vec![
        RequirementSubstitution {
            id: left,
            rule_id: "left".into(),
            rule_title: "Left".into(),
            change: SubstitutionChange::count_row(key(), first_id),
            note: String::new(),
            approved_on: None,
            created_at: None,
        },
        RequirementSubstitution {
            id: right,
            rule_id: "right".into(),
            rule_title: "Right".into(),
            change: SubstitutionChange::count_row(key(), second_id),
            note: String::new(),
            approved_on: None,
            created_at: None,
        },
    ];
    let requirements = requirements_of(vec![rule("left"), rule("right")]);
    let audit = DegreeAudit::run(
        &plan,
        &requirements,
        &CourseEquivalences::none(),
        &DegreeCourseAttributes::none(),
    );
    assert!(audit.unmatched_substitutions.is_empty());
    assert!((audit.planned_hours - 8.0).abs() < 1e-9);
    let by_rule: HashMap<String, Uuid> = audit
        .rule_results()
        .into_iter()
        .filter_map(|result| {
            result
                .used
                .first()
                .map(|use_| (result.rule.id.clone(), use_.row_id))
        })
        .collect();
    assert_eq!(by_rule.get("left").copied(), Some(first_id));
    assert_eq!(by_rule.get("right").copied(), Some(second_id));

    plan.terms[0].courses.retain(|course| course.id != first_id);
    let again = DegreeAudit::run(
        &plan,
        &requirements,
        &CourseEquivalences::none(),
        &DegreeCourseAttributes::none(),
    );
    assert_eq!(again.unmatched_substitutions.len(), 1);
    assert_eq!(again.unmatched_substitutions[0].id, left);
    let kept: HashMap<String, Uuid> = again
        .rule_results()
        .into_iter()
        .filter_map(|result| {
            result
                .used
                .first()
                .map(|use_| (result.rule.id.clone(), use_.row_id))
        })
        .collect();
    assert!(!kept.contains_key("left"));
    assert_eq!(kept.get("right").copied(), Some(second_id));
    assert!((again.planned_hours - 4.0).abs() < 1e-9);
}

#[test]
fn count_course_json_keeps_an_optional_row() {
    let row = Uuid::new_v4();
    let change = SubstitutionChange::count_row(key(), row);
    let json = serde_json::to_string(&change).unwrap();
    assert!(json.contains(&row.to_string()));
    let back: SubstitutionChange = serde_json::from_str(&json).unwrap();
    assert_eq!(back, change);

    let plain = SubstitutionChange::count_course(key());
    let plain_json = serde_json::to_string(&plain).unwrap();
    assert!(!plain_json.contains("row"));
    let legacy = r#"{"countCourse":{"subject":"CS","number":"4973"}}"#;
    let old: SubstitutionChange = serde_json::from_str(legacy).unwrap();
    assert_eq!(old, plain);
}

#[test]
fn opening_a_semester_copies_every_row_and_only_real_pins() {
    let pinned = seminar("Seminar", Some("11111"));
    let open = seminar("Seminar", None);
    let pinned_id = pinned.id;
    let open_id = open.id;
    let mut term = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    term.courses = vec![pinned, open];
    let mut data = UserData::default();
    let first = planlogic::schedule_for_semester(&mut data, &term, "202710").unwrap();
    assert!(first.is_new);
    assert_eq!(first.course_count, 2);
    assert_eq!(first.plan.courses.len(), 2);
    assert_eq!(first.plan.courses[0].pinned_crn.as_deref(), Some("11111"));
    assert_eq!(first.plan.courses[0].degree_row_id, Some(pinned_id));
    assert!(first.plan.courses[1].pinned_crn.is_none());
    assert_eq!(first.plan.courses[1].degree_row_id, Some(open_id));
    assert_eq!(first.plan.selected_crns, ["11111".to_string()]);
    let again = planlogic::schedule_for_semester(&mut data, &term, "202710").unwrap();
    assert!(!again.is_new);
    assert_eq!(again.plan.id, first.plan.id);
    assert_eq!(data.plans.len(), 1);
    assert_eq!(again.plan.courses.len(), 2);
}

#[test]
fn each_schedule_row_becomes_one_degree_row() {
    let mut two = SchedulePlan {
        id: Uuid::new_v4(),
        name: "Two".into(),
        term_code: "202710".into(),
        courses: vec![
            schedule_row("CS 4973", Some("11111")),
            schedule_row("CS 4973", Some("22222")),
        ],
        selected_crns: vec!["11111".into(), "22222".into()],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: true,
        created_at: 0.0,
        updated_at: 0.0,
    };
    let draft = planlogic::new_degree_plan_draft("Next", None, &[two.clone()], 2026, 10);
    let fall = draft
        .terms
        .iter()
        .find(|term| term.season == PlanSeason::Fall && term.year == 2026)
        .unwrap();
    assert_eq!(
        fall.courses
            .iter()
            .map(|course| course.crn.clone().unwrap())
            .collect::<Vec<_>>(),
        ["11111", "22222"]
    );

    two.courses = vec![schedule_row("CS 4973", None), schedule_row("CS 4973", None)];
    two.selected_crns.clear();
    let open = planlogic::new_degree_plan_draft("Open", None, &[two], 2026, 10);
    let fall = open
        .terms
        .iter()
        .find(|term| term.season == PlanSeason::Fall && term.year == 2026)
        .unwrap();
    assert_eq!(fall.courses.len(), 2);
    assert!(fall.courses.iter().all(|course| course.crn.is_none()));

    let one = SchedulePlan {
        id: Uuid::new_v4(),
        name: "One".into(),
        term_code: "202710".into(),
        courses: vec![schedule_row("CS 4973", None)],
        selected_crns: vec!["11111".into(), "22222".into()],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: true,
        created_at: 0.0,
        updated_at: 0.0,
    };
    let single = planlogic::new_degree_plan_draft("Single", None, &[one], 2026, 10);
    let fall = single
        .terms
        .iter()
        .find(|term| term.season == PlanSeason::Fall && term.year == 2026)
        .unwrap();
    assert_eq!(fall.courses.len(), 1);
    assert!(fall.courses[0].crn.is_none());
}

#[test]
fn a_pinned_schedule_row_fills_the_open_degree_row() {
    let mut source = plan_with(vec![seminar("Seminar", None)]);
    let schedule = SchedulePlan {
        id: Uuid::new_v4(),
        name: "Pinned".into(),
        term_code: "202710".into(),
        courses: vec![schedule_row("CS 4973", Some("11111"))],
        selected_crns: vec!["11111".into()],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: true,
        created_at: 0.0,
        updated_at: 0.0,
    };
    let draft = planlogic::new_degree_plan_draft("Draft", Some(&source), &[schedule], 2026, 10);
    let fall = draft
        .terms
        .iter()
        .find(|term| term.season == PlanSeason::Fall && term.year == 2026)
        .unwrap();
    assert_eq!(fall.courses.len(), 1);
    assert_eq!(fall.courses[0].crn.as_deref(), Some("11111"));
    source.terms[0].courses[0].crn = Some("11111".into());
    let _ = source;
}

#[test]
fn suggestions_keep_both_degree_rows() {
    let plan = plan_with(vec![
        seminar("Seminar", Some("11111")),
        seminar("Seminar", None),
    ]);
    let term = Term {
        code: "202710".into(),
        banner_description: "Fall 2026 Semester".into(),
    };
    let suggestions =
        planlogic::schedule_suggestions(&term, &[plan], &[], None, &HashSet::new(), None);
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].courses.len(), 2);
    assert_eq!(suggestions[0].crns, vec![Some("11111".into()), None]);
}
