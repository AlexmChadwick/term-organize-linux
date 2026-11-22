//! Dirty instructor addresses never survive the archive or the Banner merge.

use std::collections::HashMap;
use termorganize_core::archive;
use termorganize_core::catalog::{self, BannerSection};
use termorganize_core::model::*;

const DIRTY: &str = "a@husky.neu.edu b@gmail.com c@neu.edu. d@neu.eduX e@neu.edu";

fn instructor(email: &str) -> Instructor {
    Instructor {
        name: "Ada Lovelace".into(),
        primary: true,
        email: Some(email.into()),
    }
}

fn section(crn: &str, instructors: Vec<Instructor>) -> CourseSection {
    CourseSection {
        crn: crn.into(),
        subject: "CS".into(),
        number: "3000".into(),
        title: String::new(),
        section_number: "01".into(),
        part_of_term: String::new(),
        seat_capacity: 0,
        seats_remaining: 0,
        enrollment: 0,
        waitlist_capacity: 0,
        waitlist_count: 0,
        waitlist_remaining: 0,
        class_type: "Lecture".into(),
        honors: false,
        campus: "Boston".into(),
        instruction_method: None,
        meetings: Vec::new(),
        instructors: Some(instructors),
        cross_list_crns: Vec::new(),
        attributes: Vec::new(),
        description: None,
        prereqs: None,
        coreqs: None,
        credit_hours: None,
        credit_hour_low: None,
        credit_hour_high: None,
        restrictions: None,
    }
}

fn term() -> Term {
    Term {
        code: "202730".into(),
        banner_description: "Spring 2027 Semester".into(),
    }
}

fn dirty_catalog() -> TermCatalog {
    let instructors = vec![
        instructor(DIRTY),
        instructor("x@neu.edu."),
        instructor("y@northeastern.edu"),
    ];
    TermCatalog {
        version: 1,
        term: term(),
        generated_at: 0.0,
        sections_updated_at: 0.0,
        courses: Vec::new(),
        sections: vec![section("1", instructors)],
        subjects: Vec::new(),
        attributes: Vec::new(),
        campuses: Vec::new(),
    }
}

fn emails(catalog: &TermCatalog) -> Vec<Option<String>> {
    catalog.sections[0]
        .instructors
        .as_ref()
        .unwrap()
        .iter()
        .map(|i| i.email.clone())
        .collect()
}

fn clean() -> Vec<Option<String>> {
    vec![
        Some("e@neu.edu".into()),
        None,
        Some("y@northeastern.edu".into()),
    ]
}

#[test]
fn encode_scrubs_dirty_addresses() {
    let bytes = archive::encode(&dirty_catalog()).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    for bad in ["husky", "gmail", "x@neu.edu.", "neu.eduX", "c@neu.edu"] {
        assert!(!text.contains(bad), "{bad}");
    }
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["sections"][0]["instructors"][0]["email"], "e@neu.edu");
}

#[test]
fn decode_scrubs_dirty_addresses() {
    // Serialize without the scrub so only decode can clean the data.
    let raw = serde_json::to_vec(&dirty_catalog()).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("husky"));
    let decoded = archive::decode(&raw).unwrap();
    assert_eq!(emails(&decoded), clean());
}

#[test]
fn merge_scrubs_cached_instructors() {
    let existing = dirty_catalog();
    let fresh = BannerSection {
        course_reference_number: "1".into(),
        subject: "CS".into(),
        course_number: "3000".into(),
        ..Default::default()
    };
    let merged = catalog::merge(
        Some(&existing),
        &[fresh],
        term(),
        None,
        None,
        None,
        &HashMap::new(),
        1.0,
    );
    assert_eq!(emails(&merged), clean());
}
