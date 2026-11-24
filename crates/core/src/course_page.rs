// SPDX-License-Identifier: AGPL-3.0-only
//! State and pacing for the page of a course the loaded term does not list.
//!
//! Unofficial. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.
//!
//! The window asks Banner's catalog about the course. Each open course keeps one
//! `PageState` per (term, course), so a slow answer for another term is never shown, a
//! failed lookup says so and retries when the course is opened again, and a course the
//! user left is dropped. Lookups run through `LookupQueue`: one worker, one course at a time.

use crate::banner::CourseFee;
use crate::course_info::CourseInfo;
use crate::model::*;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Result of one independent part of the page. A failed part never hides the others.
#[derive(Clone, Debug, PartialEq)]
pub enum Section<T> {
    Loaded(T),
    Failed(String),
}

impl<T> Section<T> {
    pub fn loaded(&self) -> Option<&T> {
        match self {
            Section::Loaded(value) => Some(value),
            Section::Failed(_) => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Section::Loaded(_) => None,
            Section::Failed(message) => Some(message),
        }
    }
}

/// Everything Banner's catalog says about a course the term does not list.
#[derive(Clone, Debug, PartialEq)]
pub struct MissingCoursePage {
    /// Title, credits and requisites. `description` and `equivalents` mirror the sections below.
    pub info: CourseInfo,
    pub description: Section<()>,
    pub exclusions: Section<Vec<CourseKey>>,
    /// `None` when the lookup was made without the extra sections.
    pub attributes: Option<Section<Vec<String>>>,
    pub restrictions: Option<Section<Vec<EnrollmentRestriction>>>,
    pub fees: Option<Section<Vec<CourseFee>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PageState {
    Loading,
    /// The lookup could not be completed. Reopening the course tries again.
    Failed(String),
    /// Banner answered and does not list the course in this term.
    NotListed,
    Found(Box<MissingCoursePage>),
}

/// Heading and body for a lookup that did not finish. Matches Windows.
pub fn failed_copy(key: &CourseKey, term_name: &str, message: &str) -> (String, String) {
    let mut body = String::from(
        "Banner's course catalog didn't answer, so the course's details couldn't be checked. Check your connection and open the course again.",
    );
    let message = message.trim();
    if !message.is_empty() {
        body.push_str(&format!(" ({message})"));
    }
    (format!("{key} · Couldn't check {term_name}"), body)
}

/// Heading and body for a course Banner's catalog does not list in the term.
pub fn not_listed_copy(key: &CourseKey, term_name: &str) -> (String, String) {
    (
        format!("{key} · Not listed in {term_name}"),
        format!("Banner's course catalog doesn't list {key} in {term_name}."),
    )
}

/// `Must be enrolled in one of the following Colleges: Khoury College, CAMD`.
pub fn restriction_line(restriction: &EnrollmentRestriction) -> String {
    let lead = match restriction.kind {
        RestrictionKind::Must => "Must be enrolled in one of the following",
        RestrictionKind::Cannot => "Cannot be enrolled in one of the following",
    };
    format!(
        "{lead} {}: {}",
        restriction.category,
        restriction.values.join(", ")
    )
}

/// `Lab Fee: $100.00`, or just the description when Banner gave no amount.
pub fn fee_line(fee: &CourseFee) -> String {
    match &fee.amount {
        Some(amount) => format!("{}: {amount}", fee.description),
        None => fee.description.clone(),
    }
}

/// Handed to the worker with a lookup. The worker checks `wanted` before each request.
#[derive(Clone, Debug)]
pub struct Ticket {
    term: String,
    key: CourseKey,
    cancel: Arc<AtomicBool>,
}

impl Ticket {
    pub fn term(&self) -> &str {
        &self.term
    }

    pub fn key(&self) -> &CourseKey {
        &self.key
    }

    /// False once the course was left, or the book was reset (a term change). Both set `cancel`.
    pub fn wanted(&self) -> bool {
        !self.cancel.load(Ordering::SeqCst)
    }
}

/// Shown when a lookup ends without an answer: the worker panicked or dropped the job.
pub const LOOKUP_LOST_MESSAGE: &str = "The lookup stopped unexpectedly";

#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    Stored,
    /// The answer was for a term or window the user already left.
    Dropped,
}

struct Entry {
    state: PageState,
    cancel: Arc<AtomicBool>,
}

