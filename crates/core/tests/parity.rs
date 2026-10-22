#![allow(clippy::field_reassign_with_default)]
//! Ports of the Swift tests that specify search, schedules, degree plans, and the audit.

use std::collections::{HashMap, HashSet};
use termorganize_core::archive;
use termorganize_core::degree_audit::{
    counts_separately, is_coop_note, unchecked_reason, AuditStatus, DegreeAudit,
    DegreeCourseAttributes,
};
use termorganize_core::degree_format::{self, ImportError, SEPARATOR};
use termorganize_core::index::CatalogIndex;
use termorganize_core::legal;
use termorganize_core::links::{self, AppLink};
use termorganize_core::model::*;
use termorganize_core::nl::{self, InterpretError};
use termorganize_core::planlogic::{self, PrereqStatus};
use termorganize_core::program_parser;
use termorganize_core::schedule::{self, ScheduleCourseInput, ScheduleSlotMask};
use termorganize_core::search::{self, InstructionFilter, SearchFilters, SearchQuery};
use termorganize_core::store;
use termorganize_core::sync;
use termorganize_core::upnext;
use termorganize_core::watchlist;
use termorganize_core::CourseEquivalences;
use uuid::Uuid;

fn index() -> CatalogIndex {
    let path = archive::bundled_catalog_path_for("202710").expect("fall 2026 catalog");
    let mut catalog = archive::read(&path).expect("read catalog");
    catalog.recompute_postreqs();
    CatalogIndex::build(catalog)
}

fn section(
    crn: &str,
    subject: &str,
    number: &str,
    days: &[i32],
    start: f64,
    end: f64,
) -> CourseSection {
    CourseSection {
        crn: crn.into(),
        subject: subject.into(),
        number: number.into(),
        title: format!("{subject} {number}"),
        section_number: "01".into(),
        part_of_term: "1".into(),
        seat_capacity: 40,
        seats_remaining: 10,
        enrollment: 30,
        waitlist_capacity: 0,
        waitlist_count: 0,
        waitlist_remaining: 0,
        class_type: "Lecture".into(),
        honors: false,
        campus: "Boston".into(),
        instruction_method: Some("Traditional".into()),
        meetings: vec![Meeting {
            days: DaySet::from_indices(days.iter().copied()),
            start: Some((start * 60.0) as i32),
            end: Some((end * 60.0) as i32),
            building_code: Some("WVH".into()),
            building: Some("West Village H".into()),
            room: Some("210".into()),
            campus_code: Some("BOS".into()),
            kind: "Class".into(),
            start_date: Some("2026-09-09".into()),
            end_date: Some("2026-12-13".into()),
        }],
        instructors: Some(vec![]),
        cross_list_crns: vec![],
        attributes: vec![],
        description: None,
        prereqs: None,
        coreqs: None,
        credit_hours: None,
        credit_hour_low: None,
        credit_hour_high: None,
        restrictions: None,
    }
}

fn course_input(
    subject: &str,
    number: &str,
    required: bool,
    sections: Vec<CourseSection>,
) -> ScheduleCourseInput {
    ScheduleCourseInput::new(
        CourseKey {
            subject: subject.into(),
            number: number.into(),
        },
        required,
        sections,
    )
}

fn generate(
    courses: &[ScheduleCourseInput],
    filters: &ScheduleFilters,
) -> schedule::GenerationResult {
    schedule::generate(
        courses,
        filters,
        &SchedulePreferences::default(),
        schedule::ScheduleSort::Best,
        &schedule::Limits::default(),
    )
}

const SAMPLE_EXPORT: &str = "\
Printed: 10/03/2026 12:42
Student: SAMPLE STUDENT
StudentID: 000000000
Degree Audit Plan Name: New Plan
Program: CS-B-CSCI-BSCS
-
Completed Courses:
-
CS 1800 (4.0 Hours) Discrete Structures
-
CS 1802 (1.0 Hour) Seminar for CS 1800
-
NO AP CREDIT (0.0 Hours) No description
-
CS 2510 (4.0 Hours) Fundamentals of Computer Sci
-
CS 3500 (4.0 Hours) Object-Oriented Design
-
CS 3000 (4.0 Hours) Algorithms & Data
-
-
-------------------------------------------------------------------------------------------------------------------
Spring 2027 (9.0 Hours)
-
CS 3650 (4.0 Hours) Computer Systems
-
CS 4530 (4.0 Hours) Fundamentals of Software Engin
-
CS 2963 (1.0 Hour) Topics
-------------------------------------------------------------------------------------------------------------------
Summer Session A 2027 (4.0 Hours)
-
CS 4500 (4.0 Hours) Software Development
-------------------------------------------------------------------------------------------------------------------
Summer Session B 2027 (0.0 Hours)
-------------------------------------------------------------------------------------------------------------------
Fall 2027 (0.0 Hours)
";

