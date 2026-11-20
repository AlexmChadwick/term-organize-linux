// SPDX-License-Identifier: AGPL-3.0-only
//! `BannerClient::fetch_course_page` against a local fixture server: required and
//! independent sections, the not-listed answer, pacing and cancellation.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use termorganize_core::banner::{BannerClient, LookupControl, SubjectLookup, LOOKUP_CANCELLED};
use termorganize_core::course_page::Section;
use termorganize_core::model::*;

struct Fixture {
    client: BannerClient,
    log: Arc<Mutex<Vec<(String, Instant, String)>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Serves `routes` by the last path segment. Anything else is a 404 (not retried).
fn serve(routes: &[(&str, &str, &str)]) -> Fixture {
    let routes: HashMap<String, (String, String)> = routes
        .iter()
        .map(|(name, status, body)| (name.to_string(), (status.to_string(), body.to_string())))
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let client = BannerClient {
        base: format!("http://{}", listener.local_addr().unwrap()),
    };
    let log = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    {
        let log = log.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let target = request.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut size = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        size = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; size];
                reader.read_exact(&mut body).unwrap();
                let path = target.split('?').next().unwrap_or("");
                let name = path.rsplit('/').next().unwrap_or("").to_string();
                log.lock().unwrap().push((
                    name.clone(),
                    Instant::now(),
                    format!("{target} {}", String::from_utf8_lossy(&body)),
                ));
                let (status, body) = routes
                    .get(&name)
                    .cloned()
                    .unwrap_or(("404 Not Found".into(), String::new()));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
            }
        });
    }
    Fixture { client, log, stop }
}

fn key() -> CourseKey {
    CourseKey::parse("CS 3500").unwrap()
}

const COMBO: &str = r#"[{"code":"CS3500","description":"Software Development"}]"#;
const DETAILS: &str = "<section>Title: Software Development<br>Credit Hours: 4</section>";
const NO_PREREQ: &str = "<p>No prerequisite information available.</p>";
const DESCRIPTION: &str = "<p>Teaches <b>design</b>.</p>";
const EXCLUSIONS: &str = "<p>No mutual exclusion information available.</p>";
const ATTRIBUTES: &str = "<table><tr><td>NDDI</td><td>Differences and Diversity</td></tr></table>";
const RESTRICTIONS: &str =
    "<p>Must be enrolled in one of the following Colleges:</p><p>Khoury College</p>";
const FEES: &str = "<table><tr><td>Lab Fee</td><td>$100.00</td></tr></table>";

fn healthy() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("get_subjectcoursecombo", "200 OK", COMBO),
        ("getCourseCatalogDetails", "200 OK", DETAILS),
        ("getPrerequisites", "200 OK", NO_PREREQ),
        ("getCorequisites", "200 OK", NO_PREREQ),
        ("getCourseDescription", "200 OK", DESCRIPTION),
        ("getCourseMutuallyExclusions", "200 OK", EXCLUSIONS),
        ("getCourseAttributes", "200 OK", ATTRIBUTES),
        ("getRestrictions", "200 OK", RESTRICTIONS),
        ("getFees", "200 OK", FEES),
    ]
}

