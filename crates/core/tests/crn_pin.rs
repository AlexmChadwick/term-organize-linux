//! A chosen CRN stays the section. Special topics share a course number, so a
//! section is the term plus the CRN. A degree plan may list the same subject
//! and number more than once; a schedule still needs a distinct CRN for each copy.

use std::collections::HashSet;
use termorganize_core::catalog::{self, BannerAttribute, BannerSection};
use termorganize_core::degree_audit::{AuditStatus, DegreeAudit, DegreeCourseAttributes};
use termorganize_core::index::CatalogIndex;
use termorganize_core::model::*;
use termorganize_core::planlogic::{self, repeated_course_allowed};
use termorganize_core::program_parser;
use termorganize_core::reorder::{self, Move, Suggestion};
use termorganize_core::schedule::{self, Limits, ScheduleSort};
use termorganize_core::CourseEquivalences;
use uuid::Uuid;

fn index(courses: serde_json::Value, sections: serde_json::Value) -> CatalogIndex {
    let value = serde_json::json!({
        "version": 1,
        "term": { "code": "202710", "bannerDescription": "Fall 2026 Semester" },
        "courses": courses,
        "sections": sections,
        "subjects": [],
        "attributes": [],
        "campuses": []
    });
    CatalogIndex::build(serde_json::from_value(value).unwrap())
}

fn course(
    subject: &str,
    number: &str,
    title: &str,
    credits: f64,
    topics: bool,
) -> serde_json::Value {
    serde_json::json!({
        "subject": subject,
        "number": number,
        "title": title,
        "minCredits": credits,
        "maxCredits": credits,
        "specialTopics": topics
    })
}

fn section(crn: &str, subject: &str, number: &str, start: i32, end: i32) -> serde_json::Value {
    serde_json::json!({
        "crn": crn,
        "subject": subject,
        "number": number,
        "sectionNumber": "01",
        "meetings": [{ "days": 2, "start": start, "end": end }]
    })
}

fn plan_with(keys: &[&str], selected: &[&str]) -> SchedulePlan {
    let mut plan = SchedulePlan {
        id: Uuid::new_v4(),
        name: "Plan".into(),
        term_code: "202710".into(),
        courses: vec![],
        selected_crns: selected.iter().map(|crn| (*crn).to_string()).collect(),
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: false,
        created_at: 0.0,
        updated_at: 0.0,
    };
    for (offset, code) in keys.iter().enumerate() {
        plan.courses.push(PlanCourse {
            key: CourseKey::parse(code).unwrap(),
            is_required: true,
            excluded_crns: HashSet::new(),
            color_index: offset as i32,
            pinned_crn: None,
            degree_row_id: None,
        });
    }
    plan
}

fn crns_of(result: &schedule::GenerationResult) -> Vec<Vec<String>> {
    result
        .schedules
        .iter()
        .map(|schedule| schedule.crns.clone())
        .collect()
}

#[test]
fn generation_keeps_a_pinned_crn() {
    let catalog = index(
        serde_json::json!([
            course("CS", "2500", "Fundamentals 1", 4.0, false),
            course("CS", "2510", "Fundamentals 2", 4.0, false)
        ]),
        serde_json::json!([
            section("11111", "CS", "2500", 540, 600),
            section("22222", "CS", "2500", 660, 720),
            section("33333", "CS", "2510", 780, 840)
        ]),
    );
    let plan = plan_with(&["CS 2500", "CS 2510"], &["11111", "33333"]);
    let inputs = schedule::inputs_for(&plan, &catalog);
    assert_eq!(
        inputs[0]
            .sections
            .iter()
            .map(|section| section.crn.as_str())
            .collect::<Vec<_>>(),
        vec!["11111"]
    );
    let result = schedule::generate(
        &inputs,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        ScheduleSort::Best,
        &Limits::default(),
    );
    assert!(!result.schedules.is_empty());
    for schedule in crns_of(&result) {
        assert!(schedule.iter().any(|crn| crn == "11111"));
        assert!(schedule.iter().all(|crn| crn != "22222"));
    }
}