const PROGRAM_PAGE: &str = r#"
<html><body>
<div id="textcontainer" class="page_content"><p>About the program.</p></div>
<div id="programrequirementstextcontainer" class="tab_content">
<p>Complete all courses listed below unless otherwise indicated.</p>
<h2>Major Requirements</h2>
<table class="sc_courselist"><tbody>
<tr class="hidden noscript"><th scope="col">Code</th><th scope="col">Title</th><th scope="col" class="hourscol">Hours</th></tr>
<tr class="even areaheader firstrow"><td colspan="2"><span class="courselistcomment areaheader">Required Courses</span></td><td class="hourscol"></td></tr>
<tr class="odd"><td class="codecol"><a href="/search/?P=CS%201800" title="CS 1800" class="bubblelink code">CS&#160;1800</a><span style="margin-left:20px;" class="blockindent">and <a href="/search/?P=CS%201802" class="bubblelink code">CS&#160;1802</a></span></td><td>Discrete Structures</td><td class="hourscol">5</td></tr>
<tr class="even"><td class="codecol"><a class="bubblelink code">CS&#160;3000</a></td><td>Algorithms and Data</td><td class="hourscol">4</td></tr>
<tr class="odd"><td class="codecol"><a class="bubblelink code">CS&#160;4500</a></td><td>Software Development</td><td class="hourscol">4</td></tr>
<tr class="orclass odd"><td class="codecol orclass">or <a class="bubblelink code">CS&#160;4530</a></td><td colspan="2"> Fundamentals of Software Engineering</td></tr>
<tr class="even areaheader"><td colspan="2"><span class="courselistcomment areaheader">Security Course</span></td><td class="hourscol"></td></tr>
<tr class="odd"><td colspan="2"><span class="courselistcomment">Complete one of the following:</span></td><td class="hourscol">4</td></tr>
<tr class="even"><td class="codecol"><a class="bubblelink code">CY&#160;2550</a></td><td>Foundations of Cybersecurity</td><td class="hourscol"></td></tr>
<tr class="odd"><td class="codecol"><a class="bubblelink code">CY&#160;3740</a></td><td>Systems Security</td><td class="hourscol"></td></tr>
<tr class="even areaheader"><td colspan="2"><span class="courselistcomment areaheader">Khoury Elective Courses</span></td><td class="hourscol"></td></tr>
<tr class="odd"><td colspan="2"><span class="courselistcomment">With adviser approval, directed study may also be taken as an elective.</span></td><td class="hourscol"></td></tr>
<tr class="even"><td colspan="2"><span class="courselistcomment">Complete 8 semester hours of CS, CY, DS, or IS classes that are not already required. Choose courses within the following ranges:</span></td><td class="hourscol">8</td></tr>
<tr class="odd"><td colspan="2"><div style="margin-left:20px;" class="blockindent"><span class="courselistcomment commentindent"><a class="bubblelink code">CS&#160;2500</a> or higher, except <a class="bubblelink code">CS&#160;5010</a></span></div></td><td class="hourscol"></td></tr>
<tr class="even lastrow"><td colspan="2"><div style="margin-left:20px;" class="blockindent"><span class="courselistcomment commentindent"><a class="bubblelink code">DS&#160;2500</a> to DS 4999 but not <a class="bubblelink code">DS&#160;4900</a></span></div></td><td class="hourscol"></td></tr>
</tbody></table>
<h2>Supporting Courses</h2>
<table class="sc_courselist"><tbody>
<tr class="even areaheader firstrow"><td colspan="2"><span class="courselistcomment areaheader">Science Requirement</span></td><td class="hourscol"></td></tr>
<tr class="odd"><td colspan="2"><span class="courselistcomment">Complete two courses (and any required labs) from the following science categories:</span></td><td class="hourscol">8</td></tr>
<tr class="even areasubheader undefined subheader "><td colspan="2"><span class="courselistcomment areasubheader undefined"><i>Biology</i></span></td><td class="hourscol"></td></tr>
<tr class="odd"><td class="codecol"><a class="bubblelink code">BIOL&#160;1111</a><br/><span class="blockindent">and <a class="bubblelink code">BIOL&#160;1112</a></span></td><td>General Biology 1<br/><span class="blockindent">and Lab for BIOL 1111</span></td><td class="hourscol"></td></tr>
<tr class="even areasubheader undefined subheader "><td colspan="2"><span class="courselistcomment areasubheader undefined"><i>Mathematics</i></span></td><td class="hourscol"></td></tr>
<tr class="odd lastrow"><td class="codecol"><a class="bubblelink code">MATH&#160;2331</a></td><td>Linear Algebra</td><td class="hourscol"></td></tr>
</tbody></table>
<h2>Required General Electives</h2>
<table class="sc_courselist"><tbody>
<tr class="even lastrow firstrow"><td colspan="2"><span class="courselistcomment">Complete 8 semester hours of general electives.</span></td><td class="hourscol">8</td></tr>
</tbody></table>
<h2>Major GPA Requirement</h2>
<p>Minimum 2.000 GPA required in the major.</p>
<h2>Program Requirement</h2>
<p>52 total semester hours required</p>
<h2>Concentration in Human-Centered Computing*</h2>
<table class="sc_courselist"><tbody>
<tr class="even firstrow"><td class="codecol"><a class="bubblelink code">CS&#160;2484</a></td><td>Principles of Human-Computer Interaction</td><td class="hourscol">4</td></tr>
<tr class="odd"><td colspan="2"><span class="courselistcomment">Complete two of the following not already taken:</span></td><td class="hourscol">8</td></tr>
<tr class="even"><td class="codecol"><a class="bubblelink code">CS&#160;4520</a></td><td>Mobile Application Development</td><td class="hourscol"></td></tr>
<tr class="odd"><td class="codecol"><a class="bubblelink code">CS&#160;4550</a></td><td>Web Development</td><td class="hourscol"></td></tr>
<tr class="even lastrow"><td class="codecol"><a class="bubblelink code">DS&#160;4200</a></td><td>Information Presentation and Visualization</td><td class="hourscol"></td></tr>
</tbody></table>
<p>*This concentration requires a fall co-op pattern.</p>
<h2>Software Concentration</h2>
<table class="sc_courselist"><tbody>
<tr class="even firstrow lastrow"><td class="codecol"><a class="bubblelink code">CS&#160;4400</a></td><td>Programming Languages</td><td class="hourscol">4</td></tr>
</tbody></table>
</div>
<div id="planofstudytextcontainer" class="tab_content">
<table class="sc_courselist"><tbody><tr><td class="codecol"><a>CS&#160;9999</a></td><td>Not a requirement</td><td class="hourscol">4</td></tr></tbody></table>
</div>
</body></html>
"#;

fn program() -> CatalogProgram {
    CatalogProgram {
        title: "Computer Science, BSCS (Boston)".into(),
        path: "undergraduate/computer-information-science/computer-science/bscs/".into(),
    }
}

fn requirements() -> ProgramRequirements {
    program_parser::requirements_from_html(PROGRAM_PAGE, CatalogYear::new(2023), program())
        .expect("requirements")
}

fn plan_course(code: &str, credits: f64, title: &str) -> DegreePlanCourse {
    let key = CourseKey::parse(code).unwrap();
    DegreePlanCourse::catalog(key, title, credits)
}