fn with(
    mut routes: Vec<(&'static str, &'static str, &'static str)>,
    name: &'static str,
    status: &'static str,
    body: &'static str,
) -> Vec<(&'static str, &'static str, &'static str)> {
    routes.retain(|route| route.0 != name);
    routes.push((name, status, body));
    routes
}

fn lookup(
    fixture: &Fixture,
    extras: bool,
) -> Result<Option<termorganize_core::course_page::MissingCoursePage>, String> {
    let wanted = || true;
    let pause = || {};
    let control = LookupControl {
        extras,
        wanted: &wanted,
        pause: &pause,
    };
    fixture
        .client
        .fetch_course_page(&key(), "202710", &SubjectLookup::new(&[]), 5.0, &control)
}

fn names(fixture: &Fixture) -> Vec<String> {
    fixture
        .log
        .lock()
        .unwrap()
        .iter()
        .map(|(name, _, _)| name.clone())
        .collect()
}

#[test]
fn every_section_loads() {
    let fixture = serve(&healthy());
    let page = lookup(&fixture, true).unwrap().unwrap();
    assert_eq!(page.info.title, "Software Development");
    assert_eq!(page.info.credits_label().as_deref(), Some("4 credits"));
    assert_eq!(page.info.description.as_deref(), Some("Teaches design."));
    assert_eq!(page.description, Section::Loaded(()));
    assert_eq!(page.exclusions, Section::Loaded(vec![]));
    assert_eq!(
        page.attributes,
        Some(Section::Loaded(vec![
            "Differences and Diversity".to_string()
        ]))
    );
    let restrictions = page.restrictions.unwrap();
    assert_eq!(
        restrictions.loaded().unwrap()[0].values,
        vec!["Khoury College"]
    );
    let fees = page.fees.unwrap();
    assert_eq!(fees.loaded().unwrap()[0].amount.as_deref(), Some("$100.00"));
}

#[test]
fn only_the_description_failing_keeps_title_and_requisites() {
    let fixture = serve(&with(
        healthy(),
        "getCourseDescription",
        "404 Not Found",
        "",
    ));
    let page = lookup(&fixture, false).unwrap().unwrap();
    assert_eq!(page.info.title, "Software Development");
    assert_eq!(page.info.min_credits, Some(4.0));
    assert_eq!(page.info.description, None);
    assert_eq!(page.description.error(), Some("HTTP 404"));
}

#[test]
fn description_arriving_is_included() {
    let fixture = serve(&healthy());
    let page = lookup(&fixture, false).unwrap().unwrap();
    assert_eq!(page.info.description.as_deref(), Some("Teaches design."));
    assert!(page.attributes.is_none() && page.fees.is_none() && page.restrictions.is_none());
    assert!(!names(&fixture).contains(&"getFees".to_string()));
}

#[test]
fn a_failed_section_does_not_break_the_page_and_stops_later_requests() {
    let fixture = serve(&with(healthy(), "getCourseAttributes", "403 Forbidden", ""));
    let page = lookup(&fixture, true).unwrap().unwrap();
    assert_eq!(page.info.title, "Software Development");
    assert_eq!(page.description, Section::Loaded(()));
    assert_eq!(page.exclusions, Section::Loaded(vec![]));
    assert_eq!(page.attributes.unwrap().error(), Some("HTTP 403"));
    // Nothing is asked after the failure, and the sections left are marked, not blank.
    let log = names(&fixture);
    assert_eq!(log.last().unwrap(), "getCourseAttributes");
    assert!(page
        .restrictions
        .unwrap()
        .error()
        .unwrap()
        .starts_with("Not checked"));
    assert!(page
        .fees
        .unwrap()
        .error()
        .unwrap()
        .starts_with("Not checked"));
}

#[test]
fn a_required_failure_returns_an_error_and_asks_for_nothing_more() {
    let fixture = serve(&with(healthy(), "getPrerequisites", "404 Not Found", ""));
    assert_eq!(lookup(&fixture, true).unwrap_err(), "HTTP 404");
    assert_eq!(
        names(&fixture),
        vec![
            "get_subjectcoursecombo",
            "getCourseCatalogDetails",
            "getPrerequisites"
        ]
    );
}

#[test]
fn a_course_the_term_does_not_list_is_none_after_one_request() {
    let fixture = serve(&with(healthy(), "get_subjectcoursecombo", "200 OK", "[]"));
    assert!(lookup(&fixture, true).unwrap().is_none());
    assert_eq!(names(&fixture), vec!["get_subjectcoursecombo"]);
}

#[test]
fn restrictions_that_are_not_a_restrictions_page_fail_alone() {
    let fixture = serve(&with(
        healthy(),
        "getRestrictions",
        "200 OK",
        "<p>Something else</p>",
    ));
    let page = lookup(&fixture, true).unwrap().unwrap();
    assert!(page.restrictions.unwrap().error().is_some());
    assert!(page.attributes.unwrap().loaded().is_some());
}

#[test]
fn fee_and_attribute_text_fallback_ignores_page_chrome() {
    let chrome = "<html><head><title>Banner</title><script>x()</script></head><body><nav>Menu Sign in</nav>Home</body></html>";
    let chrome: &'static str = Box::leak(chrome.to_string().into_boxed_str());
    let routes = with(healthy(), "getFees", "200 OK", chrome);
    let fixture = serve(&with(routes, "getCourseAttributes", "200 OK", chrome));
    let page = lookup(&fixture, true).unwrap().unwrap();
    assert_eq!(
        page.attributes.unwrap().error(),
        Some("Banner returned an unexpected page")
    );
    assert!(page.fees.unwrap().error().is_some());

    use termorganize_core::banner::{course_attributes, course_fees};
    assert!(course_attributes(chrome).is_empty());
    assert!(course_fees(chrome).is_empty());
    // A plain fragment still reads line by line.
    assert_eq!(
        course_fees("Lab fee covers kits\nKit charge $25.00").len(),
        1
    );
    assert_eq!(
        course_attributes("Writing Intensive"),
        vec!["Writing Intensive"]
    );
    let long = format!("Menu {}", "x".repeat(200));
    assert!(course_attributes(&long).is_empty());
}

#[test]
fn requests_are_paced_one_gap_between_each() {
    let fixture = serve(&healthy());
    let pauses = AtomicUsize::new(0);
    let wanted = || true;
    let pause = || {
        pauses.fetch_add(1, Ordering::SeqCst);
    };
    let control = LookupControl {
        extras: true,
        wanted: &wanted,
        pause: &pause,
    };
    fixture
        .client
        .fetch_course_page(&key(), "202710", &SubjectLookup::new(&[]), 5.0, &control)
        .unwrap()
        .unwrap();
    let requests = names(&fixture).len();
    assert_eq!(requests, 9);
    assert_eq!(pauses.load(Ordering::SeqCst), requests - 1);
}

#[test]
fn the_default_pause_really_spaces_requests() {
    let fixture = serve(&healthy());
    let wanted = || true;
    let control = LookupControl::paced(false, &wanted);
    fixture
        .client
        .fetch_course_page(&key(), "202710", &SubjectLookup::new(&[]), 5.0, &control)
        .unwrap()
        .unwrap();
    let log = fixture.log.lock().unwrap();
    assert!(log.len() >= 6);
    for pair in log.windows(2) {
        assert!(
            pair[1].1.duration_since(pair[0].1)
                >= BannerClient::RESTRICTION_GAP - Duration::from_millis(30),
            "requests {} and {} were too close",
            pair[0].0,
            pair[1].0
        );
    }
}

#[test]
fn a_course_nobody_wants_any_more_stops_the_lookup() {
    let fixture = serve(&healthy());
    let asked = AtomicUsize::new(0);
    let wanted = || asked.fetch_add(1, Ordering::SeqCst) < 2;
    let pause = || {};
    let control = LookupControl {
        extras: true,
        wanted: &wanted,
        pause: &pause,
    };
    let error = fixture
        .client
        .fetch_course_page(&key(), "202710", &SubjectLookup::new(&[]), 5.0, &control)
        .unwrap_err();
    assert_eq!(error, LOOKUP_CANCELLED);
    assert_eq!(names(&fixture).len(), 2);
}

#[test]
fn term_subject_and_number_are_percent_encoded_in_requests_and_links() {
    let fixture = serve(&healthy());
    let odd = CourseKey {
        subject: "C S&x".into(),
        number: "35 00#".into(),
    };
    let wanted = || true;
    let pause = || {};
    let control = LookupControl {
        extras: false,
        wanted: &wanted,
        pause: &pause,
    };
    let _ = fixture.client.fetch_course_page(
        &odd,
        "2027 10&a=b",
        &SubjectLookup::new(&[]),
        5.0,
        &control,
    );
    let log = fixture.log.lock().unwrap();
    let request = &log[0].2;
    assert!(request.contains("searchTerm=C%20S%26x%2035%2000%23&term=2027%2010%26a%3Db"));
    assert!(!request.contains("&a=b"));
    drop(log);
    assert_eq!(
        BannerClient::catalog_page_url("2027 10&a=b", &odd),
        "https://bnrordsp.neu.edu/ssb-prod/bwckctlg.p_disp_course_detail?cat_term_in=2027%2010%26a%3Db&subj_code_in=C%20S%26x&crse_numb_in=35%2000%23"
    );
}

#[test]
fn availability_copy_only_claims_retirement_when_known() {
    use termorganize_core::course_info::CourseInfo;
    let mut info = CourseInfo::new(key(), "Software Development", "202630", 1.0);
    assert_eq!(
        info.availability_label("202710", "Fall 2027"),
        format!(
            "Not offered in Fall 2027; last listed {}",
            termorganize_core::offering::label_of_term("202630")
        )
    );
    info.is_retired = Some(true);
    assert!(info
        .availability_label("202710", "Fall 2027")
        .starts_with("No longer offered; last listed"));
    let here = CourseInfo::new(key(), "Software Development", "202710", 1.0);
    assert_eq!(
        here.availability_label("202710", "Fall 2027"),
        "Not offered in Fall 2027"
    );
}