#[test]
fn special_topics_that_share_a_number_are_not_merged() {
    let catalog = index(
        serde_json::json!([course("CS", "5976", "Special Topics", 4.0, true)]),
        serde_json::json!([
            section("44444", "CS", "5976", 540, 600),
            section("55555", "CS", "5976", 660, 720)
        ]),
    );
    let plan = plan_with(&["CS 5976"], &["44444", "55555"]);
    let resolved = schedule::ResolvedPlan::resolve(&plan, Some(&catalog));
    assert_eq!(resolved.entries.len(), 1);
    assert_eq!(resolved.entries[0].selected.len(), 2);
    assert!((resolved.credits_min - 8.0).abs() < 1e-9);
    let inputs = schedule::inputs_for(&plan, &catalog);
    assert_eq!(inputs.len(), 2, "each pinned topic is its own course");
    let result = schedule::generate(
        &inputs,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        ScheduleSort::Best,
        &Limits::default(),
    );
    assert!(!result.schedules.is_empty());
    for schedule in crns_of(&result) {
        let set: HashSet<&str> = schedule.iter().map(String::as_str).collect();
        assert_eq!(set, HashSet::from(["44444", "55555"]));
    }
}

#[test]
fn two_rows_claim_distinct_crns() {
    let catalog = index(
        serde_json::json!([course("CS", "5976", "Special Topics", 4.0, true)]),
        serde_json::json!([
            section("44444", "CS", "5976", 540, 600),
            section("55555", "CS", "5976", 660, 720)
        ]),
    );
    let mut plan = plan_with(&["CS 5976", "CS 5976"], &["44444", "55555"]);
    plan.courses[0].pinned_crn = Some("44444".into());
    plan.courses[1].pinned_crn = Some("55555".into());
    let resolved = schedule::ResolvedPlan::resolve(&plan, Some(&catalog));
    assert_eq!(resolved.entries[0].selected[0].crn, "44444");
    assert_eq!(resolved.entries[1].selected[0].crn, "55555");
    assert!((resolved.credits_min - 8.0).abs() < 1e-9);
}

#[test]
fn removing_one_topic_leaves_the_other_pin() {
    let catalog = index(
        serde_json::json!([course("CS", "5976", "Special Topics", 4.0, true)]),
        serde_json::json!([
            section("44444", "CS", "5976", 540, 600),
            section("55555", "CS", "5976", 660, 720)
        ]),
    );
    let mut data = UserData::default();
    let plan = schedule::create_plan(&mut data, "Plan", "202710");
    schedule::add_course(
        &mut data,
        plan.id,
        CourseKey::parse("CS 5976").unwrap(),
        true,
    );
    schedule::update_plan(&mut data, plan.id, |plan| {
        plan.courses[0].pinned_crn = Some("44444".into());
        plan.courses.push(PlanCourse {
            key: CourseKey::parse("CS 5976").unwrap(),
            is_required: true,
            excluded_crns: HashSet::new(),
            color_index: 1,
            pinned_crn: Some("55555".into()),
            degree_row_id: None,
        });
        plan.selected_crns = vec!["44444".into(), "55555".into()];
    });
    schedule::remove_plan_course(
        &mut data,
        plan.id,
        &CourseKey::parse("CS 5976").unwrap(),
        Some(&catalog),
    );
    let after = schedule::plan_by_id(&data, plan.id).unwrap();
    assert_eq!(after.courses.len(), 1);
    assert_eq!(after.courses[0].pinned_crn.as_deref(), Some("55555"));
    assert_eq!(after.selected_crns, vec!["55555".to_string()]);
}