fn sample_plan() -> DegreePlan {
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![
        plan_course("CS 1800", 4.0, ""),
        plan_course("CS 1802", 1.0, ""),
        plan_course("CS 3000", 4.0, ""),
        plan_course("CY 2550", 4.0, ""),
        plan_course("CS 4973", 4.0, "Topics in Computer Science"),
        plan_course("CS 2484", 4.0, ""),
        plan_course("CS 4520", 4.0, ""),
        plan_course("CS 4535", 4.0, "Prof Practicum Capstone"),
        plan_course("CS 2501", 1.0, ""),
        plan_course("MATH 1341", 4.0, ""),
        plan_course("CS 3000", 0.0, ""),
    ];
    let mut spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    spring.courses = vec![
        plan_course("CS 3800", 4.0, ""),
        plan_course("CS 4100", 4.0, ""),
        plan_course("DS 3000", 4.0, ""),
        plan_course("CS 4410", 4.0, ""),
    ];
    plan.terms = vec![spring];
    plan.catalog_year = Some(2023);
    plan.catalog_program_path = Some(program().path);
    plan.concentration_id = Some("concentration-in-human-centered-computing".into());
    plan
}

fn audit(plan: &DegreePlan, requirements: &ProgramRequirements) -> DegreeAudit {
    DegreeAudit::run(
        plan,
        requirements,
        &CourseEquivalences::none(),
        &DegreeCourseAttributes::none(),
    )
}

fn substitution(
    rule_id: &str,
    title: &str,
    change: SubstitutionChange,
    note: &str,
) -> RequirementSubstitution {
    RequirementSubstitution {
        id: Uuid::new_v4(),
        rule_id: rule_id.into(),
        rule_title: title.into(),
        change,
        note: note.into(),
        approved_on: None,
        created_at: None,
    }
}

#[test]
fn term_names_and_course_keys() {
    let fall = Term {
        code: "202710".into(),
        banner_description: "Fall 2026 Semester".into(),
    };
    assert_eq!(fall.name(), "Fall 2026");
    let spring = Term {
        code: "202630".into(),
        banner_description: "Spring 2026 Semester (View Only)".into(),
    };
    assert_eq!(spring.name(), "Spring 2026");
    let cps = Term {
        code: "202635".into(),
        banner_description: "Spring 2026 CPS Quarter (View Only)".into(),
    };
    assert_eq!(cps.name(), "Spring 2026 · CPS Quarter");
    assert_eq!(cps.short_name(), "Spring 2026");
    assert_eq!(cps.college(), College::Cps);
    assert!(cps.is_view_only());
    let bare = Term {
        code: "202710".into(),
        banner_description: String::new(),
    };
    assert_eq!(bare.season(), Season::Fall);
    assert_eq!(bare.calendar_year(), 2026);
    assert_eq!(
        Term {
            code: "202630".into(),
            banner_description: String::new()
        }
        .calendar_year(),
        2026
    );
    assert_eq!(
        CourseKey::parse("CS 3000"),
        Some(CourseKey {
            subject: "CS".into(),
            number: "3000".into()
        })
    );
    assert_eq!(CourseKey::parse("cs3000").unwrap().number, "3000");
    assert_eq!(CourseKey::parse("math-209m").unwrap().number, "209M");
    assert!(CourseKey::parse("3000").is_none());
    let a = CourseKey {
        subject: "CS".into(),
        number: "2100".into(),
    };
    let b = CourseKey {
        subject: "CS".into(),
        number: "3000".into(),
    };
    assert!(a < b);
    assert!(
        CourseKey {
            subject: "CS".into(),
            number: "900".into()
        } < CourseKey {
            subject: "CS".into(),
            number: "1800".into()
        }
    );
}

#[test]
fn day_sets_and_time_labels() {
    let mwr = DaySet::from_indices([1, 3, 4]);
    assert_eq!(mwr.compact_label(), "MWR");
    assert!(mwr.contains_day(3));
    assert!(!mwr.contains_day(2));
    assert_eq!(mwr.count(), 3);
    assert!(mwr.is_subset(DaySet::WEEKDAYS));
    assert!(!DaySet::SATURDAY.is_subset(DaySet::WEEKDAYS));
    assert_eq!(minutes_from_hhmm(1435), 875);
    assert_eq!(label_minutes(875, true), "2:35 PM");
    assert_eq!(label_minutes(0, true), "12:00 AM");
    assert_eq!(range_label(630, 695), "10:30 – 11:35 AM");
    assert_eq!(range_label(705, 805), "11:45 AM – 1:25 PM");
    assert_eq!(credit_label(4.0, 4.0), "4 credits");
    assert_eq!(credit_label(1.0, 1.0), "1 credit");
    assert_eq!(credit_label(1.0, 4.0), "1–4 credits");
}

#[test]
fn requisite_helpers_and_instructor_names() {
    let a = CourseKey {
        subject: "CS".into(),
        number: "2100".into(),
    };
    let b = CourseKey {
        subject: "CS".into(),
        number: "2510".into(),
    };
    let c = CourseKey {
        subject: "CS".into(),
        number: "1800".into(),
    };
    let nested = Requisite::And(vec![
        Requisite::Or(vec![
            Requisite::Course(a.clone()),
            Requisite::Or(vec![Requisite::Course(b.clone())]),
        ]),
        Requisite::And(vec![Requisite::Course(c.clone())]),
    ]);
    let simplified = nested.simplified().unwrap();
    assert_eq!(simplified.summary(), "(CS 2100 or CS 2510) and CS 1800");
    let mut taken = HashSet::new();
    taken.insert(a.clone());
    taken.insert(c.clone());
    assert!(simplified.is_satisfied(&taken, false));
    taken.remove(&c);
    assert!(!simplified.is_satisfied(&taken, false));
    assert_eq!(
        Instructor {
            name: "Ptitsyna, Natalia".into(),
            primary: true,
            email: None
        }
        .display_name(),
        "Natalia Ptitsyna"
    );
    assert_eq!(
        Instructor {
            name: "Staff".into(),
            primary: false,
            email: None
        }
        .display_name(),
        "Staff"
    );
}

