//! "No prerequisites" and "No corequisites" keep a loaded course whose requisite was omitted.
//! A course whose details were not loaded is unknown, so the filter leaves it out.

use termorganize_core::index::CatalogIndex;
use termorganize_core::model::*;
use termorganize_core::search::{self, SearchFilters, SearchQuery, SearchSort};

#[test]
fn none_is_a_loaded_course_with_the_requisite_omitted() {
    let open = course("CS", "1100", true, None, None);
    let prereq = course(
        "CS",
        "2500",
        true,
        Some(Requisite::Course(CourseKey {
            subject: "CS".into(),
            number: "1800".into(),
        })),
        None,
    );
    let coreq = course(
        "CS",
        "2501",
        true,
        None,
        Some(Requisite::Course(CourseKey {
            subject: "CS".into(),
            number: "2500".into(),
        })),
    );
    let test_only = course(
        "CS",
        "1800",
        true,
        Some(Requisite::Test {
            name: "Math Placement".into(),
            score: Some(30.0),
        }),
        None,
    );
    let unknown = course("CS", "9999", false, None, None);
    let empty_group = course("CS", "1000", true, Some(Requisite::And(vec![])), None);
    let unknown_group = course("CS", "1001", false, Some(Requisite::And(vec![])), None);

    assert!(open.has_no_prerequisites());
    assert!(open.has_no_corequisites());
    assert!(!prereq.has_no_prerequisites());
    assert!(prereq.has_no_corequisites());
    assert!(coreq.has_no_prerequisites());
    assert!(!coreq.has_no_corequisites());
    assert!(!test_only.has_no_prerequisites());
    assert!(!unknown.has_no_prerequisites());
    assert!(!unknown.has_no_corequisites());
    // An empty group simplifies away, so a loaded course still counts as none.
    assert!(empty_group.has_no_prerequisites());
    // The same empty group on a stub is still unknown.
    assert!(!unknown_group.has_no_prerequisites());
    assert!(!unknown_group.has_no_corequisites());

    let index = CatalogIndex::build(TermCatalog {
        version: 1,
        term: Term {
            code: "202710".into(),
            banner_description: "Fall 2026 Semester".into(),
        },
        generated_at: 0.0,
        sections_updated_at: 0.0,
        courses: vec![
            open.clone(),
            prereq.clone(),
            coreq.clone(),
            test_only.clone(),
            unknown.clone(),
        ],
        sections: vec![
            section("10001", &open),
            section("10002", &prereq),
            section("10003", &coreq),
            section("10004", &test_only),
            section("10005", &unknown),
        ],
        subjects: vec![],
        attributes: vec![],
        campuses: vec![],
    });

    let no_prereqs = SearchFilters {
        no_prerequisites: true,
        ..SearchFilters::default()
    };
    let found = search::search(&query(no_prereqs.clone()), &index);
    assert_eq!(codes(&found), vec!["CS 1100", "CS 2501"]);
    assert_eq!(no_prereqs.active_count(), 1);

    let no_coreqs = SearchFilters {
        no_corequisites: true,
        ..SearchFilters::default()
    };
    let found = search::search(&query(no_coreqs), &index);
    assert_eq!(codes(&found), vec!["CS 1100", "CS 1800", "CS 2500"]);

    let both_filters = SearchFilters {
        no_prerequisites: true,
        no_corequisites: true,
        ..SearchFilters::default()
    };
    let found = search::search(&query(both_filters.clone()), &index);
    assert_eq!(codes(&found), vec!["CS 1100"]);
    assert_eq!(both_filters.active_count(), 2);

    let untouched = search::search(&query(SearchFilters::default()), &index);
    assert_eq!(
        codes(&untouched),
        vec!["CS 1100", "CS 1800", "CS 2500", "CS 2501", "CS 9999"]
    );
}

fn query(filters: SearchFilters) -> SearchQuery {
    SearchQuery {
        text: String::new(),
        filters,
        sort: SearchSort::Relevance,
    }
}

fn codes(response: &search::SearchResponse) -> Vec<String> {
    let mut codes: Vec<String> = response
        .results
        .iter()
        .map(|result| result.course.code())
        .collect();
    codes.sort();
    codes
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
        credit_hours: None,
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

fn section(crn: &str, course: &Course) -> CourseSection {
    CourseSection {
        crn: crn.into(),
        subject: course.subject.clone(),
        number: course.number.clone(),
        title: course.title.clone(),
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
        instruction_method: Some("Traditional".into()),
        meetings: vec![],
        instructors: Some(vec![]),
        cross_list_crns: vec![],
        attributes: vec![],
        description: None,
        prereqs: None,
        coreqs: None,
        credit_hours: None,
        credit_hour_low: None,
        credit_hour_high: None,
    }
}
