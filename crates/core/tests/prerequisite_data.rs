#![allow(
    clippy::field_reassign_with_default,
    non_snake_case,
    clippy::too_many_lines
)]
//! Ports of `PrerequisiteDataTests.swift`. One test per Swift `@Test`, same name.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use termorganize_core::banner::{self, SubjectLookup};
use termorganize_core::catalog::{self, CrossListedSection};
use termorganize_core::course_info::{
    CourseInfo, CourseInfoRepository, CourseInfoSource, CourseInfoStore, Lookup, StoreState,
};
use termorganize_core::degree_audit::{
    AuditStatus, DegreeAudit, DegreeCourseAttributes, RuleResult,
};
use termorganize_core::equivalences::{
    covering_equivalents, declare_equivalence, remove_equivalence,
};
use termorganize_core::index::CatalogIndex;
use termorganize_core::model::*;
use termorganize_core::offering::{OfferingHistory, OfferingRepository, OfferingSource};
use termorganize_core::planlogic::{self, PrereqStatus};
use termorganize_core::program_parser;
use termorganize_core::reorder::{self, Move, Suggestion, MAX_SUGGESTIONS};
use termorganize_core::suggestions::{self, EquivalenceSuggestion};
use termorganize_core::titles::{self, Title};
use termorganize_core::CourseEquivalences;
use uuid::Uuid;

fn key(code: &str) -> CourseKey {
    CourseKey::parse(code).unwrap_or_else(|| panic!("bad course code {code}"))
}

fn requisite(code: &str) -> Requisite {
    Requisite::Course(key(code))
}

fn either(codes: &[&str]) -> Requisite {
    Requisite::Or(codes.iter().map(|code| requisite(code)).collect())
}

fn and(items: Vec<Requisite>) -> Requisite {
    Requisite::And(items)
}

fn set_of(keys: &[CourseKey]) -> HashSet<CourseKey> {
    keys.iter().cloned().collect()
}

fn terms(codes: &[&str]) -> Vec<Term> {
    codes
        .iter()
        .map(|code| Term {
            code: (*code).to_string(),
            banner_description: String::new(),
        })
        .collect()
}

fn banner_terms() -> Vec<Term> {
    terms(&[
        "202710", "202655", "202650", "202632", "202630", "202610", "202560", "202550", "202540",
        "202530", "202510", "202460",
    ])
}

fn course(
    code: &str,
    title: &str,
    prereqs: Option<Requisite>,
    detailed: bool,
    topics: bool,
) -> Course {
    let parsed = key(code);
    Course {
        subject: parsed.subject,
        number: parsed.number,
        title: if title.is_empty() {
            code.to_string()
        } else {
            title.to_string()
        },
        description: String::new(),
        min_credits: 4.0,
        max_credits: 4.0,
        attributes: vec![],
        prereqs,
        coreqs: None,
        postreqs: vec![],
        special_topics: topics,
        college: None,
        department: None,
        details_loaded: detailed,
    }
}

fn section(crn: &str, code: &str, title: &str, cross: &[&str]) -> CourseSection {
    let parsed = key(code);
    CourseSection {
        crn: crn.to_string(),
        subject: parsed.subject,
        number: parsed.number,
        title: if title.is_empty() {
            code.to_string()
        } else {
            title.to_string()
        },
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
        meetings: vec![],
        instructors: Some(vec![]),
        cross_list_crns: cross.iter().map(|item| (*item).to_string()).collect(),
        attributes: vec![],
        description: None,
        prereqs: None,
        coreqs: None,
    }
}

fn index(courses: Vec<Course>, sections: Vec<CourseSection>, term: &str) -> CatalogIndex {
    CatalogIndex::build(TermCatalog {
        version: 1,
        term: Term {
            code: term.to_string(),
            banner_description: "Fall 2026 Semester".into(),
        },
        generated_at: 0.0,
        sections_updated_at: 0.0,
        courses,
        sections,
        subjects: vec![],
        attributes: vec![],
        campuses: vec![],
    })
}

fn subjects() -> SubjectLookup {
    SubjectLookup::new(&[
        Subject {
            code: "CS".into(),
            name: "Computer Science".into(),
        },
        Subject {
            code: "ENGL".into(),
            name: "English".into(),
        },
        Subject {
            code: "MATH".into(),
            name: "Mathematics".into(),
        },
    ])
}

fn course_info(
    code: &str,
    term: &str,
    prerequisites: Option<Requisite>,
    equivalents: Vec<CourseKey>,
) -> CourseInfo {
    CourseInfo::entry(
        key(code),
        format!("Title of {code}"),
        Some(4.0),
        Some(4.0),
        prerequisites,
        equivalents,
        term,
        0.0,
    )
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("prerequisite-data-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct Clock(Arc<Mutex<f64>>);

impl Clock {
    fn new(start: f64) -> Self {
        Self(Arc::new(Mutex::new(start)))
    }
    fn standard() -> Self {
        Self::new(1_800_000_000.0)
    }
    fn now(&self) -> f64 {
        *self.0.lock().unwrap()
    }
    fn advance_days(&self, days: f64) {
        *self.0.lock().unwrap() += days * 86_400.0;
    }
}

struct BannerState {
    terms: Vec<Term>,
    catalog: HashMap<String, HashMap<CourseKey, CourseInfo>>,
    offered: HashMap<String, HashSet<CourseKey>>,
    failing: HashSet<String>,
    failing_courses: HashMap<String, CourseKey>,
    entry_calls: Vec<(CourseKey, String)>,
    offered_calls: Vec<(String, Vec<CourseKey>)>,
    in_flight: usize,
    max_in_flight: usize,
}

#[derive(Clone)]
struct FakeBanner {
    inner: Arc<Mutex<BannerState>>,
    clock: Clock,
    delay_ms: Arc<Mutex<u64>>,
}

impl FakeBanner {
    fn new(clock: &Clock, terms: Vec<Term>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BannerState {
                terms,
                catalog: HashMap::new(),
                offered: HashMap::new(),
                failing: HashSet::new(),
                failing_courses: HashMap::new(),
                entry_calls: Vec::new(),
                offered_calls: Vec::new(),
                in_flight: 0,
                max_in_flight: 0,
            })),
            clock: clock.clone(),
            delay_ms: Arc::new(Mutex::new(0)),
        }
    }

    fn list(&self, info: CourseInfo, term: &str) {
        self.inner
            .lock()
            .unwrap()
            .catalog
            .entry(term.to_string())
            .or_default()
            .insert(info.key.clone(), info);
    }

    fn offer(&self, term: &str, courses: &[&str]) {
        self.inner
            .lock()
            .unwrap()
            .offered
            .entry(term.to_string())
            .or_default()
            .extend(courses.iter().map(|code| key(code)));
    }

    fn fail_questions(&self, term: &str, course: Option<CourseKey>) {
        let mut state = self.inner.lock().unwrap();
        match course {
            Some(course) => {
                state.failing_courses.insert(term.to_string(), course);
            }
            None => {
                state.failing_courses.remove(term);
            }
        }
    }

    fn set_terms(&self, terms: Vec<Term>) {
        self.inner.lock().unwrap().terms = terms;
    }

    fn fail(&self, term: &str, failing: bool) {
        let mut state = self.inner.lock().unwrap();
        if failing {
            state.failing.insert(term.to_string());
        } else {
            state.failing.remove(term);
        }
    }

    fn set_delay_ms(&self, delay: u64) {
        *self.delay_ms.lock().unwrap() = delay;
    }

    fn entry_calls(&self) -> Vec<(CourseKey, String)> {
        self.inner.lock().unwrap().entry_calls.clone()
    }

    fn offered_calls(&self) -> Vec<(String, Vec<CourseKey>)> {
        self.inner.lock().unwrap().offered_calls.clone()
    }

    fn max_in_flight(&self) -> usize {
        self.inner.lock().unwrap().max_in_flight
    }

    fn reset_calls(&self) {
        let mut state = self.inner.lock().unwrap();
        state.entry_calls.clear();
        state.offered_calls.clear();
    }
}

impl CourseInfoSource for FakeBanner {
    fn terms(&self) -> Result<Vec<Term>, String> {
        Ok(self.inner.lock().unwrap().terms.clone())
    }

    fn catalog_entry(&self, key: &CourseKey, term: &str) -> Result<Option<CourseInfo>, String> {
        {
            let mut state = self.inner.lock().unwrap();
            state.entry_calls.push((key.clone(), term.to_string()));
            state.in_flight += 1;
            state.max_in_flight = state.max_in_flight.max(state.in_flight);
        }
        let delay = *self.delay_ms.lock().unwrap();
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        let mut state = self.inner.lock().unwrap();
        state.in_flight = state.in_flight.saturating_sub(1);
        if state.failing.contains(term) {
            return Err("503".into());
        }
        let Some(mut info) = state
            .catalog
            .get(term)
            .and_then(|row| row.get(key))
            .cloned()
        else {
            return Ok(None);
        };
        info.fetched_at = self.clock.now();
        Ok(Some(info))
    }
}

impl OfferingSource for FakeBanner {
    fn terms(&self) -> Result<Vec<Term>, String> {
        Ok(self.inner.lock().unwrap().terms.clone())
    }

    fn offered_courses(
        &self,
        term: &str,
        courses: &[CourseKey],
    ) -> Result<HashSet<CourseKey>, String> {
        let mut state = self.inner.lock().unwrap();
        state
            .offered_calls
            .push((term.to_string(), courses.to_vec()));
        if state.failing.contains(term) {
            return Err("503".into());
        }
        if state
            .failing_courses
            .get(term)
            .is_some_and(|failing| courses.contains(failing))
        {
            return Err("503".into());
        }
        let offered = state.offered.get(term).cloned().unwrap_or_default();
        Ok(courses
            .iter()
            .filter(|course| offered.contains(*course))
            .cloned()
            .collect())
    }
}

fn info_repo(
    banner: &FakeBanner,
    dir: &Path,
    clock: &Clock,
    max_back: usize,
    concurrency: usize,
) -> CourseInfoRepository<FakeBanner> {
    let mut repo =
        CourseInfoRepository::new(banner.clone(), dir.join("course-info.json"), clock.now());
    repo.max_terms_back = max_back;
    repo.concurrency = concurrency;
    repo
}

fn offer_repo(banner: &FakeBanner, dir: &Path, clock: &Clock) -> OfferingRepository<FakeBanner> {
    OfferingRepository::new(
        banner.clone(),
        dir.join("offering-history.json"),
        clock.now(),
    )
}

fn make_store(
    banner: &FakeBanner,
    dir: &Path,
    clock: &Clock,
) -> CourseInfoStore<FakeBanner, FakeBanner> {
    CourseInfoStore::new(
        info_repo(banner, dir, clock, 8, 2),
        offer_repo(banner, dir, clock),
        clock.now(),
    )
}

fn lookup_of(
    table: &HashMap<CourseKey, Option<Requisite>>,
    course: &CourseKey,
) -> Option<Option<Requisite>> {
    table.get(course).cloned()
}

const CHECKED: &[&str] = &[
    "202710", "202650", "202630", "202610", "202550", "202530", "202510",
];

fn history(
    offered: &[(&str, &[&str])],
    checked: Option<&[&str]>,
    keys: Option<&[&str]>,
) -> OfferingHistory {
    let checked_terms = checked
        .unwrap_or(CHECKED)
        .iter()
        .map(|code| (*code).to_string())
        .collect();
    let mut map = BTreeMap::new();
    for (code, terms) in offered {
        map.insert(
            key(code),
            terms.iter().map(|term| (*term).to_string()).collect(),
        );
    }
    OfferingHistory {
        checked_terms,
        offered: map,
        checked_keys: keys.map(|codes| codes.iter().map(|code| key(code)).collect()),
    }
}

fn planned(code: &str, credits: f64) -> DegreePlanCourse {
    DegreePlanCourse::catalog(key(code), code, credits)
}

fn semester(season: PlanSeason, year: i32, codes: &[&str], credits: f64) -> DegreePlanTerm {
    let mut term = DegreePlanTerm::new(season, year);
    term.courses = codes.iter().map(|code| planned(code, credits)).collect();
    term
}

fn degree_plan(term_list: Vec<DegreePlanTerm>, completed: &[&str]) -> DegreePlan {
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = completed.iter().map(|code| planned(code, 4.0)).collect();
    plan.terms = term_list;
    plan
}

fn known_prereqs(table: &[(&str, Requisite)]) -> impl Fn(&CourseKey) -> Option<Option<Requisite>> {
    let map: HashMap<CourseKey, Requisite> = table
        .iter()
        .map(|(code, requisite)| (key(code), requisite.clone()))
        .collect();
    move |course| Some(map.get(course).cloned())
}

fn reordering(
    plan: &DegreePlan,
    term: usize,
    blocking: &[&str],
    history: &OfferingHistory,
    prerequisites: &dyn Fn(&CourseKey) -> Option<Option<Requisite>>,
    equivalences: &CourseEquivalences,
    max_credits: f64,
) -> Vec<Suggestion> {
    let mut sorted = plan.clone();
    sorted.sort_terms();
    let id = sorted.terms[term].id;
    let blocking: Vec<CourseKey> = blocking.iter().map(|code| key(code)).collect();
    reorder::suggestions(
        plan,
        id,
        &blocking,
        history,
        prerequisites,
        equivalences,
        max_credits,
    )
}

fn titles_of(found: &[Suggestion]) -> Vec<String> {
    found.iter().map(|item| item.title.clone()).collect()
}

fn none_prereq(_: &CourseKey) -> Option<Option<Requisite>> {
    Some(None)
}

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

fn program() -> CatalogProgram {
    CatalogProgram {
        title: "Computer Science, BSCS (Boston)".into(),
        path: "undergraduate/computer-information-science/computer-science/bscs/".into(),
    }
}

fn requirements() -> ProgramRequirements {
    program_parser::requirements_from_html(PROGRAM_PAGE, CatalogYear::new(2023), program())
        .expect("program requirements")
}

fn audit_course(code: &str, credits: f64) -> DegreePlanCourse {
    DegreePlanCourse::catalog(key(code), code, credits)
}

fn audit_plan(completed: &[&str], planned_codes: &[&str]) -> DegreePlan {
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = completed
        .iter()
        .map(|code| audit_course(code, if *code == "CS 1802" { 1.0 } else { 4.0 }))
        .collect();
    if !planned_codes.is_empty() {
        plan.terms = vec![DegreePlanTerm {
            id: Uuid::new_v4(),
            season: PlanSeason::Spring,
            year: 2027,
            courses: planned_codes
                .iter()
                .map(|code| audit_course(code, 4.0))
                .collect(),
        }];
    }
    plan.catalog_year = Some(2023);
    plan.catalog_program_path = Some(program().path);
    plan.concentration_id = Some("concentration-in-human-centered-computing".into());
    plan
}

fn rule_labeled<'a>(audit: &'a DegreeAudit, label: &str) -> Option<&'a RuleResult> {
    audit
        .rule_results()
        .into_iter()
        .find(|result| result.rule.label == label)
}

fn run_audit(plan: &DegreePlan, equivalences: &CourseEquivalences) -> DegreeAudit {
    DegreeAudit::run(
        plan,
        &requirements(),
        equivalences,
        &DegreeCourseAttributes::none(),
    )
}

fn suggester_catalog() -> Vec<(CourseKey, Requisite)> {
    vec![
        (
            key("CS 3000"),
            and(vec![
                either(&["CS 2100", "CS 2510", "DS 2500"]),
                either(&["CS 1800", "MATH 1365"]),
            ]),
        ),
        (key("CS 3200"), either(&["CS 2100", "CS 2510"])),
        (
            key("CS 3386"),
            and(vec![
                Requisite::Or(vec![
                    Requisite::Or(vec![requisite("CS 2100"), requisite("CS 201M")]),
                    requisite("CS 2510"),
                ]),
                either(&["CS 2386", "CS 3540"]),
            ]),
        ),
        (key("CS 4100"), requisite("CS 2510")),
        (
            key("CS 4200"),
            and(vec![requisite("CS 2510"), requisite("MATH 1365")]),
        ),
        (key("ANTH 2000"), either(&["ANTH 1101", "POLS 1160"])),
    ]
}

fn current_courses() -> HashSet<CourseKey> {
    ["CS 2100", "DS 2500", "CS 3000", "ANTH 1101"]
        .into_iter()
        .map(key)
        .collect()
}

fn title_table() -> HashMap<CourseKey, String> {
    [
        ("IS 4300", "Human Computer Interaction"),
        ("CS 2484", "Principles of Human-Computer Interaction"),
        ("CS 3000", "Algorithms and Data"),
        ("CS 3650", "Computer Systems"),
        ("CS 2510", "Fundamentals of Computer Science 2"),
        ("CS 2515", "Computer Science 2 Honors"),
        ("CS 2100", "Program Design and Implementation 1"),
        ("CS 4550", "Web Development"),
    ]
    .into_iter()
    .map(|(code, title)| (key(code), title.to_string()))
    .collect()
}