/// Per-(term, course) lookup results for the pages that need them.
pub struct LookupBook {
    entries: HashMap<(String, CourseKey), Entry>,
}

impl Default for LookupBook {
    fn default() -> Self {
        Self::new()
    }
}

impl LookupBook {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn state(&self, term: &str, key: &CourseKey) -> Option<&PageState> {
        self.entries
            .get(&(term.to_string(), key.clone()))
            .map(|entry| &entry.state)
    }

    /// Starts a lookup when there is no entry. `None` when one is loading or has an answer.
    pub fn begin(&mut self, term: &str, key: &CourseKey) -> Option<Ticket> {
        let id = (term.to_string(), key.clone());
        if self.entries.contains_key(&id) {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.entries.insert(
            id,
            Entry {
                state: PageState::Loading,
                cancel: cancel.clone(),
            },
        );
        Some(Ticket {
            term: term.to_string(),
            key: key.clone(),
            cancel,
        })
    }

    /// Forgets a failed lookup so the next `begin` tries again. True when one was forgotten.
    pub fn forget_failed(&mut self, term: &str, key: &CourseKey) -> bool {
        let id = (term.to_string(), key.clone());
        if matches!(
            self.entries.get(&id),
            Some(Entry {
                state: PageState::Failed(_),
                ..
            })
        ) {
            self.entries.remove(&id);
            return true;
        }
        false
    }

    /// The user left the course. A running lookup is cancelled and its entry removed.
    pub fn leave(&mut self, term: &str, key: &CourseKey) {
        let id = (term.to_string(), key.clone());
        if matches!(
            self.entries.get(&id),
            Some(Entry {
                state: PageState::Loading,
                ..
            })
        ) {
            if let Some(entry) = self.entries.remove(&id) {
                entry.cancel.store(true, Ordering::SeqCst);
            }
        }
    }

    /// Term change: every lookup in flight becomes stale and every answer is forgotten.
    pub fn reset(&mut self) {
        for entry in self.entries.values() {
            entry.cancel.store(true, Ordering::SeqCst);
        }
        self.entries.clear();
    }

    /// Stores a finished lookup. Dropped (and the entry released) when the term changed,
    /// the book was reset, the course was left, or its window is gone.
    pub fn finish(
        &mut self,
        ticket: &Ticket,
        current_term: &str,
        window_open: bool,
        outcome: Result<Option<MissingCoursePage>, String>,
    ) -> Applied {
        let id = (ticket.term.clone(), ticket.key.clone());
        let ours = self
            .entries
            .get(&id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.cancel, &ticket.cancel));
        if !ours {
            return Applied::Dropped;
        }
        if ticket.term != current_term || !ticket.wanted() || !window_open {
            self.entries.remove(&id);
            return Applied::Dropped;
        }
        let state = match outcome {
            Ok(Some(page)) => PageState::Found(Box::new(page)),
            Ok(None) => PageState::NotListed,
            Err(message) => PageState::Failed(message),
        };
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.state = state;
        }
        Applied::Stored
    }
}

struct QueueState<J> {
    jobs: VecDeque<J>,
    closed: bool,
}

struct QueueShared<J> {
    state: Mutex<QueueState<J>>,
    ready: Condvar,
}

/// A fixed set of worker threads that run lookups one after another. Jobs whose
/// `wanted` check fails are dropped when their turn comes.
pub struct LookupQueue<J: Send + 'static> {
    shared: Arc<QueueShared<J>>,
}

impl<J: Send + 'static> LookupQueue<J> {
    /// `workers` is clamped to one or two so a pile of opened courses cannot flood Banner.
    pub fn spawn(
        workers: usize,
        wanted: impl Fn(&J) -> bool + Send + Sync + 'static,
        run: impl Fn(J) + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(QueueShared {
            state: Mutex::new(QueueState {
                jobs: VecDeque::new(),
                closed: false,
            }),
            ready: Condvar::new(),
        });
        let wanted = Arc::new(wanted);
        let run = Arc::new(run);
        for _ in 0..workers.clamp(1, 2) {
            let shared = shared.clone();
            let wanted = wanted.clone();
            let run = run.clone();
            std::thread::spawn(move || loop {
                let job = {
                    let mut state = shared.state.lock().unwrap();
                    loop {
                        if state.closed {
                            return;
                        }
                        if let Some(job) = state.jobs.pop_front() {
                            break job;
                        }
                        state = shared.ready.wait(state).unwrap();
                    }
                };
                if wanted(&job) {
                    // A panicking lookup must not take the worker with it: the job (and the
                    // reply channel inside it) is dropped by the unwind, so its page sees a
                    // disconnect and shows an error, and later lookups still run.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(job)));
                }
            });
        }
        Self { shared }
    }

    pub fn push(&self, job: J) {
        self.shared.state.lock().unwrap().jobs.push_back(job);
        self.shared.ready.notify_one();
    }
}

