#![allow(clippy::field_reassign_with_default, non_snake_case)]
//! Assertion-by-assertion ports of the Swift degree suites.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;
use termorganize_core::archive::{self, bundled_catalog_path};
use termorganize_core::degree_audit::{
    self, alternative_entries, collapsed_entries, counting_caption, counting_note, has_header_row,
    is_replaced, is_shown, listing, note_entries, option_entries, requirement_name, row_caption,
    split_unlisted, AuditStatus, CourseRowInfo, CourseStanding, CourseUse, DegreeAudit,
    DegreeCourseAttributes, ListEntry, NoteCheck, NoteKind, RequirementsFilter,
};
use termorganize_core::degree_format::{self, ImportError, SEPARATOR};
use termorganize_core::editing::{self, PlanCourseChoice};
use termorganize_core::index::CatalogIndex;
use termorganize_core::model::*;
use termorganize_core::nupath::{
    AttributeLookup, AttributeRepository, AttributeSource, Matching, NuCourse, NuRequirement,
};
use termorganize_core::petition;
use termorganize_core::planlogic::{self, PrereqStatus};
use termorganize_core::program_parser;
use termorganize_core::CourseEquivalences;
use uuid::Uuid;

const PROGRAM_PAGE: &str = r#"<html><body>
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
    </body></html>"#;

const NUPATH_PAGE: &str = r#"<div id="programrequirementstextcontainer" class="tab_content">
    <h2>Universitywide Requirements</h2>
    <p>All undergraduate students are required to complete the <a href="/archive/2024-2025/undergraduate/university-academics/university-wide-requirements/">Universitywide Requirements</a>.</p>
    <h2>NUpath Requirements</h2>
    <p>All undergraduate students are required to complete the <a href="/archive/2024-2025/undergraduate/university-academics/nupath/">NUpath Requirements</a>.</p>
    <h2>Major Requirements</h2>
    <table class="sc_courselist"><tbody>
    <tr class="even firstrow lastrow"><td class="codecol"><a class="bubblelink code">CS&#160;3000</a></td><td>Algorithms and Data</td><td class="hourscol">4</td></tr>
    </tbody></table>
    <h2>Khoury College GPA Requirement</h2>
    <p>Minimum cumulative 2.000 GPA required in all CS, CY, DS, and IS courses</p>
    <h2>NUpath Requirements Satisfied</h2>
    <p>Integrating Knowledge and Skills Through Experience is satisfied through co-op.</p>
    <h2>Program Requirement</h2>
    <p>134 total semester hours required</p>
    </div>"#;

const NOTES_PAGE: &str = r#"<div id="programrequirementstextcontainer" class="tab_content">
    <h2>Major Requirements</h2>
    <table class="sc_courselist"><tbody>
    <tr class="even firstrow"><td class="codecol"><a class="bubblelink code">CS&#160;3000</a></td><td>Algorithms and Data</td><td class="hourscol">4</td></tr>
    <tr class="odd"><td colspan="2"><span class="courselistcomment">Complete 8 semester hours of CS classes. Choose courses within the following ranges:</span></td><td class="hourscol">8</td></tr>
    <tr class="even lastrow"><td colspan="2"><div style="margin-left:20px;" class="blockindent"><span class="courselistcomment commentindent"><a class="bubblelink code">CS&#160;2500</a> or higher</span></div></td><td class="hourscol"></td></tr>
    </tbody></table>
    <h2>GPA Requirement</h2>
    <p>Minimum 2.000 GPA required in the major.</p>
    <h2>Credit Requirement</h2>
    <p>Complete 12 semester hours in the major.</p>
    <h2>NUpath Requirements Satisfied</h2>
    <p>Integrating Knowledge and Skills Through Experience is satisfied through co-op.</p>
    <h2>Program Requirement</h2>
    <p>12 total semester hours required</p>
    </div>"#;

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

fn key(code: &str) -> CourseKey {
    CourseKey::parse(code).unwrap_or_else(|| panic!("bad course {code}"))
}

fn course(code: &str, credits: f64, title: &str) -> DegreePlanCourse {
    DegreePlanCourse::catalog(key(code), title, credits)
}

fn bscs() -> CatalogProgram {
    CatalogProgram {
        title: "Computer Science, BSCS (Boston)".into(),
        path: "undergraduate/computer-information-science/computer-science/bscs/".into(),
    }
}

fn requirements() -> ProgramRequirements {
    program_parser::requirements_from_html(PROGRAM_PAGE, CatalogYear::new(2023), bscs())
        .expect("program page")
}

fn plan() -> DegreePlan {
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![
        course("CS 1800", 4.0, ""),
        course("CS 1802", 1.0, ""),
        course("CS 3000", 4.0, ""),
        course("CY 2550", 4.0, ""),
        course("CS 4973", 4.0, "Topics in Computer Science"),
        course("CS 2484", 4.0, ""),
        course("CS 4520", 4.0, ""),
        course("CS 4535", 4.0, "Prof Practicum Capstone"),
        course("CS 2501", 1.0, ""),
        course("MATH 1341", 4.0, ""),
        course("CS 3000", 0.0, ""),
    ];
    let mut spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    spring.courses = vec![
        course("CS 3800", 4.0, ""),
        course("CS 4100", 4.0, ""),
        course("DS 3000", 4.0, ""),
        course("CS 4410", 4.0, ""),
    ];
    plan.terms = vec![spring];
    plan.catalog_year = Some(2023);
    plan.catalog_program_path = Some(bscs().path);
    plan.concentration_id = Some("concentration-in-human-centered-computing".into());
    plan
}

fn run(plan: &DegreePlan, requirements: &ProgramRequirements) -> DegreeAudit {
    DegreeAudit::run(
        plan,
        requirements,
        &CourseEquivalences::none(),
        &DegreeCourseAttributes::none(),
    )
}

fn run_attr(
    plan: &DegreePlan,
    requirements: &ProgramRequirements,
    attributes: &DegreeCourseAttributes,
) -> DegreeAudit {
    DegreeAudit::run(plan, requirements, &CourseEquivalences::none(), attributes)
}

fn rule_named<'a>(requirements: &'a ProgramRequirements, label: &str) -> &'a RequirementRule {
    requirements
        .all_rules()
        .into_iter()
        .find(|rule| rule.label == label)
        .unwrap_or_else(|| panic!("missing rule {label}"))
}

fn rule_prefix<'a>(requirements: &'a ProgramRequirements, prefix: &str) -> &'a RequirementRule {
    requirements
        .all_rules()
        .into_iter()
        .find(|rule| rule.label.starts_with(prefix))
        .unwrap_or_else(|| panic!("missing prefix {prefix}"))
}

fn status_of(audit: &DegreeAudit, label: &str) -> Option<AuditStatus> {
    audit
        .rule_results()
        .into_iter()
        .find(|result| result.rule.label == label)
        .map(|result| result.status)
}