#[test]
fn bundled_catalog_search() {
    let index = index();
    assert!(index.courses.len() > 2500);
    assert!(index.catalog.sections.len() > 7000);
    assert!(index.courses.iter().all(|c| c.details_loaded));
    assert!(!index.instructors.is_empty());
    assert!(!index.buildings.is_empty());
    let top = |text: &str| {
        search::search(
            &SearchQuery {
                text: text.into(),
                ..SearchQuery::default()
            },
            &index,
        )
        .results
        .into_iter()
        .take(5)
        .map(|r| r.course.code())
        .collect::<Vec<_>>()
    };
    assert_eq!(top("CS 3000").first().map(String::as_str), Some("CS 3000"));
    assert_eq!(top("cs3000").first().map(String::as_str), Some("CS 3000"));
    assert_eq!(top("cs 30").first().map(String::as_str), Some("CS 3000"));
    assert_eq!(
        top("algorithms and data").first().map(String::as_str),
        Some("CS 3000")
    );
    assert_eq!(
        top("linear algebra").first().map(String::as_str),
        Some("MATH 2331")
    );
    assert_eq!(top("linalg").first().map(String::as_str), Some("MATH 2331"));
    assert!(top("orgo")
        .iter()
        .all(|c| c.starts_with("CHM") || c.starts_with("CHEM")));
    assert!(top("algoritms").iter().any(|c| c == "CS 3000"));
    let section = index.catalog.sections.first().unwrap();
    let response = search::search(
        &SearchQuery {
            text: section.crn.clone(),
            ..SearchQuery::default()
        },
        &index,
    );
    assert_eq!(
        response.crn_match.as_ref().map(|s| s.crn.as_str()),
        Some(section.crn.as_str())
    );
    let subjects = search::search(
        &SearchQuery {
            text: "cs".into(),
            ..SearchQuery::default()
        },
        &index,
    )
    .subjects;
    assert_eq!(
        subjects.iter().map(|s| s.code.as_str()).collect::<Vec<_>>(),
        vec!["CS"]
    );
    let mut filters = SearchFilters::default();
    filters.subjects.insert("CS".into());
    filters.open_seats_only = true;
    filters.allowed_days = Some(DaySet::from_indices([1, 3, 4]));
    filters.instruction = InstructionFilter::Any;
    let results = search::search(
        &SearchQuery {
            text: String::new(),
            filters: filters.clone(),
            ..SearchQuery::default()
        },
        &index,
    )
    .results;
    assert!(!results.is_empty());
    for result in &results {
        assert_eq!(result.course.subject, "CS");
        assert!(index
            .sections_for(&result.course.key())
            .iter()
            .any(|s| filters.matches_section(s)));
    }
}

#[test]
fn schedule_generator_matches_swift() {
    let a = ScheduleSlotMask::from_section(&section("1", "CS", "1", &[1, 3], 10.0, 11.0));
    let b = ScheduleSlotMask::from_section(&section("2", "CS", "2", &[3], 10.5, 12.0));
    let c = ScheduleSlotMask::from_section(&section("3", "CS", "3", &[1, 3], 11.0, 12.0));
    assert!(a.conflicts(b));
    assert!(!a.conflicts(c));
    assert!(!a.conflicts(ScheduleSlotMask::from_section(&section(
        "4",
        "CS",
        "4",
        &[2, 4],
        10.0,
        11.0
    ))));

    let courses = vec![
        course_input(
            "CS",
            "3000",
            true,
            vec![
                section("A1", "CS", "3000", &[1, 3, 4], 9.0, 10.0),
                section("A2", "CS", "3000", &[1, 3, 4], 13.0, 14.0),
            ],
        ),
        course_input(
            "MATH",
            "2331",
            true,
            vec![
                section("B1", "MATH", "2331", &[1, 3, 4], 9.5, 10.5),
                section("B2", "MATH", "2331", &[2, 5], 9.0, 10.5),
            ],
        ),
    ];
    let result = generate(&courses, &ScheduleFilters::default());
    assert_eq!(result.total_found, 3);
    assert!(result.schedules.iter().all(|s| {
        let set: HashSet<_> = s.crns.iter().cloned().collect();
        set != HashSet::from(["A1".into(), "B1".into()])
    }));

    let mut filters = ScheduleFilters::default();
    filters.earliest_start = Some(9 * 60);
    filters.free_days = DaySet::FRIDAY;
    let filtered = generate(
        &[course_input(
            "CS",
            "3000",
            true,
            vec![
                section("EARLY", "CS", "3000", &[1, 3], 8.0, 9.0),
                section("FRI", "CS", "3000", &[5], 12.0, 15.0),
                section("OK", "CS", "3000", &[2, 4], 11.0, 12.0),
            ],
        )],
        &filters,
    );
    assert_eq!(
        filtered
            .schedules
            .iter()
            .map(|s| s.crns.clone())
            .collect::<Vec<_>>(),
        vec![vec!["OK".to_string()]]
    );

    let optional = vec![
        course_input(
            "CS",
            "3000",
            true,
            vec![section("R", "CS", "3000", &[1, 3], 10.0, 11.0)],
        ),
        course_input(
            "ARTF",
            "1220",
            false,
            vec![
                section("O1", "ARTF", "1220", &[1], 10.5, 12.0),
                section("O2", "ARTF", "1220", &[2], 10.0, 12.0),
            ],
        ),
    ];
    let filled = generate(&optional, &ScheduleFilters::default());
    let sets: Vec<HashSet<String>> = filled
        .schedules
        .iter()
        .map(|s| s.crns.iter().cloned().collect())
        .collect();
    assert_eq!(sets, vec![HashSet::from(["R".into(), "O2".into()])]);
    let mut exactly = ScheduleFilters::default();
    exactly.course_count = Some(1);
    let single = generate(&optional, &exactly);
    assert_eq!(
        single
            .schedules
            .iter()
            .map(|s| s.crns.clone())
            .collect::<Vec<_>>(),
        vec![vec!["R".to_string()]]
    );

    let index = index();
    let keys = [
        CourseKey {
            subject: "CS".into(),
            number: "3000".into(),
        },
        CourseKey {
            subject: "CS".into(),
            number: "3001".into(),
        },
        CourseKey {
            subject: "MATH".into(),
            number: "2331".into(),
        },
    ];
    let real: Vec<_> = keys
        .iter()
        .map(|k| ScheduleCourseInput::new(k.clone(), true, index.sections_for(k)))
        .collect();
    let real_result = generate(&real, &ScheduleFilters::default());
    assert!(real_result.total_found > 0);
    assert!(real_result.schedules.iter().all(|s| s.crns.len() == 3));
}