fn by_title(
    retired: &str,
    candidates: &[&str],
    titles: Option<&HashMap<CourseKey, String>>,
    prerequisites: &[(CourseKey, Requisite)],
    current: Option<&HashSet<CourseKey>>,
    known: &CourseEquivalences,
    limit: usize,
) -> Vec<EquivalenceSuggestion> {
    let owned;
    let table = if let Some(titles) = titles {
        titles
    } else {
        owned = title_table();
        &owned
    };
    let candidates: Vec<CourseKey> = candidates.iter().map(|code| key(code)).collect();
    suggestions::suggestions(
        &key(retired),
        prerequisites,
        |course| current.is_none_or(|set| set.contains(course)),
        known,
        2,
        0.5,
        limit,
        |course| table.get(course).cloned(),
        &candidates,
        titles::MINIMUM_SIMILARITY,
    )
}

fn suggest(
    store: &CourseInfoStore<FakeBanner, FakeBanner>,
    course: &CourseKey,
    plan: Option<&DegreePlan>,
    catalog_titles: &HashMap<CourseKey, String>,
    also: &[CourseKey],
) -> Vec<EquivalenceSuggestion> {
    store.equivalence_suggestions(course, plan, 3, catalog_titles, also)
}

fn newest_index() -> CatalogIndex {
    let alternatives = either(&["CS 2100", "CS 2510"]);
    index(
        vec![
            course("CS 2100", "", None, true, false),
            course("CS 3000", "", Some(alternatives.clone()), true, false),
            course("CS 3200", "", Some(alternatives), true, false),
            course(
                "CS 3300",
                "",
                Some(either(&["CS 2100", "CS 2510", "DS 2500"])),
                true,
                false,
            ),
        ],
        vec![],
        "202710",
    )
}

fn sample_banner(clock: &Clock) -> FakeBanner {
    let banner = FakeBanner::new(clock, banner_terms());
    for term in ["202710", "202610", "202510"] {
        banner.offer(term, &["CS 3800", "MATH 2331"]);
    }
    for term in ["202630", "202530"] {
        banner.offer(term, &["CS 3386", "MATH 2331"]);
    }
    banner.offer("202650", &["CS 2500"]);
    banner
}

fn prereq_row(and_or: &str, open: bool, subject: &str, number: &str, close: bool) -> String {
    format!(
        "<tr><td>{and_or}</td><td>{}</td><td></td><td></td><td>{subject}</td><td>{number}</td><td>Undergraduate</td><td>D-</td><td>{}</td></tr>",
        if open { "(" } else { "" },
        if close { ")" } else { "" }
    )
}