fn sub(
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

fn replace(
    count: Option<i32>,
    like_id: Option<&str>,
    like_title: &str,
    numbers: Option<CourseNumberLimit>,
) -> SubstitutionChange {
    SubstitutionChange::Replace(RequirementReplacement {
        count,
        hours: None,
        like_rule_id: like_id.map(|id| id.to_string()),
        like_rule_title: Some(like_title.into()),
        courses: vec![],
        course_numbers: numbers,
    })
}

fn use_of(code: &str, title: &str, credits: f64, semester: Option<&str>, order: i32) -> CourseUse {
    CourseUse {
        key: key(code),
        title: title.into(),
        credits,
        semester: semester.map(|s| s.to_string()),
        order,
        by_substitution: false,
        satisfies: None,
        instance: 0,
        crn: None,
    }
}

#[test]
fn parsesRequirementTables() {
    let requirements = requirements();
    assert_eq!(requirements.total_hours, Some(52.0));
    assert_eq!(
        requirements
            .sections
            .iter()
            .map(|s| s.title.as_str())
            .collect::<Vec<_>>(),
        [
            "Major Requirements",
            "Supporting Courses",
            "Required General Electives",
            "Major GPA Requirement",
            "Program Requirement",
            "Concentration in Human-Centered Computing",
            "Software Concentration"
        ]
    );
    assert_eq!(
        requirements
            .concentrations()
            .iter()
            .map(|s| s.concentration_name())
            .collect::<Vec<_>>(),
        ["Human-Centered Computing", "Software"]
    );
    assert!(!requirements
        .all_rules()
        .iter()
        .any(|rule| rule.listed_courses().contains(&key("CS 9999"))));
    let major = &requirements.sections[0];
    assert_eq!(
        major
            .groups
            .iter()
            .map(|g| g.title.as_deref().unwrap())
            .collect::<Vec<_>>(),
        [
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
        ["CS 1800 and CS 1802", "CS 3000", "CS 4500 or CS 4530"]
    );
    assert_eq!(
        required[0].kind,
        RuleKind::Required(CourseOption {
            alternatives: vec![vec![key("CS 1800"), key("CS 1802")]]
        })
    );
    assert_eq!(required[2].hours, Some(4.0));
    let security = &major.groups[1].rules[0];
    match &security.kind {
        RuleKind::Choose {
            count,
            options,
            ranges,
            ..
        } => {
            assert_eq!(*count, Some(1));
            assert_eq!(options.len(), 2);
            assert!(ranges.is_empty());
        }
        other => panic!("not a choice: {other:?}"),
    }
    let electives = &major.groups[2].rules[0];
    match &electives.kind {
        RuleKind::Choose {
            count,
            hours,
            ranges,
            ..
        } => {
            assert!(count.is_none());
            assert_eq!(*hours, Some(8.0));
            assert_eq!(
                ranges.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
                [
                    "CS 2500 or higher, except CS 5010",
                    "DS 2500 to DS 4999 but not DS 4900"
                ]
            );
            assert!(ranges[0].contains(&key("CS 4973")));
            assert!(!ranges[0].contains(&key("CS 5010")));
            assert!(!ranges[0].contains(&key("CS 2100")));
            assert!(!ranges[1].contains(&key("DS 4900")));
            assert!(!ranges[1].contains(&key("DS 5110")));
        }
        other => panic!("not hours: {other:?}"),
    }
    assert_eq!(major.groups[2].notes.len(), 1);
    let science = &requirements.sections[1].groups[0].rules[0];
    match &science.kind {
        RuleKind::Choose { count, options, .. } => {
            assert_eq!(*count, Some(2));
            assert_eq!(
                options.iter().map(|o| o.label()).collect::<Vec<_>>(),
                ["BIOL 1111 and BIOL 1112", "MATH 2331"]
            );
        }
        other => panic!("science: {other:?}"),
    }
    assert_eq!(science.notes, ["Biology", "Mathematics"]);
    assert_eq!(
        science.option_categories,
        Some(vec![Some("Biology".into()), Some("Mathematics".into())])
    );
    assert_eq!(
        requirements.title_of_rule(&science.id).as_deref(),
        Some("Science Requirement")
    );
    assert!(
        matches!(requirements.sections[2].groups[0].rules[0].kind, RuleKind::GeneralElectives { hours } if hours == 8.0)
    );
    assert_eq!(
        requirements.sections[3].notes,
        ["Minimum 2.000 GPA required in the major."]
    );
    let title = |code: &str| requirements.course_titles.get(code).map(|s| s.as_str());
    assert_eq!(title("CS 3000"), Some("Algorithms and Data"));
    assert_eq!(
        title("CS 4530"),
        Some("Fundamentals of Software Engineering")
    );
    assert_eq!(title("BIOL 1111"), Some("General Biology 1"));
    assert_eq!(title("BIOL 1112"), Some("Lab for BIOL 1111"));
    assert_eq!(title("CS 4550"), Some("Web Development"));
}

#[test]
fn ruleIDsAreStable() {
    let first: Vec<String> = requirements()
        .all_rules()
        .into_iter()
        .map(|r| r.id.clone())
        .collect();
    let second: Vec<String> = requirements()
        .all_rules()
        .into_iter()
        .map(|r| r.id.clone())
        .collect();
    assert_eq!(first, second);
    assert_eq!(first.iter().collect::<HashSet<_>>().len(), first.len());
    assert!(first
        .iter()
        .any(|id| id == "major-requirements/required-courses/cs-4500-or-cs-4530"));
}

#[test]
fn readsEditionsFromTheSite() {
    let page = r#"<header><span class="edition">Academic Catalog 2031-2032</span></header>
        <a href="/archive/2030-2031">2030-2031 Catalog</a>
        <a href="https://catalog.northeastern.edu/archive/2029-2030/">2029-2030 Catalog</a>
        <a href="https://catalog.northeastern.edu/pdf/2029-2030.pdf">PDF</a>"#;
    assert_eq!(
        program_parser::current_edition(page),
        Some(CatalogYear::new(2031))
    );
    assert_eq!(
        program_parser::archived_editions(page),
        vec![CatalogYear::new(2030), CatalogYear::new(2029)]
    );
}

#[test]
fn editionURLsFollowTheSite() {
    let editions = CatalogEditions {
        current: CatalogYear::new(2031),
        archived: vec![CatalogYear::new(2030)],
        fetched_at: 0.0,
        is_estimated: false,
    };
    let path = "undergraduate/computer-information-science/computer-science/bscs/";
    assert_eq!(editions.url(CatalogYear::new(2031), path), "https://catalog.northeastern.edu/undergraduate/computer-information-science/computer-science/bscs/");
    assert_eq!(editions.url(CatalogYear::new(2030), path), "https://catalog.northeastern.edu/archive/2030-2031/undergraduate/computer-information-science/computer-science/bscs/");
    assert_eq!(
        CatalogYear::parse("2030–2031"),
        Some(CatalogYear::new(2030))
    );
    assert_eq!(
        CatalogYear::parse("2030-31").unwrap().to_string(),
        "2030–2031"
    );
    assert_eq!(CatalogYear::containing(2040, 6), CatalogYear::new(2039));
    assert_eq!(CatalogYear::containing(2040, 10), CatalogYear::new(2040));
    assert_eq!(
        CatalogEditions::estimated(2040, 10)
            .archived
            .last()
            .copied(),
        Some(CatalogYear::new(2016))
    );
}

#[test]
fn listsProgramsFromTheIndex() {
    let index = r###"<a href="/archive/2023-2024/undergraduate/computer-information-science/computer-science/bscs/">Computer Science, BSCS</a>
        <a href="/graduate/computer-information-science/computer-science/computer-science-ms/">Computer Science, MS (Boston)</a>
        <a href="/undergraduate/computer-information-science/computer-science/computer-science-minor/">Computer Science Minor</a>
        <a href="/undergraduate/about/">About, The University</a>
        <a href="#programs">Programs, AZ</a>"###;
    let programs = program_parser::programs_from_az(index);
    assert_eq!(
        programs
            .iter()
            .map(|p| p.title.as_str())
            .collect::<Vec<_>>(),
        ["Computer Science, BSCS", "Computer Science, MS (Boston)"]
    );
    assert_eq!(
        programs[0].path,
        "undergraduate/computer-information-science/computer-science/bscs/"
    );
    assert_eq!(programs[1].level(), ProgramLevel::Graduate);
    assert_eq!(programs[1].degree().as_deref(), Some("MS"));
    assert_eq!(programs[1].campus().as_deref(), Some("Boston"));
    assert_eq!(
        program_parser::suggested_programs(Some("CS-B-CSCI-BSCS"), &programs)
            .iter()
            .map(|p| p.degree().unwrap())
            .collect::<Vec<_>>(),
        ["BSCS"]
    );
}

#[test]
fn auditMatchesCoursesToRequirements() {
    let audit = run(&plan(), &requirements());
    assert!(!audit.needs_concentration);
    assert_eq!(
        status_of(&audit, "CS 1800 and CS 1802"),
        Some(AuditStatus::Complete)
    );
    assert_eq!(status_of(&audit, "CS 3000"), Some(AuditStatus::Complete));
    assert_eq!(
        status_of(&audit, "CS 4500 or CS 4530"),
        Some(AuditStatus::Missing)
    );
    assert_eq!(
        status_of(&audit, "Complete one of the following:"),
        Some(AuditStatus::Complete)
    );
    assert_eq!(status_of(&audit, "CS 2484"), Some(AuditStatus::Complete));
    let hcc = audit
        .rule_results()
        .into_iter()
        .find(|r| r.rule.label == "Complete two of the following not already taken:")
        .unwrap();
    assert_eq!(hcc.status, AuditStatus::Partial);
    assert_eq!(hcc.remaining_count, Some(1));
    let electives = audit
        .rule_results()
        .into_iter()
        .find(|r| r.rule.label.starts_with("Complete 8 semester hours of CS"))
        .unwrap();
    assert_eq!(electives.status, AuditStatus::Complete);
    assert_eq!(
        electives
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<HashSet<_>>(),
        HashSet::from(["CS 4973".into(), "CS 4535".into()])
    );
    assert_eq!(
        status_of(
            &audit,
            "Complete two courses (and any required labs) from the following science categories:"
        ),
        Some(AuditStatus::Missing)
    );
    assert!(!audit
        .sections
        .iter()
        .any(|s| s.section.title == "Software Concentration"));
    assert_eq!(audit.completed_hours, 34.0);
    assert_eq!(audit.planned_hours, 16.0);
    assert_eq!(audit.required_hours, Some(52.0));
}

#[test]
fn substitutionsAndPetitions() {
    let requirements = requirements();
    let mut plan = plan();
    let hcc = rule_named(
        &requirements,
        "Complete two of the following not already taken:",
    );
    let engineering = rule_named(&requirements, "CS 4500 or CS 4530");
    let science = rule_prefix(&requirements, "Complete two courses");
    let electives = rule_prefix(&requirements, "Complete 8 semester hours of CS");
    let software = rule_named(&requirements, "CS 4400");
    plan.substitutions = vec![
        sub(
            &hcc.id,
            "HCI electives",
            SubstitutionChange::CountCourse(key("CS 4973")),
            "",
        ),
        sub(
            &engineering.id,
            "Software engineering",
            SubstitutionChange::CountCourse(key("CS 4535")),
            "",
        ),
        sub(
            &science.id,
            "Science Requirement",
            replace(
                Some(2),
                Some(&electives.id),
                "Khoury Elective Courses",
                None,
            ),
            "Petition approved",
        ),
        sub(&software.id, "CS 4400", SubstitutionChange::Waive, ""),
        sub(
            &electives.id,
            "Electives",
            SubstitutionChange::CountCourse(key("CS 4999")),
            "",
        ),
    ];
    let audit = run(&plan, &requirements);
    let hcc_result = audit.result_for(&hcc.id).unwrap();
    assert_eq!(hcc_result.status, AuditStatus::Complete);
    assert!(hcc_result
        .used
        .iter()
        .any(|u| u.key.to_string() == "CS 4973" && u.by_substitution));
    let engineering_result = audit.result_for(&engineering.id).unwrap();
    assert_eq!(engineering_result.status, AuditStatus::Complete);
    assert_eq!(
        engineering_result
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<Vec<_>>(),
        ["CS 4535"]
    );
    let elective_result = audit.result_for(&electives.id).unwrap();
    assert_eq!(elective_result.status, AuditStatus::Planned);
    assert_eq!(
        elective_result
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<HashSet<_>>(),
        HashSet::from(["DS 3000".into(), "CS 4410".into()])
    );
    let science_result = audit.result_for(&science.id).unwrap();
    assert_eq!(science_result.status, AuditStatus::Planned);
    assert_eq!(
        science_result
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<HashSet<_>>(),
        HashSet::from(["CS 3800".into(), "CS 4100".into()])
    );
    assert_eq!(
        science_result
            .substitutions
            .first()
            .map(|s| s.note.as_str()),
        Some("Petition approved")
    );
    assert_eq!(
        audit
            .unmatched_substitutions
            .iter()
            .map(|s| s.rule_title.as_str())
            .collect::<HashSet<_>>(),
        HashSet::from(["CS 4400", "Electives"])
    );
    let counted: Vec<CourseKey> = audit
        .rule_results()
        .into_iter()
        .flat_map(|r| r.used.iter().map(|u| u.key.clone()))
        .collect();
    assert_eq!(counted.len(), counted.iter().collect::<HashSet<_>>().len());
}

#[test]
fn shortfallLandsOnTheHoursRequirement() {
    let requirements = requirements();
    let mut plan = DegreePlan {
        name: "Short".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![
        course("CS 1800", 4.0, ""),
        course("CS 1802", 1.0, ""),
        course("CS 3000", 4.0, ""),
        course("CY 2550", 4.0, ""),
    ];
    let mut spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    spring.courses = vec![
        course("CS 3386", 4.0, ""),
        course("CS 4050", 4.0, ""),
        course("CS 2963", 1.0, ""),
    ];
    plan.terms = vec![spring];
    plan.concentration_id = Some("concentration-in-human-centered-computing".into());
    let science = rule_prefix(&requirements, "Complete two courses");
    let electives = rule_prefix(&requirements, "Complete 8 semester hours of CS");
    plan.substitutions = vec![sub(
        &science.id,
        "Science Requirement",
        replace(
            Some(2),
            Some(&electives.id),
            "Khoury Elective Courses",
            None,
        ),
        "",
    )];
    let audit = run(&plan, &requirements);
    let science_result = audit.result_for(&science.id).unwrap();
    assert_eq!(science_result.status, AuditStatus::Planned);
    assert_eq!(
        science_result
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<HashSet<_>>(),
        HashSet::from(["CS 3386".into(), "CS 4050".into()])
    );
    let elective_result = audit.result_for(&electives.id).unwrap();
    assert_eq!(elective_result.status, AuditStatus::Partial);
    assert_eq!(
        elective_result
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<Vec<_>>(),
        ["CS 2963"]
    );
    assert_eq!(elective_result.remaining_hours, Some(7.0));
}

#[test]
fn rebalancesToMeetShortRequirements() {
    let page = r#"<div id="programrequirementstextcontainer" class="tab_content">
        <h2>Major Requirements</h2>
        <table class="sc_courselist"><tbody>
        <tr class="even areaheader firstrow"><td colspan="2"><span class="courselistcomment areaheader">Computing and Social Issues</span></td><td class="hourscol"></td></tr>
        <tr class="odd"><td colspan="2"><span class="courselistcomment">Complete one of the following:</span></td><td class="hourscol">4</td></tr>
        <tr class="even"><td class="codecol"><a class="bubblelink code">CY&#160;4170</a></td><td>The Law, Ethics, and Policy of Data and Digital Technologies</td><td class="hourscol"></td></tr>
        <tr class="odd"><td class="codecol"><a class="bubblelink code">HIST&#160;2220</a></td><td>History of Technology</td><td class="hourscol"></td></tr>
        <tr class="even areaheader"><td colspan="2"><span class="courselistcomment areaheader">Humanities</span></td><td class="hourscol"></td></tr>
        <tr class="odd"><td colspan="2"><span class="courselistcomment">Complete one of the following:</span></td><td class="hourscol">4</td></tr>
        <tr class="even"><td class="codecol"><a class="bubblelink code">HIST&#160;2220</a></td><td>History of Technology</td><td class="hourscol"></td></tr>
        <tr class="odd"><td class="codecol"><a class="bubblelink code">PHIL&#160;1145</a></td><td>Technology and Human Values</td><td class="hourscol"></td></tr>
        <tr class="even areaheader"><td colspan="2"><span class="courselistcomment areaheader">Khoury Elective Courses</span></td><td class="hourscol"></td></tr>
        <tr class="odd"><td colspan="2"><span class="courselistcomment">Complete 8 semester hours of CS, CY, DS, or IS classes that are not already required. Choose courses within the following ranges:</span></td><td class="hourscol">8</td></tr>
        <tr class="even"><td colspan="2"><div style="margin-left:20px;" class="blockindent"><span class="courselistcomment commentindent"><a class="bubblelink code">CS&#160;2500</a> or higher</span></div></td><td class="hourscol"></td></tr>
        <tr class="odd lastrow"><td colspan="2"><div style="margin-left:20px;" class="blockindent"><span class="courselistcomment commentindent"><a class="bubblelink code">CY&#160;2000</a> or higher</span></div></td><td class="hourscol"></td></tr>
        </tbody></table>
        <h2>Required General Electives</h2>
        <table class="sc_courselist"><tbody>
        <tr class="even lastrow firstrow"><td colspan="2"><span class="courselistcomment">Complete 4 semester hours of general electives.</span></td><td class="hourscol">4</td></tr>
        </tbody></table>
        </div>"#;
    let requirements =
        program_parser::requirements_from_html(page, CatalogYear::new(2024), bscs()).unwrap();
    let rules = requirements.all_rules();
    assert_eq!(rules.len(), 4);
    let (social, humanities, electives) = (rules[0], rules[1], rules[2]);
    let audit_of = |completed: &[&str], planned: &[&str]| {
        let mut plan = DegreePlan {
            name: "Plan".into(),
            ..DegreePlan::default()
        };
        plan.completed = completed.iter().map(|code| course(code, 4.0, "")).collect();
        let mut spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
        spring.courses = planned.iter().map(|code| course(code, 4.0, "")).collect();
        plan.terms = vec![spring];
        run(&plan, &requirements)
    };
    let courses_of = |audit: &DegreeAudit, rule: &RequirementRule| {
        audit
            .result_for(&rule.id)
            .unwrap()
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<HashSet<_>>()
    };
    let swap = audit_of(
        &["CY 4170", "CS 3200", "PHIL 1145"],
        &["HIST 2220", "MKTG 4510"],
    );
    assert_eq!(swap.satisfied_count(), swap.total_count());
    assert_eq!(
        courses_of(&swap, electives),
        HashSet::from(["CS 3200".into(), "CY 4170".into()])
    );
    assert_eq!(
        courses_of(&swap, social),
        HashSet::from(["HIST 2220".into()])
    );
    assert_eq!(
        courses_of(&swap, humanities),
        HashSet::from(["PHIL 1145".into()])
    );
    assert_eq!(
        swap.general_electives
            .iter()
            .map(|u| u.key.to_string())
            .collect::<Vec<_>>(),
        ["MKTG 4510"]
    );
    let chain = audit_of(
        &["CY 4170", "CS 3200", "HIST 2220"],
        &["PHIL 1145", "MKTG 4510"],
    );
    assert_eq!(chain.satisfied_count(), chain.total_count());
    assert_eq!(
        courses_of(&chain, electives),
        HashSet::from(["CS 3200".into(), "CY 4170".into()])
    );
    assert_eq!(
        chain.result_for(&electives.id).unwrap().status,
        AuditStatus::Complete
    );
    assert_eq!(
        courses_of(&chain, social),
        HashSet::from(["HIST 2220".into()])
    );
    assert_eq!(
        courses_of(&chain, humanities),
        HashSet::from(["PHIL 1145".into()])
    );
    let counted: Vec<CourseKey> = chain
        .rule_results()
        .into_iter()
        .flat_map(|r| r.used.iter().map(|u| u.key.clone()))
        .collect();
    assert_eq!(counted.len(), counted.iter().collect::<HashSet<_>>().len());
    let short = audit_of(&["CY 4170", "CS 3200"], &["MKTG 4510"]);
    assert_eq!(
        short.result_for(&social.id).unwrap().status,
        AuditStatus::Complete
    );
    assert_eq!(
        short.result_for(&electives.id).unwrap().remaining_hours,
        Some(4.0)
    );
}

#[test]
fn petitionsAskWhetherCourseNumberMinimumsApply() {
    let requirements = requirements();
    let electives = rule_prefix(&requirements, "Complete 8 semester hours of CS");
    let science = rule_prefix(&requirements, "Complete two courses");
    let mut plan = DegreePlan {
        name: "Petition".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![
        course("CS 1800", 4.0, ""),
        course("CS 1802", 1.0, ""),
        course("CS 3000", 4.0, ""),
        course("CS 4500", 4.0, ""),
        course("CY 2550", 4.0, ""),
        course("CS 2386", 4.0, "Game Programming 1"),
        course("CS 2484", 4.0, ""),
        course("CS 4520", 4.0, ""),
        course("CS 4550", 4.0, ""),
    ];
    let mut spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    spring.courses = vec![
        course("CS 3386", 4.0, "Game Programming 2"),
        course("CS 3200", 4.0, ""),
        course("CS 4100", 4.0, ""),
        course("MKTG 2602", 4.0, ""),
        course("PHIL 1102", 4.0, ""),
    ];
    plan.terms = vec![spring];
    plan.concentration_id = Some("concentration-in-human-centered-computing".into());
    let petition = |limit: Option<CourseNumberLimit>| {
        sub(
            &science.id,
            "Science Requirement",
            replace(
                Some(2),
                Some(&electives.id),
                "Khoury Elective Courses",
                limit,
            ),
            "",
        )
    };
    plan.substitutions = vec![petition(None)];
    let unanswered = run(&plan, &requirements);
    assert_eq!(
        unanswered.result_for(&electives.id).unwrap().status,
        AuditStatus::Partial
    );
    let questions = petition::questions(
        &plan,
        &requirements,
        &CourseEquivalences::none(),
        Some(&unanswered),
    );
    assert_eq!(questions.len(), 1);
    let question = &questions[0];
    assert_eq!(question.courses, vec![key("CS 2386")]);
    assert_eq!(
        question.question(),
        "Does your Science Requirement petition accept any CS or DS course?"
    );
    assert!(
        question
            .explanation()
            .contains("start at CS 2500 or DS 2500"),
        "{}",
        question.explanation()
    );
    assert!(
        question.explanation().contains("CS 2386 counts"),
        "{}",
        question.explanation()
    );
    plan.substitutions = vec![petition(Some(CourseNumberLimit::AnyNumber))];
    let any_number = run(&plan, &requirements);
    assert!(any_number
        .result_for(&science.id)
        .unwrap()
        .used
        .iter()
        .any(|u| u.key.to_string() == "CS 2386"));
    assert_eq!(
        any_number.result_for(&electives.id).unwrap().status,
        AuditStatus::Planned
    );
    assert!(
        petition::questions(&plan, &requirements, &CourseEquivalences::none(), None).is_empty()
    );
    match any_number
        .result_for(&science.id)
        .unwrap()
        .effective_kind
        .clone()
    {
        Some(RuleKind::Choose { ranges, .. }) => {
            assert_eq!(
                ranges.iter().map(|r| r.label.clone()).collect::<Vec<_>>(),
                [
                    "Any CS course, except CS 5010",
                    "DS courses up to DS 4999, except DS 4900"
                ]
            );
        }
        other => panic!("the petition should take courses like the electives: {other:?}"),
    }
    plan.substitutions = vec![petition(Some(CourseNumberLimit::AtLeast(2000)))];
    assert_eq!(
        run(&plan, &requirements)
            .result_for(&electives.id)
            .unwrap()
            .status,
        AuditStatus::Planned
    );
    plan.substitutions = vec![petition(Some(CourseNumberLimit::AtLeast(3000)))];
    assert_eq!(
        run(&plan, &requirements)
            .result_for(&electives.id)
            .unwrap()
            .status,
        AuditStatus::Partial
    );
    plan.substitutions = vec![petition(Some(CourseNumberLimit::SameAsRequirement))];
    assert!(
        petition::questions(&plan, &requirements, &CourseEquivalences::none(), None).is_empty()
    );
    let same = run(&plan, &requirements);
    let miss = &same.near_misses(&electives.id)[0];
    assert_eq!(miss.course.key.to_string(), "CS 2386");
    assert_eq!(miss.reason, "Below CS 2500");
    let old =
        r#"{"count":2,"likeRuleID":"x","likeRuleTitle":"Khoury Elective Courses","courses":[]}"#;
    let decoded: RequirementReplacement = serde_json::from_str(old).unwrap();
    assert!(decoded.course_numbers.is_none());
    let summary = RequirementReplacement {
        count: Some(1),
        hours: None,
        like_rule_id: None,
        like_rule_title: Some("Khoury Elective Courses".into()),
        courses: vec![],
        course_numbers: Some(CourseNumberLimit::AnyNumber),
    };
    assert_eq!(
        summary.summary(),
        "1 course like Khoury Elective Courses, any course number"
    );
}

#[test]
fn topicsCoursesCountEachTime() {
    let requirements = requirements();
    let electives = rule_prefix(&requirements, "Complete 8 semester hours of CS");
    let mut plan = DegreePlan {
        name: "Topics".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![
        course("CS 1800", 4.0, ""),
        course("CS 1802", 1.0, ""),
        course("CS 3000", 4.0, ""),
        course("CS 4500", 4.0, ""),
        course("CY 2550", 4.0, ""),
        course("CS 4973", 4.0, "Topics in Computer Science"),
        course("MKTG 2602", 4.0, "Marketing Management"),
    ];
    let mut spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    spring.courses = vec![
        course("CS 4973", 4.0, "Topics in Computer Science"),
        course("MKTG 2602", 4.0, "Marketing Management"),
    ];
    plan.terms = vec![spring];
    let audit = run(&plan, &requirements);
    let result = audit.result_for(&electives.id).unwrap();
    assert_eq!(result.status, AuditStatus::Planned);
    let mut ids: Vec<String> = result.used.iter().map(|u| u.id()).collect();
    ids.sort();
    assert_eq!(ids, ["CS 4973", "CS 4973 (2)"]);
    let counted: Vec<&CourseUse> = audit
        .rule_results()
        .into_iter()
        .flat_map(|r| r.used.iter())
        .chain(audit.unused.iter())
        .collect();
    assert_eq!(
        counted
            .iter()
            .filter(|u| u.key.to_string() == "MKTG 2602")
            .count(),
        1
    );
    assert_eq!(
        counted.iter().map(|u| u.id()).collect::<HashSet<_>>().len(),
        counted.len()
    );
    assert_eq!(
        planlogic::duplicate_keys(&plan),
        HashSet::from([key("MKTG 2602")])
    );
}

#[test]
fn requiredAlternativesMakeRoom() {
    let page = r#"<div id="programrequirementstextcontainer" class="tab_content">
        <h2>Major Requirements</h2>
        <table class="sc_courselist"><tbody>
        <tr class="even areaheader firstrow"><td colspan="2"><span class="courselistcomment areaheader">Software Engineering</span></td><td class="hourscol"></td></tr>
        <tr class="odd"><td class="codecol"><a class="bubblelink code">CS&#160;4500</a></td><td>Software Development</td><td class="hourscol">4</td></tr>
        <tr class="orclass even"><td class="codecol orclass">or <a class="bubblelink code">CS&#160;4530</a></td><td colspan="2">Fundamentals of Software Engineering</td></tr>
        <tr class="odd areaheader"><td colspan="2"><span class="courselistcomment areaheader">Concentration Courses</span></td><td class="hourscol"></td></tr>
        <tr class="even"><td colspan="2"><span class="courselistcomment">Complete two of the following:</span></td><td class="hourscol">8</td></tr>
        <tr class="odd"><td class="codecol"><a class="bubblelink code">CS&#160;4530</a></td><td>Fundamentals of Software Engineering</td><td class="hourscol"></td></tr>
        <tr class="even lastrow"><td class="codecol"><a class="bubblelink code">CS&#160;4550</a></td><td>Web Development</td><td class="hourscol"></td></tr>
        </tbody></table>
        </div>"#;
    let requirements =
        program_parser::requirements_from_html(page, CatalogYear::new(2024), bscs()).unwrap();
    let rules = requirements.all_rules();
    assert_eq!(rules.len(), 2);
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![course("CS 4530", 4.0, ""), course("CS 4550", 4.0, "")];
    let mut spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    spring.courses = vec![course("CS 4500", 4.0, "")];
    plan.terms = vec![spring];
    let audit = run(&plan, &requirements);
    assert_eq!(audit.satisfied_count(), 2);
    assert_eq!(
        audit
            .result_for(&rules[0].id)
            .unwrap()
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<Vec<_>>(),
        ["CS 4500"]
    );
    assert_eq!(
        audit
            .result_for(&rules[1].id)
            .unwrap()
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<HashSet<_>>(),
        HashSet::from(["CS 4530".into(), "CS 4550".into()])
    );
}

#[test]
fn substitutionsFollowRequirementsAcrossCatalogYears() {
    let requirements = requirements();
    let science = rule_prefix(&requirements, "Complete two courses");
    let electives = rule_prefix(&requirements, "Complete 8 semester hours of CS");
    let mut plan = plan();
    plan.substitutions = vec![
        sub(
            "supporting-courses/science-requirement/complete-any-two-courses-from-last-year",
            "Science Requirement",
            replace(
                Some(2),
                Some("major-requirements/khoury-elective-courses/older-wording"),
                "Khoury Elective Courses",
                Some(CourseNumberLimit::SameAsRequirement),
            ),
            "",
        ),
        sub(
            "gone",
            "A requirement this year doesn't have",
            SubstitutionChange::Waive,
            "",
        ),
    ];
    let audit = run(&plan, &requirements);
    let result = audit.result_for(&science.id).unwrap();
    assert_eq!(result.substitutions.len(), 1);
    assert_eq!(result.status, AuditStatus::Complete);
    assert_eq!(
        result
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<HashSet<_>>(),
        HashSet::from(["CS 4973".into(), "CS 4535".into()])
    );
    assert_eq!(
        audit
            .placements
            .get(&plan.substitutions[0].id)
            .map(|s| s.as_str()),
        Some(science.id.as_str())
    );
    assert_eq!(
        audit
            .unmatched_substitutions
            .iter()
            .map(|s| s.rule_title.as_str())
            .collect::<Vec<_>>(),
        ["A requirement this year doesn't have"]
    );
    if let SubstitutionChange::Replace(replacement) = &plan.substitutions[0].change {
        assert_eq!(replacement.like_ranges(&requirements).len(), 2);
    } else {
        panic!("expected a replacement");
    }
    assert_eq!(
        audit.result_for(&electives.id).unwrap().status,
        AuditStatus::Planned
    );
}

#[test]
fn hourRequirementsKeepOnlyWhatTheyNeed() {
    let requirements = requirements();
    let electives = rule_prefix(&requirements, "Complete 8 semester hours of CS");
    let mut plan = DegreePlan {
        name: "Hours".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![
        course("CS 1800", 4.0, ""),
        course("CS 1802", 1.0, ""),
        course("CS 3000", 4.0, ""),
        course("CS 4500", 4.0, ""),
        course("CY 2550", 4.0, ""),
        course("CS 4100", 4.0, ""),
        course("MATH 1341", 4.0, ""),
        course("PHIL 1102", 4.0, ""),
    ];
    let mut spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    spring.courses = vec![course("CS 2963", 1.0, "Topics")];
    let mut summer = DegreePlanTerm::new(PlanSeason::SummerB, 2027);
    summer.courses = vec![course("CS 3200", 4.0, "")];
    plan.terms = vec![spring, summer];
    let audit = run(&plan, &requirements);
    let result = audit.result_for(&electives.id).unwrap();
    assert_eq!(result.status, AuditStatus::Planned);
    assert_eq!(
        result
            .used
            .iter()
            .map(|u| u.key.to_string())
            .collect::<HashSet<_>>(),
        HashSet::from(["CS 4100".into(), "CS 3200".into()])
    );
    assert_eq!(result.used.iter().map(|u| u.credits).sum::<f64>(), 8.0);
    assert_eq!(
        audit
            .general_electives
            .iter()
            .map(|u| u.key.to_string())
            .collect::<HashSet<_>>(),
        HashSet::from(["MATH 1341".into(), "PHIL 1102".into()])
    );
    assert_eq!(
        audit
            .unused
            .iter()
            .map(|u| u.key.to_string())
            .collect::<Vec<_>>(),
        ["CS 2963"]
    );
    let uses = [("A", 5.0, true), ("B", 4.0, false), ("C", 4.0, false)]
        .into_iter()
        .enumerate()
        .map(|(index, (title, credits, completed))| {
            use_of(
                &format!("CS {}", 4000 + index),
                title,
                credits,
                if completed { None } else { Some("Spring 2027") },
                index as i32,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        DegreeAudit::fewest_credits_choice(&uses, 8.0).unwrap(),
        HashSet::from(["CS 4001".into(), "CS 4002".into()])
    );
}

#[test]
fn checksHoursInTheMajorAndCoop() {
    let page = r#"<div id="programrequirementstextcontainer" class="tab_content">
        <h2>Major Requirements</h2>
        <table class="sc_courselist"><tbody>
        <tr class="even firstrow"><td class="codecol"><a class="bubblelink code">CS&#160;3000</a></td><td>Algorithms and Data</td><td class="hourscol">4</td></tr>
        <tr class="odd"><td colspan="2"><span class="courselistcomment">Complete 8 semester hours of CS classes. Choose courses within the following ranges:</span></td><td class="hourscol">8</td></tr>
        <tr class="even lastrow"><td colspan="2"><div style="margin-left:20px;" class="blockindent"><span class="courselistcomment commentindent"><a class="bubblelink code">CS&#160;2500</a> or higher</span></div></td><td class="hourscol"></td></tr>
        </tbody></table>
        <h2>Writing Requirement</h2>
        <table class="sc_courselist"><tbody>
        <tr class="even firstrow lastrow"><td class="codecol"><a class="bubblelink code">ENGW&#160;1111</a></td><td>First-Year Writing</td><td class="hourscol">4</td></tr>
        </tbody></table>
        <h2>Required General Electives</h2>
        <table class="sc_courselist"><tbody>
        <tr class="even lastrow firstrow"><td colspan="2"><span class="courselistcomment">Complete 4 semester hours of general electives.</span></td><td class="hourscol">4</td></tr>
        </tbody></table>
        <h2>GPA Requirement</h2>
        <p>Minimum 2.000 GPA required in the major.</p>
        <h2>Credit Requirement</h2>
        <p>Complete 16 semester hours in the major.</p>
        <h2>NUpath Requirements Satisfied</h2>
        <p>Integrating Knowledge and Skills Through Experience is satisfied through co-op.</p>
        </div>"#;
    let requirements =
        program_parser::requirements_from_html(page, CatalogYear::new(2024), bscs()).unwrap();
    let audit_of = |completed: &[&str], planned: &[&str]| {
        let mut plan = DegreePlan {
            name: "Checks".into(),
            ..DegreePlan::default()
        };
        plan.completed = completed
            .iter()
            .map(|code| {
                if *code == "COOP 3946" {
                    course(code, 0.0, "Co-op Work Experience - HT")
                } else {
                    course(code, 4.0, "")
                }
            })
            .collect();
        let mut fall = DegreePlanTerm::new(PlanSeason::Fall, 2027);
        fall.courses = planned.iter().map(|code| course(code, 4.0, "")).collect();
        plan.terms = vec![fall];
        run(&plan, &requirements)
    };
    let done = audit_of(
        &[
            "CS 3000",
            "CS 3200",
            "CS 3500",
            "CS 4100",
            "ENGW 1111",
            "MKTG 2602",
            "COOP 3946",
            "CS 1210",
        ],
        &[],
    );
    assert_eq!(done.note_checks.len(), 2);
    let hours = done
        .note_checks
        .iter()
        .find(|c| matches!(c.kind, NoteKind::MajorHours { .. }))
        .unwrap();
    assert_eq!(hours.status, AuditStatus::Complete);
    assert_eq!(hours.detail, "16 of 16 semester hours");
    let coop = done
        .note_checks
        .iter()
        .find(|c| matches!(c.kind, NoteKind::Coop))
        .unwrap();
    assert_eq!(coop.status, AuditStatus::Complete);
    assert_eq!(coop.detail, "COOP 3946 completed");
    assert_eq!(done.satisfied_count(), done.total_count());
    let short = audit_of(&["CS 3000", "CS 3200", "CS 1210"], &["CS 3500"]);
    assert_eq!(
        short
            .note_checks
            .iter()
            .find(|c| matches!(c.kind, NoteKind::MajorHours { .. }))
            .unwrap()
            .status,
        AuditStatus::Partial
    );
    assert_eq!(
        short
            .note_checks
            .iter()
            .find(|c| matches!(c.kind, NoteKind::Coop))
            .unwrap()
            .detail,
        "No co-op in the plan"
    );
    assert!(short.satisfied_count() < short.total_count());
}

#[test]
fn waiversAndConcentrationChoice() {
    let requirements = requirements();
    let mut plan = plan();
    let security = rule_named(&requirements, "Complete one of the following:");
    plan.substitutions = vec![sub(
        &security.id,
        "Security Course",
        SubstitutionChange::Waive,
        "",
    )];
    plan.concentration_id = None;
    let audit = run(&plan, &requirements);
    assert_eq!(
        audit.result_for(&security.id).unwrap().status,
        AuditStatus::Waived
    );
    assert!(audit.needs_concentration);
    assert!(!audit.sections.iter().any(|s| s.section.is_concentration));
    assert_ne!(
        audit
            .rule_counting(&key("CY 2550"))
            .map(|r| r.rule.id.as_str()),
        Some(security.id.as_str())
    );
}

#[test]
fn userDataSkipsUnreadableEntries() {
    let mut data = UserData::default();
    data.degree_plans = vec![
        plan(),
        DegreePlan {
            name: "Second".into(),
            ..DegreePlan::default()
        },
    ];
    let mut json = serde_json::to_value(&data).unwrap();
    json["degreePlans"][0]["completed"] = serde_json::Value::String("not a list".into());
    json["searchHistory"] = serde_json::json!(["cs 3000", 42, "orgo"]);
    let decoded = UserData::from_json_lossy(&json.to_string());
    assert_eq!(
        decoded
            .degree_plans
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["Second"]
    );
    assert_eq!(decoded.search_history, ["cs 3000", "orgo"]);
}

#[test]
fn olderSavedPlansStillDecode() {
    let mut plan = plan();
    plan.substitutions = vec![sub("x", "X", SubstitutionChange::Waive, "")];
    let mut json = serde_json::to_value(&plan).unwrap();
    for key in [
        "substitutions",
        "catalogYear",
        "catalogProgramPath",
        "catalogProgramTitle",
        "concentrationID",
    ] {
        json.as_object_mut().unwrap().remove(key);
    }
    let decoded: DegreePlan = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.name, "Plan");
    assert_eq!(decoded.completed.len(), plan.completed.len());
    assert!(decoded.substitutions.is_empty());
    assert!(decoded.catalog_year.is_none());
    let round_trip: DegreePlan =
        serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    assert_eq!(round_trip, plan);
}

fn nupath_program() -> CatalogProgram {
    CatalogProgram {
        title: "Computer Science, BSCS (Boston)".into(),
        path: "undergraduate/computer-information-science/computer-science/bscs/".into(),
    }
}

fn nupath_requirements(universitywide: Option<UniversitywideRequirements>) -> ProgramRequirements {
    let mut requirements = program_parser::requirements_from_html(
        NUPATH_PAGE,
        CatalogYear::new(2024),
        nupath_program(),
    )
    .unwrap();
    requirements.universitywide = universitywide;
    requirements
}

fn nupath_plan() -> DegreePlan {
    let mut plan = DegreePlan {
        name: "NUpath".into(),
        ..DegreePlan::default()
    };
    plan.completed = [
        "ENGW 1111",
        "MATH 1341",
        "PHYS 1151",
        "CS 3200",
        "PHIL 1145",
        "ARTH 1111",
        "HIST 1130",
        "CS 3000",
    ]
    .iter()
    .map(|code| course(code, 4.0, ""))
    .collect();
    plan.completed
        .push(course("COOP 3946", 0.0, "Co-op Work Experience - HT"));
    let mut spring = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    spring.courses = vec![
        course("ENGW 3302", 4.0, ""),
        course("CS 4535", 4.0, ""),
        course("CS 3484", 4.0, ""),
    ];
    plan.terms = vec![spring];
    plan
}

fn nupath_table() -> Vec<(&'static str, Vec<NuPath>)> {
    vec![
        ("ENGW 1111", vec![NuPath::Wf]),
        ("MATH 1341", vec![NuPath::Fq]),
        ("PHYS 1151", vec![NuPath::Nd]),
        ("CS 3200", vec![NuPath::Ad, NuPath::Fq]),
        ("PHIL 1145", vec![NuPath::Er, NuPath::Si]),
        ("ARTH 1111", vec![NuPath::Ic, NuPath::Ei]),
        ("HIST 1130", vec![NuPath::Dd, NuPath::Si]),
        ("CS 3000", vec![NuPath::Fq]),
        ("ENGW 3302", vec![NuPath::Wd]),
        ("CS 4535", vec![NuPath::Ce, NuPath::Ex, NuPath::Wi]),
        ("CS 3484", vec![NuPath::Wi]),
    ]
}

fn attributes_of(table: &[(&str, Vec<NuPath>)], loading: &[&str]) -> DegreeCourseAttributes {
    DegreeCourseAttributes {
        nupaths: table
            .iter()
            .map(|(code, paths)| (key(code), paths.iter().copied().collect()))
            .collect(),
        loading: loading.iter().map(|code| key(code)).collect(),
    }
}

fn nupath_of(audit: &DegreeAudit) -> Option<&NoteCheck> {
    audit
        .note_checks
        .iter()
        .find(|check| matches!(check.kind, NoteKind::Nupath))
}

fn nupath_item(check: Option<&NoteCheck>, path: NuPath) -> Option<&degree_audit::NoteItem> {
    check.and_then(|check| check.items.iter().find(|item| item.id == path.code()))
}

#[test]
fn everyNUpathRequirementIsChecked() {
    let audit = run_attr(
        &nupath_plan(),
        &nupath_requirements(None),
        &attributes_of(&nupath_table(), &[]),
    );
    let check = nupath_of(&audit).unwrap();
    assert_eq!(
        check
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        NuRequirement::ALL
            .iter()
            .map(|req| req.path.code())
            .collect::<Vec<_>>()
    );
    assert!(check
        .items
        .iter()
        .all(|item| item.status.is_some_and(|status| status.is_satisfied())));
    assert_eq!(check.status, AuditStatus::Planned);
    assert!(check.counts);
    assert_eq!(check.detail, "13 of 13 met");
    assert_eq!(
        nupath_item(Some(check), NuPath::Wf).unwrap().detail,
        "ENGW 1111 completed"
    );
    assert_eq!(
        nupath_item(Some(check), NuPath::Wi).unwrap().courses,
        ["CS 4535", "CS 3484"]
    );
    assert_eq!(
        nupath_item(Some(check), NuPath::Wi).unwrap().detail,
        "CS 4535 planned · Spring 2027 · CS 3484 planned · Spring 2027"
    );
    assert_eq!(
        nupath_item(Some(check), NuPath::Ce).unwrap().courses,
        ["CS 4535"]
    );
    assert_eq!(
        nupath_item(Some(check), NuPath::Ex).unwrap().courses,
        ["COOP 3946"]
    );
    assert_eq!(
        nupath_item(Some(check), NuPath::Ex).unwrap().status,
        Some(AuditStatus::Complete)
    );
    let mut areas: HashMap<String, i32> = HashMap::new();
    for requirement in NuRequirement::ALL.iter().filter(|req| req.limited) {
        for id in &nupath_item(Some(check), requirement.path).unwrap().courses {
            *areas.entry(id.clone()).or_default() += 1;
        }
    }
    assert!(areas.values().all(|count| *count <= 2));
    assert!(audit
        .counted_checks()
        .iter()
        .any(|check| matches!(check.kind, NoteKind::Nupath) && check.status.is_satisfied()));
}

#[test]
fn aCourseCountsForTwoAreasAtMost() {
    let mut table = nupath_table();
    table.iter_mut().find(|row| row.0 == "ARTH 1111").unwrap().1 =
        vec![NuPath::Ic, NuPath::Ei, NuPath::Dd];
    table.iter_mut().find(|row| row.0 == "HIST 1130").unwrap().1 = vec![];
    let baseline = run_attr(
        &nupath_plan(),
        &nupath_requirements(None),
        &attributes_of(&nupath_table(), &[]),
    );
    let audit = run_attr(
        &nupath_plan(),
        &nupath_requirements(None),
        &attributes_of(&table, &[]),
    );
    let check = nupath_of(&audit).unwrap();
    let open: Vec<_> = check
        .items
        .iter()
        .filter(|item| !item.status.is_some_and(|status| status.is_satisfied()))
        .collect();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].status, Some(AuditStatus::Missing));
    assert_eq!(open[0].detail, "No course in the plan has it");
    assert_eq!(check.status, AuditStatus::Partial);
    assert!(check.counts);
    assert!(
        check.detail.starts_with("12 of 13 met · missing "),
        "{}",
        check.detail
    );
    assert_eq!(audit.satisfied_count(), baseline.satisfied_count() - 1);
    assert_eq!(audit.total_count(), baseline.total_count());
}

#[test]
fn unknownCoursesKeepAShortfallFromCounting() {
    let requirements = nupath_requirements(None);
    let mut table: Vec<(&str, Vec<NuPath>)> = nupath_table()
        .into_iter()
        .filter(|row| row.0 != "HIST 1130")
        .collect();
    let audit = run_attr(&nupath_plan(), &requirements, &attributes_of(&table, &[]));
    let check = nupath_of(&audit).unwrap();
    assert_eq!(nupath_item(Some(check), NuPath::Dd).unwrap().status, None);
    assert_eq!(
        nupath_item(Some(check), NuPath::Dd).unwrap().detail,
        "Not found among the courses with NUpath data"
    );
    assert!(!check.counts);
    assert_eq!(check.detail, "12 of 13 met · no NUpath data for HIST 1130");
    assert!(!audit
        .counted_checks()
        .iter()
        .any(|check| matches!(check.kind, NoteKind::Nupath)));
    let loading = run_attr(
        &nupath_plan(),
        &requirements,
        &attributes_of(&table, &["HIST 1130"]),
    );
    let pending = nupath_of(&loading).unwrap();
    assert_eq!(
        nupath_item(Some(pending), NuPath::Dd).unwrap().detail,
        "Looking up the plan’s courses…"
    );
    assert_eq!(pending.detail, "12 of 13 met · looking up 1 course");
    for row in &mut table {
        if row.0 == "ARTH 1111" {
            row.1 = vec![NuPath::Ic, NuPath::Dd];
        }
        if row.0 == "CS 3000" {
            row.1 = vec![NuPath::Ei];
        }
    }
    let met_audit = run_attr(&nupath_plan(), &requirements, &attributes_of(&table, &[]));
    let met = nupath_of(&met_audit).unwrap();
    assert!(met.counts && met.status == AuditStatus::Planned);
}

#[test]
fn withoutAttributesNUpathIsNotChecked() {
    assert!(nupath_of(&run(&nupath_plan(), &nupath_requirements(None))).is_none());
}

#[test]
fn aRenumberedCourseUsesItsReplacementsAttributes() {
    let mut plan = nupath_plan();
    plan.completed[2] = course("PHYS 1145", 4.0, "");
    let mut table: Vec<(&str, Vec<NuPath>)> = nupath_table()
        .into_iter()
        .filter(|row| row.0 != "PHYS 1151")
        .collect();
    table.push(("PHYS 1149", vec![NuPath::Nd]));
    plan.course_equivalences = vec![CourseEquivalence {
        id: Uuid::new_v4(),
        course: key("PHYS 1145"),
        counts_as: key("PHYS 1149"),
        note: "Renumbered".into(),
    }];
    let renamed = run_attr(
        &plan,
        &nupath_requirements(None),
        &attributes_of(&table, &[]),
    );
    let check = nupath_of(&renamed).unwrap();
    assert_eq!(
        nupath_item(Some(check), NuPath::Nd).unwrap().courses,
        ["PHYS 1145"]
    );
    assert!(check.counts);
}

#[test]
fn withoutACoopAnExperienceCourseCounts() {
    let mut plan = nupath_plan();
    plan.completed
        .retain(|course| course.key.as_ref().map(|key| key.subject.as_str()) != Some("COOP"));
    let audit = run_attr(
        &plan,
        &nupath_requirements(None),
        &attributes_of(&nupath_table(), &[]),
    );
    let coop = audit
        .note_checks
        .iter()
        .find(|check| matches!(check.kind, NoteKind::Coop))
        .unwrap();
    assert_eq!(coop.status, AuditStatus::Planned);
    assert_eq!(coop.detail, "No co-op in the plan · CS 4535 (EX) planned");
    assert_eq!(
        nupath_item(nupath_of(&audit), NuPath::Ex).unwrap().courses,
        ["CS 4535"]
    );
}

#[test]
fn matchingIsOptimal() {
    let mut generator = SplitMix64 { state: 2016 };
    let areas: Vec<NuPath> = NuRequirement::ALL
        .iter()
        .filter(|req| req.limited)
        .map(|req| req.path)
        .collect();
    for _ in 0..600 {
        let mut wanted = areas.clone();
        shuffle(&mut wanted, &mut generator);
        let count = generator.int(1, 5) as usize;
        wanted.truncate(count);
        let course_count = generator.int(1, 5) as usize;
        let courses: Vec<NuCourse> = (0..course_count)
            .map(|index| NuCourse {
                id: format!("C{index}"),
                paths: wanted
                    .iter()
                    .copied()
                    .filter(|_| generator.int(0, 9) < 4)
                    .collect(),
                is_completed: generator.bool(),
            })
            .collect();
        let requirements: Vec<NuRequirement> = wanted
            .iter()
            .copied()
            .map(|path| NuRequirement {
                path,
                count: 1,
                limited: true,
            })
            .collect();
        let matching = Matching::with_requirements(&courses, &requirements, &HashMap::new());
        let chosen: Vec<(NuPath, String)> = wanted
            .iter()
            .filter_map(|area| {
                matching
                    .courses_for(*area)
                    .into_iter()
                    .next()
                    .map(|id| (*area, id))
            })
            .collect();
        let completed = chosen
            .iter()
            .filter(|pair| {
                courses
                    .iter()
                    .find(|course| course.id == pair.1)
                    .is_some_and(|course| course.is_completed)
            })
            .count();
        assert!(chosen.iter().all(|pair| courses
            .iter()
            .find(|course| course.id == pair.1)
            .is_some_and(|course| course.paths.contains(&pair.0))));
        let mut loads: HashMap<&str, usize> = HashMap::new();
        for pair in &chosen {
            *loads.entry(pair.1.as_str()).or_default() += 1;
        }
        assert!(loads.values().all(|count| *count <= 2));
        let mut best = (0usize, 0usize);
        fn search(
            position: usize,
            wanted: &[NuPath],
            courses: &[NuCourse],
            load: &mut [usize],
            met: usize,
            done: usize,
            best: &mut (usize, usize),
        ) {
            if position >= wanted.len() {
                if (met, done) > *best {
                    *best = (met, done);
                }
                return;
            }
            search(position + 1, wanted, courses, load, met, done, best);
            for (index, course) in courses.iter().enumerate() {
                if course.paths.contains(&wanted[position]) && load[index] < 2 {
                    load[index] += 1;
                    search(
                        position + 1,
                        wanted,
                        courses,
                        load,
                        met + 1,
                        done + usize::from(course.is_completed),
                        best,
                    );
                    load[index] -= 1;
                }
            }
        }
        search(
            0,
            &wanted,
            &courses,
            &mut vec![0; courses.len()],
            0,
            0,
            &mut best,
        );
        assert_eq!(chosen.len(), best.0);
        assert_eq!(completed, best.1);
    }
}

#[test]
fn readsTheUniversitywideMinimums() {
    let archived = r#"<div id="textcontainer"><p>Note: Individual program requirements may exceed the following minima.</p>
        <ul><li>Minimum 128 total semester hours required</li><li>Minimum 2.000 GPA required</li></ul>
        <p>Students must earn a minimum of 64 Northeastern University semester hours in order to receive a bachelor’s degree.</p></div>"#;
    let minimums = program_parser::universitywide_requirements(archived).unwrap();
    assert_eq!(minimums.total_hours, Some(128.0));
    assert_eq!(minimums.gpa, Some(2.0));
    assert_eq!(minimums.residency_hours, Some(64.0));
    assert!(!minimums.requires_experiential_learning);
    let current = archived.replacen("</div>", "<p>Students must complete all university and program requirements, including the experiential learning graduation requirement.</p></div>", 1);
    assert!(
        program_parser::universitywide_requirements(&current)
            .unwrap()
            .requires_experiential_learning
    );
    assert!(program_parser::universitywide_requirements(
        r#"<div id="textcontainer"><p>Nothing here.</p></div>"#
    )
    .is_none());
    assert_eq!(
        program_parser::universitywide_link(NUPATH_PAGE).as_deref(),
        Some("undergraduate/university-academics/university-wide-requirements/")
    );
}

#[test]
fn checksTheUniversitywideMinimumsItCan() {
    let minimums = UniversitywideRequirements {
        total_hours: Some(128.0),
        gpa: Some(2.0),
        residency_hours: Some(64.0),
        requires_experiential_learning: false,
        fetched_at: 0.0,
    };
    let audit = run_attr(
        &nupath_plan(),
        &nupath_requirements(Some(minimums)),
        &attributes_of(&nupath_table(), &[]),
    );
    let check = audit
        .note_checks
        .iter()
        .find(|check| matches!(check.kind, NoteKind::Universitywide))
        .unwrap();
    assert_eq!(
        check
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["total-hours", "gpa", "residency"]
    );
    assert_eq!(check.items[0].status, Some(AuditStatus::Partial));
    assert_eq!(
        check.items[0].detail,
        "44 of 128 semester hours · 12 planned"
    );
    assert_eq!(check.items[1].status, None);
    assert_eq!(
        check.items[1].detail,
        "Not checked: plans don’t include grades"
    );
    assert_eq!(check.items[2].status, None);
    assert_eq!(check.status, AuditStatus::Partial);
    assert!(check.counts);
    assert_eq!(
        check.detail,
        "44 of 128 semester hours · 12 planned · GPA and residency not checked"
    );
    let unread = run_attr(
        &nupath_plan(),
        &nupath_requirements(None),
        &attributes_of(&nupath_table(), &[]),
    );
    assert!(!unread
        .note_checks
        .iter()
        .any(|check| matches!(check.kind, NoteKind::Universitywide)));
}

#[test]
fn checksTheProgramsTotalHours() {
    let audit = run(&nupath_plan(), &nupath_requirements(None));
    let check = audit
        .note_checks
        .iter()
        .find(|check| matches!(check.kind, NoteKind::TotalHours { .. }))
        .unwrap();
    assert_eq!(check.status, AuditStatus::Partial);
    assert_eq!(check.detail, "44 of 134 semester hours · 12 planned");
}

#[test]
fn saysWhySomeSentencesArentChecked() {
    assert_eq!(
        degree_audit::unchecked_reason(
            "Minimum cumulative 2.000 GPA required in all CS, CY, DS, and IS courses"
        ),
        Some("Not checked: plans don’t include grades")
    );
    assert_eq!(
        degree_audit::unchecked_reason(
            "Students must earn a minimum of 64 Northeastern University semester hours."
        ),
        Some("Not checked: a plan doesn’t say where credits were earned")
    );
    assert_eq!(
        degree_audit::unchecked_reason("Pick one of the following concentrations and complete four courses in that concentration."),
        Some("Checked through the concentration you choose")
    );
    assert_eq!(
        degree_audit::unchecked_reason("Courses may not be taken pass/fail."),
        None
    );
}

type CodesByCourse = HashMap<CourseKey, HashSet<String>>;
type BannerRows<'a> = &'a [(&'a str, &'a [(&'a str, &'a [&'a str])])];
type AttributeCalls = Rc<RefCell<Vec<(String, Vec<CourseKey>)>>>;

#[derive(Clone)]
struct FakeSections {
    by_term: HashMap<String, CodesByCourse>,
    calls: AttributeCalls,
    failing: Rc<RefCell<HashSet<String>>>,
}

impl FakeSections {
    fn new(rows: BannerRows<'_>) -> Self {
        let mut by_term = HashMap::new();
        for (term, courses) in rows {
            let mut map = HashMap::new();
            for (code, attributes) in *courses {
                map.insert(
                    key(code),
                    attributes.iter().map(|code| (*code).to_string()).collect(),
                );
            }
            by_term.insert((*term).to_string(), map);
        }
        Self {
            by_term,
            calls: Rc::new(RefCell::new(vec![])),
            failing: Rc::new(RefCell::new(HashSet::new())),
        }
    }
}

impl AttributeSource for FakeSections {
    fn terms(&self) -> Result<Vec<Term>, String> {
        Ok([
            "202710", "202655", "202650", "202630", "202610", "202560", "202550", "202540",
            "202530", "202510",
        ]
        .into_iter()
        .map(|code| Term {
            code: code.into(),
            banner_description: String::new(),
        })
        .collect())
    }
    fn section_attributes(
        &self,
        term: &str,
        courses: &[CourseKey],
    ) -> Result<HashMap<CourseKey, HashSet<String>>, String> {
        self.calls
            .borrow_mut()
            .push((term.to_string(), courses.to_vec()));
        if self.failing.borrow().contains(term) {
            return Err("503".into());
        }
        Ok(self
            .by_term
            .get(term)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|(course, _)| courses.contains(course))
            .collect())
    }
}

fn loaded_index() -> CatalogIndex {
    let course = Course {
        subject: "ENGW".into(),
        number: "1111".into(),
        title: "First-Year Writing".into(),
        description: String::new(),
        min_credits: 4.0,
        max_credits: 4.0,
        attributes: vec!["NCW1".into()],
        prereqs: None,
        coreqs: None,
        postreqs: vec![],
        special_topics: false,
        college: None,
        department: None,
        details_loaded: true,
    };
    let section = CourseSection {
        crn: "10001".into(),
        subject: "ENGW".into(),
        number: "1111".into(),
        title: "First-Year Writing".into(),
        section_number: "01".into(),
        part_of_term: "1".into(),
        seat_capacity: 20,
        seats_remaining: 2,
        enrollment: 18,
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
        attributes: vec!["NCW1".into()],
        description: None,
        prereqs: None,
        coreqs: None,
    };
    CatalogIndex::build(TermCatalog {
        version: 1,
        term: Term {
            code: "202710".into(),
            banner_description: "Fall 2026 Semester".into(),
        },
        generated_at: 0.0,
        sections_updated_at: 0.0,
        courses: vec![course],
        sections: vec![section],
        subjects: vec![],
        attributes: vec![],
        campuses: vec![],
    })
}

#[test]
fn searchesRecentTermsUntilACourseHasSections() {
    let banner = FakeSections::new(&[
        ("202630", &[("CS 3386", &["NCFQ", "NCEI"])]),
        (
            "202610",
            &[("HIST 1130", &["NCDD", "NCSI"]), ("CS 3386", &["NCFQ"])],
        ),
    ]);
    let directory = std::env::temp_dir().join(format!("attributes-{}", Uuid::new_v4()));
    let keys = vec![
        key("ENGW 1111"),
        key("CS 3386"),
        key("HIST 1130"),
        key("ZZZZ 9999"),
    ];
    let mut repository = AttributeRepository::new(banner.clone(), &directory);
    let results = repository.lookup(&keys, &[loaded_index()]);
    assert_eq!(
        results.get(&key("ENGW 1111")),
        Some(&AttributeLookup::Found {
            codes: HashSet::from(["NCW1".into()]),
            term: "202710".into()
        })
    );
    assert_eq!(
        results.get(&key("CS 3386")),
        Some(&AttributeLookup::Found {
            codes: HashSet::from(["NCFQ".into(), "NCEI".into()]),
            term: "202630".into()
        })
    );
    assert_eq!(
        results.get(&key("HIST 1130")),
        Some(&AttributeLookup::Found {
            codes: HashSet::from(["NCDD".into(), "NCSI".into()]),
            term: "202610".into()
        })
    );
    assert_eq!(
        results.get(&key("ZZZZ 9999")),
        Some(&AttributeLookup::NotFound)
    );
    let calls = banner.calls.borrow().clone();
    assert!(!calls
        .iter()
        .any(|(term, _)| term == "202710" || term == "202655"));
    assert_eq!(
        calls
            .iter()
            .find(|(term, _)| term == "202610")
            .map(|(_, courses)| courses.clone())
            .as_deref(),
        Some(&[key("HIST 1130"), key("ZZZZ 9999")][..])
    );
    assert_eq!(
        calls
            .iter()
            .find(|(term, _)| term == "202560")
            .map(|(_, courses)| courses.clone())
            .as_deref(),
        Some(&[key("ZZZZ 9999")][..])
    );
    let mut again = AttributeRepository::new(banner.clone(), &directory);
    let call_count = banner.calls.borrow().len();
    assert_eq!(again.lookup(&keys, &[loaded_index()]), results);
    assert_eq!(banner.calls.borrow().len(), call_count);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn failuresAreNotCached() {
    let banner = FakeSections::new(&[("202610", &[("HIST 1130", &["NCDD"])])]);
    banner.failing.borrow_mut().insert("202650".into());
    let directory = std::env::temp_dir().join(format!("attributes-{}", Uuid::new_v4()));
    let mut repository = AttributeRepository::new(banner.clone(), &directory);
    assert_eq!(
        repository.lookup(&[key("HIST 1130")], &[]),
        HashMap::from([(key("HIST 1130"), AttributeLookup::Failed)])
    );
    banner.failing.borrow_mut().clear();
    assert_eq!(
        repository.lookup(&[key("HIST 1130")], &[]),
        HashMap::from([(
            key("HIST 1130"),
            AttributeLookup::Found {
                codes: HashSet::from(["NCDD".into()]),
                term: "202610".into()
            }
        )])
    );
    let _ = std::fs::remove_dir_all(directory);
}

struct PoolCourse {
    key: CourseKey,
    credits: f64,
}

fn pool() -> Vec<PoolCourse> {
    [
        ("CS 1200", 1.0),
        ("CS 1800", 4.0),
        ("CS 1802", 1.0),
        ("CS 2500", 4.0),
        ("CS 2501", 1.0),
        ("CS 2510", 4.0),
        ("CS 3000", 4.0),
        ("CS 3200", 4.0),
        ("CS 3500", 4.0),
        ("CS 3501", 1.0),
        ("CS 3650", 4.0),
        ("CS 4100", 4.0),
        ("CS 4400", 4.0),
        ("CS 4530", 4.0),
        ("DS 3000", 4.0),
        ("DS 4400", 4.0),
        ("MATH 1341", 4.0),
        ("MATH 2331", 4.0),
    ]
    .into_iter()
    .map(|(code, credits)| PoolCourse {
        key: key(code),
        credits,
    })
    .collect()
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn int(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (hi - lo + 1) as u64;
        lo + (self.next_u64() % span) as i32
    }
    fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

fn shuffle<T>(items: &mut [T], rng: &mut SplitMix64) {
    for index in (1..items.len()).rev() {
        let swap = rng.int(0, index as i32) as usize;
        items.swap(index, swap);
    }
}

fn simple_rule(id: &str, kind: RuleKind) -> RequirementRule {
    RequirementRule {
        id: id.into(),
        kind,
        label: id.into(),
        hours: None,
        notes: vec![],
        option_categories: None,
    }
}

fn requirements_of(rules: Vec<RequirementRule>) -> ProgramRequirements {
    ProgramRequirements {
        year: CatalogYear::new(2024),
        program: CatalogProgram {
            title: "Random, BS".into(),
            path: "undergraduate/random/".into(),
        },
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
        total_hours: None,
        fetched_at: 0.0,
        course_titles: std::collections::BTreeMap::new(),
        universitywide: None,
    }
}

#[test]
fn matchesAsManyRequirementsAsPossible() {
    let pool = pool();
    let mut generator = SplitMix64 { state: 2026 };
    let mut checked = 0;
    for _ in 0..160 {
        let count = generator.int(3, 6) as usize;
        let mut order: Vec<usize> = (0..pool.len()).collect();
        shuffle(&mut order, &mut generator);
        let courses: Vec<&PoolCourse> = order
            .into_iter()
            .take(count)
            .map(|index| &pool[index])
            .collect();
        let mut plan = DegreePlan {
            name: "Random".into(),
            ..DegreePlan::default()
        };
        let mut term = DegreePlanTerm::new(PlanSeason::Spring, 2027);
        for course in &courses {
            let line = DegreePlanCourse::catalog(course.key.clone(), "", course.credits);
            if generator.bool() {
                plan.completed.push(line);
            } else {
                term.courses.push(line);
            }
        }
        plan.terms = vec![term];
        let rules = random_rules(&courses, &pool, &mut generator);
        let pins = if generator.int(0, 9) < 3 {
            vec![(
                generator.int(0, courses.len() as i32 - 1) as usize,
                generator.int(0, rules.len() as i32 - 1) as usize,
            )]
        } else {
            vec![]
        };
        plan.substitutions = pins
            .iter()
            .map(|(course_index, rule_index)| {
                sub(
                    &rules[*rule_index].id,
                    &rules[*rule_index].id,
                    SubstitutionChange::CountCourse(courses[*course_index].key.clone()),
                    "",
                )
            })
            .collect();
        let requirements = requirements_of(rules.clone());
        let audit = run(&plan, &requirements);
        let possible = most_that_can_be_met(&rules, &courses, &pins);
        let specific = audit
            .rule_results()
            .into_iter()
            .filter(|result| {
                !result.rule.degree_is_general_electives() && result.status.is_satisfied()
            })
            .count();
        assert_eq!(specific, possible.0, "specific shortfall");
        if possible.1 {
            assert_eq!(
                audit.satisfied_count(),
                audit.total_count(),
                "everything was possible"
            );
        }
        let counted: Vec<String> = audit
            .rule_results()
            .into_iter()
            .flat_map(|result| result.used.iter().map(|use_| use_.id()))
            .collect();
        assert_eq!(counted.len(), counted.iter().collect::<HashSet<_>>().len());
        checked += 1;
    }
    assert_eq!(checked, 160);
}

fn draw_key(generator: &mut SplitMix64, pool: &[PoolCourse], keys: &[CourseKey]) -> CourseKey {
    if generator.int(0, 4) == 0 {
        pool[generator.int(0, pool.len() as i32 - 1) as usize]
            .key
            .clone()
    } else {
        keys[generator.int(0, keys.len() as i32 - 1) as usize].clone()
    }
}

fn distinct_keys(keys: Vec<CourseKey>) -> Vec<CourseKey> {
    let mut seen = HashSet::new();
    keys.into_iter()
        .filter(|key| seen.insert(key.clone()))
        .collect()
}

fn random_rules(
    courses: &[&PoolCourse],
    pool: &[PoolCourse],
    generator: &mut SplitMix64,
) -> Vec<RequirementRule> {
    let keys: Vec<CourseKey> = courses.iter().map(|course| course.key.clone()).collect();
    let mut rules = vec![];
    let total = generator.int(2, 4);
    let with_electives = generator.bool();
    let limit = if with_electives { total - 1 } else { total };
    for index in 0..limit {
        let kind = match generator.int(0, 7) {
            0 => RuleKind::Required(CourseOption {
                alternatives: vec![vec![draw_key(generator, pool, &keys)]],
            }),
            1 => RuleKind::Required(CourseOption {
                alternatives: vec![distinct_keys(vec![
                    draw_key(generator, pool, &keys),
                    draw_key(generator, pool, &keys),
                ])],
            }),
            2 => RuleKind::Required(CourseOption {
                alternatives: vec![
                    vec![draw_key(generator, pool, &keys)],
                    vec![draw_key(generator, pool, &keys)],
                ],
            }),
            3 => {
                let n = generator.int(2, 4);
                let mut drawn = Vec::new();
                for _ in 0..n {
                    drawn.push(draw_key(generator, pool, &keys));
                }
                let options = distinct_keys(drawn);
                RuleKind::Choose {
                    count: Some(generator.int(1, 2)),
                    hours: None,
                    options: options
                        .into_iter()
                        .map(|key| CourseOption {
                            alternatives: vec![vec![key]],
                        })
                        .collect(),
                    ranges: vec![],
                }
            }
            4 => {
                let lowest = [2000, 3000][generator.int(0, 1) as usize];
                RuleKind::Choose {
                    count: Some(generator.int(1, 2)),
                    hours: None,
                    options: vec![],
                    ranges: vec![CourseRange {
                        subject: "CS".into(),
                        lowest,
                        highest: None,
                        except: vec![],
                        label: format!("CS {lowest} or higher"),
                    }],
                }
            }
            5 => RuleKind::Choose {
                count: None,
                hours: Some([5.0, 8.0][generator.int(0, 1) as usize]),
                options: vec![],
                ranges: vec![
                    CourseRange {
                        subject: "CS".into(),
                        lowest: 2000,
                        highest: None,
                        except: vec![],
                        label: "CS 2000 or higher".into(),
                    },
                    CourseRange {
                        subject: "DS".into(),
                        lowest: 2000,
                        highest: None,
                        except: vec![],
                        label: "DS 2000 or higher".into(),
                    },
                ],
            },
            6 => {
                let pair = distinct_keys(vec![
                    draw_key(generator, pool, &keys),
                    draw_key(generator, pool, &keys),
                ]);
                let n = generator.int(1, 2);
                let mut drawn = Vec::new();
                for _ in 0..n {
                    let key = draw_key(generator, pool, &keys);
                    if !pair.contains(&key) {
                        drawn.push(key);
                    }
                }
                let singles = distinct_keys(drawn);
                let mut options = vec![CourseOption {
                    alternatives: vec![pair],
                }];
                options.extend(singles.into_iter().map(|key| CourseOption {
                    alternatives: vec![vec![key]],
                }));
                RuleKind::Choose {
                    count: Some(generator.int(1, 2)),
                    hours: None,
                    options,
                    ranges: vec![],
                }
            }
            _ => {
                let approved = vec![draw_key(generator, pool, &keys)];
                let ranges = vec![CourseRange {
                    subject: "CS".into(),
                    lowest: 2500,
                    highest: None,
                    except: vec![],
                    label: "CS 2500 or higher".into(),
                }];
                if generator.bool() {
                    RuleKind::Choose {
                        count: Some(generator.int(1, 2)),
                        hours: None,
                        options: vec![CourseOption {
                            alternatives: vec![approved],
                        }],
                        ranges,
                    }
                } else {
                    RuleKind::Choose {
                        count: None,
                        hours: Some([4.0, 8.0][generator.int(0, 1) as usize]),
                        options: vec![CourseOption {
                            alternatives: vec![approved],
                        }],
                        ranges,
                    }
                }
            }
        };
        rules.push(simple_rule(&format!("rule-{index}"), kind));
    }
    if with_electives {
        rules.push(simple_rule(
            "general",
            RuleKind::GeneralElectives {
                hours: [4.0, 8.0][generator.int(0, 1) as usize],
            },
        ));
    }
    rules
}

fn most_that_can_be_met(
    rules: &[RequirementRule],
    courses: &[&PoolCourse],
    pins: &[(usize, usize)],
) -> (usize, bool) {
    let mut assignment = vec![rules.len(); courses.len()];
    let mut pinned = HashMap::new();
    for (course_index, rule_index) in pins {
        pinned.entry(*course_index).or_insert(*rule_index);
    }
    let mut best_specific = 0usize;
    let mut everything = false;
    fn is_met(
        rule_index: usize,
        rules: &[RequirementRule],
        courses: &[&PoolCourse],
        assignment: &[usize],
        pinned: &HashMap<usize, usize>,
    ) -> bool {
        let rule = &rules[rule_index];
        let mine: Vec<&PoolCourse> = courses
            .iter()
            .enumerate()
            .filter(|(index, _)| assignment[*index] == rule_index && !pinned.contains_key(index))
            .map(|(_, course)| *course)
            .collect();
        let placed: Vec<&PoolCourse> = courses
            .iter()
            .enumerate()
            .filter(|(index, _)| pinned.get(index) == Some(&rule_index))
            .map(|(_, course)| *course)
            .collect();
        let keys: HashSet<CourseKey> = mine.iter().map(|course| course.key.clone()).collect();
        let placed_keys: HashSet<CourseKey> =
            placed.iter().map(|course| course.key.clone()).collect();
        let complete = |alternative: &[CourseKey]| alternative.iter().all(|key| keys.contains(key));
        match &rule.kind {
            RuleKind::Required(option) => {
                !placed.is_empty()
                    || option
                        .alternatives
                        .iter()
                        .any(|alternative| complete(alternative))
            }
            RuleKind::Choose {
                count,
                hours,
                options,
                ranges,
            } => {
                let open: Vec<&CourseOption> = options
                    .iter()
                    .filter(|option| {
                        option
                            .all_courses()
                            .iter()
                            .all(|key| !placed_keys.contains(key))
                    })
                    .collect();
                let listed: HashSet<CourseKey> = open
                    .iter()
                    .flat_map(|option| option.all_courses())
                    .collect();
                let option_units = open
                    .iter()
                    .filter(|option| {
                        option
                            .alternatives
                            .iter()
                            .any(|alternative| complete(alternative))
                    })
                    .count();
                let in_ranges: Vec<&PoolCourse> = mine
                    .iter()
                    .copied()
                    .filter(|course| {
                        !listed.contains(&course.key)
                            && ranges.iter().any(|range| range.contains(&course.key))
                    })
                    .collect();
                if let Some(count) = count {
                    placed.len()
                        + option_units
                        + in_ranges
                            .iter()
                            .filter(|course| course.credits >= 2.0)
                            .count()
                        >= *count as usize
                } else if let Some(hours) = hours {
                    let from_options: Vec<&PoolCourse> = mine
                        .iter()
                        .copied()
                        .filter(|course| {
                            open.iter().any(|option| {
                                option.all_courses().contains(&course.key)
                                    && option
                                        .alternatives
                                        .iter()
                                        .any(|alternative| complete(alternative))
                            })
                        })
                        .collect();
                    placed
                        .iter()
                        .chain(from_options.iter())
                        .chain(in_ranges.iter())
                        .map(|course| course.credits)
                        .sum::<f64>()
                        >= *hours
                } else {
                    placed.len() + option_units >= 1
                }
            }
            RuleKind::GeneralElectives { hours } => {
                mine.iter()
                    .chain(placed.iter())
                    .map(|course| course.credits)
                    .sum::<f64>()
                    >= *hours
            }
        }
    }
    fn assign(
        position: usize,
        rules: &[RequirementRule],
        courses: &[&PoolCourse],
        assignment: &mut [usize],
        pinned: &HashMap<usize, usize>,
        best_specific: &mut usize,
        everything: &mut bool,
    ) {
        if position >= courses.len() {
            let met: Vec<usize> = (0..rules.len())
                .filter(|index| is_met(*index, rules, courses, assignment, pinned))
                .collect();
            let specific = met
                .iter()
                .filter(|index| !rules[**index].degree_is_general_electives())
                .count();
            *best_specific = (*best_specific).max(specific);
            if met.len() == rules.len() {
                *everything = true;
            }
            return;
        }
        if let Some(rule) = pinned.get(&position) {
            assignment[position] = *rule;
            assign(
                position + 1,
                rules,
                courses,
                assignment,
                pinned,
                best_specific,
                everything,
            );
            return;
        }
        for bucket in 0..=rules.len() {
            assignment[position] = bucket;
            assign(
                position + 1,
                rules,
                courses,
                assignment,
                pinned,
                best_specific,
                everything,
            );
        }
    }
    assign(
        0,
        rules,
        courses,
        &mut assignment,
        &pinned,
        &mut best_specific,
        &mut everything,
    );
    (best_specific, everything)
}

#[test]
fn largePlansStayFast() {
    let pool = pool();
    let mut rules = vec![];
    let mut cs: Vec<CourseKey> = pool
        .iter()
        .filter(|course| course.key.subject == "CS" && course.credits >= 4.0)
        .map(|course| course.key.clone())
        .collect();
    for index in 0..6 {
        let mut generator = SplitMix64 { state: index };
        shuffle(&mut cs, &mut generator);
        let options = cs
            .iter()
            .take(5)
            .cloned()
            .map(|key| CourseOption {
                alternatives: vec![vec![key]],
            })
            .collect();
        rules.push(simple_rule(
            &format!("list-{index}"),
            RuleKind::Choose {
                count: Some(2),
                hours: None,
                options,
                ranges: vec![],
            },
        ));
    }
    rules.push(simple_rule(
        "electives",
        RuleKind::Choose {
            count: None,
            hours: Some(12.0),
            options: vec![],
            ranges: vec![CourseRange {
                subject: "CS".into(),
                lowest: 2000,
                highest: None,
                except: vec![],
                label: "CS 2000 or higher".into(),
            }],
        },
    ));
    rules.push(simple_rule(
        "petition",
        RuleKind::Choose {
            count: Some(3),
            hours: None,
            options: vec![],
            ranges: vec![CourseRange {
                subject: "CS".into(),
                lowest: 0,
                highest: None,
                except: vec![],
                label: "Any CS course".into(),
            }],
        },
    ));
    rules.push(simple_rule(
        "general",
        RuleKind::GeneralElectives { hours: 16.0 },
    ));
    let mut plan = DegreePlan {
        name: "Big".into(),
        ..DegreePlan::default()
    };
    plan.completed = pool
        .iter()
        .map(|course| DegreePlanCourse::catalog(course.key.clone(), "", course.credits))
        .collect();
    let requirements = requirements_of(rules.clone());
    let start = Instant::now();
    let audit = run(&plan, &requirements);
    assert!(
        start.elapsed().as_secs_f64() < 2.0,
        "elapsed {:?}",
        start.elapsed()
    );
    assert_eq!(audit.total_count(), rules.len());
}

fn renumbered() -> (
    DegreePlan,
    RequirementSubstitution,
    RequirementSubstitution,
    ProgramRequirements,
) {
    let requirements = requirements();
    let mut plan = plan();
    plan.completed
        .retain(|course| course.key.as_ref() != Some(&key("CS 3000")));
    plan.completed.push(course("CS 3001", 4.0, ""));
    let required = rule_named(&requirements, "CS 3000");
    let security = rule_named(&requirements, "Complete one of the following:");
    let renumbering = sub(
        &required.id,
        "CS 3000",
        SubstitutionChange::CountCourse(key("CS 3001")),
        "Approved",
    );
    let waiver = sub(
        &security.id,
        "Security Course",
        SubstitutionChange::Waive,
        "",
    );
    plan.substitutions = vec![renumbering.clone(), waiver.clone()];
    (plan, renumbering, waiver, requirements)
}

#[test]
fn onlyASubstitutionForOneNamedCourseIsARenumbering() {
    let (plan, renumbering, waiver, requirements) = renumbered();
    let either = rule_named(&requirements, "CS 4500 or CS 4530");
    let mut other = plan.clone();
    other.substitutions.push(sub(
        &either.id,
        "CS 4500 or CS 4530",
        SubstitutionChange::CountCourse(key("CS 4535")),
        "",
    ));
    other.substitutions.push(sub(
        &renumbering.rule_id,
        "CS 3000",
        SubstitutionChange::CountCourse(key("CS 3000")),
        "",
    ));
    assert_eq!(
        editing::renumberable(&other, &requirements),
        HashSet::from([renumbering.id])
    );
    assert!(!editing::renumberable(&other, &requirements).contains(&waiver.id));
    assert_eq!(
        editing::courses_to_look_up(&other, &requirements),
        vec![key("CS 3000"), key("CS 3001")]
    );
    let mut none = plan;
    none.substitutions = vec![waiver];
    assert!(editing::courses_to_look_up(&none, &requirements).is_empty());
}

#[test]
fn turnsASubstitutionIntoTheEquivalenceItAmountsTo() {
    let (plan, renumbering, waiver, requirements) = renumbered();
    let mut data = UserData::default();
    let id = data.add_degree_plan(plan);
    let change = editing::convert_renumbering(&renumbering, id, &requirements, &mut data).unwrap();
    assert_eq!(change.course, key("CS 3001"));
    assert_eq!(change.counts_as, key("CS 3000"));
    assert_eq!(change.message(), "CS 3001 now counts as CS 3000");
    assert!(change.can_undo());
    let after = data.degree_plans.iter().find(|plan| plan.id == id).unwrap();
    assert_eq!(
        after
            .substitutions
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![waiver.id]
    );
    assert_eq!(
        after
            .course_equivalences
            .iter()
            .map(|item| item.course.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 3001")]
    );
    assert_eq!(after.course_equivalences[0].counts_as, key("CS 3000"));
    assert_eq!(after.course_equivalences[0].note, "Renumbered");
    let audit = run(after, &requirements);
    let result = audit.result_for(&renumbering.rule_id).unwrap();
    assert_eq!(result.status, AuditStatus::Complete);
    assert_eq!(
        result
            .used
            .iter()
            .map(|use_| use_.key.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 3001")]
    );
    assert_eq!(result.used[0].satisfies, Some(key("CS 3000")));
    assert!(!result.used[0].by_substitution);
    assert!(editing::convert_renumbering(&renumbering, id, &requirements, &mut data).is_none());
}

#[test]
fn undoPutsTheSubstitutionBackWhereItWas() {
    let (plan, renumbering, waiver, requirements) = renumbered();
    let mut data = UserData::default();
    let id = data.add_degree_plan(plan);
    let change = editing::convert_renumbering(&renumbering, id, &requirements, &mut data).unwrap();
    let extra = sub("elsewhere", "Elsewhere", SubstitutionChange::Waive, "");
    data.degree_plans
        .iter_mut()
        .find(|plan| plan.id == id)
        .unwrap()
        .substitutions
        .push(extra.clone());
    change.undo(id, &mut data);
    let restored = data.degree_plans.iter().find(|plan| plan.id == id).unwrap();
    assert_eq!(
        restored
            .substitutions
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![renumbering.id, waiver.id, extra.id]
    );
    assert!(restored.course_equivalences.is_empty());
    change.undo(id, &mut data);
    assert_eq!(
        data.degree_plans
            .iter()
            .find(|plan| plan.id == id)
            .unwrap()
            .substitutions
            .len(),
        3
    );
}

#[test]
fn declaresARenumberingWithoutASubstitution() {
    let mut data = UserData::default();
    let id = data.add_degree_plan(DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    });
    assert!(editing::declare_renumbering(key("CS 3000"), key("CS 3000"), id, &mut data).is_none());
    let change =
        editing::declare_renumbering(key("CS 3001"), key("CS 3000"), id, &mut data).unwrap();
    assert!(change.substitution.is_none() && change.can_undo());
    assert_eq!(
        data.degree_plans
            .iter()
            .find(|plan| plan.id == id)
            .unwrap()
            .course_equivalences
            .iter()
            .map(|item| item.note.as_str())
            .collect::<Vec<_>>(),
        ["Renumbered"]
    );
    let again =
        editing::declare_renumbering(key("CS 3001"), key("CS 3000"), id, &mut data).unwrap();
    assert!(!again.can_undo());
    assert_eq!(
        data.degree_plans
            .iter()
            .find(|plan| plan.id == id)
            .unwrap()
            .course_equivalences
            .len(),
        1
    );
    change.undo(id, &mut data);
    assert!(data
        .degree_plans
        .iter()
        .find(|plan| plan.id == id)
        .unwrap()
        .course_equivalences
        .is_empty());
}

#[test]
fn asksAboutARetiredRequiredCourseOnly() {
    let (mut plan, renumbering, _, requirements) = renumbered();
    let retired = editing::likely_renumberings(&plan, &requirements, |course| {
        Some(course == &key("CS 3000"))
    });
    assert_eq!(
        retired
            .iter()
            .map(|item| item.substitution.id)
            .collect::<Vec<_>>(),
        vec![renumbering.id]
    );
    assert_eq!(
        retired[0].question(),
        "Is CS 3001 the new number of CS 3000?"
    );
    assert!(editing::likely_renumberings(&plan, &requirements, |_| None).is_empty());
    assert!(editing::likely_renumberings(&plan, &requirements, |_| Some(false)).is_empty());
    assert!(editing::declare_equivalence(
        &mut plan,
        &key("CS 3001"),
        &key("CS 3000"),
        ""
    ));
    assert!(
        editing::likely_renumberings(&plan, &requirements, |course| Some(
            course == &key("CS 3000")
        ))
        .is_empty()
    );
}

#[test]
fn aKeptSubstitutionIsNotAskedAboutAgain() {
    let (plan, renumbering, waiver, requirements) = renumbered();
    let retired = |course: &CourseKey| Some(course == &key("CS 3000"));
    let asked = editing::likely_renumbering_map(&plan, &requirements, &HashSet::new(), retired);
    assert_eq!(
        asked.keys().copied().collect::<HashSet<_>>(),
        HashSet::from([renumbering.id])
    );
    assert!(!asked.contains_key(&waiver.id));
    let kept = editing::pair_id(&key("CS 3001"), &key("CS 3000"));
    assert!(
        editing::likely_renumbering_map(&plan, &requirements, &HashSet::from([kept]), retired)
            .is_empty()
    );
    let other = editing::pair_id(&key("CS 3001"), &key("CS 4535"));
    assert_eq!(
        editing::likely_renumbering_map(&plan, &requirements, &HashSet::from([other]), retired)
            .len(),
        1
    );
    assert!(
        editing::likely_renumbering_map(&plan, &requirements, &HashSet::new(), |_| None).is_empty()
    );
}

#[test]
fn aSubstituteSitsUnderTheRequirementItReplaces() {
    let requirements = requirements();
    let mut plan = plan();
    let either = rule_named(&requirements, "CS 4500 or CS 4530");
    let security = rule_named(&requirements, "Complete one of the following:");
    plan.substitutions = vec![
        sub(
            &either.id,
            "CS 4500 or CS 4530",
            SubstitutionChange::CountCourse(key("CS 4535")),
            "Approved by advising",
        ),
        sub(
            &security.id,
            "Security Course",
            SubstitutionChange::CountCourse(key("CS 4973")),
            "",
        ),
    ];
    let audit = run(&plan, &requirements);
    let either_result = audit.result_for(&either.id).unwrap();
    let (substitutes, others) = split_unlisted(
        either_result,
        &either.listed_courses().into_iter().collect(),
    );
    assert_eq!(
        substitutes
            .iter()
            .map(|use_| use_.key.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 4535")]
    );
    assert!(others.is_empty());
    assert_eq!(
        requirement_name(either_result, &requirements),
        "CS 4500 or CS 4530"
    );
    assert_eq!(
        counting_note(&key("CS 4535"), either_result).as_deref(),
        Some("Approved by advising")
    );
    assert_eq!(
        counting_caption(&key("CS 4535"), either_result, &requirements),
        "Counts for CS 4500 or CS 4530 by substitution · Approved by advising"
    );
    let security_result = audit.result_for(&security.id).unwrap();
    assert_eq!(
        split_unlisted(
            security_result,
            &security.listed_courses().into_iter().collect()
        )
        .0
        .iter()
        .map(|use_| use_.key.clone())
        .collect::<Vec<_>>(),
        vec![key("CS 4973")]
    );
    assert_eq!(
        counting_caption(&key("CS 4973"), security_result, &requirements),
        "Counts for Security Course by substitution"
    );
    let info = CourseRowInfo {
        status: CourseStanding::Planned("Spring 2027".into()),
        is_counted_here: true,
        by_substitution: true,
        caption: Some("ignored".into()),
    };
    assert_eq!(
        row_caption(&info, &key("CS 4535"), either_result, &requirements, true).as_deref(),
        Some("Approved by advising")
    );
    assert_eq!(
        row_caption(&info, &key("CS 4535"), either_result, &requirements, false).as_deref(),
        Some("Counts for CS 4500 or CS 4530 by substitution · Approved by advising")
    );
    let plain = CourseRowInfo {
        status: CourseStanding::NotInPlan,
        is_counted_here: false,
        by_substitution: false,
        caption: Some("No longer offered".into()),
    };
    assert_eq!(
        row_caption(&plain, &key("CS 4535"), either_result, &requirements, false).as_deref(),
        Some("No longer offered")
    );
    let mut listed = plan;
    listed.substitutions = vec![sub(
        &security.id,
        "Security Course",
        SubstitutionChange::CountCourse(key("CY 3740")),
        "",
    )];
    listed.completed.push(course("CY 3740", 4.0, ""));
    let listed_result = run(&listed, &requirements)
        .result_for(&security.id)
        .unwrap()
        .clone();
    assert!(split_unlisted(
        &listed_result,
        &security.listed_courses().into_iter().collect()
    )
    .0
    .is_empty());
}

#[test]
fn showsEachSentenceWithItsCheckForTheFilter() {
    let notes = |completed: &[&str], planned: &[&str]| {
        let requirements = program_parser::requirements_from_html(
            NOTES_PAGE,
            CatalogYear::new(2024),
            CatalogProgram {
                title: "Notes, BS".into(),
                path: "undergraduate/notes/".into(),
            },
        )
        .unwrap();
        let mut plan = DegreePlan {
            name: "Notes".into(),
            ..DegreePlan::default()
        };
        plan.completed = completed
            .iter()
            .map(|code| {
                if *code == "COOP 3946" {
                    course(code, 0.0, "Co-op Work Experience - HT")
                } else {
                    course(code, 4.0, "")
                }
            })
            .collect();
        let mut fall = DegreePlanTerm::new(PlanSeason::Fall, 2027);
        fall.courses = planned.iter().map(|code| course(code, 4.0, "")).collect();
        plan.terms = vec![fall];
        (run(&plan, &requirements), requirements)
    };
    let entries = |filter: RequirementsFilter,
                   audit: &DegreeAudit,
                   requirements: &ProgramRequirements| {
        let mut found = HashMap::new();
        for section in &requirements.sections {
            for entry in note_entries(section, audit, filter, requirements.total_hours.is_some()) {
                found.insert(section.title.clone(), entry);
            }
        }
        found
    };
    let (done, requirements) = notes(&["CS 3000", "CS 3200", "CS 3500", "COOP 3946"], &[]);
    let all = entries(RequirementsFilter::All, &done, &requirements);
    assert_eq!(
        all.keys().map(|key| key.as_str()).collect::<HashSet<_>>(),
        HashSet::from([
            "GPA Requirement",
            "Credit Requirement",
            "NUpath Requirements Satisfied",
            "Program Requirement"
        ])
    );
    assert!(all["GPA Requirement"].check.is_none());
    assert_eq!(
        all["Credit Requirement"].check.as_ref().unwrap().status,
        AuditStatus::Complete
    );
    assert_eq!(
        all["NUpath Requirements Satisfied"]
            .check
            .as_ref()
            .unwrap()
            .detail,
        "COOP 3946 completed"
    );
    assert!(all["Program Requirement"].check.is_some());
    assert_eq!(
        entries(RequirementsFilter::Done, &done, &requirements)
            .keys()
            .map(|key| key.as_str())
            .collect::<HashSet<_>>(),
        HashSet::from([
            "Credit Requirement",
            "NUpath Requirements Satisfied",
            "Program Requirement"
        ])
    );
    assert!(entries(RequirementsFilter::Remaining, &done, &requirements).is_empty());
    let (short, short_requirements) = notes(&["CS 3000"], &["CS 3200"]);
    let remaining = entries(RequirementsFilter::Remaining, &short, &short_requirements);
    assert_eq!(
        remaining
            .keys()
            .map(|key| key.as_str())
            .collect::<HashSet<_>>(),
        HashSet::from([
            "Credit Requirement",
            "NUpath Requirements Satisfied",
            "Program Requirement"
        ])
    );
    assert_eq!(
        remaining["NUpath Requirements Satisfied"]
            .check
            .as_ref()
            .unwrap()
            .detail,
        "No co-op in the plan"
    );
    assert!(!remaining.contains_key("GPA Requirement"));
    assert!(
        !entries(RequirementsFilter::Done, &short, &short_requirements)
            .contains_key("NUpath Requirements Satisfied")
    );
}

#[test]
fn aCheckThatDoesNotCountYetShowsInAllOnly() {
    let mut check = NoteCheck {
        section_id: "nupath-requirements".into(),
        note: "NUpath".into(),
        kind: NoteKind::Nupath,
        status: AuditStatus::Missing,
        detail: "looking up 2 courses".into(),
        items: vec![],
        counts: false,
    };
    assert!(RequirementsFilter::All.includes(&check));
    assert!(!RequirementsFilter::Remaining.includes(&check));
    assert!(!RequirementsFilter::Done.includes(&check));
    check.counts = true;
    assert!(RequirementsFilter::Remaining.includes(&check));
    assert!(!RequirementsFilter::Done.includes(&check));
}

#[test]
fn parsesTheExport() {
    let result = degree_format::parse(SAMPLE_EXPORT, Some("view-plan.pdf")).unwrap();
    let plan = result.plan;
    assert_eq!(plan.name, "New Plan");
    assert_eq!(plan.program.as_deref(), Some("CS-B-CSCI-BSCS"));
    assert_eq!(plan.student_name.as_deref(), Some("SAMPLE STUDENT"));
    assert_eq!(plan.imported_from.as_deref(), Some("view-plan.pdf"));
    assert!(plan.printed_at.is_some());
    assert_eq!(plan.completed.len(), 6);
    assert_eq!(plan.completed[1].credits, 1.0);
    assert!(plan.completed[2].key.is_none());
    assert_eq!(plan.completed[2].label, "NO AP CREDIT");
    assert_eq!(plan.completed[0].key, Some(key("CS 1800")));
    assert_eq!(
        plan.terms
            .iter()
            .map(|term| term.name())
            .collect::<Vec<_>>(),
        [
            "Spring 2027",
            "Summer Session A 2027",
            "Summer Session B 2027",
            "Fall 2027"
        ]
    );
    assert_eq!(plan.terms[0].courses.len(), 3);
    assert_eq!(plan.terms[0].credits(), 9.0);
    assert!(plan.terms[2].courses.is_empty());
    assert!(result.skipped_lines.is_empty());
}

#[test]
fn parsesRawPaddedLines() {
    let raw = format!(
        "Degree Audit Plan Name:\tMy Plan\nProgram:\t\tCS-B\nCompleted Courses:  \nGAME1110 (4.0 Hours) Introduction to Video Game De\nCS  2500 (4.0 Hours) Fundamentals of Computer Sci \n{}\nSpring  2027 (4.0 Hours) \nCS  3484 (4.0 Hours) GUI Programming\n",
        "-".repeat(115)
    );
    let plan = degree_format::parse(&raw, None).unwrap().plan;
    assert_eq!(plan.name, "My Plan");
    assert_eq!(
        plan.completed
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        ["GAME 1110", "CS 2500"]
    );
    assert_eq!(plan.completed[1].title, "Fundamentals of Computer Sci");
    assert_eq!(plan.terms[0].season, PlanSeason::Spring);
    assert_eq!(plan.terms[0].courses[0].key, Some(key("CS 3484")));
}

#[test]
fn rejectsOtherDocuments() {
    assert!(matches!(
        degree_format::parse("Hello world\nThis is a receipt\nTotal $4.00", None),
        Err(ImportError::NotADegreePlan)
    ));
}

#[test]
fn cleansUpExtractionArtifacts() {
    let text = format!(
        "Degree Audit Plan Name:\u{0}New Plan\nProgram:\u{0}\u{0}CS-B-CSCI-BSCS\nStudent:\u{0}\u{0}SAMPLE STUDENT\nCompleted Courses:\n-ENGW3302 (4.0 Hours) Advanced Writing in Tech Prof\n- CS 1800 (4.0 Hours) Discrete\u{200B} Structures\n{SEPARATOR}\n-Spring 2027 (4.0 Hours)\nCS\u{00A0}3484 (4.0 Hours) GUI Programming\n"
    );
    let plan = degree_format::parse(&text, None).unwrap().plan;
    assert_eq!(plan.name, "New Plan");
    assert_eq!(plan.program.as_deref(), Some("CS-B-CSCI-BSCS"));
    assert_eq!(plan.student_name.as_deref(), Some("SAMPLE STUDENT"));
    assert_eq!(
        plan.completed
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        ["ENGW 3302", "CS 1800"]
    );
    assert_eq!(plan.completed[1].title, "Discrete Structures");
    assert_eq!(plan.terms.len(), 1);
    assert_eq!(
        plan.terms[0]
            .courses
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        ["CS 3484"]
    );
}

#[test]
fn formatsLikeTheAudit() {
    let plan = degree_format::parse(SAMPLE_EXPORT, None).unwrap().plan;
    assert_eq!(
        degree_format::course_line(&plan.completed[0]),
        "CS  1800 (4.0 Hours) Discrete Structures"
    );
    assert_eq!(
        degree_format::course_line(&plan.completed[1]),
        "CS  1802 (1.0 Hour) Seminar for CS 1800"
    );
    assert_eq!(
        degree_format::course_line(&plan.completed[2]),
        "NO AP CREDIT (0.0 Hours) No description"
    );
    assert_eq!(
        degree_format::term_header(&plan.terms[0]),
        "Spring  2027 (9.0 Hours) "
    );
    assert_eq!(
        degree_format::term_header(&plan.terms[1]),
        "Summer Session A 2027 (4.0 Hours) "
    );
    assert_eq!(SEPARATOR.len(), 115);
    let text = degree_format::text(&plan, degree_format::now_unix(), false);
    assert!(!text.contains("StudentID"));
    assert!(text.contains("Degree Audit Plan Name:\tNew Plan"));
    assert!(text.contains("Program:\t\tCS-B-CSCI-BSCS"));
}

#[test]
fn pdfRoundTrip() {
    let original = degree_format::parse(SAMPLE_EXPORT, None).unwrap().plan;
    let pdf = degree_format::pdf(&original, degree_format::now_unix(), true);
    let text = degree_format::pdf_text(&pdf);
    let parsed = degree_format::parse(&text, None).unwrap().plan;
    assert_eq!(parsed.name, original.name);
    assert_eq!(parsed.program, original.program);
    assert_eq!(parsed.student_name, original.student_name);
    assert_eq!(
        parsed
            .completed
            .iter()
            .map(|course| course.label.clone())
            .collect::<Vec<_>>(),
        original
            .completed
            .iter()
            .map(|course| course.label.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        parsed
            .completed
            .iter()
            .map(|course| course.credits)
            .collect::<Vec<_>>(),
        original
            .completed
            .iter()
            .map(|course| course.credits)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        parsed
            .terms
            .iter()
            .map(|term| term.name())
            .collect::<Vec<_>>(),
        original
            .terms
            .iter()
            .map(|term| term.name())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        parsed
            .terms
            .iter()
            .map(|term| term
                .courses
                .iter()
                .map(|course| course.label.clone())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        original
            .terms
            .iter()
            .map(|term| term
                .courses
                .iter()
                .map(|course| course.label.clone())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
}

#[test]
fn longPlansSpanPages() {
    let mut plan = degree_format::parse(SAMPLE_EXPORT, None).unwrap().plan;
    plan.completed = (0..60)
        .map(|index| {
            DegreePlanCourse::catalog(
                CourseKey {
                    subject: "CS".into(),
                    number: format!("{}", 1000 + index),
                },
                format!("Course {index}"),
                4.0,
            )
        })
        .collect();
    let pdf = degree_format::pdf(&plan, degree_format::now_unix(), false);
    let parsed = degree_format::parse(&degree_format::pdf_text(&pdf), None)
        .unwrap()
        .plan;
    assert_eq!(parsed.completed.len(), 60);
    assert_eq!(parsed.terms.len(), plan.terms.len());
}

#[test]
fn prerequisiteChecks() {
    let cs2100 = key("CS 2100");
    let cs2510 = key("CS 2510");
    let cs1800 = key("CS 1800");
    let requisite = Requisite::And(vec![
        Requisite::Or(vec![
            Requisite::Course(cs2100.clone()),
            Requisite::Course(cs2510.clone()),
        ]),
        Requisite::Course(cs1800.clone()),
    ]);
    let equivalences = CourseEquivalences::none();
    assert_eq!(
        planlogic::status(
            Some(&requisite),
            &HashSet::from([cs2510.clone(), cs1800.clone()]),
            &equivalences
        ),
        PrereqStatus::Met
    );
    assert_eq!(
        planlogic::status(
            Some(&requisite),
            &HashSet::from([cs2510.clone()]),
            &equivalences
        ),
        PrereqStatus::Missing("CS 1800".into())
    );
    assert_eq!(
        planlogic::status(
            Some(&requisite),
            &HashSet::from([cs1800.clone()]),
            &equivalences
        ),
        PrereqStatus::Missing("CS 2100 or CS 2510".into())
    );
    assert_eq!(
        planlogic::status(Some(&requisite), &HashSet::new(), &equivalences),
        PrereqStatus::Missing("(CS 2100 or CS 2510) and CS 1800".into())
    );
    assert_eq!(
        planlogic::status(None, &HashSet::new(), &equivalences),
        PrereqStatus::Met
    );
    assert_eq!(
        planlogic::status(
            Some(&Requisite::Or(vec![
                Requisite::Test {
                    name: "Placement".into(),
                    score: None
                },
                Requisite::Course(cs2100)
            ])),
            &HashSet::new(),
            &equivalences
        ),
        PrereqStatus::Met
    );
}

fn course_req(code: &str) -> Requisite {
    Requisite::Course(key(code))
}

#[test]
fn unmetSummariesAreShort() {
    let equivalences = CourseEquivalences::none();
    let programming = Requisite::Or(vec![
        course_req("CS 2100"),
        course_req("CS 201M"),
        course_req("CS 2510"),
        course_req("DS 2500"),
    ]);
    let discrete = Requisite::Or(vec![
        course_req("CS 1800"),
        course_req("MATH 209M"),
        course_req("EECE 2160"),
    ]);
    let cs3000 = Requisite::Or(vec![
        Requisite::And(vec![programming.clone(), discrete.clone()]),
        course_req("CS 2700"),
    ]);
    assert_eq!(
        planlogic::unmet_summary(&programming, &HashSet::new(), &equivalences).as_deref(),
        Some("CS 2100, CS 201M or 2 others")
    );
    assert_eq!(
        planlogic::unmet_summary(&discrete, &HashSet::new(), &equivalences).as_deref(),
        Some("CS 1800 or MATH 209M or EECE 2160")
    );
    assert_eq!(
        planlogic::unmet_summary(&cs3000, &HashSet::new(), &equivalences).as_deref(),
        Some("CS 2700 or another option")
    );
    let taken = HashSet::from([key("CS 1800")]);
    assert_eq!(
        planlogic::unmet_summary(&cs3000, &taken, &equivalences).as_deref(),
        Some("(CS 2100, CS 201M or 2 others) or another option")
    );
    assert_eq!(
        planlogic::missing_count(&cs3000, &HashSet::new(), &equivalences),
        1
    );
    let both = HashSet::from([key("CS 1800"), key("CS 2510")]);
    assert_eq!(
        planlogic::unmet_summary(&cs3000, &both, &equivalences),
        None
    );
    let nursing = Requisite::And(
        [
            "NRSG 3344",
            "NRSG 3345",
            "NRSG 3400",
            "NRSG 3401",
            "NRSG 3420",
        ]
        .into_iter()
        .map(|code| Requisite::Or(vec![course_req(code), course_req(code), course_req(code)]))
        .collect(),
    );
    assert_eq!(
        planlogic::unmet_summary(&nursing, &HashSet::new(), &equivalences).as_deref(),
        Some("NRSG 3344, NRSG 3345 and 3 more")
    );
    assert_eq!(
        planlogic::unmet_summary(
            &Requisite::And(vec![nursing, course_req("CS 1800")]),
            &HashSet::new(),
            &equivalences
        )
        .as_deref(),
        Some("(NRSG 3344, NRSG 3345 and 3 more) and CS 1800")
    );
}

#[test]
fn coursesBeforeATermIncludeEarlierTerms() {
    let plan = degree_format::parse(SAMPLE_EXPORT, None).unwrap().plan;
    let before = planlogic::courses_before(&plan, 1);
    assert!(before.contains(&key("CS 3650")));
    assert!(before.contains(&key("CS 1800")));
    assert!(!before.contains(&key("CS 4500")));
}

#[test]
fn autofillPrefersTheExactTermThenTheClosest() {
    let mut plan = degree_format::parse(SAMPLE_EXPORT, None).unwrap().plan;
    plan.is_primary = true;
    let spring = Term {
        code: "202730".into(),
        banner_description: "Spring 2027 Semester".into(),
    };
    let exact =
        planlogic::schedule_suggestions(&spring, &[plan.clone()], &[], None, &HashSet::new(), None);
    assert!(exact[0].is_exact_term);
    assert_eq!(exact[0].courses.len(), 3);
    let fall = Term {
        code: "202710".into(),
        banner_description: "Fall 2026 Semester".into(),
    };
    let closest =
        planlogic::schedule_suggestions(&fall, &[plan.clone()], &[], None, &HashSet::new(), None);
    assert!(!closest[0].is_exact_term);
    assert!(
        closest[0].title.contains("Spring 2027"),
        "{}",
        closest[0].title
    );
    let partial = planlogic::schedule_suggestions(
        &spring,
        &[plan],
        &[],
        None,
        &HashSet::from([key("CS 3650")]),
        None,
    );
    assert_eq!(partial[0].courses.len(), 2);
}

fn bundled_index() -> CatalogIndex {
    let path = bundled_catalog_path().expect("bundled catalog");
    let mut catalog = archive::read(&path).expect("read catalog");
    catalog.recompute_postreqs();
    CatalogIndex::build(catalog)
}

#[test]
fn autofillMarksCoursesNotOfferedThisTerm() {
    let index = bundled_index();
    let mut plan = DegreePlan {
        name: "Test".into(),
        ..DegreePlan::default()
    };
    plan.terms = vec![DegreePlanTerm {
        courses: vec![
            course("CS 3000", 4.0, "Algorithms"),
            course("CS 9999", 4.0, "Nope"),
        ],
        ..DegreePlanTerm::new(PlanSeason::Fall, 2026)
    }];
    let term = index.catalog.term.clone();
    let suggestions =
        planlogic::schedule_suggestions(&term, &[plan], &[], None, &HashSet::new(), Some(&index));
    assert_eq!(suggestions[0].courses, vec![key("CS 3000")]);
    assert_eq!(suggestions[0].unavailable, vec![key("CS 9999")]);
}

#[test]
fn newPlanDraftCopiesTheLatestPlanAndAddsSchedules() {
    let source = degree_format::parse(SAMPLE_EXPORT, None).unwrap().plan;
    let schedule = SchedulePlan {
        id: Uuid::new_v4(),
        name: "Plan A".into(),
        term_code: "202730".into(),
        courses: vec![
            PlanCourse {
                key: key("CS 3650"),
                is_required: true,
                excluded_crns: HashSet::new(),
                color_index: 0,

                pinned_crn: None,
            },
            PlanCourse {
                key: key("MATH 2331"),
                is_required: true,
                excluded_crns: HashSet::new(),
                color_index: 1,

                pinned_crn: None,
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
    let draft = planlogic::new_degree_plan_draft("Draft", Some(&source), &[schedule], 2026, 10);
    assert_eq!(draft.completed.len(), source.completed.len());
    assert_ne!(draft.completed[0].id, source.completed[0].id);
    let spring = draft
        .terms
        .iter()
        .find(|term| term.season == PlanSeason::Spring && term.year == 2027)
        .unwrap();
    assert_eq!(
        spring
            .courses
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        ["CS 3650", "CS 4530", "CS 2963", "MATH 2331"]
    );
}

#[test]
fn termMatching() {
    let summer_a = DegreePlanTerm::new(PlanSeason::SummerA, 2027);
    assert!(summer_a.matches_term(&Term {
        code: "202740".into(),
        banner_description: String::new()
    }));
    assert!(summer_a.matches_term(&Term {
        code: "202750".into(),
        banner_description: String::new()
    }));
    assert!(!summer_a.matches_term(&Term {
        code: "202760".into(),
        banner_description: String::new()
    }));
    assert!(
        DegreePlanTerm::new(PlanSeason::Fall, 2026).matches_term(&Term {
            code: "202710".into(),
            banner_description: String::new()
        })
    );
}

#[test]
fn storeKeepsOnePrimaryPlan() {
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
            .find(|plan| plan.id == first)
            .unwrap()
            .is_primary
    );
    assert!(
        !data
            .degree_plans
            .iter()
            .find(|plan| plan.id == second)
            .unwrap()
            .is_primary
    );
    data.set_primary_degree_plan(second);
    assert_eq!(data.primary_degree_plan().unwrap().id, second);
    data.delete_degree_plan(second);
    assert_eq!(data.primary_degree_plan().unwrap().id, first);
    assert!(
        data.degree_plans
            .iter()
            .find(|plan| plan.id == first)
            .unwrap()
            .is_primary
    );
}

fn mac_plan() -> DegreePlan {
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.terms = vec![
        DegreePlanTerm {
            courses: vec![
                course("CS 3000", 4.0, "CS 3000"),
                course("CS 3500", 4.0, "CS 3500"),
                course("CS 3650", 4.0, "CS 3650"),
            ],
            ..DegreePlanTerm::new(PlanSeason::Spring, 2027)
        },
        DegreePlanTerm {
            courses: vec![course("CS 4400", 4.0, "CS 4400")],
            ..DegreePlanTerm::new(PlanSeason::Fall, 2027)
        },
    ];
    plan
}

fn codes(term: &DegreePlanTerm) -> Vec<String> {
    term.courses
        .iter()
        .map(|course| {
            course
                .key
                .as_ref()
                .map(|key| key.to_string())
                .unwrap_or_else(|| course.label.clone())
        })
        .collect()
}

#[test]
fn reordersWithinASemester() {
    let mut plan = mac_plan();
    let spring = plan.terms[0].id;
    let a = plan.terms[0].courses[0].id;
    let b = plan.terms[0].courses[1].id;
    let c = plan.terms[0].courses[2].id;
    editing::move_course_at(a, spring, 3, &mut plan);
    assert_eq!(codes(&plan.terms[0]), ["CS 3500", "CS 3650", "CS 3000"]);
    editing::move_course_at(a, spring, 1, &mut plan);
    assert_eq!(codes(&plan.terms[0]), ["CS 3500", "CS 3000", "CS 3650"]);
    editing::move_course_at(c, spring, 0, &mut plan);
    assert_eq!(codes(&plan.terms[0]), ["CS 3650", "CS 3500", "CS 3000"]);
    editing::move_course_at(b, spring, 1, &mut plan);
    assert_eq!(codes(&plan.terms[0]), ["CS 3650", "CS 3500", "CS 3000"]);
    editing::move_course_at(b, spring, 2, &mut plan);
    assert_eq!(codes(&plan.terms[0]), ["CS 3650", "CS 3500", "CS 3000"]);
}

#[test]
fn movesToAPositionInAnotherSemester() {
    let mut plan = mac_plan();
    let spring = plan.terms[0].id;
    let fall = plan.terms[1].id;
    let b = plan.terms[0].courses[1].id;
    editing::move_course_at(b, fall, 0, &mut plan);
    assert_eq!(codes(&plan.terms[0]), ["CS 3000", "CS 3650"]);
    assert_eq!(codes(&plan.terms[1]), ["CS 3500", "CS 4400"]);
    let a = plan.terms[0].courses[0].id;
    editing::move_course_at(a, fall, 99, &mut plan);
    assert_eq!(codes(&plan.terms[1]), ["CS 3500", "CS 4400", "CS 3000"]);
    let c = plan.terms[0].courses[0].id;
    editing::move_course_at(c, fall, -5, &mut plan);
    assert_eq!(
        codes(&plan.terms[1]),
        ["CS 3650", "CS 3500", "CS 4400", "CS 3000"]
    );
    assert!(plan.terms[0].courses.is_empty());
    let _ = spring;
}

#[test]
fn addsADraggedCourseAtAPosition() {
    let mut plan = mac_plan();
    let spring = plan.terms[0].id;
    let drop = |plan: &mut DegreePlan, code: &str, position: i32| {
        let added = course(code, 4.0, code);
        let id = added.id;
        editing::add(added, spring, plan);
        editing::move_course_at(id, spring, position, plan);
    };
    drop(&mut plan, "CS 4500", 1);
    assert_eq!(
        codes(&plan.terms[0]),
        ["CS 3000", "CS 4500", "CS 3500", "CS 3650"]
    );
    drop(&mut plan, "CS 4530", 0);
    assert_eq!(
        codes(&plan.terms[0]),
        ["CS 4530", "CS 3000", "CS 4500", "CS 3500", "CS 3650"]
    );
    let end = plan.terms[0].courses.len() as i32;
    drop(&mut plan, "CS 4535", end);
    assert_eq!(
        codes(&plan.terms[0]),
        ["CS 4530", "CS 3000", "CS 4500", "CS 3500", "CS 3650", "CS 4535"]
    );
    assert_eq!(codes(&plan.terms[1]), ["CS 4400"]);
}

#[test]
fn ignoresACourseOrSemesterThatIsGone() {
    let mut plan = mac_plan();
    let before = plan.clone();
    editing::move_course_at(Uuid::new_v4(), plan.terms[1].id, 0, &mut plan);
    editing::move_course_at(plan.terms[0].courses[0].id, Uuid::new_v4(), 0, &mut plan);
    assert_eq!(plan, before);
}

#[test]
fn buildsAScheduleFromASemesterOnce() {
    let mut data = UserData::default();
    let semester = mac_plan().terms.into_iter().next().unwrap();
    let first = planlogic::schedule_for_semester(&mut data, &semester, "202730").unwrap();
    assert!(first.is_new);
    assert_eq!(first.course_count, 3);
    assert_eq!(first.plan.name, "Spring 2027 plan");
    assert_eq!(
        data.plans
            .iter()
            .find(|plan| plan.id == first.plan.id)
            .unwrap()
            .courses
            .iter()
            .map(|course| course.key.to_string())
            .collect::<Vec<_>>(),
        ["CS 3000", "CS 3500", "CS 3650"]
    );
    let again = planlogic::schedule_for_semester(&mut data, &semester, "202730").unwrap();
    assert!(!again.is_new);
    assert_eq!(again.plan.id, first.plan.id);
    assert_eq!(data.plans.len(), 1);
    let empty = DegreePlanTerm {
        courses: vec![DegreePlanCourse::other("NO AP CREDIT", "", 0.0)],
        ..DegreePlanTerm::new(PlanSeason::SummerA, 2027)
    };
    assert!(planlogic::schedule_for_semester(&mut data, &empty, "202740").is_none());
}

#[test]
fn summarizesAPlanForTheOverview() {
    let now = planlogic::now_unix();
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    assert_eq!(planlogic::completed_line(&plan), "No completed courses yet");
    assert_eq!(planlogic::next_line(&plan, now), "No semesters planned yet");
    plan.completed = vec![
        course("CS 1800", 4.0, "CS 1800"),
        course("CS 1802", 1.0, "CS 1802"),
    ];
    assert_eq!(planlogic::completed_line(&plan), "5 credits completed");
    plan.terms = vec![DegreePlanTerm {
        courses: vec![course("CS 1800", 4.0, "CS 1800")],
        ..DegreePlanTerm::new(PlanSeason::Fall, 2001)
    }];
    assert_eq!(planlogic::next_line(&plan, now), "No upcoming semesters");
    plan.terms
        .push(DegreePlanTerm::new(PlanSeason::Spring, 2099));
    assert_eq!(planlogic::next_line(&plan, now), "No courses planned yet");
    plan.terms[1].courses = vec![
        course("CS 3000", 4.0, "CS 3000"),
        course("CS 3500", 5.0, "CS 3500"),
    ];
    assert_eq!(
        planlogic::next_summary(&plan, now).as_deref(),
        Some("Spring 2099 · 9 credits · 2 courses")
    );
    assert_eq!(
        planlogic::next_line(&plan, now),
        "Next: Spring 2099 · 9 credits · 2 courses"
    );
}

#[test]
fn laysRequirementsOutLikeTheCatalog() {
    let requirements = requirements();
    let audit = run(&plan(), &requirements);
    let either = audit
        .rule_results()
        .into_iter()
        .find(|result| result.rule.label == "CS 4500 or CS 4530")
        .unwrap();
    assert!(!has_header_row(either));
    assert!(!is_replaced(either));
    let RuleKind::Required(option) = &either.rule.kind else {
        panic!("not a required course")
    };
    assert_eq!(
        alternative_entries(option)
            .iter()
            .map(|entry| entry.courses())
            .collect::<Vec<_>>(),
        vec![vec![key("CS 4500")], vec![key("CS 4530")]]
    );
    let security = audit
        .rule_results()
        .into_iter()
        .find(|result| {
            result
                .rule
                .label
                .starts_with("Complete one of the following")
        })
        .unwrap();
    assert!(has_header_row(security));
    let (options, ranges) = listing(&security.rule);
    assert_eq!(options.len(), 2);
    assert!(ranges.is_empty());
    let science = audit
        .rule_results()
        .into_iter()
        .find(|result| result.rule.label.starts_with("Complete two courses"))
        .unwrap();
    let (science_options, _) = listing(&science.rule);
    let listed: HashSet<CourseKey> = science_options
        .iter()
        .flat_map(|option| option.all_courses())
        .collect();
    let entries = option_entries(&science_options, &science.rule, |_| None);
    assert_eq!(
        entries
            .iter()
            .filter_map(|entry| entry
                .category
                .as_ref()
                .map(|category| format!("{} {category}", entry.id)))
            .collect::<Vec<_>>(),
        ["0 Biology", "1 Mathematics"]
    );
    let counted = use_of("BIOL 1112", "", 1.0, None, 0);
    assert!(is_shown(&counted, &listed));
    let other = use_of("CS 4973", "", 4.0, None, 1);
    assert!(!is_shown(&other, &listed));
}

#[test]
fn longListsKeepTheOptionsThatMatter() {
    let entries: Vec<ListEntry> = (0..10)
        .map(|id| ListEntry {
            id,
            alternatives: vec![vec![CourseKey {
                subject: "CS".into(),
                number: format!("{}", 4000 + id),
            }]],
            category: None,
        })
        .collect();
    let relevant = |entry: &ListEntry| [2, 7].contains(&entry.id);
    assert_eq!(
        collapsed_entries(&entries, 3, relevant)
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 7]
    );
    assert_eq!(
        collapsed_entries(&entries, 0, relevant)
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        [2, 7]
    );
}

#[test]
fn findsPlanCoursesByCodeOrTitle() {
    let choices = vec![
        PlanCourseChoice {
            key: key("CS 3000"),
            title: "Algorithms and Data".into(),
            credits: 4.0,
            semester: None,
            semester_short_name: None,
        },
        PlanCourseChoice {
            key: key("MATH 1341"),
            title: "Calculus 1".into(),
            credits: 4.0,
            semester: Some("Spring 2027".into()),
            semester_short_name: Some("Spring 2027".into()),
        },
    ];
    assert_eq!(editing::matching_choices(&choices, "").len(), 2);
    assert_eq!(
        editing::matching_choices(&choices, "cs 3000")
            .iter()
            .map(|choice| choice.key.to_string())
            .collect::<Vec<_>>(),
        ["CS 3000"]
    );
    assert_eq!(
        editing::matching_choices(&choices, "math1341")
            .iter()
            .map(|choice| choice.key.to_string())
            .collect::<Vec<_>>(),
        ["MATH 1341"]
    );
    assert_eq!(
        editing::matching_choices(&choices, "calculus")
            .iter()
            .map(|choice| choice.key.to_string())
            .collect::<Vec<_>>(),
        ["MATH 1341"]
    );
    assert!(editing::matching_choices(&choices, "chemistry").is_empty());
}
