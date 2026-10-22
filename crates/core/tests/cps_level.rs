//! CPS-only detection and the undergrad/grad search band.

use std::collections::HashMap;
use termorganize_core::banner::enrollment_restrictions;
use termorganize_core::catalog::{self, BannerSection};
use termorganize_core::index::CatalogIndex;
use termorganize_core::model::*;
use termorganize_core::search::{self, CourseLevel, SearchFilters, SearchQuery, SearchSort};

const ITC_ONLY: &str = r#"
<section aria-labelledby="restrictions">
    <div class="infoicon"><span class="status-bold">Not all restrictions are applicable.</span></div> <br/>
    <span class="status-bold">Must be enrolled in one of the following Levels:</span><br/>
    <span class="detail-popup-indentation">CPS - Undergraduate Semester (UC)</span><br/>
</section>
"#;

const MIXED_LEVELS: &str = r#"
<section aria-labelledby="restrictions">
    <span class="status-bold">Must be enrolled in one of the following Levels:</span><br/>
    <span class="detail-popup-indentation">Graduate (GR)</span><br/>
    <span class="detail-popup-indentation">CPS - Undergraduate Semester(UC)</span><br/>
    <span class="detail-popup-indentation">Undergraduate(UG)</span><br/>
    <span class="status-bold">Must be enrolled in one of the following Colleges:</span><br/>
    <span class="detail-popup-indentation">Khoury Coll of Comp Sciences(CS)</span><br/>
    <span class="detail-popup-indentation">College of Engineering(EN)</span><br/>
</section>
"#;

const CPS_COLLEGE: &str = r#"
<section aria-labelledby="restrictions">
    <span class="status-bold">Must be enrolled in one of the following Colleges:</span><br/>
    <span class="detail-popup-indentation">College of Professional Studies (CPS)</span><br/>
</section>
"#;

const CPS_COLLEGE_SHORT: &str = r#"
<section aria-labelledby="restrictions">
    <span class="status-bold">Must be enrolled in one of the following Colleges:</span><br/>
    <span class="detail-popup-indentation">Coll of Prof Studies(CPS)</span><br/>
</section>
"#;

const CANNOT_ONLY: &str = r#"
<section aria-labelledby="restrictions">
    <span class="status-bold">Cannot be enrolled in one of the following Levels:</span><br/>
    <span class="detail-popup-indentation">CPS - Undergraduate Semester (UC)</span><br/>
    <span class="status-bold">Cannot be enrolled in one of the following Colleges:</span><br/>
    <span class="detail-popup-indentation">College of Professional Studies (CPS)</span><br/>
</section>
"#;

#[test]
fn cps_detection_uses_must_lists_only() {
    let only = enrollment_restrictions(ITC_ONLY).unwrap();
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].kind, RestrictionKind::Must);
    assert_eq!(only[0].category, "Levels");
    assert_eq!(only[0].values, ["CPS - Undergraduate Semester (UC)"]);
    assert!(is_cps_only(&only));

    let mixed = enrollment_restrictions(MIXED_LEVELS).unwrap();
    assert!(mixed
        .iter()
        .any(|item| item.is_levels() && item.values.len() == 3));
    assert!(!is_cps_only(&mixed));

    assert!(is_cps_only(&enrollment_restrictions(CPS_COLLEGE).unwrap()));
    assert!(is_cps_only(
        &enrollment_restrictions(CPS_COLLEGE_SHORT).unwrap()
    ));
    let cannot = enrollment_restrictions(CANNOT_ONLY).unwrap();
    assert!(cannot
        .iter()
        .all(|item| item.kind == RestrictionKind::Cannot));
    assert!(!is_cps_only(&cannot));
    assert!(enrollment_restrictions("<p>Banner is down</p>").is_none());
}

#[test]
fn course_note_is_all_some_or_nothing() {
    let cps = Some(vec![EnrollmentRestriction {
        kind: RestrictionKind::Must,
        category: "Levels".into(),
        values: vec!["CPS - Undergraduate Semester (UC)".into()],
    }]);
    let open = Some(vec![EnrollmentRestriction {
        kind: RestrictionKind::Must,
        category: "Levels".into(),
        values: vec![
            "Graduate (GR)".into(),
            "CPS - Undergraduate Semester(UC)".into(),
            "Undergraduate(UG)".into(),
        ],
    }]);
    let both = [section("1", cps.clone()), section("2", cps.clone())];
    assert_eq!(
        cps_note(both.iter().map(|s| s.cps_only())),
        Some(CPS_ONLY_NOTE)
    );
    assert_eq!(
        course_cps_only(both.iter().map(|s| s.cps_only())),
        Some(true)
    );
    assert_eq!(both[0].cps_only(), Some(true));

    let mixed = [section("1", cps.clone()), section("2", open.clone())];
    assert_eq!(
        cps_note(mixed.iter().map(|s| s.cps_only())),
        Some(CPS_SOME_NOTE)
    );
    assert_eq!(
        course_cps_only(mixed.iter().map(|s| s.cps_only())),
        Some(false)
    );
    assert_eq!(mixed[1].cps_only(), Some(false));

    let partial = [section("1", cps), section("2", None)];
    assert_eq!(
        cps_note(partial.iter().map(|s| s.cps_only())),
        Some(CPS_ONLY_NOTE)
    );
    assert_eq!(partial[1].cps_only(), None);
    assert_eq!(
        course_cps_only(partial.iter().map(|s| s.cps_only())),
        Some(true)
    );

    let unknown = [section("1", None), section("2", None)];
    assert_eq!(cps_note(unknown.iter().map(|s| s.cps_only())), None);
    assert_eq!(course_cps_only(unknown.iter().map(|s| s.cps_only())), None);

    let none_cps = [section("1", open)];
    assert_eq!(cps_note(none_cps.iter().map(|s| s.cps_only())), None);
    assert_eq!(
        course_cps_only(none_cps.iter().map(|s| s.cps_only())),
        Some(false)
    );
}