#[test]
fn typeaheadCodesMustMatchExactly() {
    let json = r#"[{"code":"CS3386","description":"CS3386 Computer Science"},{"code":"CS33860","description":"CS33860 Computer Science"}]"#;
    let items: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
    let pairs: Vec<(String, String)> = items
        .iter()
        .map(|item| {
            (
                item["code"].as_str().unwrap().to_string(),
                item["description"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(banner::catalog_lists(&pairs, &key("CS 3386")));
    assert!(!banner::catalog_lists(&pairs, &key("CS 338")));
    assert!(!banner::catalog_lists(&[], &key("CS 3386")));
}

#[test]
fn readsCatalogDetailsAndCredits() {
    let html = r#"
        <section aria-labelledby="catalog">
        <span class="status-bold">Title:</span><span>Directed Study</span><br/>
        <span class="status-bold">College:</span>
        <span>Coll of Soc Sci &amp; Humanities  SH</span>
        <br/>
        <span class="status-bold">Department:</span>
        <span>Sociology and Anthropology  SOCL</span><br/>
        <span class="status-bold">Hours:</span><br/>
        <span class="indent-left">Credit Hours:</span><span
        class="credit-hours-direction">1 TO 4</span><br/>
        <div class="indent-left"><span class="credit-hours-direction">Lecture: 1 TO 4</span><br/></div>
        <span class="status-bold">Levels:</span><br/>
        </section>
        "#;
    let details = banner::catalog_details(html);
    assert_eq!(details.title.as_deref(), Some("Directed Study"));
    assert_eq!(
        details.department.as_deref(),
        Some("Sociology and Anthropology")
    );
    assert_eq!(details.min_credits, Some(1.0));
    assert_eq!(details.max_credits, Some(4.0));
    let fixed = banner::catalog_details(
        r#"<span class="indent-left">Credit Hours:</span><span class="credit-hours-direction">4</span><br/>"#,
    );
    assert_eq!(fixed.min_credits, Some(4.0));
    assert_eq!(fixed.max_credits, Some(4.0));
    assert_eq!(
        banner::credit_range("1.000 TO 4.000").map(|range| range.1),
        Some(4.0)
    );
    assert_eq!(banner::credit_range("0.5").map(|range| range.0), Some(0.5));
    assert!(banner::credit_range("TBA").is_none());
}

#[test]
fn readsCoursePrerequisites() {
    let html = format!(
        "<section><table class=\"basePreqTable\"><thead><tr><th>And/Or</th></tr></thead><tbody>{}</tbody></table></section>",
        [
            prereq_row("", true, "", "", false),
            prereq_row("", true, "Computer Science", "2100", false),
            prereq_row("Or", false, "Computer Science - MMC", "201M", true),
            prereq_row("Or", false, "Computer Science", "2510", true),
            prereq_row("And", true, "Computer Science", "2386", false),
            prereq_row("Or", false, "Computer Science", "3540", true),
        ]
        .join("")
    );
    let parsed = banner::prerequisites(&html, &subjects()).expect("prerequisites");
    assert_eq!(
        parsed
            .referenced_courses()
            .iter()
            .map(|course| course.to_string())
            .collect::<Vec<_>>(),
        vec!["CS 2100", "CS 201M", "CS 2510", "CS 2386", "CS 3540"]
    );
    assert!(parsed.is_satisfied(&set_of(&[key("CS 2510"), key("CS 3540")]), true));
    assert!(!parsed.is_satisfied(&set_of(&[key("CS 2510")]), true));
    let empty = "<section aria-labelledby=\"preReqs\"><h3>Catalog Prerequisites</h3> No prerequisite information available.</section>";
    assert!(banner::prerequisites(empty, &subjects()).is_none());
}

#[test]
fn readsMutualExclusions() {
    let none = r#"
        <section aria-labelledby="mexc">
            No Mutual Exclusion information available.
        </section>
        "#;
    assert!(banner::mutual_exclusions(none, &subjects()).is_empty());
    let table = r#"
        <section aria-labelledby="mexc"><table><thead><tr><th>Subject</th><th>Course Number</th><th>Title</th></tr></thead><tbody>
        <tr><td>Computer Science</td><td>2510</td><td>Fundamentals of Computer Science 2</td></tr>
        <tr><td>Mathematics</td><td>1341</td><td>Calculus 1</td></tr>
        </tbody></table></section>
        "#;
    assert_eq!(
        banner::mutual_exclusions(table, &subjects()),
        vec![key("CS 2510"), key("MATH 1341")]
    );
    let leading = "<section><table><tbody><tr><td>Or</td><td>Computer Science</td><td>2500</td></tr></tbody></table></section>";
    assert_eq!(
        banner::mutual_exclusions(leading, &subjects()),
        vec![key("CS 2500")]
    );
    let text = "<section>Credit cannot be earned for both CS 2510 and CS 2100.</section>";
    assert_eq!(
        banner::mutual_exclusions(text, &subjects()),
        vec![key("CS 2510"), key("CS 2100")]
    );
}

#[test]
fn readsCrossListedSections() {
    let html = r#"
        <section aria-labelledby="xlstSections">
            <span class="status-bold">Section Enrollment:</span> 4<br/>
            Sections:<br/>
        <table>
            <thead><tr><th>CRN</th><th>Subject</th><th>Course Number</th><th>Title</th><th>Section</th></tr></thead>
            <tbody>
                <tr><td>22773</td><td>English</td><td>2690</td><td>Boston in Literature</td><td>01</td></tr>
            </tbody>
        </table>
        </section>
        "#;
    let sections = catalog::cross_listed_sections(html, &subjects());
    assert_eq!(
        sections,
        vec![CrossListedSection {
            crn: "22773".into(),
            course: key("ENGL 2690"),
            title: "Boston in Literature".into(),
            section: "01".into()
        }]
    );
    assert!(catalog::cross_listed_sections(
        "<section>No cross list sections</section>",
        &subjects()
    )
    .is_empty());
}

#[test]
fn courseInfoNormalizesEquivalents() {
    let info = CourseInfo::entry(
        key("CS 2100"),
        "PDI 1",
        Some(4.0),
        Some(4.0),
        None,
        vec![
            key("CS 2510"),
            key("CS 2100"),
            key("CS 2510"),
            key("CS 2000"),
        ],
        "202710",
        0.0,
    );
    assert_eq!(info.equivalents, vec![key("CS 2000"), key("CS 2510")]);
    assert_eq!(info.credits_label().as_deref(), Some("4 credits"));
    assert_eq!(
        CourseInfo::entry(
            key("CS 4973"),
            "Topics",
            Some(1.0),
            Some(4.0),
            None,
            vec![],
            "202710",
            0.0
        )
        .credits_label()
        .as_deref(),
        Some("1–4 credits")
    );
    assert!(CourseInfo::new(key("CS 4973"), "Topics", "202710", 0.0)
        .credits_label()
        .is_none());
}

#[test]
fn groupsAreSymmetric() {
    let cs2100 = key("CS 2100");
    let cs2510 = key("CS 2510");
    let engl = key("ENGL 1102");
    let engw = key("ENGW 1102");
    let cs3100 = key("CS 3100");
    let equivalences = CourseEquivalences::from_pairs(&[
        (cs2100.clone(), cs2510.clone()),
        (engw.clone(), engl.clone()),
    ]);
    assert_eq!(
        equivalences.equivalents(&cs2100),
        BTreeSet::from([cs2510.clone()])
    );
    assert_eq!(
        equivalences.equivalents(&cs2510),
        BTreeSet::from([cs2100.clone()])
    );
    assert!(equivalences.are_equivalent(&engl, &engw));
    assert!(equivalences.are_equivalent(&cs2100, &cs2100));
    assert!(!equivalences.are_equivalent(&cs2100, &engl));
    assert!(!equivalences.equivalents(&cs2100).contains(&cs2100));
    assert!(equivalences.equivalents(&cs3100).is_empty());
    assert_eq!(equivalences.group_of(&cs3100), vec![cs3100]);
    assert!(CourseEquivalences::none().is_empty());
}

#[test]
fn chainsAreOneGroup() {
    let cs2000 = key("CS 2000");
    let cs2100 = key("CS 2100");
    let cs2500 = key("CS 2500");
    let forward = CourseEquivalences::from_pairs(&[
        (cs2500.clone(), cs2000.clone()),
        (cs2000.clone(), cs2100.clone()),
    ]);
    let backward = CourseEquivalences::from_pairs(&[
        (cs2100.clone(), cs2000.clone()),
        (cs2000.clone(), cs2500.clone()),
    ]);
    assert_eq!(forward, backward);
    assert_eq!(forward.groups.len(), 1);
    assert_eq!(
        forward.equivalents(&cs2500),
        BTreeSet::from([cs2000.clone(), cs2100.clone()])
    );
    assert!(forward.are_equivalent(&cs2500, &cs2100));
    assert_eq!(forward.group_of(&cs2100), vec![cs2000, cs2100, cs2500]);
}

#[test]
fn overlappingGroupsMergeAndSingletonsAreDropped() {
    let cs2000 = key("CS 2000");
    let cs2100 = key("CS 2100");
    let cs2500 = key("CS 2500");
    let cs2510 = key("CS 2510");
    let cs3100 = key("CS 3100");
    let cs3500 = key("CS 3500");
    let equivalences = CourseEquivalences::from_groups(&[
        vec![cs2500.clone(), cs2000.clone()],
        vec![cs3100.clone(), cs3500.clone()],
        vec![cs2000.clone(), cs2100.clone()],
        vec![cs2510.clone()],
        vec![],
        vec![cs2510.clone(), cs2510.clone()],
    ]);
    assert_eq!(
        equivalences.groups,
        vec![
            vec![cs2000.clone(), cs2100.clone(), cs2500.clone()],
            vec![cs3100.clone(), cs3500.clone()]
        ]
    );
    assert_eq!(
        equivalences.courses(),
        BTreeSet::from([cs2000, cs2100, cs2500, cs3100, cs3500])
    );
    assert!(!equivalences.courses().contains(&cs2510));
}

#[test]
fn expandingWhatWasTaken() {
    let cs2100 = key("CS 2100");
    let cs2510 = key("CS 2510");
    let cs3100 = key("CS 3100");
    let cs3500 = key("CS 3500");
    let engl = key("ENGL 1102");
    let equivalences =
        CourseEquivalences::from_pairs(&[(cs2100.clone(), cs2510.clone()), (cs3100, cs3500)]);
    let taken = set_of(&[cs2510.clone(), engl.clone()]);
    assert_eq!(
        equivalences.expand(&taken),
        set_of(&[cs2510.clone(), cs2100, engl.clone()])
    );
    assert!(equivalences.expand(&HashSet::new()).is_empty());
    assert_eq!(CourseEquivalences::none().expand(&taken), taken);
}

#[test]
fn satisfierIsTheTakenCourse() {
    let cs2000 = key("CS 2000");
    let cs2100 = key("CS 2100");
    let cs2500 = key("CS 2500");
    let cs3100 = key("CS 3100");
    let cs2510 = key("CS 2510");
    let equivalences = CourseEquivalences::from_pairs(&[
        (cs2500.clone(), cs2000.clone()),
        (cs2000.clone(), cs2100.clone()),
    ]);
    assert_eq!(
        equivalences
            .satisfier(&cs2100, &set_of(&[cs2100.clone(), cs2500.clone()]))
            .cloned(),
        Some(cs2100.clone())
    );
    assert_eq!(
        equivalences
            .satisfier(&cs2100, &set_of(std::slice::from_ref(&cs2500)))
            .cloned(),
        Some(cs2500.clone())
    );
    assert_eq!(
        equivalences
            .satisfier(&cs2100, &set_of(&[cs2500.clone(), cs2000.clone()]))
            .cloned(),
        Some(cs2000.clone())
    );
    assert_eq!(
        equivalences.satisfiers(&cs2100, &set_of(&[cs2500.clone(), cs2000.clone()])),
        vec![cs2000, cs2500]
    );
    assert!(equivalences
        .satisfier(&cs2100, &set_of(std::slice::from_ref(&cs3100)))
        .is_none());
    assert_eq!(
        equivalences
            .satisfier(&cs3100, &set_of(std::slice::from_ref(&cs3100)))
            .cloned(),
        Some(cs3100)
    );
    assert!(CourseEquivalences::none()
        .satisfier(&cs2100, &set_of(&[cs2510]))
        .is_none());
}

#[test]
fn mergingTwoSources() {
    let cs2000 = key("CS 2000");
    let cs2100 = key("CS 2100");
    let cs2500 = key("CS 2500");
    let cs2510 = key("CS 2510");
    let cs3100 = key("CS 3100");
    let cs3500 = key("CS 3500");
    let banner = CourseEquivalences::from_pairs(&[(cs2100.clone(), cs2510.clone())]);
    let student = CourseEquivalences::from_pairs(&[
        (cs2510.clone(), cs2500.clone()),
        (cs3100.clone(), cs3500.clone()),
    ]);
    let merged = banner.merging(&student);
    assert!(merged.are_equivalent(&cs2100, &cs2500));
    assert!(merged.are_equivalent(&cs3100, &cs3500));
    assert_eq!(banner.merging(&CourseEquivalences::none()), banner);
    assert_eq!(CourseEquivalences::none().merging(&student), student);
    assert!(banner
        .adding(cs2000, cs2100)
        .are_equivalent(&key("CS 2000"), &cs2510));
}

#[test]
fn roundTripsThroughJSON() {
    let equivalences = CourseEquivalences::from_pairs(&[
        (key("CS 2100"), key("CS 2510")),
        (key("CS 3100"), key("CS 3500")),
    ]);
    let decoded: CourseEquivalences =
        serde_json::from_slice(&serde_json::to_vec(&equivalences).unwrap()).unwrap();
    assert_eq!(decoded, equivalences);
}

#[test]
fn studentDeclarationsBecomeGroups() {
    let cs2000 = key("CS 2000");
    let cs2100 = key("CS 2100");
    let cs2500 = key("CS 2500");
    let cs2510 = key("CS 2510");
    let declared = vec![
        CourseEquivalence {
            id: Uuid::new_v4(),
            course: cs2510.clone(),
            counts_as: cs2100.clone(),
            note: "Renumbered".into(),
        },
        CourseEquivalence {
            id: Uuid::new_v4(),
            course: cs2500.clone(),
            counts_as: cs2000.clone(),
            note: String::new(),
        },
        CourseEquivalence {
            id: Uuid::new_v4(),
            course: cs2000.clone(),
            counts_as: cs2100.clone(),
            note: String::new(),
        },
    ];
    let equivalences = CourseEquivalences::from_declarations(&declared);
    assert_eq!(
        equivalences.groups,
        vec![vec![cs2000, cs2100.clone(), cs2500, cs2510.clone()]]
    );
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    let added = declare_equivalence(&mut plan, cs2510, cs2100.clone(), "Renumbered");
    let reversed = declare_equivalence(&mut plan, cs2100.clone(), key("CS 2510"), "");
    let same = declare_equivalence(&mut plan, cs2100.clone(), cs2100, "");
    assert!(added && !reversed && !same);
    assert_eq!(plan.course_equivalences.len(), 1);
    assert!(
        CourseEquivalences::from_declarations(&plan.course_equivalences)
            .are_equivalent(&key("CS 2100"), &key("CS 2510"))
    );
    let id = plan.course_equivalences[0].id;
    remove_equivalence(&mut plan, id);
    assert!(CourseEquivalences::from_declarations(&plan.course_equivalences).is_empty());
}

#[test]
fn plansKeepTheirEquivalences() {
    let cs2100 = key("CS 2100");
    let cs2510 = key("CS 2510");
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    declare_equivalence(&mut plan, cs2510.clone(), cs2100, "Renumbered");
    let round_trip: DegreePlan =
        serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    assert_eq!(round_trip, plan);
    assert_eq!(
        round_trip
            .course_equivalences
            .first()
            .map(|item| item.note.as_str()),
        Some("Renumbered")
    );
    let mut value = serde_json::to_value(&plan).unwrap();
    value.as_object_mut().unwrap().remove("courseEquivalences");
    let older: DegreePlan = serde_json::from_value(value.clone()).unwrap();
    assert!(older.course_equivalences.is_empty() && older.name == "Plan");
    let draft = planlogic::new_degree_plan_draft("Draft", Some(&plan), &[], 2026, 10);
    assert_eq!(
        draft
            .course_equivalences
            .iter()
            .map(|item| item.course.clone())
            .collect::<Vec<_>>(),
        vec![cs2510]
    );
    assert_ne!(
        draft.course_equivalences.first().map(|item| item.id),
        plan.course_equivalences.first().map(|item| item.id)
    );
    value["courseEquivalences"] = serde_json::json!("not a list");
    match serde_json::from_value::<DegreePlan>(value) {
        Ok(broken) => assert!(broken.course_equivalences.is_empty() && broken.name == "Plan"),
        Err(err) => panic!(
            "expected a plan named \"Plan\" with empty course equivalences; decode error: {err}"
        ),
    }
}

#[test]
fn crossListedCoursesAreEquivalentOnlyWhenTheyAreTheSameCourse() {
    let courses = vec![
        course("AFCS 1225", "Gender, Race, and Medicine", None, true, false),
        course(
            "HIST 1225",
            "Gender, Race, and Medicine ",
            None,
            true,
            false,
        ),
        course("WMNS 1225", "Gender, Race, and Medicine", None, true, false),
        course("ALY 6983", "Special Topics", None, true, true),
        course("CS 6983", "Special Topics", None, true, true),
        course("CS 3000", "Algorithms and Data", None, true, false),
        course("CS 5800", "Algorithms", None, true, false),
        course("ARCH 5310", "Design Tactics", None, true, false),
        course("LARC 5310", "Urban Landscape Seminar", None, true, false),
    ];
    let sections = vec![
        section("1", "AFCS 1225", "", &["2", "3"]),
        section("2", "HIST 1225", "", &["1", "3"]),
        section("3", "WMNS 1225", "", &["1", "2"]),
        section("4", "ALY 6983", "", &["5"]),
        section("5", "CS 6983", "", &["4"]),
        section("6", "CS 3000", "", &["7"]),
        section("7", "CS 5800", "", &["6"]),
        section("8", "ARCH 5310", "", &["9"]),
        section("9", "LARC 5310", "", &["8"]),
    ];
    let equivalences = CourseEquivalences::from_cross_listed(&index(courses, sections, "202710"));
    assert_eq!(
        equivalences.groups,
        vec![vec![key("AFCS 1225"), key("HIST 1225"), key("WMNS 1225")]]
    );
}

#[test]
fn equivalentCoursesSatisfyARequisite() {
    let cs1800 = key("CS 1800");
    let cs2100 = key("CS 2100");
    let cs2510 = key("CS 2510");
    let needed = and(vec![requisite("CS 2100"), requisite("CS 1800")]);
    let renumbered = CourseEquivalences::from_pairs(&[(cs2100.clone(), cs2510.clone())]);
    assert_eq!(
        planlogic::status(
            Some(&needed),
            &set_of(&[cs2510.clone(), cs1800.clone()]),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Missing("CS 2100".into())
    );
    assert_eq!(
        planlogic::status(
            Some(&needed),
            &set_of(&[cs2510.clone(), cs1800]),
            &renumbered
        ),
        PrereqStatus::Met
    );
    assert_eq!(
        planlogic::status(Some(&needed), &set_of(&[cs2510]), &renumbered),
        PrereqStatus::Missing("CS 1800".into())
    );
    let chain = CourseEquivalences::from_pairs(&[
        (key("CS 2500"), key("CS 2000")),
        (key("CS 2000"), cs2100.clone()),
    ]);
    assert_eq!(
        planlogic::status(
            Some(&Requisite::Course(cs2100)),
            &set_of(&[key("CS 2500")]),
            &chain
        ),
        PrereqStatus::Met
    );
}

#[test]
fn summariesAndCountsKnowEquivalents() {
    let cs1800 = key("CS 1800");
    let cs2100 = key("CS 2100");
    let cs2510 = key("CS 2510");
    let needed = and(vec![either(&["CS 2100", "CS 201M"]), requisite("CS 1800")]);
    let renumbered = CourseEquivalences::from_pairs(&[(cs2100, cs2510.clone())]);
    assert_eq!(
        planlogic::unmet_summary(
            &needed,
            &set_of(std::slice::from_ref(&cs2510)),
            &CourseEquivalences::none()
        )
        .as_deref(),
        Some("(CS 2100 or CS 201M) and CS 1800")
    );
    assert_eq!(
        planlogic::unmet_summary(&needed, &set_of(std::slice::from_ref(&cs2510)), &renumbered)
            .as_deref(),
        Some("CS 1800")
    );
    assert!(planlogic::unmet_summary(
        &needed,
        &set_of(&[cs2510.clone(), cs1800.clone()]),
        &renumbered
    )
    .is_none());
    assert_eq!(
        planlogic::missing_count(
            &needed,
            &set_of(std::slice::from_ref(&cs2510)),
            &CourseEquivalences::none()
        ),
        2
    );
    assert_eq!(
        planlogic::missing_count(&needed, &set_of(&[cs2510]), &renumbered),
        1
    );
}

#[test]
fn saysWhichEquivalentCoveredACourse() {
    let cs2100 = key("CS 2100");
    let cs2510 = key("CS 2510");
    let needed = either(&["CS 2100", "CS 1800"]);
    let renumbered = CourseEquivalences::from_pairs(&[(cs2100.clone(), cs2510.clone())]);
    let mut expected = BTreeMap::new();
    expected.insert(cs2100.clone(), cs2510.clone());
    assert_eq!(
        covering_equivalents(&needed, &set_of(std::slice::from_ref(&cs2510)), &renumbered),
        expected
    );
    assert!(covering_equivalents(&needed, &set_of(&[cs2510, cs2100]), &renumbered).is_empty());
}

fn analysis_plan(completed: &[&str], planned_codes: &[&str]) -> DegreePlan {
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = completed
        .iter()
        .map(|code| DegreePlanCourse::catalog(key(code), *code, 4.0))
        .collect();
    plan.terms = vec![semester(PlanSeason::Spring, 2027, planned_codes, 4.0)];
    plan
}

#[test]
fn coursesTheTermDoesNotListAreCheckedThroughTheLookup() {
    let cs3386 = key("CS 3386");
    let cs4050 = key("CS 4050");
    let plan = analysis_plan(&["CS 1800", "CS 2510"], &["CS 3386", "CS 4050", "CS 2963"]);
    let needs = and(vec![either(&["CS 2100", "CS 2510"]), requisite("CS 2386")]);
    let mut table = HashMap::new();
    table.insert(cs3386.clone(), Some(needs));
    table.insert(cs4050.clone(), None);
    let term_index = index(
        vec![course("CS 1800", "", None, true, false)],
        vec![],
        "202710",
    );
    let lookup = |course: &CourseKey| lookup_of(&table, course);
    assert_eq!(
        planlogic::prerequisite_status(
            &cs3386,
            0,
            &plan,
            Some(&term_index),
            None,
            &CourseEquivalences::none()
        ),
        PrereqStatus::Unknown
    );
    assert_eq!(
        planlogic::prerequisite_status(
            &cs3386,
            0,
            &plan,
            Some(&term_index),
            Some(&lookup),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Missing("CS 2386".into())
    );
    assert_eq!(
        planlogic::prerequisite_status(
            &cs4050,
            0,
            &plan,
            Some(&term_index),
            Some(&lookup),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Met
    );
    assert_eq!(
        planlogic::prerequisite_status(
            &key("CS 2963"),
            0,
            &plan,
            Some(&term_index),
            Some(&lookup),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Unknown
    );
    assert_eq!(
        planlogic::prerequisite_status(
            &cs3386,
            0,
            &plan,
            None,
            Some(&lookup),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Missing("CS 2386".into())
    );
}

#[test]
fn theTermsOwnRequisitesWinWhenItHasDetails() {
    let plan = analysis_plan(&["CS 1800"], &["CS 3000"]);
    let listed = index(
        vec![course(
            "CS 3000",
            "",
            Some(requisite("CS 1800")),
            true,
            false,
        )],
        vec![],
        "202710",
    );
    let wrong = |_: &CourseKey| Some(Some(requisite("CS 9999")));
    assert_eq!(
        planlogic::prerequisite_status(
            &key("CS 3000"),
            0,
            &plan,
            Some(&listed),
            Some(&wrong),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Met
    );
    let stub = index(
        vec![course("CS 3000", "", None, false, false)],
        vec![],
        "202710",
    );
    assert_eq!(
        planlogic::prerequisite_status(
            &key("CS 3000"),
            0,
            &plan,
            Some(&stub),
            None,
            &CourseEquivalences::none()
        ),
        PrereqStatus::Unknown
    );
    assert_eq!(
        planlogic::prerequisite_status(
            &key("CS 3000"),
            0,
            &plan,
            Some(&stub),
            Some(&wrong),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Missing("CS 9999".into())
    );
}

#[test]
fn equivalencesApplyToPlannedCourses() {
    let cs2100 = key("CS 2100");
    let cs2510 = key("CS 2510");
    let cs3386 = key("CS 3386");
    let plan = analysis_plan(&["CS 2510"], &["CS 3386"]);
    let mut table = HashMap::new();
    table.insert(cs3386.clone(), Some(Requisite::Course(cs2100.clone())));
    let lookup = |course: &CourseKey| lookup_of(&table, course);
    let renumbered = CourseEquivalences::from_pairs(&[(cs2100.clone(), cs2510.clone())]);
    assert_eq!(
        planlogic::prerequisite_status(
            &cs3386,
            0,
            &plan,
            None,
            Some(&lookup),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Missing("CS 2100".into())
    );
    assert_eq!(
        planlogic::prerequisite_status(&cs3386, 0, &plan, None, Some(&lookup), &renumbered),
        PrereqStatus::Met
    );
    let mut declaring = plan;
    declare_equivalence(&mut declaring, cs2510, cs2100, "");
    assert_eq!(
        planlogic::prerequisite_status(
            &cs3386,
            0,
            &declaring,
            None,
            Some(&lookup),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Met
    );
}

#[test]
fn theLoadedCatalogsPrerequisitesStillWorkWithoutNewArguments() {
    let plan = analysis_plan(&["CS 2510"], &["CS 3000"]);
    let listed = index(
        vec![course(
            "CS 3000",
            "",
            Some(either(&["CS 2100", "CS 2510"])),
            true,
            false,
        )],
        vec![],
        "202710",
    );
    assert_eq!(
        planlogic::prerequisite_status(
            &key("CS 3000"),
            0,
            &plan,
            Some(&listed),
            None,
            &CourseEquivalences::none()
        ),
        PrereqStatus::Met
    );
}

#[test]
fn aRequirementIsMetByAnEquivalentCourse() {
    let cs3000 = key("CS 3000");
    let cs3990 = key("CS 3990");
    let student = audit_plan(&["CS 1800", "CS 1802", "CS 3990"], &[]);
    let without = run_audit(&student, &CourseEquivalences::none());
    assert_eq!(
        rule_labeled(&without, "CS 3000").map(|rule| rule.status),
        Some(AuditStatus::Missing)
    );
    let audit = run_audit(
        &student,
        &CourseEquivalences::from_pairs(&[(cs3000.clone(), cs3990.clone())]),
    );
    let rule = rule_labeled(&audit, "CS 3000").expect("CS 3000");
    assert_eq!(rule.status, AuditStatus::Complete);
    let use_ = rule.used.first().expect("used");
    assert_eq!(use_.key, cs3990);
    assert_eq!(use_.satisfies.as_ref(), Some(&cs3000));
    assert!(use_.satisfies.is_some());
    assert_eq!(use_.counts_as_label().as_deref(), Some("counts as CS 3000"));
    assert!(use_.is_completed());
    assert_eq!(rule.use_for(&cs3000).map(|item| &item.key), Some(&cs3990));
    assert!(rule.use_for(&cs3990).is_none());
    let plain = rule_labeled(&audit, "CS 1800 and CS 1802").expect("CS 1800 and CS 1802");
    assert!(plain
        .used
        .iter()
        .all(|item| item.satisfies.is_none() && item.counts_as_label().is_none()));
    assert_eq!(
        plain.use_for(&key("CS 1800")).map(|item| &item.key),
        Some(&key("CS 1800"))
    );
}

#[test]
fn theStudentsDeclaredEquivalencesApplyOnTheirOwn() {
    let cs3000 = key("CS 3000");
    let mut student = audit_plan(&["CS 1800", "CS 1802", "CS 3990"], &[]);
    declare_equivalence(&mut student, key("CS 3990"), cs3000.clone(), "");
    let audit = run_audit(&student, &CourseEquivalences::none());
    assert_eq!(
        rule_labeled(&audit, "CS 3000").map(|rule| rule.status),
        Some(AuditStatus::Complete)
    );
    assert_eq!(
        rule_labeled(&audit, "CS 3000")
            .and_then(|rule| rule.used.first())
            .and_then(|item| item.satisfies.clone()),
        Some(cs3000)
    );
}

#[test]
fn plannedEquivalentsCountAsPlanned() {
    let cs3000 = key("CS 3000");
    let student = audit_plan(&["CS 1800", "CS 1802"], &["CS 3990"]);
    let audit = run_audit(
        &student,
        &CourseEquivalences::from_pairs(&[(cs3000.clone(), key("CS 3990"))]),
    );
    let rule = rule_labeled(&audit, "CS 3000").expect("CS 3000");
    assert_eq!(rule.status, AuditStatus::Planned);
    assert_eq!(
        rule.used.first().map(|item| item.is_completed()),
        Some(false)
    );
    assert_eq!(
        rule.used.first().and_then(|item| item.satisfies.clone()),
        Some(cs3000)
    );
}

#[test]
fn everyCourseStillCountsOnce() {
    let cs3000 = key("CS 3000");
    let cs3990 = key("CS 3990");
    let student = audit_plan(&["CS 1800", "CS 1802", "CS 3000", "CS 3990"], &[]);
    let audit = run_audit(
        &student,
        &CourseEquivalences::from_pairs(&[(cs3000.clone(), cs3990.clone())]),
    );
    let rule = rule_labeled(&audit, "CS 3000").expect("CS 3000");
    assert_eq!(
        rule.used
            .iter()
            .map(|item| item.key.clone())
            .collect::<Vec<_>>(),
        vec![cs3000]
    );
    assert!(rule.used.first().unwrap().satisfies.is_none());
    let counted: Vec<CourseKey> = audit
        .rule_results()
        .into_iter()
        .flat_map(|result| result.used.iter().map(|item| item.key.clone()))
        .collect();
    let unique: HashSet<CourseKey> = counted.iter().cloned().collect();
    assert_eq!(counted.len(), unique.len());
    let mut elsewhere: Vec<CourseKey> = audit
        .rule_results()
        .into_iter()
        .filter(|result| result.rule.label != "CS 3000")
        .flat_map(|result| result.used.iter().map(|item| item.key.clone()))
        .collect();
    elsewhere.extend(audit.general_electives.iter().map(|item| item.key.clone()));
    elsewhere.extend(audit.unused.iter().map(|item| item.key.clone()));
    assert_eq!(
        elsewhere.iter().filter(|course| *course == &cs3990).count(),
        1
    );
}

#[test]
fn aChainOfRenumberingsReachesTheRequirement() {
    let cs3000 = key("CS 3000");
    let cs3990 = key("CS 3990");
    let cs3991 = key("CS 3991");
    let student = audit_plan(&["CS 1800", "CS 1802", "CS 3991"], &[]);
    let chain =
        CourseEquivalences::from_pairs(&[(cs3000, cs3990.clone()), (cs3990, cs3991.clone())]);
    let audit = run_audit(&student, &chain);
    assert_eq!(
        rule_labeled(&audit, "CS 3000").map(|rule| rule.status),
        Some(AuditStatus::Complete)
    );
    assert_eq!(
        rule_labeled(&audit, "CS 3000")
            .and_then(|rule| rule.used.first())
            .map(|item| item.key.clone()),
        Some(cs3991)
    );
}

#[test]
fn alternativesAcceptEquivalents() {
    let cs4530 = key("CS 4530");
    let student = audit_plan(&["CS 1800", "CS 1802", "CS 4999"], &[]);
    let audit = run_audit(
        &student,
        &CourseEquivalences::from_pairs(&[(cs4530.clone(), key("CS 4999"))]),
    );
    let rule = rule_labeled(&audit, "CS 4500 or CS 4530").expect("CS 4500 or CS 4530");
    assert_eq!(rule.status, AuditStatus::Complete);
    assert_eq!(
        rule.used.first().and_then(|item| item.satisfies.clone()),
        Some(cs4530)
    );
}

#[test]
fn rangesAcceptEquivalentsOfCoursesInThem() {
    let student = audit_plan(&["CS 1800", "CS 1802", "CS 2100", "CS 4973"], &[]);
    fn electives(audit: &DegreeAudit) -> Option<&RuleResult> {
        audit.rule_results().into_iter().find(|result| {
            result
                .rule
                .label
                .starts_with("Complete 8 semester hours of CS")
        })
    }
    let without = run_audit(&student, &CourseEquivalences::none());
    assert_eq!(
        electives(&without).map(|rule| rule.status),
        Some(AuditStatus::Partial)
    );
    assert_eq!(
        electives(&without).map(|rule| rule
            .used
            .iter()
            .map(|item| item.key.clone())
            .collect::<Vec<_>>()),
        Some(vec![key("CS 4973")])
    );
    let renumbered = CourseEquivalences::from_pairs(&[(key("CS 2100"), key("CS 2510"))]);
    let audit = run_audit(&student, &renumbered);
    let rule = electives(&audit).expect("electives");
    assert_eq!(rule.status, AuditStatus::Complete);
    let use_ = rule
        .used
        .iter()
        .find(|item| item.key.to_string() == "CS 2100")
        .expect("CS 2100");
    assert_eq!(
        use_.satisfies
            .as_ref()
            .map(|course| course.to_string())
            .as_deref(),
        Some("CS 2510")
    );
}

#[test]
fn substitutionsAcceptEquivalentsToo() {
    let cs3000 = key("CS 3000");
    let cs3990 = key("CS 3990");
    let mut student = audit_plan(&["CS 1800", "CS 1802", "CS 3990"], &[]);
    let requirements = requirements();
    let rule = requirements
        .all_rules()
        .into_iter()
        .find(|rule| rule.label == "CS 4500 or CS 4530")
        .expect("rule")
        .clone();
    student.substitutions = vec![RequirementSubstitution {
        id: Uuid::new_v4(),
        rule_id: rule.id.clone(),
        rule_title: "Software development".into(),
        change: SubstitutionChange::CountCourse(cs3000.clone()),
        note: String::new(),
        approved_on: None,
        created_at: None,
    }];
    let audit = DegreeAudit::run(
        &student,
        &requirements,
        &CourseEquivalences::from_pairs(&[(cs3000, cs3990.clone())]),
        &DegreeCourseAttributes::none(),
    );
    let result = audit.result_for(&rule.id).expect("result");
    assert_eq!(
        result.used.first().map(|item| item.key.clone()),
        Some(cs3990)
    );
    assert_eq!(
        result.used.first().map(|item| item.by_substitution),
        Some(true)
    );
    assert!(audit.unmatched_substitutions.is_empty());
}

#[test]
fn searchesTheNewestTermThenEarlierFallsAndSprings() {
    assert_eq!(
        CourseInfoRepository::<FakeBanner>::catalog_terms(&banner_terms(), 3),
        vec!["202710", "202630", "202610", "202530"]
    );
    assert_eq!(
        CourseInfoRepository::<FakeBanner>::catalog_terms(&banner_terms(), 0),
        vec!["202710"]
    );
    assert_eq!(
        CourseInfoRepository::<FakeBanner>::catalog_terms(
            &terms(&["202650", "202630", "202610"]),
            8
        ),
        vec!["202650", "202630", "202610"]
    );
    assert!(
        CourseInfoRepository::<FakeBanner>::catalog_terms(&terms(&["202655", "202632"]), 8)
            .is_empty()
    );
}

#[test]
fn looksInTheLoadedCatalogsFirst() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(course_info("CS 3000", "202710", None, vec![]), "202710");
    banner.list(course_info("CS 4050", "202630", None, vec![]), "202630");
    let mut repository = info_repo(&banner, dir.path(), &clock, 8, 2);
    let loaded = index(
        vec![
            course("CS 3000", "", Some(requisite("CS 1800")), true, false),
            course("CS 4050", "", None, false, false),
        ],
        vec![],
        "202710",
    );
    let results = repository.lookup(&[key("CS 3000"), key("CS 4050")], &[loaded]);
    match results.get(&key("CS 3000")) {
        Some(Lookup::Found(info)) => {
            assert_eq!(info.prerequisites, Some(requisite("CS 1800")));
            assert_eq!(info.source_term, "202710");
            assert_eq!(info.is_retired, Some(false));
        }
        other => panic!("CS 3000 not found: {other:?}"),
    }
    assert_eq!(
        banner
            .entry_calls()
            .iter()
            .map(|(course, _)| course.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 4050"), key("CS 4050")]
    );
    assert_eq!(
        banner
            .entry_calls()
            .iter()
            .map(|(_, term)| term.clone())
            .collect::<Vec<_>>(),
        vec!["202710".to_string(), "202630".to_string()]
    );
    match results.get(&key("CS 4050")) {
        Some(Lookup::Found(info)) => {
            assert_eq!(info.source_term, "202630");
            assert_eq!(info.is_retired, Some(true));
        }
        other => panic!("CS 4050 not found: {other:?}"),
    }
}

#[test]
fn walksBackThroughPreviousYearsAndRemembersTheTerm() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(
        course_info("CS 2510", "202530", None, vec![key("CS 2100")]),
        "202530",
    );
    let mut repository = info_repo(&banner, dir.path(), &clock, 8, 2);
    match repository.lookup_one(&key("CS 2510"), &[]) {
        Lookup::Found(info) => {
            assert_eq!(info.source_term, "202530");
            assert_eq!(info.is_retired, Some(true));
            assert_eq!(info.equivalents, vec![key("CS 2100")]);
        }
        other => panic!("not found: {other:?}"),
    }
    assert_eq!(
        banner
            .entry_calls()
            .iter()
            .map(|(_, term)| term.clone())
            .collect::<Vec<_>>(),
        vec!["202710", "202630", "202610", "202530"]
    );
}

#[test]
fn foundCoursesAreCachedOnDiskForALongTime() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(
        course_info("CS 3386", "202710", Some(requisite("CS 2386")), vec![]),
        "202710",
    );
    let mut first = info_repo(&banner, dir.path(), &clock, 8, 2);
    let _ = first.lookup_one(&key("CS 3386"), &[]);
    assert_eq!(banner.entry_calls().len(), 1);

    clock.advance_days(30.0);
    let mut second = info_repo(
        &FakeBanner::new(&clock, banner_terms()),
        dir.path(),
        &clock,
        8,
        2,
    );
    let cached = match second.cached_lookup(&key("CS 3386")) {
        Some(Lookup::Found(info)) => info,
        other => panic!("not cached: {other:?}"),
    };
    assert_eq!(cached.prerequisites, Some(requisite("CS 2386")));
    assert_eq!(cached.is_retired, Some(false));
    assert_eq!(
        second.lookup_one(&key("CS 3386"), &[]),
        Lookup::Found(cached)
    );
    assert_eq!(banner.entry_calls().len(), 1);

    clock.advance_days(200.0);
    let failing = FakeBanner::new(&clock, banner_terms());
    failing.fail("202710", true);
    let mut third = info_repo(&failing, dir.path(), &clock, 8, 2);
    assert!(third.cached_lookup(&key("CS 3386")).is_none());
    match third.lookup_one(&key("CS 3386"), &[]) {
        Lookup::Found(stale) => assert_eq!(stale.prerequisites, Some(requisite("CS 2386"))),
        other => panic!("stale entry not used: {other:?}"),
    }
    banner.list(
        course_info("CS 3386", "202710", Some(requisite("CS 2000")), vec![]),
        "202710",
    );
    let mut refreshed = info_repo(&banner, dir.path(), &clock, 8, 2);
    match refreshed.lookup_one(&key("CS 3386"), &[]) {
        Lookup::Found(fresh) => assert_eq!(fresh.prerequisites, Some(requisite("CS 2000"))),
        other => panic!("not refreshed: {other:?}"),
    }
}

#[test]
fn missingCoursesAreRememberedForAWeek() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    let mut repository = info_repo(&banner, dir.path(), &clock, 3, 2);
    assert_eq!(
        repository.lookup_one(&key("CS 2963"), &[]),
        Lookup::NotFound
    );
    assert_eq!(
        banner
            .entry_calls()
            .iter()
            .map(|(_, term)| term.clone())
            .collect::<Vec<_>>(),
        vec!["202710", "202630", "202610", "202530"]
    );
    banner.reset_calls();

    clock.advance_days(6.0);
    repository.now = clock.now();
    assert_eq!(
        repository.lookup_one(&key("CS 2963"), &[]),
        Lookup::NotFound
    );
    assert!(banner.entry_calls().is_empty());
    let mut relaunched = info_repo(&banner, dir.path(), &clock, 8, 2);
    assert_eq!(
        relaunched.cached_lookup(&key("CS 2963")),
        Some(Lookup::NotFound)
    );

    clock.advance_days(2.0);
    repository.now = clock.now();
    assert!(repository.cached_lookup(&key("CS 2963")).is_none());
    assert_eq!(
        repository.lookup_one(&key("CS 2963"), &[]),
        Lookup::NotFound
    );
    assert_eq!(banner.entry_calls().len(), 4);

    clock.advance_days(8.0);
    repository.now = clock.now();
    banner.list(course_info("CS 2963", "202710", None, vec![]), "202710");
    assert!(
        matches!(
            repository.lookup_one(&key("CS 2963"), &[]),
            Lookup::Found(_)
        ),
        "not found after it appeared"
    );
}

#[test]
fn failuresAreNotCached() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(course_info("CS 4050", "202630", None, vec![]), "202630");
    banner.fail("202630", true);
    let mut repository = info_repo(&banner, dir.path(), &clock, 8, 2);
    assert_eq!(repository.lookup_one(&key("CS 4050"), &[]), Lookup::Failed);
    assert!(repository.cached_lookup(&key("CS 4050")).is_none());
    banner.fail("202630", false);
    match repository.lookup_one(&key("CS 4050"), &[]) {
        Lookup::Found(info) => assert_eq!(info.source_term, "202630"),
        other => panic!("not found after the network came back: {other:?}"),
    }
}

#[test]
fn termListFailureIsAFailureToo() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let mut repository = info_repo(&FakeBanner::new(&clock, vec![]), dir.path(), &clock, 8, 2);
    assert_eq!(repository.lookup_one(&key("CS 3386"), &[]), Lookup::Failed);
}

#[test]
fn fewCoursesAreLookedUpAtATime() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.set_delay_ms(80);
    let codes: Vec<String> = (1..=6).map(|n| format!("CS 30{}", 10 + n)).collect();
    for code in &codes {
        banner.list(course_info(code, "202710", None, vec![]), "202710");
    }
    let mut repository = info_repo(&banner, dir.path(), &clock, 8, 2);
    let wanted: Vec<CourseKey> = codes.iter().map(|code| key(code)).collect();
    let results = repository.lookup(&wanted, &[]);
    assert_eq!(results.len(), 6);
    assert!(results
        .values()
        .all(|result| matches!(result, Lookup::Found(_))));
    assert_eq!(banner.max_in_flight(), 2);
    banner.reset_calls();
    let _ = repository.lookup(&[key("CS 9001"), key("CS 9001")], &[]);
    assert_eq!(
        banner
            .entry_calls()
            .iter()
            .filter(|(_, term)| term == "202710")
            .count(),
        1
    );
}

#[test]
fn loadsInfoForCoursesTheTermDoesNotList() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(
        course_info("CS 3386", "202710", Some(requisite("CS 2386")), vec![]),
        "202710",
    );
    banner.list(course_info("CS 4050", "202630", None, vec![]), "202630");
    let mut store = make_store(&banner, dir.path(), &clock);
    let loaded = index(
        vec![course(
            "CS 3000",
            "",
            Some(requisite("CS 1800")),
            true,
            false,
        )],
        vec![],
        "202710",
    );
    assert_eq!(store.state(&key("CS 3386")), StoreState::Unknown);
    assert!(store.info(&key("CS 3386")).is_none());
    store.load(
        &[
            key("CS 3386"),
            key("CS 4050"),
            key("CS 2963"),
            key("CS 3000"),
        ],
        std::slice::from_ref(&loaded),
    );
    assert_eq!(
        store
            .info(&key("CS 3386"))
            .and_then(|info| info.prerequisites.clone()),
        Some(requisite("CS 2386"))
    );
    assert_eq!(
        store
            .info(&key("CS 4050"))
            .map(|info| info.source_term.as_str()),
        Some("202630")
    );
    assert_eq!(
        store
            .info(&key("CS 3000"))
            .and_then(|info| info.prerequisites.clone()),
        Some(requisite("CS 1800"))
    );
    assert_eq!(store.state(&key("CS 2963")), StoreState::NotFound);
    assert!(store.info(&key("CS 2963")).is_none());
    let table = store.requisite_lookup();
    assert_eq!(
        lookup_of(&table, &key("CS 3386")),
        Some(Some(requisite("CS 2386")))
    );
    assert_eq!(lookup_of(&table, &key("CS 4050")), Some(None));
    assert_eq!(lookup_of(&table, &key("CS 2963")), None);
    assert_eq!(lookup_of(&table, &key("MATH 1")), None);
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![DegreePlanCourse::catalog(key("CS 2386"), "", 4.0)];
    plan.terms = vec![semester(PlanSeason::Spring, 2027, &["CS 3386"], 4.0)];
    let lookup = |course: &CourseKey| lookup_of(&table, course);
    assert_eq!(
        planlogic::prerequisite_status(
            &key("CS 3386"),
            0,
            &plan,
            Some(&loaded),
            Some(&lookup),
            &CourseEquivalences::none()
        ),
        PrereqStatus::Met
    );
}

#[test]
fn loadingAgainChangesNothing() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(course_info("CS 3386", "202710", None, vec![]), "202710");
    let mut store = make_store(&banner, dir.path(), &clock);
    let keys = [key("CS 3386"), key("CS 2963")];
    store.load(&keys, &[]);
    let version = store.version;
    assert!(version > 0);
    let calls = banner.entry_calls().len();
    store.load(&keys, &[]);
    assert_eq!(store.version, version);
    assert_eq!(banner.entry_calls().len(), calls);
}

#[test]
fn failedLoadsCanBeRetried() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(course_info("CS 3386", "202710", None, vec![]), "202710");
    banner.fail("202710", true);
    let mut store = make_store(&banner, dir.path(), &clock);
    store.load(&[key("CS 3386")], &[]);
    assert_eq!(store.state(&key("CS 3386")), StoreState::Failed);
    banner.fail("202710", false);
    store.load(&[key("CS 3386")], &[]);
    assert_eq!(store.state(&key("CS 3386")), StoreState::Failed);
    let mut again = make_store(&banner, dir.path(), &clock);
    again.load(&[key("CS 3386")], &[]);
    assert!(
        matches!(again.state(&key("CS 3386")), StoreState::Found(_)),
        "not found after the network came back"
    );
}

#[test]
fn equivalencesMergeBannerDeclaredAndCrossListed() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(
        course_info("CS 2100", "202710", None, vec![key("CS 2510")]),
        "202710",
    );
    let mut store = make_store(&banner, dir.path(), &clock);
    let loaded = index(
        vec![
            course("HIST 1225", "Gender, Race, and Medicine", None, true, false),
            course("WMNS 1225", "Gender, Race, and Medicine", None, true, false),
        ],
        vec![
            section("1", "HIST 1225", "", &["2"]),
            section("2", "WMNS 1225", "", &["1"]),
        ],
        "202710",
    );
    store.load(&[key("CS 2100")], &[loaded]);
    let from_banner = store.equivalences(None);
    assert!(from_banner.are_equivalent(&key("CS 2100"), &key("CS 2510")));
    assert!(from_banner.are_equivalent(&key("HIST 1225"), &key("WMNS 1225")));
    assert!(!from_banner.are_equivalent(&key("CS 3100"), &key("CS 3500")));
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    declare_equivalence(&mut plan, key("CS 3500"), key("CS 3100"), "");
    declare_equivalence(&mut plan, key("CS 2500"), key("CS 2510"), "");
    let merged = store.equivalences(Some(&plan));
    assert!(merged.are_equivalent(&key("CS 3100"), &key("CS 3500")));
    assert!(merged.are_equivalent(&key("CS 2500"), &key("CS 2100")));
    assert!(merged.are_equivalent(&key("HIST 1225"), &key("WMNS 1225")));
}

#[test]
fn coursesTheTermDoesNotOfferAreCheckedFromPreviousYears() {
    let loaded = index(
        vec![course("CS 1800", "", None, true, false)],
        vec![],
        "202710",
    );
    let cs3386 = key("CS 3386");
    let cs4050 = key("CS 4050");
    let cs2963 = key("CS 2963");
    assert!(
        loaded.course(&cs3386).is_none()
            && loaded.course(&cs4050).is_none()
            && loaded.course(&cs2963).is_none()
    );
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    let needs = and(vec![
        either(&["CS 2100", "CS 201M", "CS 2510"]),
        either(&["CS 2386", "CS 3540"]),
    ]);
    banner.list(
        course_info("CS 3386", "202710", Some(needs), vec![]),
        "202710",
    );
    banner.list(course_info("CS 4050", "202630", None, vec![]), "202630");
    let mut store = make_store(&banner, dir.path(), &clock);
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = ["CS 1800", "CS 2000", "CS 2386"]
        .iter()
        .map(|code| DegreePlanCourse::catalog(key(code), *code, 4.0))
        .collect();
    plan.terms = vec![semester(
        PlanSeason::Spring,
        2027,
        &["CS 3386", "CS 4050", "CS 2963"],
        4.0,
    )];
    store.load(
        &[cs3386.clone(), cs4050.clone(), cs2963.clone()],
        std::slice::from_ref(&loaded),
    );
    let status = |course: &CourseKey,
                  plan: &DegreePlan,
                  store: &CourseInfoStore<FakeBanner, FakeBanner>| {
        let table = store.requisite_lookup();
        let lookup = |item: &CourseKey| lookup_of(&table, item);
        let equivalences = store.equivalences(Some(plan));
        planlogic::prerequisite_status(course, 0, plan, Some(&loaded), Some(&lookup), &equivalences)
    };
    assert_eq!(
        status(&cs3386, &plan, &store),
        PrereqStatus::Missing("CS 2100 or CS 201M or CS 2510".into())
    );
    assert_eq!(status(&cs4050, &plan, &store), PrereqStatus::Met);
    assert_eq!(
        store.info(&cs4050).and_then(|info| info.is_retired),
        Some(true)
    );
    assert_eq!(status(&cs2963, &plan, &store), PrereqStatus::Unknown);
    assert_eq!(store.state(&cs2963), StoreState::NotFound);
    declare_equivalence(
        &mut plan,
        key("CS 2000"),
        key("CS 2100"),
        "Curriculum change",
    );
    assert_eq!(status(&cs3386, &plan, &store), PrereqStatus::Met);
    let completed: HashSet<CourseKey> = plan.completed_keys().into_iter().collect();
    assert_eq!(
        store
            .equivalences(Some(&plan))
            .satisfier(&key("CS 2100"), &completed)
            .cloned(),
        Some(key("CS 2000"))
    );
}

#[test]
fn titlesAreReducedToTheirTellingWords() {
    assert_eq!(
        Title::parse("Principles of Human-Computer Interaction").words,
        BTreeSet::from(["human".into(), "computer".into(), "interaction".into()])
    );
    assert_eq!(
        Title::parse("PRNCPLES of Human Computer Interaction").words,
        BTreeSet::from(["human".into(), "computer".into(), "interaction".into()])
    );
    assert_eq!(
        Title::parse("Intr to Data Structures & Algorithms").words,
        BTreeSet::from(["data".into(), "structure".into(), "algorithm".into()])
    );
    assert_eq!(
        Title::parse("Introduction to Women’s Studies").words,
        BTreeSet::from(["women".into(), "study".into()])
    );
    assert_eq!(
        Title::parse("Fundamentals of Software Engineering").words,
        BTreeSet::from(["software".into(), "engineering".into()])
    );
    assert_eq!(
        Title::parse("Fundamentals of Computer Science 2").parts,
        BTreeSet::from(["2".into()])
    );
    assert_eq!(
        Title::parse("Computer Science II").parts,
        BTreeSet::from(["2".into()])
    );
    assert_eq!(
        Title::parse("Computer Science II").words,
        BTreeSet::from(["computer".into(), "science".into()])
    );
    assert!(Title::parse("Topics").words.is_empty());
}

#[test]
fn aRenumberedCourseKeepsItsTitleAlike() {
    assert_eq!(
        titles::similarity(
            "Human Computer Interaction",
            "Principles of Human-Computer Interaction"
        ),
        Some(1.0)
    );
    assert_eq!(
        titles::similarity(
            "Principles of Human-Computer Interaction",
            "Human Computer Interaction"
        ),
        Some(1.0)
    );
    assert_eq!(
        titles::similarity(
            "Prncples of Human-Computer Interaction",
            "Human Computer Interaction"
        ),
        Some(1.0)
    );
    assert_eq!(
        titles::similarity("First-Year Writing", "First Year Writing"),
        Some(1.0)
    );
}

#[test]
fn titlesThatOnlyShareAWordAreNotAlike() {
    assert!(titles::similarity("Human Computer Interaction", "Computer Systems").is_none());
    assert!(
        titles::similarity("Human Computer Interaction", "Human Factors in Computing").is_none()
    );
    assert!(titles::similarity("Data Science", "Data Structures").is_none());
    assert!(titles::similarity("Cell Biology", "Cell Culture Techniques").is_none());
    assert!(titles::similarity("Computer Systems", "Computer Networks").is_none());
}

#[test]
fn similarityIsTheShareOfWordsOrContainment() {
    let studio = titles::similarity("Object-Oriented Design", "Object-Oriented Design Studio")
        .expect("studio");
    assert!((studio - 0.75).abs() < 0.0001);
    let theory = titles::similarity("Machine Learning", "Machine Learning Theory").expect("theory");
    assert!((theory - 2.0 / 3.0).abs() < 0.0001);
    assert!(titles::similarity("Machine Learning Theory", "Machine Learning Practice").is_none());
    assert_eq!(
        titles::similarity("Cell Biology", "Cell Biology of Cancer Metastasis"),
        Some(0.6)
    );
    assert_eq!(
        titles::similarity_min("Cell Biology", "Cell Biology of Cancer Metastasis", 0.8),
        Some(0.8)
    );
    assert_eq!(
        titles::similarity_min("Machine Learning Theory", "Machine Learning Practice", 0.4),
        Some(0.5)
    );
}

#[test]
fn courseSequencesAreNotAlike() {
    assert!(titles::similarity(
        "Fundamentals of Computer Science 1",
        "Fundamentals of Computer Science 2"
    )
    .is_none());
    assert!(titles::similarity(
        "Calculus 1 for Science and Engineering",
        "Calculus 2 for Science and Engineering"
    )
    .is_none());
    assert!(titles::similarity("General Chemistry I", "General Chemistry II").is_none());
    assert!(titles::similarity("Physics for Engineering 1", "Physics for Engineering 3").is_none());
    assert_eq!(
        titles::similarity("Fundamentals of Computer Science 2", "Computer Science 2"),
        Some(1.0)
    );
    assert_eq!(
        titles::similarity("Computer Science I", "Computer Science"),
        Some(1.0)
    );
    assert_eq!(
        titles::similarity("Computer Science 1", "Computer Science I"),
        Some(1.0)
    );
    assert!(titles::similarity("Computer Science 2", "Computer Science I").is_none());
}

#[test]
fn aCourseAndItsLabOrAdvancedVersionAreNotAlike() {
    assert!(titles::similarity(
        "Sensation and Perception",
        "Laboratory in Sensation and Perception"
    )
    .is_none());
    assert!(titles::similarity("General Chemistry 1", "General Chemistry 1 Lab").is_none());
    assert!(titles::similarity(
        "Fundamentals of Computer Science 2",
        "Lab for Fundamentals of Computer Science 2"
    )
    .is_none());
    assert!(titles::similarity("Machine Learning", "Advanced Machine Learning").is_none());
    assert!(titles::similarity(
        "Advanced Machine Learning",
        "Advanced Machine Learning Methods"
    )
    .is_some());
    assert_eq!(
        titles::similarity("General Chemistry Lab", "General Chemistry Laboratory"),
        Some(1.0)
    );
    assert_eq!(
        Title::parse("Lab for Computer Systems").qualifiers,
        BTreeSet::from(["lab".into()])
    );
}

#[test]
fn shortAndGenericTitlesAreNeverAlike() {
    assert!(titles::similarity("Seminar", "Seminar").is_none());
    assert!(titles::similarity("Algorithms", "Algorithms and Data").is_none());
    assert!(titles::similarity("Calculus", "Calculus for Engineers").is_none());
    assert!(titles::similarity("Special Topics", "Special Topics").is_none());
    assert!(titles::similarity("Directed Study", "Directed Study").is_none());
    assert!(titles::similarity("Research Seminar", "Research Seminar").is_none());
    assert!(titles::similarity("Co-op Work Experience", "Co-op Work Experience").is_none());
    assert!(titles::similarity("Topics", "Topics in Computer Science").is_none());
    assert!(titles::similarity("Introduction to", "Introduction to Computer Science").is_none());
    assert!(titles::similarity("", "Human Computer Interaction").is_none());
    assert!(titles::similarity("", "").is_none());
    assert!(!Title::parse("Seminar").is_distinctive());
    assert!(!Title::parse("Directed Study").is_distinctive());
    assert!(Title::parse("Machine Learning").is_distinctive());
    assert_eq!(
        titles::similarity("Machine Learning", "Machine Learning"),
        Some(1.0)
    );
    assert_eq!(
        titles::similarity(
            "Seminar in Human Computer Interaction",
            "Human Computer Interaction"
        ),
        Some(0.75)
    );
}

#[test]
fn cutOffAuditTitlesStillMatch() {
    assert_eq!(
        titles::similarity(
            "Fundamentals of Software Engin",
            "Fundamentals of Software Engineering"
        ),
        Some(1.0)
    );
    assert_eq!(
        titles::similarity(
            "Fundamentals of Software Engineering",
            "Fundamentals of Software Engin"
        ),
        Some(1.0)
    );
    let cut = titles::similarity(
        "Principles of Human-Computer I",
        "Human Computer Interaction",
    );
    assert!(cut.is_some_and(|score| score > 0.66));
    assert!(titles::similarity(
        "Fundamentals of Software Engin",
        "Fundamentals of Software Testing"
    )
    .is_none());
    assert_eq!(
        Title::parse("Fundamentals of Software Engin")
            .cut_off_word
            .as_deref(),
        Some("engin")
    );
    assert_eq!(Title::parse("Software Engineering").cut_off_word, None);
}

#[test]
fn suggestsTheCurrentCoursesListedBesideTheRetiredOne() {
    let current = current_courses();
    let found = suggestions::suggestions(
        &key("CS 2510"),
        &suggester_catalog(),
        |course| current.contains(course),
        &CourseEquivalences::none(),
        2,
        0.5,
        3,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY,
    );
    assert_eq!(
        found
            .iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 2100")]
    );
    let suggestion = found.first().expect("suggestion");
    assert_eq!(suggestion.course, key("CS 2510"));
    assert_eq!((suggestion.together, suggestion.total), (3, 5));
    assert_eq!(suggestion.confidence(), 0.6);
    assert_eq!(
        suggestion.evidence(),
        "Listed together in 3 of the 5 prerequisites that name CS 2510"
    );
    assert_eq!(suggestion.id(), "CS 2510|CS 2100");
}

#[test]
fn thresholdsAndLimitsApply() {
    let current = current_courses();
    let wide = suggestions::suggestions(
        &key("CS 2510"),
        &suggester_catalog(),
        |course| current.contains(course),
        &CourseEquivalences::none(),
        1,
        0.2,
        3,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY,
    );
    assert_eq!(
        wide.iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 2100"), key("DS 2500")]
    );
    let limited = suggestions::suggestions(
        &key("CS 2510"),
        &suggester_catalog(),
        |course| current.contains(course),
        &CourseEquivalences::none(),
        1,
        0.2,
        1,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY,
    );
    assert_eq!(
        limited
            .iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 2100")]
    );
    let high = suggestions::suggestions(
        &key("CS 2510"),
        &suggester_catalog(),
        |course| current.contains(course),
        &CourseEquivalences::none(),
        2,
        0.8,
        3,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY,
    );
    assert!(high.is_empty());
}

#[test]
fn leavesOutWhatIsKnownOrNotCurrent() {
    let current = current_courses();
    let known = CourseEquivalences::from_pairs(&[(key("CS 2510"), key("CS 2100"))]);
    assert!(suggestions::suggestions(
        &key("CS 2510"),
        &suggester_catalog(),
        |course| current.contains(course),
        &known,
        2,
        0.5,
        3,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY
    )
    .is_empty());
    assert!(suggestions::suggestions(
        &key("CS 2510"),
        &suggester_catalog(),
        |_| false,
        &CourseEquivalences::none(),
        2,
        0.5,
        3,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY
    )
    .is_empty());
    assert!(suggestions::suggestions(
        &key("CS 9999"),
        &suggester_catalog(),
        |_| true,
        &CourseEquivalences::none(),
        2,
        0.5,
        3,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY
    )
    .is_empty());
    assert!(suggestions::suggestions(
        &key("CS 4100"),
        &suggester_catalog(),
        |_| true,
        &CourseEquivalences::none(),
        2,
        0.5,
        3,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY
    )
    .is_empty());
    let own = vec![(key("CS 2510"), either(&["CS 2510", "CS 2100"]))];
    assert!(suggestions::suggestions(
        &key("CS 2510"),
        &own,
        |_| true,
        &CourseEquivalences::none(),
        2,
        0.5,
        3,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY
    )
    .is_empty());
}

#[test]
fn aSingleDependentReadsNaturally() {
    let one = vec![
        (key("CS 3000"), either(&["CS 2100", "CS 2510"])),
        (key("CS 3001"), either(&["CS 2100", "CS 2510"])),
    ];
    let found = suggestions::suggestions(
        &key("CS 2510"),
        &one,
        |_| true,
        &CourseEquivalences::none(),
        2,
        0.5,
        3,
        |_| None,
        &[],
        titles::MINIMUM_SIMILARITY,
    );
    assert_eq!(
        found.first().map(|item| item.evidence()),
        Some("Listed together in 2 of the 2 prerequisites that name CS 2510".into())
    );
    let lone = EquivalenceSuggestion {
        course: key("CS 2510"),
        candidate: key("CS 2100"),
        together: 1,
        total: 1,
        similar_title: None,
        title_score: None,
    };
    assert_eq!(
        lone.evidence(),
        "Listed together in the prerequisites of the 1 course that names CS 2510"
    );
}

#[test]
fn suggestsAStudentCourseWithASimilarTitle() {
    let found = by_title(
        "IS 4300",
        &["CS 3000", "CS 2484", "CS 3650", "CS 2510"],
        None,
        &[],
        None,
        &CourseEquivalences::none(),
        3,
    );
    assert_eq!(
        found
            .iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 2484")]
    );
    let suggestion = found.first().expect("suggestion");
    assert_eq!(suggestion.course, key("IS 4300"));
    assert_eq!((suggestion.together, suggestion.total), (0, 0));
    assert!(!suggestion.is_listed_together());
    assert_eq!(suggestion.confidence(), 0.0);
    assert_eq!(
        suggestion.similar_title.as_deref(),
        Some("Principles of Human-Computer Interaction")
    );
    assert_eq!(suggestion.title_score, Some(1.0));
    assert_eq!(
        suggestion.evidence(),
        "Similar title: Principles of Human-Computer Interaction"
    );
    assert_eq!(suggestion.id(), "IS 4300|CS 2484");
}

#[test]
fn aTrueNegativeIsNotSuggested() {
    assert!(by_title(
        "IS 4300",
        &["CS 3650", "CS 3000", "CS 4550"],
        None,
        &[],
        None,
        &CourseEquivalences::none(),
        3
    )
    .is_empty());
}

#[test]
fn genericTitlesGiveNoTitleEvidence() {
    for generic in [
        "Seminar",
        "Special Topics",
        "Directed Study",
        "Topics",
        "Introduction to",
        "",
    ] {
        let both = HashMap::from([
            (key("IS 4300"), generic.to_string()),
            (key("CS 3000"), generic.to_string()),
        ]);
        assert!(by_title(
            "IS 4300",
            &["CS 3000"],
            Some(&both),
            &[],
            None,
            &CourseEquivalences::none(),
            3
        )
        .is_empty());
        let retired_generic = HashMap::from([
            (key("IS 4300"), generic.to_string()),
            (key("CS 3000"), "Human Computer Interaction".into()),
        ]);
        assert!(by_title(
            "IS 4300",
            &["CS 3000"],
            Some(&retired_generic),
            &[],
            None,
            &CourseEquivalences::none(),
            3
        )
        .is_empty());
        let candidate_generic = HashMap::from([
            (key("IS 4300"), "Human Computer Interaction".into()),
            (key("CS 3000"), generic.to_string()),
        ]);
        assert!(by_title(
            "IS 4300",
            &["CS 3000"],
            Some(&candidate_generic),
            &[],
            None,
            &CourseEquivalences::none(),
            3
        )
        .is_empty());
    }
    let only_candidate = HashMap::from([(key("CS 3000"), "Human Computer Interaction".into())]);
    assert!(by_title(
        "IS 4300",
        &["CS 3000"],
        Some(&only_candidate),
        &[],
        None,
        &CourseEquivalences::none(),
        3
    )
    .is_empty());
    let only_retired = HashMap::from([(key("IS 4300"), "Human Computer Interaction".into())]);
    assert!(by_title(
        "IS 4300",
        &["CS 3000"],
        Some(&only_retired),
        &[],
        None,
        &CourseEquivalences::none(),
        3
    )
    .is_empty());
    let sequence = HashMap::from([
        (key("CS 2510"), "Fundamentals of Computer Science 2".into()),
        (key("CS 2501"), "Fundamentals of Computer Science 1".into()),
    ]);
    assert!(by_title(
        "CS 2510",
        &["CS 2501"],
        Some(&sequence),
        &[],
        None,
        &CourseEquivalences::none(),
        3
    )
    .is_empty());
}

#[test]
fn leavesOutTheRetiredCourseAndWhatIsKnown() {
    assert!(by_title(
        "IS 4300",
        &["IS 4300"],
        None,
        &[],
        None,
        &CourseEquivalences::none(),
        3
    )
    .is_empty());
    let known = CourseEquivalences::from_pairs(&[(key("IS 4300"), key("CS 2484"))]);
    assert!(by_title("IS 4300", &["CS 2484"], None, &[], None, &known, 3).is_empty());
    let chain = CourseEquivalences::from_pairs(&[
        (key("IS 4300"), key("CS 9000")),
        (key("CS 9000"), key("CS 2484")),
    ]);
    assert!(by_title("IS 4300", &["CS 2484"], None, &[], None, &chain, 3).is_empty());
}

#[test]
fn titleCandidatesNeedNotBeCurrent() {
    let empty = HashSet::new();
    assert_eq!(
        by_title(
            "IS 4300",
            &["CS 2484"],
            None,
            &[],
            Some(&empty),
            &CourseEquivalences::none(),
            3
        )
        .iter()
        .map(|item| item.candidate.clone())
        .collect::<Vec<_>>(),
        vec![key("CS 2484")]
    );
}

#[test]
fn titleSuggestionsAreRankedByHowAlikeTheTitlesAre() {
    let mut table = title_table();
    table.insert(
        key("CS 5100"),
        "Human Computer Interaction Methods and Practice".into(),
    );
    table.insert(key("CS 5200"), "Human-Computer Interaction Studio".into());
    let found = by_title(
        "IS 4300",
        &["CS 5100", "CS 2484", "CS 5200"],
        Some(&table),
        &[],
        None,
        &CourseEquivalences::none(),
        3,
    );
    assert_eq!(
        found
            .iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 2484"), key("CS 5200"), key("CS 5100")]
    );
    assert_eq!(
        found
            .iter()
            .map(|item| item.title_score)
            .collect::<Vec<_>>(),
        vec![Some(1.0), Some(0.75), Some(0.6)]
    );
}

#[test]
fn titlesRankBelowListedTogetherAndBothKindsAreReturned() {
    let prerequisites = vec![
        (key("CS 4520"), either(&["IS 4300", "CS 2484", "CS 4550"])),
        (key("CS 4540"), either(&["IS 4300", "CS 2484", "CS 4550"])),
        (key("CS 4600"), requisite("IS 4300")),
    ];
    let mut table = title_table();
    table.insert(key("CS 5100"), "Human Computer Interaction Studio".into());
    let found = by_title(
        "IS 4300",
        &["CS 5100", "CS 2484"],
        Some(&table),
        &prerequisites,
        None,
        &CourseEquivalences::none(),
        3,
    );
    assert_eq!(
        found
            .iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 2484"), key("CS 4550"), key("CS 5100")]
    );
    let both = found.first().expect("both");
    assert_eq!((both.together, both.total), (2, 3));
    assert!(both.is_listed_together());
    assert_eq!(
        both.similar_title.as_deref(),
        Some("Principles of Human-Computer Interaction")
    );
    assert_eq!(both.title_score, Some(1.0));
    assert_eq!(
        both.evidence(),
        "Listed together in 2 of the 3 prerequisites that name IS 4300 · similar title"
    );
    assert!(found[1].similar_title.is_none());
    assert_eq!(
        found[1].evidence(),
        "Listed together in 2 of the 3 prerequisites that name IS 4300"
    );
    assert_eq!((found[2].together, found[2].total), (0, 3));
    assert_eq!(
        found[2].evidence(),
        "Similar title: Human Computer Interaction Studio"
    );
}

#[test]
fn aTitleSuggestionKeepsAPlaceWhenThePrerequisitesWouldFillTheList() {
    let mut listing: Vec<(CourseKey, Requisite)> = (1..=4)
        .map(|number| {
            (
                key(&format!("CS 40{number}0")),
                either(&["IS 4300", "CS 2110", "CS 2210", "CS 2310"]),
            )
        })
        .collect();
    listing.push((key("CS 4500"), either(&["IS 4300", "CS 2484"])));
    let found = by_title(
        "IS 4300",
        &["CS 2484"],
        None,
        &listing,
        None,
        &CourseEquivalences::none(),
        3,
    );
    assert_eq!(found.len(), 3);
    assert_eq!(
        found.last().map(|item| item.candidate.clone()),
        Some(key("CS 2484"))
    );
    assert_eq!(found.last().map(|item| item.together), Some(0));
    assert!(found[..found.len() - 1]
        .iter()
        .all(|item| item.is_listed_together()));
    let one = by_title(
        "IS 4300",
        &["CS 2484"],
        None,
        &listing,
        None,
        &CourseEquivalences::none(),
        1,
    );
    assert_eq!(one.len(), 1);
    assert_eq!(
        one.first().map(|item| item.is_listed_together()),
        Some(true)
    );
    let two = by_title(
        "IS 4300",
        &["CS 2484"],
        None,
        &listing,
        None,
        &CourseEquivalences::none(),
        2,
    );
    assert_eq!(
        two.iter()
            .map(|item| item.is_listed_together())
            .collect::<Vec<_>>(),
        vec![true, false]
    );
    assert!(by_title(
        "IS 4300",
        &["CS 2484"],
        None,
        &listing,
        None,
        &CourseEquivalences::none(),
        0
    )
    .is_empty());
}

#[test]
fn suggestsReplacementsForRetiredCoursesOnly() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(course_info("CS 2510", "202530", None, vec![]), "202530");
    banner.list(course_info("CS 3386", "202710", None, vec![]), "202710");
    let mut store = make_store(&banner, dir.path(), &clock);
    let loaded = newest_index();
    assert!(store
        .equivalence_suggestions(&key("CS 2510"), None, 3, &HashMap::new(), &[])
        .is_empty());
    store.load(
        &[
            key("CS 2510"),
            key("CS 3386"),
            key("CS 3000"),
            key("CS 2963"),
        ],
        &[loaded],
    );
    assert_eq!(
        store.info(&key("CS 2510")).and_then(|info| info.is_retired),
        Some(true)
    );
    let found = suggest(&store, &key("CS 2510"), None, &HashMap::new(), &[]);
    assert_eq!(
        found
            .iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 2100")]
    );
    assert_eq!(
        found.first().map(|item| (item.together, item.total)),
        Some((3, 3))
    );
    assert!(suggest(&store, &key("CS 3000"), None, &HashMap::new(), &[]).is_empty());
    assert!(suggest(&store, &key("CS 3386"), None, &HashMap::new(), &[]).is_empty());
    assert_eq!(store.state(&key("CS 2963")), StoreState::NotFound);
    assert!(suggest(&store, &key("CS 2963"), None, &HashMap::new(), &[]).is_empty());
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    declare_equivalence(&mut plan, key("CS 2510"), key("CS 2100"), "");
    assert!(suggest(&store, &key("CS 2510"), Some(&plan), &HashMap::new(), &[]).is_empty());
}

#[test]
fn suggestsTheStudentsCourseWithASimilarTitle() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(
        CourseInfo::entry(
            key("IS 4300"),
            "Human Computer Interaction",
            Some(4.0),
            Some(4.0),
            None,
            vec![],
            "202530",
            0.0,
        ),
        "202530",
    );
    let mut store = make_store(&banner, dir.path(), &clock);
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![
        DegreePlanCourse::catalog(key("CS 2484"), "Principles of Human-Computer I", 4.0),
        DegreePlanCourse::catalog(key("CS 3650"), "Computer Systems", 4.0),
    ];
    plan.terms = vec![semester(PlanSeason::Spring, 2027, &["CS 4550"], 4.0)];
    plan.terms[0].courses[0].title = "Web Development".into();
    store.load(
        &[key("IS 4300"), key("CS 2484"), key("CS 3650")],
        &[newest_index()],
    );
    assert_eq!(
        store.info(&key("IS 4300")).and_then(|info| info.is_retired),
        Some(true)
    );
    let found = suggest(&store, &key("IS 4300"), Some(&plan), &HashMap::new(), &[]);
    assert_eq!(
        found
            .iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 2484")]
    );
    let suggestion = found.first().expect("suggestion");
    assert_eq!(
        suggestion.similar_title.as_deref(),
        Some("Principles of Human-Computer I")
    );
    assert!(!suggestion.is_listed_together());
    assert_eq!(
        suggestion.evidence(),
        "Similar title: Principles of Human-Computer I"
    );
    assert!(suggest(&store, &key("IS 4300"), None, &HashMap::new(), &[]).is_empty());
    let catalog = HashMap::from([
        (
            key("CS 2484"),
            "Principles of Human-Computer Interaction".into(),
        ),
        (key("CS 4550"), "Web Development".into()),
    ]);
    let listed = suggest(
        &store,
        &key("IS 4300"),
        None,
        &catalog,
        &[key("CS 2484"), key("CS 4550")],
    );
    assert_eq!(
        listed
            .iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 2484")]
    );
    assert_eq!(
        listed
            .first()
            .and_then(|item| item.similar_title.clone())
            .as_deref(),
        Some("Principles of Human-Computer Interaction")
    );
    let titled = suggest(
        &store,
        &key("IS 4300"),
        Some(&plan),
        &HashMap::from([(key("IS 4300"), "Computer Systems".into())]),
        &[],
    );
    assert_eq!(
        titled
            .iter()
            .map(|item| item.candidate.clone())
            .collect::<Vec<_>>(),
        vec![key("CS 3650")]
    );
    declare_equivalence(&mut plan, key("CS 2484"), key("IS 4300"), "");
    assert!(suggest(&store, &key("IS 4300"), Some(&plan), &HashMap::new(), &[]).is_empty());
}

#[test]
fn titlesComeFromTheBestSource() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(
        CourseInfo::entry(
            key("IS 4300"),
            "Human Computer Interaction",
            None,
            None,
            None,
            vec![],
            "202530",
            0.0,
        ),
        "202530",
    );
    banner.list(
        CourseInfo::entry(
            key("CS 2484"),
            "Principles of Human-Computer Interaction (Banner)",
            None,
            None,
            None,
            vec![],
            "202710",
            0.0,
        ),
        "202710",
    );
    let mut store = make_store(&banner, dir.path(), &clock);
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![DegreePlanCourse::catalog(
        key("CS 2484"),
        "Principles of Human-Computer I",
        4.0,
    )];
    store.load(&[key("IS 4300"), key("CS 2484")], &[newest_index()]);
    assert_eq!(
        suggest(&store, &key("IS 4300"), Some(&plan), &HashMap::new(), &[])
            .first()
            .and_then(|item| item.similar_title.clone())
            .as_deref(),
        Some("Principles of Human-Computer Interaction (Banner)")
    );
    let catalog = HashMap::from([(
        key("CS 2484"),
        "Human-Computer Interaction Principles (catalog)".into(),
    )]);
    assert_eq!(
        suggest(&store, &key("IS 4300"), Some(&plan), &catalog, &[])
            .first()
            .and_then(|item| item.similar_title.clone())
            .as_deref(),
        Some("Human-Computer Interaction Principles (catalog)")
    );
}