#[test]
fn duplicate_course_numbers_require_distinct_crns() {
    let key = CourseKey::parse("CS 5976").unwrap();
    assert!(repeated_course_allowed(Vec::<Option<&str>>::new(), None));
    assert!(!repeated_course_allowed([None], None));
    assert!(!repeated_course_allowed([Some("44444")], None));
    assert!(!repeated_course_allowed([Some("44444")], Some("44444")));
    assert!(repeated_course_allowed([Some("44444")], Some("55555")));
    assert!(!repeated_course_allowed(
        [None, Some("44444")],
        Some("55555")
    ));

    let mut allowed = DegreePlan::default();
    let mut term = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    let mut first = DegreePlanCourse::catalog(key.clone(), "Directed Study", 4.0);
    first.crn = Some("44444".into());
    let mut second = DegreePlanCourse::catalog(key.clone(), "Directed Study", 4.0);
    second.crn = Some("55555".into());
    term.courses = vec![first, second];
    allowed.terms = vec![term];
    assert!(planlogic::duplicate_keys(&allowed).is_empty());
    assert!((allowed.planned_credits() - 8.0).abs() < 1e-9);

    let mut blocked = allowed.clone();
    blocked.terms[0].courses[1].crn = Some("44444".into());
    assert_eq!(
        planlogic::duplicate_keys(&blocked),
        HashSet::from([key.clone()])
    );

    let mut unpinned = allowed.clone();
    unpinned.terms[0].courses[0].crn = None;
    unpinned.terms[0].courses[1].crn = None;
    assert_eq!(
        planlogic::duplicate_keys(&unpinned),
        HashSet::from([key.clone()])
    );
}

#[test]
fn audit_nupath_and_credits_count_each_pinned_copy() {
    let page = r#"<div id="programrequirementstextcontainer" class="tab_content">
        <h2>Major Requirements</h2>
        <table class="sc_courselist"><tbody>
        <tr class="even"><td colspan="2"><span class="courselistcomment">Complete 8 semester hours of the following:</span></td><td class="hourscol">8</td></tr>
        <tr class="odd lastrow"><td class="codecol"><a class="bubblelink code">CS&#160;5976</a></td><td>Directed Study</td><td class="hourscol">4</td></tr>
        </tbody></table>
        <p>Complete the NUpath requirements.</p>
        </div>"#;
    let requirements = program_parser::requirements_from_html(
        page,
        CatalogYear::new(2025),
        CatalogProgram {
            title: "Test".into(),
            path: "test".into(),
        },
    )
    .expect("requirements");
    let key = CourseKey::parse("CS 5976").unwrap();
    let mut plan = DegreePlan {
        name: "Topics".into(),
        ..DegreePlan::default()
    };
    let mut term = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    let mut first = DegreePlanCourse::catalog(key.clone(), "Directed Study", 4.0);
    first.crn = Some("44444".into());
    let mut second = DegreePlanCourse::catalog(key.clone(), "Directed Study", 4.0);
    second.crn = Some("55555".into());
    term.courses = vec![first.clone(), second];
    plan.terms = vec![term];
    let mut attributes = DegreeCourseAttributes::none();
    attributes
        .nupaths
        .insert(key.clone(), HashSet::from([NuPath::Wi]));
    let audit = DegreeAudit::run(
        &plan,
        &requirements,
        &CourseEquivalences::none(),
        &attributes,
    );
    assert!((audit.planned_hours - 8.0).abs() < 1e-9);
    let nupath = audit
        .note_checks
        .iter()
        .find(|check| {
            matches!(
                check.kind,
                termorganize_core::degree_audit::NoteKind::Nupath
            )
        })
        .expect("nupath");
    let writing = nupath
        .items
        .iter()
        .find(|item| item.title.contains("Writing Intensive"))
        .expect("writing intensive");
    assert_eq!(writing.status, Some(AuditStatus::Planned));
    assert_eq!(writing.courses.len(), 2);

    let mut once = plan.clone();
    once.terms[0].courses[1].crn = None;
    once.terms[0].courses[0].crn = None;
    let merged = DegreeAudit::run(
        &once,
        &requirements,
        &CourseEquivalences::none(),
        &attributes,
    );
    assert!(
        (merged.planned_hours - 8.0).abs() < 1e-9,
        "each copy still counts, including a repeat with no CRN"
    );
    let mut ids: Vec<String> = merged
        .rule_results()
        .into_iter()
        .flat_map(|result| result.used.iter())
        .chain(merged.unused.iter())
        .map(|use_| use_.id())
        .filter(|id| id.starts_with("CS 5976"))
        .collect();
    ids.sort();
    assert_eq!(ids, ["CS 5976", "CS 5976 (2)"]);
    let _ = first;
}

