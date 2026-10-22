//! The degree plan editor can place the same subject and number on one term twice
//! when each row has its own manually picked CRN. The screen calls these methods.

use termorganize_core::editing;
use termorganize_core::model::*;
use termorganize_core::planlogic;

static DIRECTED: &str = "CS 4990";

#[test]
fn the_editor_adds_the_same_course_twice_on_one_term_when_each_row_has_a_crn() {
    let fall = term(PlanSeason::Fall, 2026);
    let fall_id = fall.id;
    let mut plan = DegreePlan {
        name: "Topics".into(),
        terms: vec![fall, term(PlanSeason::Spring, 2027)],
        ..DegreePlan::default()
    };

    assert!(editing::block_reason(&plan, &key(), Some("11111"), None).is_none());
    assert!(editing::try_add(row(Some("11111")), fall_id, &mut plan));
    let first = plan.terms[0].courses[0].id;
    assert!(editing::set_selected_crn(first, Some("11111"), &mut plan));

    assert_eq!(editing::term_with(&key(), &plan), Some(fall_id));
    assert!(editing::block_reason(&plan, &key(), Some("22222"), None).is_none());
    assert!(editing::try_add(row(Some("22222")), fall_id, &mut plan));

    let rows: Vec<_> = plan.terms[0]
        .courses
        .iter()
        .map(|course| course.crn.clone())
        .collect();
    assert_eq!(rows, vec![Some("11111".into()), Some("22222".into())]);
    assert!((plan.terms[0].credits() - 8.0).abs() < 1e-9);
    assert!(plan.terms[1].courses.is_empty());
    assert!(planlogic::duplicate_keys(&plan).is_empty());
}

#[test]
fn a_second_row_without_its_own_crn_is_refused() {
    let fall = term(PlanSeason::Fall, 2026);
    let fall_id = fall.id;
    let mut plan = DegreePlan {
        terms: vec![fall],
        ..DegreePlan::default()
    };
    assert!(editing::try_add(row(None), fall_id, &mut plan));

    assert_eq!(
        editing::block_reason(&plan, &key(), Some("  "), None).as_deref(),
        Some("CS 4990 is already on this plan. A second listing needs its own CRN.")
    );
    assert!(!editing::try_add(row(None), fall_id, &mut plan));
    assert!(!editing::try_add(row(Some("22222")), fall_id, &mut plan));
    assert_eq!(
        editing::block_reason(&plan, &key(), Some("22222"), None).as_deref(),
        Some(
            "CS 4990 is already on this plan. Set a CRN on each existing listing before adding another."
        )
    );
    assert_eq!(plan.terms[0].courses.len(), 1);
}

#[test]
fn the_same_crn_cannot_be_saved_twice() {
    let fall = term(PlanSeason::Fall, 2026);
    let fall_id = fall.id;
    let mut plan = DegreePlan {
        terms: vec![fall],
        ..DegreePlan::default()
    };
    assert!(editing::try_add(row(Some("11111")), fall_id, &mut plan));
    let first = plan.terms[0].courses[0].id;

    assert!(editing::block_reason(&plan, &key(), Some("22222"), None).is_none());
    assert!(editing::try_add(row(Some("22222")), fall_id, &mut plan));
    assert_eq!(
        editing::block_reason(&plan, &key(), Some(" 22222 "), None).as_deref(),
        Some("CS 4990 already uses CRN 22222.")
    );
    assert!(!editing::try_add(row(Some("22222")), fall_id, &mut plan));
    assert!(!editing::set_selected_crn(first, Some("22222"), &mut plan));
    assert_eq!(plan.terms[0].courses[0].crn.as_deref(), Some("11111"));
    assert_eq!(plan.terms[0].courses.len(), 2);
}

#[test]
fn the_only_row_can_take_or_clear_a_crn() {
    let fall = term(PlanSeason::Fall, 2026);
    let fall_id = fall.id;
    let mut plan = DegreePlan {
        terms: vec![fall],
        ..DegreePlan::default()
    };
    assert!(editing::try_add(row(None), fall_id, &mut plan));
    let id = plan.terms[0].courses[0].id;

    assert!(editing::set_selected_crn(id, Some(" 11111 "), &mut plan));
    assert_eq!(plan.terms[0].courses[0].crn.as_deref(), Some("11111"));
    assert!(editing::set_selected_crn(id, Some(""), &mut plan));
    assert!(plan.terms[0].courses[0].crn.is_none());
    assert!(!editing::set_selected_crn(
        uuid::Uuid::new_v4(),
        Some("33333"),
        &mut plan
    ));
}

fn key() -> CourseKey {
    CourseKey::parse(DIRECTED).unwrap()
}

fn row(crn: Option<&str>) -> DegreePlanCourse {
    let mut course = DegreePlanCourse::catalog(key(), "Directed Study", 4.0);
    course.crn = crn.map(str::to_string);
    course
}

fn term(season: PlanSeason, year: i32) -> DegreePlanTerm {
    DegreePlanTerm::new(season, year)
}