#[test]
fn currentCoursesGetNoTitleSuggestions() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(
        CourseInfo::entry(
            key("IS 4300"),
            "Human Computer Interaction",
            None,
            None,
            None,
            vec![],
            "202710",
            0.0,
        ),
        "202710",
    );
    let mut store = make_store(&banner, dir.path(), &clock);
    let mut plan = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    plan.completed = vec![DegreePlanCourse::catalog(
        key("CS 2484"),
        "Principles of Human-Computer Interaction",
        4.0,
    )];
    store.load(&[key("IS 4300"), key("CS 2484")], &[newest_index()]);
    assert_eq!(
        store.info(&key("IS 4300")).and_then(|info| info.is_retired),
        Some(false)
    );
    assert!(suggest(&store, &key("IS 4300"), Some(&plan), &HashMap::new(), &[]).is_empty());
}

#[test]
fn loadedCatalogsDoNotDropBannerEquivalents() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = FakeBanner::new(&clock, banner_terms());
    banner.list(
        course_info("CS 2100", "202710", None, vec![key("CS 2510")]),
        "202710",
    );
    let mut store = make_store(&banner, dir.path(), &clock);
    store.load(&[key("CS 2100")], &[]);
    assert!(store
        .equivalences(None)
        .are_equivalent(&key("CS 2100"), &key("CS 2510")));
    store.load(
        &[key("CS 2100")],
        &[index(
            vec![course(
                "CS 2100",
                "",
                Some(requisite("CS 1800")),
                true,
                false,
            )],
            vec![],
            "202710",
        )],
    );
    assert_eq!(
        store
            .info(&key("CS 2100"))
            .and_then(|info| info.prerequisites.clone()),
        Some(requisite("CS 1800"))
    );
    assert_eq!(
        store.info(&key("CS 2100")).and_then(|info| info.is_retired),
        Some(false)
    );
    assert!(store
        .equivalences(None)
        .are_equivalent(&key("CS 2100"), &key("CS 2510")));
    store.load(
        &[key("CS 2100")],
        &[index(
            vec![course(
                "CS 2100",
                "",
                Some(requisite("CS 1000")),
                true,
                false,
            )],
            vec![],
            "202610",
        )],
    );
    assert_eq!(
        store
            .info(&key("CS 2100"))
            .and_then(|info| info.prerequisites.clone()),
        Some(requisite("CS 1800"))
    );
}