#[test]
fn degree_plan_text_and_pdf() {
    let parsed = degree_format::parse(SAMPLE_EXPORT, Some("view-plan.pdf")).unwrap();
    let plan = parsed.plan;
    assert_eq!(plan.name, "New Plan");
    assert_eq!(plan.program.as_deref(), Some("CS-B-CSCI-BSCS"));
    assert_eq!(plan.student_name.as_deref(), Some("SAMPLE STUDENT"));
    assert_eq!(plan.imported_from.as_deref(), Some("view-plan.pdf"));
    assert!(plan.printed_at.is_some());
    assert_eq!(plan.completed.len(), 6);
    assert_eq!(plan.completed[1].credits, 1.0);
    assert!(plan.completed[2].key.is_none());
    assert_eq!(plan.completed[2].label, "NO AP CREDIT");
    assert_eq!(
        plan.completed[0]
            .key
            .as_ref()
            .map(|k| k.to_string())
            .as_deref(),
        Some("CS 1800")
    );
    assert_eq!(
        plan.terms.iter().map(|t| t.name()).collect::<Vec<_>>(),
        vec![
            "Spring 2027",
            "Summer Session A 2027",
            "Summer Session B 2027",
            "Fall 2027"
        ]
    );
    assert_eq!(plan.terms[0].courses.len(), 3);
    assert_eq!(plan.terms[0].credits(), 9.0);
    assert!(plan.terms[2].courses.is_empty());
    assert!(parsed.skipped_lines.is_empty());

    let raw = format!(
        "Degree Audit Plan Name:\tMy Plan\nProgram:\t\tCS-B\nCompleted Courses:  \nGAME1110 (4.0 Hours) Introduction to Video Game De\nCS  2500 (4.0 Hours) Fundamentals of Computer Sci \n{}\nSpring  2027 (4.0 Hours) \nCS  3484 (4.0 Hours) GUI Programming\n",
        "-".repeat(115)
    );
    let padded = degree_format::parse(&raw, None).unwrap().plan;
    assert_eq!(
        padded
            .completed
            .iter()
            .map(|c| c.label.clone())
            .collect::<Vec<_>>(),
        vec!["GAME 1110", "CS 2500"]
    );
    assert_eq!(padded.terms[0].season, PlanSeason::Spring);

    assert!(matches!(
        degree_format::parse("Hello world\nThis is a receipt\nTotal $4.00", None),
        Err(ImportError::NotADegreePlan)
    ));
    assert_eq!(SEPARATOR.chars().count(), 115);
    assert_eq!(
        degree_format::course_line(&plan.completed[0]),
        "CS  1800 (4.0 Hours) Discrete Structures"
    );
    assert_eq!(
        degree_format::course_line(&plan.completed[1]),
        "CS  1802 (1.0 Hour) Seminar for CS 1800"
    );
    assert_eq!(
        degree_format::term_header(&plan.terms[0]),
        "Spring  2027 (9.0 Hours) "
    );
    let text = degree_format::text(&plan, 0, false);
    assert!(!text.contains("StudentID"));
    assert!(text.contains("Degree Audit Plan Name:\tNew Plan"));

    let pdf = degree_format::pdf(&plan, 1_759_500_000, true);
    let again = degree_format::parse(&degree_format::pdf_text(&pdf), None)
        .unwrap()
        .plan;
    assert_eq!(again.name, plan.name);
    assert_eq!(
        again
            .completed
            .iter()
            .map(|c| c.label.clone())
            .collect::<Vec<_>>(),
        plan.completed
            .iter()
            .map(|c| c.label.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        again.terms.iter().map(|t| t.name()).collect::<Vec<_>>(),
        plan.terms.iter().map(|t| t.name()).collect::<Vec<_>>()
    );
}

#[test]
fn prerequisite_summaries_and_autofill() {
    let cs2100 = CourseKey {
        subject: "CS".into(),
        number: "2100".into(),
    };
    let cs2510 = CourseKey {
        subject: "CS".into(),
        number: "2510".into(),
    };
    let cs1800 = CourseKey {
        subject: "CS".into(),
        number: "1800".into(),
    };
    let requisite = Requisite::And(vec![
        Requisite::Or(vec![
            Requisite::Course(cs2100.clone()),
            Requisite::Course(cs2510.clone()),
        ]),
        Requisite::Course(cs1800.clone()),
    ]);
    let none = CourseEquivalences::none();
    let both: HashSet<_> = [cs2510.clone(), cs1800.clone()].into_iter().collect();
    assert_eq!(
        planlogic::status(Some(&requisite), &both, &none),
        PrereqStatus::Met
    );
    let only_prog: HashSet<_> = [cs2510.clone()].into_iter().collect();
    assert_eq!(
        planlogic::status(Some(&requisite), &only_prog, &none),
        PrereqStatus::Missing("CS 1800".into())
    );
    let only_disc: HashSet<_> = [cs1800.clone()].into_iter().collect();
    assert_eq!(
        planlogic::status(Some(&requisite), &only_disc, &none),
        PrereqStatus::Missing("CS 2100 or CS 2510".into())
    );
    assert_eq!(
        planlogic::status(Some(&requisite), &HashSet::new(), &none),
        PrereqStatus::Missing("(CS 2100 or CS 2510) and CS 1800".into())
    );
    assert_eq!(
        planlogic::status(None, &HashSet::new(), &none),
        PrereqStatus::Met
    );

    let key = |code: &str| Requisite::Course(CourseKey::parse(code).unwrap());
    let programming = Requisite::Or(vec![
        key("CS 2100"),
        key("CS 201M"),
        key("CS 2510"),
        key("DS 2500"),
    ]);
    let discrete = Requisite::Or(vec![key("CS 1800"), key("MATH 209M"), key("EECE 2160")]);
    let cs3000 = Requisite::Or(vec![
        Requisite::And(vec![programming.clone(), discrete]),
        key("CS 2700"),
    ]);
    assert_eq!(
        planlogic::unmet_summary(&programming, &HashSet::new(), &none).as_deref(),
        Some("CS 2100, CS 201M or 2 others")
    );
    assert_eq!(
        planlogic::unmet_summary(&cs3000, &HashSet::new(), &none).as_deref(),
        Some("CS 2700 or another option")
    );
    assert_eq!(planlogic::missing_count(&cs3000, &HashSet::new(), &none), 1);

    let plan = degree_format::parse(SAMPLE_EXPORT, None).unwrap().plan;
    let before = planlogic::courses_before(&plan, 1);
    assert!(before.contains(&CourseKey::parse("CS 3650").unwrap()));
    assert!(before.contains(&CourseKey::parse("CS 1800").unwrap()));
    assert!(!before.contains(&CourseKey::parse("CS 4500").unwrap()));

    let mut primary = plan.clone();
    primary.is_primary = true;
    let spring = Term {
        code: "202730".into(),
        banner_description: "Spring 2027 Semester".into(),
    };
    let exact = planlogic::schedule_suggestions(
        &spring,
        &[primary.clone()],
        &[],
        None,
        &HashSet::new(),
        None,
    );
    assert!(exact[0].is_exact_term);
    assert_eq!(exact[0].courses.len(), 3);
    let fall = Term {
        code: "202710".into(),
        banner_description: "Fall 2026 Semester".into(),
    };
    let closest = planlogic::schedule_suggestions(
        &fall,
        &[primary.clone()],
        &[],
        None,
        &HashSet::new(),
        None,
    );
    assert!(!closest[0].is_exact_term);
    assert!(closest[0].title.contains("Spring 2027"));
    let partial = planlogic::schedule_suggestions(
        &spring,
        &[primary.clone()],
        &[],
        None,
        &HashSet::from([CourseKey::parse("CS 3650").unwrap()]),
        None,
    );
    assert_eq!(partial[0].courses.len(), 2);

    let index = index();
    let mut offered = DegreePlan {
        name: "Test".into(),
        ..DegreePlan::default()
    };
    offered.terms = vec![DegreePlanTerm::new(PlanSeason::Fall, 2026)];
    offered.terms[0].courses = vec![
        plan_course("CS 3000", 4.0, "Algorithms"),
        plan_course("CS 9999", 4.0, "Nope"),
    ];
    let suggestions = planlogic::schedule_suggestions(
        &index.catalog.term,
        &[offered],
        &[],
        None,
        &HashSet::new(),
        Some(&index),
    );
    assert_eq!(
        suggestions[0].courses,
        vec![CourseKey::parse("CS 3000").unwrap()]
    );
    assert_eq!(
        suggestions[0].unavailable,
        vec![CourseKey::parse("CS 9999").unwrap()]
    );

    let mut schedule = SchedulePlan {
        id: Uuid::new_v4(),
        name: "Plan A".into(),
        term_code: "202730".into(),
        courses: vec![
            PlanCourse {
                key: CourseKey::parse("CS 3650").unwrap(),
                is_required: true,
                excluded_crns: HashSet::new(),
                color_index: 0,

                pinned_crn: None,
                degree_row_id: None,
                credits: None,
            },
            PlanCourse {
                key: CourseKey::parse("MATH 2331").unwrap(),
                is_required: true,
                excluded_crns: HashSet::new(),
                color_index: 1,

                pinned_crn: None,
                degree_row_id: None,
                credits: None,
            },
        ],
        selected_crns: vec![],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: true,
        created_at: 0.0,
        updated_at: 0.0,
    };
    let draft =
        planlogic::new_degree_plan_draft("Draft", Some(&primary), &[schedule.clone()], 2026, 10);
    assert_eq!(draft.completed.len(), primary.completed.len());
    assert_ne!(draft.completed[0].id, primary.completed[0].id);
    let spring_term = draft
        .terms
        .iter()
        .find(|t| t.season == PlanSeason::Spring && t.year == 2027)
        .unwrap();
    assert_eq!(
        spring_term
            .courses
            .iter()
            .map(|c| c.label.clone())
            .collect::<Vec<_>>(),
        vec!["CS 3650", "CS 4530", "CS 2963", "MATH 2331"]
    );
    let _ = &mut schedule;

    assert!(
        DegreePlanTerm::new(PlanSeason::SummerA, 2027).matches_term(&Term {
            code: "202740".into(),
            banner_description: String::new()
        })
    );
    assert!(
        DegreePlanTerm::new(PlanSeason::SummerA, 2027).matches_term(&Term {
            code: "202750".into(),
            banner_description: String::new()
        })
    );
    assert!(
        !DegreePlanTerm::new(PlanSeason::SummerA, 2027).matches_term(&Term {
            code: "202760".into(),
            banner_description: String::new()
        })
    );

    let mut data = UserData::default();
    let first = data.add_degree_plan(DegreePlan {
        name: "A".into(),
        ..DegreePlan::default()
    });
    let second = data.add_degree_plan(DegreePlan {
        name: "B".into(),
        ..DegreePlan::default()
    });
    assert!(
        data.degree_plans
            .iter()
            .find(|p| p.id == first)
            .unwrap()
            .is_primary
    );
    assert!(
        !data
            .degree_plans
            .iter()
            .find(|p| p.id == second)
            .unwrap()
            .is_primary
    );
    data.set_primary_degree_plan(second);
    assert_eq!(data.primary_degree_plan().unwrap().id, second);
    data.delete_degree_plan(second);
    assert_eq!(data.primary_degree_plan().unwrap().id, first);
}

#[test]
fn catalog_requirements_and_audit() {
    let requirements = requirements();
    assert_eq!(requirements.total_hours, Some(52.0));
    assert_eq!(
        requirements
            .sections
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Major Requirements",
            "Supporting Courses",
            "Required General Electives",
            "Major GPA Requirement",
            "Program Requirement",
            "Concentration in Human-Centered Computing",
            "Software Concentration",
        ]
    );
    assert_eq!(
        requirements
            .concentrations()
            .iter()
            .map(|s| s.concentration_name())
            .collect::<Vec<_>>(),
        vec![
            "Human-Centered Computing".to_string(),
            "Software".to_string()
        ]
    );
    assert!(!requirements
        .all_rules()
        .iter()
        .any(|r| r.listed_courses().iter().any(|k| k.number == "9999")));
    let major = &requirements.sections[0];
    assert_eq!(
        major
            .groups
            .iter()
            .map(|g| g.title.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec![
            "Required Courses",
            "Security Course",
            "Khoury Elective Courses"
        ]
    );
    let required = &major.groups[0].rules;
    assert_eq!(
        required
            .iter()
            .map(|r| r.label.as_str())
            .collect::<Vec<_>>(),
        vec!["CS 1800 and CS 1802", "CS 3000", "CS 4500 or CS 4530"]
    );
    assert!(required.iter().any(|r| r
        .id
        .contains("major-requirements/required-courses/cs-4500-or-cs-4530")));
    assert_eq!(
        requirements
            .course_titles
            .get("CS 3000")
            .map(String::as_str),
        Some("Algorithms and Data")
    );
    assert_eq!(
        requirements
            .course_titles
            .get("CS 4530")
            .map(String::as_str),
        Some("Fundamentals of Software Engineering")
    );
    assert_eq!(
        requirements
            .title_of_rule(&requirements.sections[1].groups[0].rules[0].id)
            .as_deref(),
        Some("Science Requirement")
    );

    let page = r#"<header><span class="edition">Academic Catalog 2031-2032</span></header><a href="/archive/2030-2031">2030-2031 Catalog</a><a href="https://catalog.northeastern.edu/archive/2029-2030/">2029-2030 Catalog</a>"#;
    assert_eq!(
        program_parser::current_edition(page).unwrap().start_year,
        2031
    );
    assert_eq!(
        program_parser::archived_editions(page)
            .iter()
            .map(|y| y.start_year)
            .collect::<Vec<_>>(),
        vec![2030, 2029]
    );
    assert_eq!(CatalogYear::parse("2030-31").unwrap().label(), "2030–2031");
    let editions = CatalogEditions::estimated(2040, 10);
    assert_eq!(editions.url(CatalogYear::new(2040), "undergraduate/computer-information-science/computer-science/bscs/"), "https://catalog.northeastern.edu/undergraduate/computer-information-science/computer-science/bscs/");
    assert!(editions
        .url(CatalogYear::new(2030), "undergraduate/x/")
        .contains("/archive/2030-2031/"));

    let index_html = r#"
        <a href="/archive/2023-2024/undergraduate/computer-information-science/computer-science/bscs/">Computer Science, BSCS</a>
        <a href="/graduate/computer-information-science/computer-science/computer-science-ms/">Computer Science, MS (Boston)</a>
        <a href="/undergraduate/computer-information-science/computer-science/computer-science-minor/">Computer Science Minor</a>
        <a href="/undergraduate/about/">About, The University</a>
    "#;
    let programs = program_parser::programs_from_az(index_html);
    assert_eq!(
        programs
            .iter()
            .map(|p| p.title.as_str())
            .collect::<Vec<_>>(),
        vec!["Computer Science, BSCS", "Computer Science, MS (Boston)"]
    );
    assert_eq!(programs[1].degree().as_deref(), Some("MS"));
    assert_eq!(programs[1].campus().as_deref(), Some("Boston"));

    let result = audit(&sample_plan(), &requirements);
    let status = |label: &str| {
        result
            .rule_results()
            .into_iter()
            .find(|r| r.rule.label == label)
            .map(|r| r.status)
    };
    assert!(!result.needs_concentration);
    assert_eq!(status("CS 1800 and CS 1802"), Some(AuditStatus::Complete));
    assert_eq!(status("CS 3000"), Some(AuditStatus::Complete));
    assert_eq!(status("CS 4500 or CS 4530"), Some(AuditStatus::Missing));
    assert_eq!(result.completed_hours, 34.0);
    assert_eq!(result.planned_hours, 16.0);
    assert_eq!(result.required_hours, Some(52.0));
    assert!(!result
        .sections
        .iter()
        .any(|s| s.section.title == "Software Concentration"));
}

#[test]
fn watchlist_links_sync_and_disclaimer() {
    let mut item = WatchItem {
        term_code: "202710".into(),
        crn: "12345".into(),
        course_key: CourseKey::parse("CS 3000").unwrap(),
        course_title: "Algorithms".into(),
        section_label: "Lecture 01".into(),
        added_at: 0.0,
        last_seats_remaining: 0,
        last_waitlist_remaining: 0,
        seat_capacity: 40,
        last_checked_at: None,
        notify_on_waitlist: true,
        last_notified_at: None,
    };
    let mut live_section = section("12345", "CS", "3000", &[1], 10.0, 11.0);
    live_section.seats_remaining = 2;
    let mut live = HashMap::new();
    live.insert("202710".into(), vec![live_section]);
    let outcome = watchlist::compare(&[item.clone()], &live, 10.0);
    assert_eq!(outcome.alerts.len(), 1);
    assert!(outcome.alerts[0].title().contains("CS 3000"));
    assert!(outcome.alerts[0].body().contains("CRN 12345"));
    item.last_seats_remaining = 2;
    let quiet = watchlist::compare(&[item], &live, 11.0);
    assert!(quiet.alerts.is_empty());

    assert_eq!(
        links::parse("termorganize://course/202710/CS/3000"),
        Some(AppLink::Course {
            term: "202710".into(),
            subject: "CS".into(),
            number: "3000".into()
        })
    );
    assert_eq!(
        links::parse("neuclasssearch://section/202710/12345"),
        Some(AppLink::Section {
            term: "202710".into(),
            crn: "12345".into()
        })
    );
    assert_eq!(
        links::parse("termorganize://search?q=orgo+lab"),
        Some(AppLink::Search {
            query: "orgo lab".into()
        })
    );
    assert!(legal::UNOFFICIAL_NOTICE.contains("Not affiliated"));
    assert!(legal::UNOFFICIAL_NOTICE.contains("Northeastern"));
    assert!(legal::UNOFFICIAL_NOTICE.contains("Sandbox"));

    let local = sync::Record {
        key: "schedule:a".into(),
        kind: "schedule".into(),
        updated_at: 2.0,
        deleted: false,
        payload: serde_json::json!({"n": 1}),
    };
    let remote = sync::Record {
        key: "schedule:a".into(),
        kind: "schedule".into(),
        updated_at: 2.0,
        deleted: false,
        payload: serde_json::json!({"n": 2}),
    };
    assert_eq!(
        sync::merge_record(Some(&local), Some(&remote), None)
            .unwrap()
            .payload["n"],
        2
    );
    let newer = sync::Record {
        key: "schedule:a".into(),
        kind: "schedule".into(),
        updated_at: 3.0,
        deleted: false,
        payload: serde_json::json!({"n": 3}),
    };
    assert_eq!(
        sync::merge_record(Some(&newer), Some(&remote), None)
            .unwrap()
            .payload["n"],
        3
    );
    let deleted = sync::Record {
        key: "schedule:a".into(),
        kind: "schedule".into(),
        updated_at: 4.0,
        deleted: true,
        payload: serde_json::json!({}),
    };
    assert!(sync::merge_record(Some(&local), Some(&deleted), Some(1.0)).is_none());
    assert!(sync::merge_record(Some(&local), Some(&deleted), Some(5.0)).is_some());

    let draft = nl::interpret("no classes before 10 and Fridays off, and I need lunch").unwrap();
    let prefs = draft.applied(&SchedulePreferences::default());
    assert!(prefs.earliest_start.is_some());
    assert!(prefs.days_off.contains(DaySet::FRIDAY));
    assert!(prefs.lunch.is_some());
    assert!(matches!(nl::interpret(""), Err(InterpretError::Empty)));

    let mut plan = DegreePlan::default();
    plan.student_name = Some("Hidden".into());
    plan.student_id = Some("1".into());
    let mut data = UserData::default();
    data.degree_plans.push(plan);
    let exported = store::export_json(&data, false);
    assert!(!exported.contains("Hidden"));
    assert!(is_coop_note("Satisfied through a co-op"));
    assert!(unchecked_reason("Minimum 2.000 GPA required").is_some());
    assert!(!counts_separately("Algorithms", 4.0, "Algorithms", 0.0));
    assert!(counts_separately(
        "Topics in Graphics",
        4.0,
        "Topics in Security",
        4.0
    ));

    let section = section("9", "CS", "3000", &[3], 9.0, 10.0);
    let items = upnext::occurrences(&[section], 3, 9 * 60 + 30, "2026-09-16");
    assert!(upnext::up_next(&items).unwrap().in_progress);
    assert!(upnext::summary(&items).contains("CS 3000"));
}

#[test]
fn substitutions_petitions_and_rebalance() {
    let requirements = requirements();
    let mut plan = sample_plan();
    let hcc = requirements
        .all_rules()
        .into_iter()
        .find(|r| r.label == "Complete two of the following not already taken:")
        .unwrap();
    let engineering = requirements
        .all_rules()
        .into_iter()
        .find(|r| r.label == "CS 4500 or CS 4530")
        .unwrap();
    let science = requirements
        .all_rules()
        .into_iter()
        .find(|r| r.label.starts_with("Complete two courses"))
        .unwrap();
    let electives = requirements
        .all_rules()
        .into_iter()
        .find(|r| r.label.starts_with("Complete 8 semester hours of CS"))
        .unwrap();
    let software = requirements
        .all_rules()
        .into_iter()
        .find(|r| r.label == "CS 4400")
        .unwrap();
    plan.substitutions = vec![
        substitution(
            &hcc.id,
            "HCI electives",
            SubstitutionChange::count_course(CourseKey::parse("CS 4973").unwrap()),
            "",
        ),
        substitution(
            &engineering.id,
            "Software engineering",
            SubstitutionChange::count_course(CourseKey::parse("CS 4535").unwrap()),
            "",
        ),
        substitution(
            &science.id,
            "Science Requirement",
            SubstitutionChange::Replace(RequirementReplacement {
                count: Some(2),
                hours: None,
                like_rule_id: Some(electives.id.clone()),
                like_rule_title: Some("Khoury Elective Courses".into()),
                courses: vec![],
                course_numbers: None,
            }),
            "Petition approved",
        ),
        substitution(&software.id, "CS 4400", SubstitutionChange::Waive, ""),
        substitution(
            &electives.id,
            "Electives",
            SubstitutionChange::count_course(CourseKey::parse("CS 4999").unwrap()),
            "",
        ),
    ];
    let result = audit(&plan, &requirements);
    let hcc_result = result.result_for(&hcc.id).unwrap();
    assert_eq!(hcc_result.status, AuditStatus::Complete);
    assert!(hcc_result
        .used
        .iter()
        .any(|u| u.key.to_string() == "CS 4973" && u.by_substitution));
    let eng = result.result_for(&engineering.id).unwrap();
    assert_eq!(eng.status, AuditStatus::Complete);
    assert_eq!(
        eng.used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<Vec<_>>(),
        vec!["CS 4535".to_string()]
    );
    let elective_result = result.result_for(&electives.id).unwrap();
    assert_eq!(elective_result.status, AuditStatus::Planned);
    let used: HashSet<_> = elective_result
        .used
        .iter()
        .map(|u| u.key.to_string())
        .collect();
    assert_eq!(used, HashSet::from(["DS 3000".into(), "CS 4410".into()]));
    let science_result = result.result_for(&science.id).unwrap();
    assert_eq!(science_result.status, AuditStatus::Planned);
    let science_used: HashSet<_> = science_result
        .used
        .iter()
        .map(|u| u.key.to_string())
        .collect();
    assert_eq!(
        science_used,
        HashSet::from(["CS 3800".into(), "CS 4100".into()])
    );
    assert_eq!(
        science_result.substitutions.first().unwrap().note,
        "Petition approved"
    );
    let unmatched: HashSet<_> = result
        .unmatched_substitutions
        .iter()
        .map(|s| s.rule_title.clone())
        .collect();
    assert_eq!(
        unmatched,
        HashSet::from(["CS 4400".into(), "Electives".into()])
    );
}