#[test]
fn catalog_refresh_keeps_cached_restrictions() {
    let restrictions = enrollment_restrictions(ITC_ONLY).unwrap();
    let mut cached = section("34087", Some(restrictions));
    cached.subject = "ITC".into();
    cached.number = "1000".into();
    let existing = TermCatalog {
        version: 1,
        term: term(),
        generated_at: 1.0,
        sections_updated_at: 1.0,
        courses: vec![course("ITC", "1000", true, None, None)],
        sections: vec![cached],
        subjects: vec![],
        attributes: vec![],
        campuses: vec![],
    };
    let fresh = BannerSection {
        course_reference_number: "34087".into(),
        subject: "ITC".into(),
        course_number: "1000".into(),
        course_title: Some("Computer Applications".into()),
        credit_hours: Some(4.0),
        ..BannerSection::default()
    };
    let merged = catalog::merge(
        Some(&existing),
        &[fresh],
        term(),
        None,
        None,
        None,
        &HashMap::new(),
        2.0,
    );
    let kept = merged
        .sections
        .iter()
        .find(|section| section.crn == "34087")
        .unwrap();
    assert_eq!(kept.cps_only(), Some(true));
}

#[test]
fn level_filter_splits_at_5000_and_combines_with_requisites() {
    let under = course("CS", "4999", true, None, None);
    let grad = course("CS", "5000", true, None, None);
    let under_prereq = course(
        "CS",
        "2500",
        true,
        Some(Requisite::Course(CourseKey {
            subject: "CS".into(),
            number: "1800".into(),
        })),
        None,
    );
    let unknown = course("CS", "1800", false, None, None);
    let index = CatalogIndex::build(TermCatalog {
        version: 1,
        term: term(),
        generated_at: 0.0,
        sections_updated_at: 0.0,
        courses: vec![under.clone(), grad.clone(), under_prereq, unknown],
        sections: vec![],
        subjects: vec![],
        attributes: vec![],
        campuses: vec![],
    });

    let undergrad = SearchFilters {
        course_level: CourseLevel::Undergrad,
        ..SearchFilters::default()
    };
    assert_eq!(undergrad.active_count(), 1);
    assert_eq!(
        codes(&search::search(&query(undergrad), &index)),
        vec!["CS 1800", "CS 2500", "CS 4999"]
    );

    let grad_filter = SearchFilters {
        course_level: CourseLevel::Grad,
        ..SearchFilters::default()
    };
    assert_eq!(
        codes(&search::search(&query(grad_filter), &index)),
        vec!["CS 5000"]
    );

    let both = SearchFilters {
        course_level: CourseLevel::Undergrad,
        no_prerequisites: true,
        ..SearchFilters::default()
    };
    assert_eq!(both.active_count(), 2);
    assert_eq!(
        codes(&search::search(&query(both), &index)),
        vec!["CS 4999"]
    );

    let grad_open = SearchFilters {
        course_level: CourseLevel::Grad,
        no_prerequisites: true,
        no_corequisites: true,
        ..SearchFilters::default()
    };
    assert_eq!(
        codes(&search::search(&query(grad_open), &index)),
        vec!["CS 5000"]
    );
    assert_eq!(SearchFilters::default().active_count(), 0);
    assert!(SearchFilters::default().matches_level(&under));
    assert!(!SearchFilters {
        course_level: CourseLevel::Grad,
        ..SearchFilters::default()
    }
    .matches_level(&under));
    assert!(SearchFilters {
        course_level: CourseLevel::Grad,
        ..SearchFilters::default()
    }
    .matches_level(&grad));
}

fn term() -> Term {
    Term {
        code: "202730".into(),
        banner_description: "Spring 2027 Semester".into(),
    }
}

fn query(filters: SearchFilters) -> SearchQuery {
    SearchQuery {
        text: String::new(),
        filters,
        sort: SearchSort::CourseNumber,
    }
}

fn codes(response: &search::SearchResponse) -> Vec<String> {
    response
        .results
        .iter()
        .map(|result| result.course.code())
        .collect()
}

fn course(
    subject: &str,
    number: &str,
    loaded: bool,
    prereqs: Option<Requisite>,
    coreqs: Option<Requisite>,
) -> Course {
    Course {
        subject: subject.into(),
        number: number.into(),
        title: format!("{subject} {number}"),
        description: String::new(),
        min_credits: 4.0,
        max_credits: 4.0,
        credit_hours: Some(4.0),
        attributes: vec![],
        prereqs,
        coreqs,
        postreqs: vec![],
        special_topics: false,
        college: None,
        department: None,
        details_loaded: loaded,
    }
}

fn section(crn: &str, restrictions: Option<Vec<EnrollmentRestriction>>) -> CourseSection {
    CourseSection {
        crn: crn.into(),
        subject: "CS".into(),
        number: "1000".into(),
        title: String::new(),
        section_number: "01".into(),
        part_of_term: "1".into(),
        seat_capacity: 20,
        seats_remaining: 5,
        enrollment: 15,
        waitlist_capacity: 0,
        waitlist_count: 0,
        waitlist_remaining: 0,
        class_type: "Lecture".into(),
        honors: false,
        campus: "Boston".into(),
        instruction_method: None,
        meetings: vec![],
        instructors: None,
        cross_list_crns: vec![],
        attributes: vec![],
        description: None,
        prereqs: None,
        coreqs: None,
        credit_hours: None,
        credit_hour_low: None,
        credit_hour_high: None,
        restrictions,
    }
}