#[test]
fn seasonsAndLikelyOfferings() {
    let history = history(
        &[
            ("CS 3386", &["202630", "202530"]),
            ("CS 3800", &["202710", "202610", "202510"]),
            ("CS 2500", &["202650", "202550"]),
        ],
        None,
        None,
    );
    assert_eq!(
        history.seasons(&key("CS 3386")),
        BTreeSet::from([PlanSeason::Spring])
    );
    assert_eq!(
        history.seasons(&key("CS 2500")),
        BTreeSet::from([PlanSeason::SummerFull])
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 3386"), PlanSeason::Spring),
        Some(true)
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 3386"), PlanSeason::Fall),
        Some(false)
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 2500"), PlanSeason::SummerA),
        Some(true)
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 2500"), PlanSeason::SummerB),
        Some(true)
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 3386"), PlanSeason::SummerA),
        Some(false)
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 3386"), PlanSeason::Winter),
        None
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 9999"), PlanSeason::Spring),
        Some(false)
    );
}

#[test]
fn unknownWhenNothingWasChecked() {
    assert_eq!(
        OfferingHistory::empty().is_likely_offered(&key("CS 3386"), PlanSeason::Spring),
        None
    );
    assert_eq!(
        OfferingHistory::empty().summary(&key("CS 3386")),
        "Offering history unknown"
    );
    let partial = history(&[("CS 3386", &["202630"])], None, Some(&["CS 3386"]));
    assert_eq!(
        partial.is_likely_offered(&key("CS 3386"), PlanSeason::Spring),
        Some(true)
    );
    assert_eq!(
        partial.is_likely_offered(&key("CS 3800"), PlanSeason::Spring),
        None
    );
    assert_eq!(partial.summary(&key("CS 3800")), "Offering history unknown");
    assert_eq!(
        partial.summary_in(&key("CS 3800"), PlanSeason::Spring),
        "Offering history unknown"
    );
}