impl<J: Send + 'static> Drop for LookupQueue<J> {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().closed = true;
        self.shared.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::Duration;

    fn key() -> CourseKey {
        CourseKey::parse("CS 3500").unwrap()
    }

    fn page() -> MissingCoursePage {
        MissingCoursePage {
            info: CourseInfo::new(key(), "Software Development", "202710", 1.0),
            description: Section::Loaded(()),
            exclusions: Section::Loaded(vec![]),
            attributes: None,
            restrictions: None,
            fees: None,
        }
    }

    #[test]
    fn answer_for_a_term_the_user_switched_away_from_is_dropped() {
        let mut book = LookupBook::new();
        let ticket = book.begin("202710", &key()).unwrap();
        assert_eq!(
            book.finish(&ticket, "202730", true, Ok(Some(page()))),
            Applied::Dropped
        );
        assert!(book.state("202710", &key()).is_none());
        assert!(book.state("202730", &key()).is_none());
    }

    #[test]
    fn reset_makes_an_in_flight_lookup_stale_even_for_the_same_term() {
        let mut book = LookupBook::new();
        let ticket = book.begin("202710", &key()).unwrap();
        book.reset();
        assert!(!ticket.wanted());
        let second = book.begin("202710", &key()).unwrap();
        assert!(second.wanted());
        assert_eq!(
            book.finish(&ticket, "202710", true, Ok(Some(page()))),
            Applied::Dropped
        );
        assert_eq!(book.state("202710", &key()), Some(&PageState::Loading));
        assert_eq!(
            book.finish(&second, "202710", true, Ok(None)),
            Applied::Stored
        );
        assert_eq!(book.state("202710", &key()), Some(&PageState::NotListed));
    }

    #[test]
    fn answer_after_the_window_closed_is_dropped_and_reopening_looks_again() {
        let mut book = LookupBook::new();
        let ticket = book.begin("202710", &key()).unwrap();
        assert_eq!(
            book.finish(&ticket, "202710", false, Ok(Some(page()))),
            Applied::Dropped
        );
        assert!(book.begin("202710", &key()).is_some());
    }

    #[test]
    fn leaving_a_course_cancels_its_lookup() {
        let mut book = LookupBook::new();
        let ticket = book.begin("202710", &key()).unwrap();
        book.leave("202710", &key());
        assert!(!ticket.wanted());
        assert_eq!(
            book.finish(&ticket, "202710", true, Ok(Some(page()))),
            Applied::Dropped
        );
    }

    #[test]
    fn leaving_keeps_answers_that_already_arrived() {
        let mut book = LookupBook::new();
        let ticket = book.begin("202710", &key()).unwrap();
        book.finish(&ticket, "202710", true, Ok(Some(page())));
        book.leave("202710", &key());
        assert!(matches!(
            book.state("202710", &key()),
            Some(PageState::Found(_))
        ));
    }

    #[test]
    fn a_loading_course_is_not_looked_up_twice() {
        let mut book = LookupBook::new();
        assert!(book.begin("202710", &key()).is_some());
        assert!(book.begin("202710", &key()).is_none());
        assert!(book.begin("202730", &key()).is_some());
    }

    #[test]
    fn failed_lookup_is_kept_for_display_and_retried_only_when_forgotten() {
        let mut book = LookupBook::new();
        let ticket = book.begin("202710", &key()).unwrap();
        book.finish(&ticket, "202710", true, Err("HTTP 503".into()));
        assert_eq!(
            book.state("202710", &key()),
            Some(&PageState::Failed("HTTP 503".into()))
        );
        // Rendering the failure again must not start another lookup.
        assert!(book.begin("202710", &key()).is_none());
        assert!(book.forget_failed("202710", &key()));
        assert!(book.begin("202710", &key()).is_some());
        assert!(!book.forget_failed("202710", &CourseKey::parse("CS 1000").unwrap()));
    }

    #[test]
    fn not_listed_is_a_final_answer_for_that_term() {
        let mut book = LookupBook::new();
        let ticket = book.begin("202710", &key()).unwrap();
        book.finish(&ticket, "202710", true, Ok(None));
        assert!(!book.forget_failed("202710", &key()));
        assert!(book.begin("202710", &key()).is_none());
    }

    #[test]
    fn restriction_and_fee_lines() {
        let restriction = EnrollmentRestriction {
            kind: RestrictionKind::Must,
            category: "Colleges".into(),
            values: vec!["Khoury College".into(), "CAMD".into()],
        };
        assert_eq!(
            restriction_line(&restriction),
            "Must be enrolled in one of the following Colleges: Khoury College, CAMD"
        );
        let cannot = EnrollmentRestriction {
            kind: RestrictionKind::Cannot,
            ..restriction
        };
        assert!(restriction_line(&cannot).starts_with("Cannot be enrolled"));
        let fee = CourseFee {
            description: "Lab Fee".into(),
            amount: Some("$100.00".into()),
        };
        assert_eq!(fee_line(&fee), "Lab Fee: $100.00");
        let bare = CourseFee {
            description: "Materials $20".into(),
            amount: None,
        };
        assert_eq!(fee_line(&bare), "Materials $20");
    }

    #[test]
    fn failure_and_not_listed_copy_matches_windows() {
        let (heading, body) = failed_copy(&key(), "Fall 2027", "HTTP 503");
        assert_eq!(heading, "CS 3500 · Couldn't check Fall 2027");
        assert_eq!(
            body,
            "Banner's course catalog didn't answer, so the course's details couldn't be checked. Check your connection and open the course again. (HTTP 503)"
        );
        let (heading, body) = failed_copy(&key(), "Fall 2027", " ");
        assert!(heading.ends_with("Couldn't check Fall 2027"));
        assert!(body.ends_with("open the course again."));
        let (heading, body) = not_listed_copy(&key(), "Fall 2027");
        assert_eq!(heading, "CS 3500 · Not listed in Fall 2027");
        assert!(body.contains("doesn't list CS 3500 in Fall 2027"));
    }

    #[test]
    fn queue_runs_one_job_at_a_time_and_drops_jobs_nobody_wants() {
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (done_tx, done_rx) = mpsc::channel::<u32>();
        let done_tx = Mutex::new(done_tx);
        let queue = {
            let running = running.clone();
            let peak = peak.clone();
            LookupQueue::spawn(
                1,
                |job: &(u32, bool)| job.1,
                move |job| {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    running.fetch_sub(1, Ordering::SeqCst);
                    done_tx.lock().unwrap().send(job.0).unwrap();
                },
            )
        };
        queue.push((1, true));
        queue.push((2, false));
        queue.push((3, true));
        queue.push((4, true));
        let finished: Vec<u32> = (0..3)
            .map(|_| done_rx.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert_eq!(finished, vec![1, 3, 4]);
        assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_panicking_lookup_drops_its_reply_and_the_worker_keeps_serving() {
        let (reply_tx, reply_rx) = mpsc::channel::<u32>();
        let (done_tx, done_rx) = mpsc::channel::<u32>();
        let done_tx = Mutex::new(done_tx);
        let queue = LookupQueue::spawn(
            1,
            |_: &(u32, Option<mpsc::Sender<u32>>)| true,
            move |job| {
                if job.0 == 1 {
                    let _reply = job.1;
                    panic!("simulated lookup panic");
                }
                done_tx.lock().unwrap().send(job.0).unwrap();
            },
        );
        queue.push((1, Some(reply_tx)));
        queue.push((2, None));
        // The page waiting on the first job sees a disconnect, not a hang.
        assert_eq!(
            reply_rx.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
        // With one worker, the second job only runs if the worker survived.
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)), Ok(2));
    }

    #[test]
    fn queue_never_runs_more_than_two_workers() {
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let done_tx = Mutex::new(done_tx);
        let queue = {
            let running = running.clone();
            let peak = peak.clone();
            LookupQueue::spawn(
                16,
                |_: &u32| true,
                move |_| {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    running.fetch_sub(1, Ordering::SeqCst);
                    done_tx.lock().unwrap().send(()).unwrap();
                },
            )
        };
        for job in 0..8 {
            queue.push(job);
        }
        for _ in 0..8 {
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= 2);
    }
}