#[test]
fn reordering_a_pinned_course_keeps_its_crn() {
    let key = CourseKey::parse("CS 5976").unwrap();
    let mut first = DegreePlanCourse::catalog(key.clone(), "Networks", 4.0);
    first.crn = Some("44444".into());
    let mut second = DegreePlanCourse::catalog(key.clone(), "Graphics", 4.0);
    second.crn = Some("55555".into());
    let fall = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    let spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    let mut plan = DegreePlan {
        terms: vec![
            DegreePlanTerm {
                courses: vec![first, second],
                ..fall
            },
            spring.clone(),
        ],
        ..DegreePlan::default()
    };
    let from = plan.terms[0].id;
    let to = plan.terms[1].id;
    reorder::apply(
        &Suggestion {
            id: "move".into(),
            moves: vec![Move {
                course: key.clone(),
                from,
                to,
            }],
            title: "Move CS 5976".into(),
            reasons: vec![],
            adds_semester: None,
        },
        &mut plan,
    );
    assert_eq!(plan.terms[0].courses.len(), 1);
    assert_eq!(plan.terms[1].courses.len(), 1);
    let crns: HashSet<_> = plan
        .terms
        .iter()
        .flat_map(|term| term.courses.iter())
        .map(|course| course.crn.clone().unwrap())
        .collect();
    assert_eq!(crns, HashSet::from(["44444".into(), "55555".into()]));

    let mut same = DegreePlanCourse::catalog(key.clone(), "Directed Study", 4.0);
    same.crn = None;
    plan.terms[1].courses.push(same);
    reorder::apply(
        &Suggestion {
            id: "again".into(),
            moves: vec![Move {
                course: key,
                from: plan.terms[0].id,
                to: plan.terms[1].id,
            }],
            title: "Move CS 5976".into(),
            reasons: vec![],
            adds_semester: None,
        },
        &mut plan,
    );
    assert!(plan.terms[0].courses.is_empty());
    assert_eq!(plan.terms[1].courses.len(), 3);
    let crns: Vec<_> = plan.terms[1]
        .courses
        .iter()
        .map(|course| course.crn.clone())
        .collect();
    assert_eq!(crns, vec![Some("44444".into()), None, Some("55555".into())]);
    let _ = spring;
}

#[test]
fn refreshing_a_topics_number_does_not_drop_the_other_crn() {
    let mut catalog = index(
        serde_json::json!([course("CS", "5976", "Special Topics", 4.0, true)]),
        serde_json::json!([
            section("44444", "CS", "5976", 540, 600),
            section("55555", "CS", "5976", 660, 720)
        ]),
    )
    .catalog;
    let key = CourseKey::parse("CS 5976").unwrap();
    catalog::apply_course_refresh(
        &mut catalog,
        &key,
        &[BannerSection {
            course_reference_number: "44444".into(),
            course_number: "5976".into(),
            subject: "CS".into(),
            course_title: Some("Special Topics".into()),
            section_attributes: Some(vec![BannerAttribute {
                code: "TOPC".into(),
                description: Some("Special Topics".into()),
            }]),
            ..BannerSection::default()
        }],
        1.0,
    );
    let crns: HashSet<_> = catalog
        .sections
        .iter()
        .map(|section| section.crn.as_str())
        .collect();
    assert_eq!(crns, HashSet::from(["44444", "55555"]));
}

#[test]
fn refreshing_a_normal_course_still_replaces_its_sections() {
    let mut catalog = index(
        serde_json::json!([course("CS", "2500", "Fundamentals 1", 4.0, false)]),
        serde_json::json!([
            section("11111", "CS", "2500", 540, 600),
            section("22222", "CS", "2500", 660, 720)
        ]),
    )
    .catalog;
    let key = CourseKey::parse("CS 2500").unwrap();
    catalog::apply_course_refresh(
        &mut catalog,
        &key,
        &[BannerSection {
            course_reference_number: "11111".into(),
            course_number: "2500".into(),
            subject: "CS".into(),
            course_title: Some("Fundamentals of Computer Science 1".into()),
            ..BannerSection::default()
        }],
        1.0,
    );
    let crns: HashSet<_> = catalog
        .sections
        .iter()
        .map(|section| section.crn.as_str())
        .collect();
    assert_eq!(crns, HashSet::from(["11111"]));
}