#[test]
fn summaries() {
    let record = history(
        &[
            ("CS 3386", &["202630", "202530"]),
            (
                "CS 3800",
                &["202710", "202610", "202510", "202630", "202530"],
            ),
            ("CS 4100", &["202530"]),
            ("CS 2500", &["202650", "202550"]),
            ("CS 2510", &["202650", "202630"]),
        ],
        None,
        None,
    );
    assert_eq!(record.summary(&key("CS 3386")), "Offered every spring");
    assert_eq!(
        record.summary(&key("CS 3800")),
        "Offered every fall and spring"
    );
    assert_eq!(record.summary(&key("CS 4100")), "Last offered Spring 2025");
    assert_eq!(record.summary(&key("CS 2500")), "Offered every summer");
    assert_eq!(record.summary(&key("CS 2510")), "Last offered Summer 2026");
    assert_eq!(
        record.summary(&key("CS 9999")),
        "Not offered in the last 2 years"
    );
    let mixed = history(
        &[("CS 3800", &["202710", "202610", "202510", "202630"])],
        None,
        None,
    );
    assert_eq!(
        mixed.summary(&key("CS 3800")),
        "Offered every fall, and sometimes in the spring"
    );
    assert_eq!(
        history(
            &[(
                "CS 3800",
                &["202710", "202610", "202510", "202630", "202550"]
            )],
            None,
            None
        )
        .summary(&key("CS 3800")),
        "Offered every fall, and sometimes in the spring and summer"
    );
    assert_eq!(
        record.summary_in(&key("CS 3386"), PlanSeason::Spring),
        "Offered every spring"
    );
    assert_eq!(
        record.summary_in(&key("CS 4100"), PlanSeason::Spring),
        "Offered in Spring 2025"
    );
    assert_eq!(
        record.summary_in(&key("CS 3386"), PlanSeason::Fall),
        "Not offered in a fall term in the last 2 years"
    );
    assert_eq!(
        record.summary_in(&key("CS 2500"), PlanSeason::SummerA),
        "Offered every summer"
    );
}

#[test]
fn historiesMergeAndRoundTrip() {
    let first = history(
        &[("CS 3386", &["202630"])],
        Some(&["202710", "202630"]),
        Some(&["CS 3386"]),
    );
    let second = history(
        &[("CS 3800", &["202710"])],
        Some(&["202710", "202610"]),
        Some(&["CS 3800"]),
    );
    let merged = first.merging(&second);
    assert_eq!(merged.checked_terms, vec!["202710", "202630", "202610"]);
    assert_eq!(
        merged.offered.get(&key("CS 3386")),
        Some(&BTreeSet::from(["202630".to_string()]))
    );
    assert_eq!(
        merged.offered.get(&key("CS 3800")),
        Some(&BTreeSet::from(["202710".to_string()]))
    );
    assert_eq!(
        merged.checked_keys,
        Some(BTreeSet::from([key("CS 3386"), key("CS 3800")]))
    );
    assert!(first
        .merging(&history(&[], None, None))
        .checked_keys
        .is_none());
    for original in [
        merged,
        history(&[("CS 3386", &["202630", "202530"])], None, None),
        OfferingHistory::empty(),
    ] {
        let decoded: OfferingHistory =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!(decoded, original);
    }
}

#[test]
fn coversTwoYearsOfTermsSummersIncluded() {
    let neu: Vec<Term> = banner_terms()
        .into_iter()
        .filter(|term| term.college() == College::Neu)
        .collect();
    let window: Vec<String> = OfferingRepository::<FakeBanner>::window(&neu, 24)
        .into_iter()
        .map(|term| term.code)
        .collect();
    assert_eq!(
        window,
        vec![
            "202710", "202650", "202630", "202610", "202560", "202550", "202540", "202530",
            "202510"
        ]
    );
    assert!(OfferingRepository::<FakeBanner>::window(&[], 24).is_empty());
}

#[test]
fn asksOncePerTermAboutTheCoursesAndCachesThePast() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = sample_banner(&clock);
    let mut repository = offer_repo(&banner, dir.path(), &clock);
    let keys = vec![
        key("CS 3386"),
        key("CS 3800"),
        key("MATH 2331"),
        key("CS 2500"),
    ];
    let history = repository.history(&keys, &[]);
    assert_eq!(
        history.checked_terms,
        vec![
            "202710", "202650", "202630", "202610", "202560", "202550", "202540", "202530",
            "202510"
        ]
    );
    assert_eq!(banner.offered_calls().len(), 9);
    let expected: HashSet<CourseKey> = keys.iter().cloned().collect();
    assert!(banner.offered_calls().iter().all(|(_, courses)| courses
        .iter()
        .cloned()
        .collect::<HashSet<_>>()
        == expected));
    assert_eq!(
        history.is_likely_offered(&key("CS 3386"), PlanSeason::Spring),
        Some(true)
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 3386"), PlanSeason::Fall),
        Some(false)
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 3800"), PlanSeason::Fall),
        Some(true)
    );
    assert_eq!(
        history.is_likely_offered(&key("CS 2500"), PlanSeason::SummerA),
        Some(true)
    );
    assert_eq!(
        history.summary(&key("MATH 2331")),
        "Offered every fall and spring"
    );
    assert_eq!(history.checked_keys, Some(keys.iter().cloned().collect()));

    banner.reset_calls();
    let _ = repository.history(&keys, &[]);
    assert!(banner.offered_calls().is_empty());
    clock.advance_days(30.0);
    let mut relaunched = offer_repo(&banner, dir.path(), &clock);
    let _ = relaunched.history(&keys, &[]);
    assert_eq!(
        banner
            .offered_calls()
            .iter()
            .map(|(term, _)| term.clone())
            .collect::<Vec<_>>(),
        vec!["202710".to_string()]
    );
    banner.reset_calls();
    let _ = relaunched.history(&[key("PHYS 1151"), key("CS 3386")], &[]);
    assert_eq!(banner.offered_calls().len(), 9);
    assert!(banner
        .offered_calls()
        .iter()
        .all(|(_, courses)| courses == &vec![key("PHYS 1151")]));
}

#[test]
fn bigQuestionsAreBatchedAndKeepWhatWasFound() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = sample_banner(&clock);
    let numbers: Vec<String> = (1..=12).map(|n| format!("CS 30{}", 10 + n)).collect();
    let refs: Vec<&str> = numbers.iter().map(String::as_str).collect();
    banner.offer("202630", &refs);
    let mut repository = offer_repo(&banner, dir.path(), &clock);
    let keys: Vec<CourseKey> = numbers.iter().map(|code| key(code)).collect();
    banner.fail_questions("202630", Some(key("CS 3022")));
    let partial = repository.history(&keys, &[]);
    assert!(
        !partial.checked_terms.iter().any(|term| term == "202630")
            && partial.checked_terms.iter().any(|term| term == "202530")
    );
    assert_eq!(
        banner
            .offered_calls()
            .iter()
            .filter(|(term, _)| term == "202630")
            .map(|(_, courses)| courses.len())
            .collect::<Vec<_>>(),
        vec![10, 2]
    );
    assert_eq!(
        banner
            .offered_calls()
            .iter()
            .filter(|(term, _)| term == "202710")
            .map(|(_, courses)| courses.len())
            .collect::<Vec<_>>(),
        vec![10, 2]
    );
    banner.fail_questions("202630", None);
    banner.reset_calls();
    let full = repository.history(&keys, &[]);
    assert_eq!(
        banner
            .offered_calls()
            .iter()
            .map(|(term, _)| term.clone())
            .collect::<Vec<_>>(),
        vec!["202630".to_string()]
    );
    assert_eq!(
        banner
            .offered_calls()
            .first()
            .map(|(_, courses)| courses.clone()),
        Some(vec![key("CS 3021"), key("CS 3022")])
    );
    assert!(full.checked_terms.iter().any(|term| term == "202630"));
    assert!(keys
        .iter()
        .all(|course| full.is_likely_offered(course, PlanSeason::Spring) == Some(true)));
}

#[test]
fn theCurrentTermIsRefreshedWeekly() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = sample_banner(&clock);
    let mut repository = offer_repo(&banner, dir.path(), &clock);
    let _ = repository.history(&[key("CS 3800")], &[]);
    banner.reset_calls();
    clock.advance_days(6.0);
    repository.now = clock.now();
    let _ = repository.history(&[key("CS 3800")], &[]);
    assert!(banner.offered_calls().is_empty());
    clock.advance_days(2.0);
    repository.now = clock.now();
    let _ = repository.history(&[key("CS 3800")], &[]);
    assert_eq!(
        banner
            .offered_calls()
            .iter()
            .map(|(term, _)| term.clone())
            .collect::<Vec<_>>(),
        vec!["202710".to_string()]
    );
}

#[test]
fn aTermReadBeforeItStartedIsReadAgain() {
    let dir = TempDir::new();
    let early = Clock::new(1_780_000_000.0);
    let banner = FakeBanner::new(&early, banner_terms());
    banner.offer("202710", &["CS 3800"]);
    let mut first = offer_repo(&banner, dir.path(), &early);
    let _ = first.history(&[key("CS 3800")], &[]);
    assert!(banner
        .offered_calls()
        .iter()
        .any(|(term, _)| term == "202710"));

    early.advance_days(230.0);
    banner.reset_calls();
    banner.set_terms(
        terms(&["202730"])
            .into_iter()
            .chain(banner_terms())
            .collect(),
    );
    let mut second = offer_repo(&banner, dir.path(), &early);
    let _ = second.history(&[key("CS 3800")], &[]);
    let asked: HashSet<String> = banner
        .offered_calls()
        .iter()
        .map(|(term, _)| term.clone())
        .collect();
    assert!(asked.contains("202730") && asked.contains("202710"));
    assert!(!asked.contains("202630") && !asked.contains("202610"));

    early.advance_days(60.0);
    banner.reset_calls();
    banner.set_terms(
        terms(&["202750", "202730"])
            .into_iter()
            .chain(banner_terms())
            .collect(),
    );
    let mut third = offer_repo(&banner, dir.path(), &early);
    let _ = third.history(&[key("CS 3800")], &[]);
    assert!(!banner
        .offered_calls()
        .iter()
        .any(|(term, _)| term == "202710"));
}

#[test]
fn aTermThatCouldNotBeReachedIsNotReportedAsNotOffered() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = sample_banner(&clock);
    banner.fail("202630", true);
    let mut repository = offer_repo(&banner, dir.path(), &clock);
    let history = repository.history(&[key("CS 3386")], &[]);
    assert!(!history.checked_terms.iter().any(|term| term == "202630"));
    assert!(history.checked_terms.iter().any(|term| term == "202530"));
    assert_eq!(
        history.is_likely_offered(&key("CS 3386"), PlanSeason::Spring),
        Some(true)
    );

    let offline_dir = TempDir::new();
    let offline = FakeBanner::new(&clock, banner_terms());
    for term in banner_terms() {
        offline.fail(&term.code, true);
    }
    let mut none = offer_repo(&offline, offline_dir.path(), &clock);
    let none = none.history(&[key("CS 3386")], &[]);
    assert!(none.checked_terms.is_empty());
    assert_eq!(
        none.is_likely_offered(&key("CS 3386"), PlanSeason::Spring),
        None
    );
}

#[test]
fn loadedCatalogsAnswerWithoutARequest() {
    let dir = TempDir::new();
    let clock = Clock::standard();
    let banner = sample_banner(&clock);
    let mut repository = offer_repo(&banner, dir.path(), &clock);
    let fall_2026 = index(
        vec![
            course("CS 3386", "", None, true, false),
            course("CS 3800", "", None, true, false),
        ],
        vec![section("1", "CS 3800", "", &[])],
        "202710",
    );
    let history = repository.history(&[key("CS 3386"), key("CS 3800")], &[fall_2026]);
    assert!(!banner
        .offered_calls()
        .iter()
        .any(|(term, _)| term == "202710"));
    assert!(history.checked_terms.iter().any(|term| term == "202710"));
    assert!(history
        .offered
        .get(&key("CS 3800"))
        .is_some_and(|terms| terms.contains("202710")));
    assert!(!history
        .offered
        .get(&key("CS 3386"))
        .is_some_and(|terms| terms.contains("202710")));
}

const ALL_TERMS: &[&str] = &[
    "202710", "202650", "202630", "202610", "202550", "202530", "202510",
];
const FALL: &[&str] = &["202710", "202610", "202510"];
const SPRING: &[&str] = &["202630", "202530"];

fn taught(codes: &[&str], terms: &[&str]) -> Vec<(String, Vec<String>)> {
    codes
        .iter()
        .map(|code| {
            (
                (*code).to_string(),
                terms.iter().map(|term| (*term).to_string()).collect(),
            )
        })
        .collect()
}

fn reorder_history(offered: &[(&str, &[&str])]) -> OfferingHistory {
    history(offered, Some(ALL_TERMS), None)
}

#[test]
fn aCourseOnlyMovesToASeasonItIsTaughtIn() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386", "CS 3800"], 4.0),
            semester(PlanSeason::Spring, 2027, &["CS 4100"], 4.0),
            semester(PlanSeason::Fall, 2027, &[], 4.0),
            semester(PlanSeason::Spring, 2028, &[], 4.0),
        ],
        &[],
    );
    let history = reorder_history(&[
        ("CS 3386", SPRING),
        ("CS 3800", FALL),
        (
            "CS 4100",
            &["202710", "202610", "202510", "202630", "202530"],
        ),
    ]);
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(
        found.first().map(|item| item.title.as_str()),
        Some("Move CS 3386 to Spring 2027")
    );
    assert_eq!(
        found.first().map(|item| item.moves.clone()),
        Some(vec![Move {
            course: key("CS 3386"),
            from: plan.terms[0].id,
            to: plan.terms[1].id
        }])
    );
    assert!(!titles_of(&found)
        .iter()
        .any(|title| title.contains("Fall 2027")));
    let singles: Vec<Suggestion> = found
        .iter()
        .filter(|item| item.moves.len() == 1)
        .cloned()
        .collect();
    assert_eq!(
        titles_of(&singles),
        vec![
            "Move CS 3386 to Spring 2027".to_string(),
            "Move CS 3386 to Spring 2028".to_string()
        ]
    );
    assert_eq!(
        found
            .first()
            .and_then(|item| item.reasons.first())
            .map(String::as_str),
        Some("Offered every spring")
    );
    assert_eq!(
        found
            .first()
            .and_then(|item| item.reasons.last())
            .map(String::as_str),
        Some("Spring 2027 would have 8 credits")
    );
    assert!(found.first().unwrap().adds_semester.is_none());
}

#[test]
fn aCourseNeverTaughtAnywhereHasNoSuggestions() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &[], 4.0),
        ],
        &[],
    );
    assert!(reordering(
        &plan,
        0,
        &["CS 3386"],
        &reorder_history(&[]),
        &none_prereq,
        &CourseEquivalences::none(),
        20.0
    )
    .is_empty());
}

#[test]
fn unknownHistoryIsAllowedButRankedLower() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 9001", "CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &[], 4.0),
        ],
        &[],
    );
    let history = history(&[("CS 3386", SPRING)], Some(ALL_TERMS), Some(&["CS 3386"]));
    let found = reordering(
        &plan,
        0,
        &["CS 9001", "CS 3386"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(
        titles_of(&found),
        vec![
            "Move CS 3386 to Spring 2027".to_string(),
            "Move CS 9001 to Spring 2027".to_string()
        ]
    );
    assert_eq!(found.len(), 2);
    assert_eq!(
        found[1].reasons.first().map(String::as_str),
        Some("Offering history unknown")
    );
    assert_eq!(
        found[0].reasons.first().map(String::as_str),
        Some("Offered every spring")
    );
}

#[test]
fn aCourseNeedsItsPrerequisitesBeforeTheSemester() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &["CS 2386"], 4.0),
            semester(PlanSeason::Fall, 2027, &[], 4.0),
            semester(PlanSeason::Spring, 2028, &[], 4.0),
        ],
        &[],
    );
    let history = reorder_history(&[
        ("CS 3386", SPRING),
        (
            "CS 2386",
            &["202710", "202610", "202510", "202630", "202530"],
        ),
    ]);
    let needs = known_prereqs(&[("CS 3386", requisite("CS 2386"))]);
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &needs,
        &CourseEquivalences::none(),
        20.0,
    );
    let singles: Vec<Suggestion> = found
        .iter()
        .filter(|item| item.moves.len() == 1)
        .cloned()
        .collect();
    assert_eq!(
        titles_of(&singles),
        vec!["Move CS 3386 to Spring 2028".to_string()]
    );
    assert!(singles
        .first()
        .unwrap()
        .reasons
        .iter()
        .any(|reason| reason == "Prerequisites met by then"));
    assert!(titles_of(&found)
        .iter()
        .any(|title| title == "Move CS 3386 to Spring 2027 and CS 2386 to Fall 2026"));
}