#[test]
fn autofill_keeps_each_pinned_topic() {
    let catalog = index(
        serde_json::json!([course("CS", "5976", "Special Topics", 4.0, true)]),
        serde_json::json!([
            section("44444", "CS", "5976", 540, 600),
            section("55555", "CS", "5976", 660, 720)
        ]),
    );
    let mut plan = DegreePlan {
        name: "Degree".into(),
        ..DegreePlan::default()
    };
    let mut term = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    let mut first =
        DegreePlanCourse::catalog(CourseKey::parse("CS 5976").unwrap(), "Networks", 4.0);
    first.crn = Some("44444".into());
    let mut second =
        DegreePlanCourse::catalog(CourseKey::parse("CS 5976").unwrap(), "Graphics", 4.0);
    second.crn = Some("55555".into());
    term.courses = vec![first, second];
    plan.terms = vec![term];
    let suggestions = planlogic::schedule_suggestions(
        &catalog.catalog.term,
        &[plan],
        &[],
        None,
        &HashSet::new(),
        Some(&catalog),
    );
    assert_eq!(suggestions[0].courses.len(), 2);
    assert_eq!(
        suggestions[0].crns,
        vec![Some("44444".into()), Some("55555".into())]
    );

    let mut plain = DegreePlan {
        name: "Plain".into(),
        ..DegreePlan::default()
    };
    let mut term = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    term.courses = vec![
        DegreePlanCourse::catalog(CourseKey::parse("CS 2500").unwrap(), "Fundamentals", 4.0),
        DegreePlanCourse::catalog(CourseKey::parse("CS 2500").unwrap(), "Fundamentals", 4.0),
    ];
    plain.terms = vec![term];
    let catalog = index(
        serde_json::json!([course("CS", "2500", "Fundamentals 1", 4.0, false)]),
        serde_json::json!([section("11111", "CS", "2500", 540, 600)]),
    );
    let suggestions = planlogic::schedule_suggestions(
        &catalog.catalog.term,
        &[plain],
        &[],
        None,
        &HashSet::new(),
        Some(&catalog),
    );
    assert_eq!(suggestions[0].courses.len(), 2);
    assert_eq!(suggestions[0].crns, vec![None, None]);
}

#[test]
fn a_missing_pinned_crn_stays_locked() {
    let catalog = index(
        serde_json::json!([
            course(
                "CS",
                "2500",
                "Fundamentals of Computer Science 1",
                4.0,
                false
            ),
            course("MATH", "1341", "Calculus 1", 4.0, false),
        ]),
        serde_json::json!([
            section("11111", "CS", "2500", 540, 600),
            section("22222", "CS", "2500", 660, 720),
            section("33333", "MATH", "1341", 780, 840),
        ]),
    );
    let mut plan = plan_with(&["CS 2500"], &["99999"]);
    plan.courses[0].pinned_crn = Some("99999".into());
    let inputs = schedule::inputs_for(&plan, &catalog);
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].sections.is_empty());
    assert_eq!(inputs[0].unavailable_pin.as_deref(), Some("99999"));
    let result = schedule::generate(
        &inputs,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        ScheduleSort::Best,
        &Limits::default(),
    );
    assert!(result.schedules.is_empty());
    let reasons = schedule::no_results_reasons(&result.diagnostics, "Fall 2026");
    assert_eq!(reasons[0].title, "CS 2500 is unavailable");
    assert_eq!(
        reasons[0].detail.as_deref(),
        Some(
            "CRN 99999 is no longer in the Fall 2026 catalog. Another section was not substituted."
        )
    );
    let analysis = schedule::analyze_blockers(
        &inputs,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        Some(&result.diagnostics),
        1.0,
        4,
    );
    assert_eq!(
        analysis.unavailable,
        vec![CourseKey::parse("CS 2500").unwrap()]
    );
    assert!(analysis.not_offered.is_empty());

    let mut saved = plan_with(&["CS 2500"], &["99999"]);
    let inputs = schedule::inputs_for(&saved, &catalog);
    assert!(inputs[0].sections.is_empty());
    assert_eq!(inputs[0].unavailable_pin.as_deref(), Some("99999"));

    saved.courses[0].pinned_crn = Some("33333".into());
    let inputs = schedule::inputs_for(&saved, &catalog);
    assert!(inputs[0].sections.is_empty());
    assert_eq!(inputs[0].unavailable_pin.as_deref(), Some("33333"));
}