#[test]
fn nothingThatNeedsACourseMayBePlannedAtOrBeforeItsNewSemester() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &[], 4.0),
            semester(PlanSeason::Fall, 2027, &[], 4.0),
            semester(PlanSeason::Spring, 2028, &["CS 4500"], 4.0),
        ],
        &[],
    );
    let history = reorder_history(&[("CS 3386", SPRING), ("CS 4500", SPRING)]);
    let needs = known_prereqs(&[("CS 4500", requisite("CS 3386"))]);
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &needs,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(
        titles_of(&found),
        vec!["Move CS 3386 to Spring 2027".to_string()]
    );
    let tight = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &["CS 4500"], 4.0),
            semester(PlanSeason::Fall, 2027, &[], 4.0),
            semester(PlanSeason::Spring, 2028, &[], 4.0),
        ],
        &[],
    );
    assert!(reordering(
        &tight,
        0,
        &["CS 3386"],
        &history,
        &needs,
        &CourseEquivalences::none(),
        20.0
    )
    .is_empty());
}

#[test]
fn equivalentCoursesCountForPrerequisites() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &[], 4.0),
        ],
        &["CS 2510"],
    );
    let history = reorder_history(&[("CS 3386", SPRING)]);
    let needs = known_prereqs(&[("CS 3386", requisite("CS 2100"))]);
    assert!(reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &needs,
        &CourseEquivalences::none(),
        20.0
    )
    .is_empty());
    let renumbered = CourseEquivalences::from_pairs(&[(key("CS 2100"), key("CS 2510"))]);
    let found = reordering(&plan, 0, &["CS 3386"], &history, &needs, &renumbered, 20.0);
    assert_eq!(
        titles_of(&found),
        vec!["Move CS 3386 to Spring 2027".to_string()]
    );
    assert!(found
        .first()
        .unwrap()
        .reasons
        .iter()
        .any(|reason| reason == "Prerequisites still met"));
    let mut declared = plan;
    declare_equivalence(&mut declared, key("CS 2510"), key("CS 2100"), "");
    assert_eq!(
        reordering(
            &declared,
            0,
            &["CS 3386"],
            &history,
            &needs,
            &CourseEquivalences::none(),
            20.0
        )
        .len(),
        1
    );
}

#[test]
fn unknownPrerequisitesAreAllowedButRankedLower() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386", "CS 3387"], 4.0),
            semester(PlanSeason::Spring, 2027, &[], 4.0),
        ],
        &[],
    );
    let history = reorder_history(&[("CS 3386", SPRING), ("CS 3387", SPRING)]);
    let prerequisites = |course: &CourseKey| {
        if course == &key("CS 3386") {
            None
        } else {
            Some(None)
        }
    };
    let found = reordering(
        &plan,
        0,
        &["CS 3386", "CS 3387"],
        &history,
        &prerequisites,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(
        titles_of(&found),
        vec![
            "Move CS 3387 to Spring 2027".to_string(),
            "Move CS 3386 to Spring 2027".to_string()
        ]
    );
    assert_eq!(found.len(), 2);
    assert!(found[1]
        .reasons
        .iter()
        .any(|reason| reason == "Prerequisites couldn’t be checked"));
}

#[test]
fn aSemesterStaysUnderTheCreditLimit() {
    let full = ["CS 4100", "CS 4200", "CS 4300", "CS 4400", "CS 4500"];
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &full, 4.0),
            semester(PlanSeason::Fall, 2027, &[], 4.0),
            semester(PlanSeason::Spring, 2028, &[], 4.0),
        ],
        &[],
    );
    let mut pairs = vec![(
        "CS 3386".to_string(),
        SPRING
            .iter()
            .map(|term| (*term).to_string())
            .collect::<Vec<_>>(),
    )];
    pairs.extend(taught(
        &full,
        &["202710", "202610", "202510", "202630", "202530"],
    ));
    let mut offered = BTreeMap::new();
    for (code, terms) in pairs {
        offered.insert(key(&code), terms.into_iter().collect());
    }
    let history = OfferingHistory {
        checked_terms: ALL_TERMS.iter().map(|term| (*term).to_string()).collect(),
        offered,
        checked_keys: None,
    };
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    let singles: Vec<Suggestion> = found
        .iter()
        .filter(|item| item.moves.len() == 1)
        .cloned()
        .collect();
    assert_eq!(
        titles_of(&singles),
        vec!["Move CS 3386 to Spring 2028".to_string()]
    );
    let relaxed = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        24.0,
    );
    assert_eq!(
        titles_of(
            &relaxed
                .iter()
                .filter(|item| item.moves.len() == 1)
                .cloned()
                .collect::<Vec<_>>()
        )
        .first()
        .map(String::as_str),
        Some("Move CS 3386 to Spring 2027")
    );
    assert_eq!(
        relaxed
            .first()
            .and_then(|item| item.reasons.last())
            .map(String::as_str),
        Some("Spring 2027 would have 24 credits")
    );
}

#[test]
fn swapsTradeACourseThatIsTaughtNow() {
    let full = ["CS 3800", "CS 4200", "CS 4300", "CS 4400", "CS 4500"];
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &full, 4.0),
        ],
        &[],
    );
    let mut offered = BTreeMap::new();
    offered.insert(
        key("CS 3386"),
        SPRING.iter().map(|term| (*term).to_string()).collect(),
    );
    offered.insert(
        key("CS 3800"),
        ["202710", "202610", "202510", "202630", "202530"]
            .into_iter()
            .map(str::to_string)
            .collect(),
    );
    for code in &full[1..] {
        offered.insert(
            key(code),
            SPRING.iter().map(|term| (*term).to_string()).collect(),
        );
    }
    let history = OfferingHistory {
        checked_terms: ALL_TERMS.iter().map(|term| (*term).to_string()).collect(),
        offered,
        checked_keys: None,
    };
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(
        titles_of(&found),
        vec!["Move CS 3386 to Spring 2027 and CS 3800 to Fall 2026".to_string()]
    );
    let swap = found.first().expect("swap");
    assert_eq!(
        swap.moves,
        vec![
            Move {
                course: key("CS 3386"),
                from: plan.terms[0].id,
                to: plan.terms[1].id
            },
            Move {
                course: key("CS 3800"),
                from: plan.terms[1].id,
                to: plan.terms[0].id
            },
        ]
    );
    assert!(swap
        .reasons
        .iter()
        .any(|reason| reason == "CS 3386: offered every spring"));
    assert!(swap
        .reasons
        .iter()
        .any(|reason| reason == "CS 3800: offered every fall"));
    assert!(swap
        .reasons
        .iter()
        .any(|reason| reason == "Fall 2026 would have 4 credits"));
    assert!(swap
        .reasons
        .iter()
        .any(|reason| reason == "Spring 2027 would have 20 credits"));
}

#[test]
fn swapsRankAfterSingleMoves() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386", "CS 3800"], 4.0),
            semester(PlanSeason::Spring, 2027, &["CS 4100"], 4.0),
            semester(PlanSeason::Spring, 2028, &[], 4.0),
        ],
        &[],
    );
    let history = reorder_history(&[
        ("CS 3386", SPRING),
        ("CS 3800", FALL),
        (
            "CS 4100",
            &["202710", "202610", "202510", "202630", "202530"],
        ),
    ]);
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(
        titles_of(&found),
        vec![
            "Move CS 3386 to Spring 2027".to_string(),
            "Move CS 3386 to Spring 2028".to_string(),
            "Move CS 3386 to Spring 2027 and CS 4100 to Fall 2026".to_string(),
        ]
    );
    assert_eq!(
        found
            .iter()
            .map(|item| item.moves.len())
            .collect::<Vec<_>>(),
        vec![1, 1, 2]
    );
}

#[test]
fn swapPartnersMustBeTaughtInTheSemesterTheyMoveTo() {
    let full = ["CS 4100", "CS 4200", "CS 4300", "CS 4400", "CS 4500"];
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &full, 4.0),
        ],
        &[],
    );
    let mut offered = BTreeMap::new();
    offered.insert(
        key("CS 3386"),
        SPRING.iter().map(|term| (*term).to_string()).collect(),
    );
    for code in &full {
        offered.insert(
            key(code),
            SPRING.iter().map(|term| (*term).to_string()).collect(),
        );
    }
    let history = OfferingHistory {
        checked_terms: ALL_TERMS.iter().map(|term| (*term).to_string()).collect(),
        offered,
        checked_keys: None,
    };
    assert!(
        reordering(
            &plan,
            0,
            &["CS 3386"],
            &history,
            &none_prereq,
            &CourseEquivalences::none(),
            20.0
        )
        .iter()
        .filter(|item| item.moves.len() == 2)
        .count()
            == 0
    );
}

#[test]
fn swapPartnersKeepTheirOwnPrerequisites() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &["CS 3800", "CS 3700"], 4.0),
        ],
        &[],
    );
    let both = &["202710", "202610", "202510", "202630", "202530"];
    let history = reorder_history(&[("CS 3386", SPRING), ("CS 3800", both), ("CS 3700", both)]);
    let needs = known_prereqs(&[("CS 3800", requisite("CS 3700"))]);
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &needs,
        &CourseEquivalences::none(),
        8.0,
    );
    assert_eq!(
        titles_of(&found),
        vec!["Move CS 3386 to Spring 2027 and CS 3700 to Fall 2026".to_string()]
    );
}

#[test]
fn aSemesterIsAddedWhenNoExistingOneWorks() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Fall, 2027, &["CS 3800"], 4.0),
        ],
        &[],
    );
    let history = reorder_history(&[("CS 3386", SPRING), ("CS 3800", FALL)]);
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(found.len(), 1);
    let suggestion = &found[0];
    let added = suggestion.adds_semester.as_ref().expect("added");
    assert_eq!((added.season, added.year), (PlanSeason::Spring, 2028));
    assert_eq!(
        suggestion.title,
        "Move CS 3386 to a new Spring 2028 semester"
    );
    assert_eq!(
        suggestion.moves,
        vec![Move {
            course: key("CS 3386"),
            from: plan.terms[0].id,
            to: added.id
        }]
    );
    assert_eq!(
        suggestion.reasons.last().map(String::as_str),
        Some("Adds Spring 2028 to your plan, with 4 credits")
    );
    let again = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(
        again.first().map(|item| item.id.as_str()),
        Some(suggestion.id.as_str())
    );
    assert_eq!(
        again
            .first()
            .and_then(|item| item.adds_semester.as_ref())
            .map(|term| term.id),
        Some(added.id)
    );
}

#[test]
fn theAddedSemesterIsInASeasonTheCourseIsTaughtIn() {
    let plan = degree_plan(
        vec![semester(PlanSeason::Spring, 2027, &["CS 3800"], 4.0)],
        &[],
    );
    let found = reordering(
        &plan,
        0,
        &["CS 3800"],
        &reorder_history(&[("CS 3800", FALL)]),
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(
        found
            .first()
            .and_then(|item| item.adds_semester.as_ref())
            .map(|term| (term.season, term.year)),
        Some((PlanSeason::Fall, 2027))
    );
}

#[test]
fn applyingAMoveChangesThePlan() {
    let mut plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386", "CS 3800"], 4.0),
            semester(PlanSeason::Spring, 2027, &["CS 4100"], 4.0),
            semester(PlanSeason::Spring, 2028, &[], 4.0),
        ],
        &[],
    );
    let original = plan.clone();
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &reorder_history(&[
            ("CS 3386", SPRING),
            ("CS 3800", FALL),
            (
                "CS 4100",
                &["202710", "202610", "202510", "202630", "202530"],
            ),
        ]),
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    let move_ = found.first().expect("move");
    let moved = original.terms[0].courses[0].id;
    reorder::apply(move_, &mut plan);
    assert_eq!(
        plan.terms[0]
            .courses
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        vec!["CS 3800"]
    );
    assert_eq!(
        plan.terms[1]
            .courses
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        vec!["CS 4100", "CS 3386"]
    );
    assert_eq!(
        plan.terms[1].courses.last().map(|course| course.id),
        Some(moved)
    );
    assert_eq!(plan.terms.len(), 3);
    let before = plan.clone();
    reorder::apply(move_, &mut plan);
    assert_eq!(plan, before);
}

#[test]
fn applyingASwapMovesBothCourses() {
    let full = ["CS 3800", "CS 4200", "CS 4300", "CS 4400", "CS 4500"];
    let mut plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &full, 4.0),
        ],
        &[],
    );
    let mut offered = BTreeMap::new();
    offered.insert(
        key("CS 3386"),
        SPRING.iter().map(|term| (*term).to_string()).collect(),
    );
    offered.insert(
        key("CS 3800"),
        FALL.iter().map(|term| (*term).to_string()).collect(),
    );
    for code in &full[1..] {
        offered.insert(
            key(code),
            SPRING.iter().map(|term| (*term).to_string()).collect(),
        );
    }
    let history = OfferingHistory {
        checked_terms: ALL_TERMS.iter().map(|term| (*term).to_string()).collect(),
        offered,
        checked_keys: None,
    };
    let swap = reordering(
        &plan,
        0,
        &["CS 3386"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    )
    .into_iter()
    .next()
    .expect("swap");
    reorder::apply(&swap, &mut plan);
    assert_eq!(
        plan.terms[0]
            .courses
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        vec!["CS 3800"]
    );
    assert_eq!(
        plan.terms[1]
            .courses
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        vec!["CS 4200", "CS 4300", "CS 4400", "CS 4500", "CS 3386"]
    );
}

#[test]
fn applyingAddsTheNewSemester() {
    let mut plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386", "CS 3800"], 4.0),
            semester(PlanSeason::Fall, 2027, &[], 4.0),
        ],
        &[],
    );
    let original = plan.clone();
    let suggestion = reordering(
        &plan,
        0,
        &["CS 3386"],
        &reorder_history(&[("CS 3386", SPRING), ("CS 3800", FALL)]),
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    )
    .into_iter()
    .next()
    .expect("suggestion");
    reorder::apply(&suggestion, &mut plan);
    assert_eq!(
        plan.terms
            .iter()
            .map(|term| term.name())
            .collect::<Vec<_>>(),
        vec![
            "Fall 2026".to_string(),
            "Fall 2027".to_string(),
            "Spring 2028".to_string()
        ]
    );
    assert_eq!(
        plan.terms
            .last()
            .unwrap()
            .courses
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        vec!["CS 3386"]
    );
    assert_eq!(
        plan.terms[0]
            .courses
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        vec!["CS 3800"]
    );
    let mut other = original;
    other
        .terms
        .push(semester(PlanSeason::Spring, 2028, &["CS 4100"], 4.0));
    reorder::apply(&suggestion, &mut other);
    assert_eq!(other.terms.len(), 3);
    assert_eq!(
        other
            .terms
            .last()
            .unwrap()
            .courses
            .iter()
            .map(|course| course.label.as_str())
            .collect::<Vec<_>>(),
        vec!["CS 4100", "CS 3386"]
    );
}

#[test]
fn staleOrUnknownRequestsAreIgnored() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &[], 4.0),
        ],
        &[],
    );
    let history = reorder_history(&[("CS 3386", SPRING)]);
    assert!(reorder::suggestions(
        &plan,
        Uuid::new_v4(),
        &[key("CS 3386")],
        &history,
        none_prereq,
        &CourseEquivalences::none(),
        20.0
    )
    .is_empty());
    assert!(reordering(
        &plan,
        0,
        &["CS 1234"],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0
    )
    .is_empty());
    assert!(reordering(
        &plan,
        0,
        &[],
        &history,
        &none_prereq,
        &CourseEquivalences::none(),
        20.0
    )
    .is_empty());
}

#[test]
fn atMostAFewSuggestionsAreReturned() {
    let plan = degree_plan(
        vec![
            semester(PlanSeason::Fall, 2026, &["CS 3386"], 4.0),
            semester(PlanSeason::Spring, 2027, &[], 4.0),
            semester(PlanSeason::Spring, 2028, &[], 4.0),
            semester(PlanSeason::Spring, 2029, &[], 4.0),
            semester(PlanSeason::Spring, 2030, &[], 4.0),
            semester(PlanSeason::Spring, 2031, &[], 4.0),
            semester(PlanSeason::Spring, 2032, &[], 4.0),
        ],
        &[],
    );
    let found = reordering(
        &plan,
        0,
        &["CS 3386"],
        &reorder_history(&[("CS 3386", SPRING)]),
        &none_prereq,
        &CourseEquivalences::none(),
        20.0,
    );
    assert_eq!(found.len(), MAX_SUGGESTIONS);
    assert_eq!(
        titles_of(&found).first().map(String::as_str),
        Some("Move CS 3386 to Spring 2027")
    );
}
