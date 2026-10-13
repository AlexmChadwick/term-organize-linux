//! Seat and waitlist alerts. A section alerts when it goes from no seats to some,
//! or (when asked) when a full section's waitlist gains room.

use crate::links::section_url;
use crate::model::*;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlertKind { Seats, Waitlist }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchlistAlert {
    pub kind: AlertKind,
    pub term_code: String,
    pub crn: String,
    pub course_key: CourseKey,
    pub course_title: String,
    pub section_name: String,
    pub seats_remaining: i32,
    pub seat_capacity: i32,
    pub waitlist_remaining: i32,
}

impl WatchlistAlert {
    pub fn title(&self) -> String {
        match self.kind {
            AlertKind::Seats => format!("Seats open in {}", self.course_key),
            AlertKind::Waitlist => format!("Waitlist spot open in {}", self.course_key),
        }
    }
    pub fn body(&self) -> String {
        match self.kind {
            AlertKind::Seats => {
                let seats = if self.seat_capacity > 0 {
                    format!("{} of {} seats", self.seats_remaining, self.seat_capacity)
                } else if self.seats_remaining == 1 {
                    "1 seat".into()
                } else {
                    format!("{} seats", self.seats_remaining)
                };
                format!("{seats} in {} (CRN {}). Register soon!", self.section_name, self.crn)
            }
            AlertKind::Waitlist => {
                let spots = if self.waitlist_remaining == 1 { "1 waitlist spot".into() } else { format!("{} waitlist spots", self.waitlist_remaining) };
                format!("{spots} open in {} (CRN {}). Join soon!", self.section_name, self.crn)
            }
        }
    }
    pub fn identifier(&self) -> String {
        let kind = match self.kind { AlertKind::Seats => "seats", AlertKind::Waitlist => "waitlist" };
        format!("watch-{kind}-{}-{}", self.term_code, self.crn)
    }
    pub fn url(&self) -> String {
        section_url(&self.term_code, &self.crn)
    }
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub items: Vec<WatchItem>,
    pub alerts: Vec<WatchlistAlert>,
    pub missing_ids: Vec<String>,
    /// Term codes for which fresh data was available, sorted.
    pub checked_term_codes: Vec<String>,
}

/// Availability of a watched section as of its last check (`WatchlistStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchStatus {
    Open,
    Waitlist,
    Full,
    Missing,
}

impl WatchStatus {
    pub fn of(item: &WatchItem, is_missing: bool) -> Self {
        if is_missing {
            Self::Missing
        } else if item.last_seats_remaining > 0 {
            Self::Open
        } else if item.last_waitlist_remaining > 0 {
            Self::Waitlist
        } else {
            Self::Full
        }
    }

    pub fn detail(self, item: &WatchItem) -> String {
        match self {
            Self::Open if item.seat_capacity > 0 => format!("{} of {} open", item.last_seats_remaining, item.seat_capacity),
            Self::Open => format!("{} open", item.last_seats_remaining),
            Self::Waitlist => {
                let spots = item.last_waitlist_remaining;
                let word = if spots == 1 { "spot" } else { "spots" };
                format!("Full · {spots} waitlist {word} open")
            }
            Self::Full => "Full".into(),
            Self::Missing => "No longer listed".into(),
        }
    }

    pub fn spoken_detail(self, item: &WatchItem) -> String {
        if self != Self::Open {
            return self.detail(item);
        }
        let seats = item.last_seats_remaining;
        if item.seat_capacity > 0 {
            format!("{seats} of {} seats open", item.seat_capacity)
        } else if seats == 1 {
            "1 seat open".into()
        } else {
            format!("{seats} seats open")
        }
    }
}

/// `just now`, `5 min ago`, `3 hr ago`, `yesterday`, `4 days ago` (`WatchlistFormat.ago`).
pub fn ago(then: f64, now: f64) -> String {
    let seconds = (now - then).max(0.0);
    if seconds < 60.0 {
        "just now".into()
    } else if seconds < 3_600.0 {
        format!("{} min ago", (seconds / 60.0) as i64)
    } else if seconds < 86_400.0 {
        format!("{} hr ago", (seconds / 3_600.0) as i64)
    } else {
        let days = (seconds / 86_400.0) as i64;
        if days == 1 { "yesterday".into() } else { format!("{days} days ago") }
    }
}

pub fn section_name(section: &CourseSection) -> String {
    let kind = if section.class_type.is_empty() { "Section" } else { section.class_type.as_str() };
    if section.section_number.is_empty() { kind.to_string() } else { format!("{kind} {}", section.section_number) }
}

/// `live` maps term code to the sections fetched for that term. Terms absent from the map are left alone.
pub fn compare(items: &[WatchItem], live: &HashMap<String, Vec<CourseSection>>, checked_at: f64) -> Outcome {
    let mut next = Vec::new();
    let mut alerts = Vec::new();
    let mut missing = Vec::new();
    let mut checked_term_codes: Vec<String> = live.keys().cloned().collect();
    checked_term_codes.sort();
    for item in items {
        let Some(sections) = live.get(&item.term_code) else {
            next.push(item.clone());
            continue;
        };
        let found = sections.iter().find(|s| s.crn == item.crn);
        let Some(section) = found else {
            let mut kept = item.clone();
            kept.last_checked_at = Some(checked_at);
            missing.push(kept.id());
            next.push(kept);
            continue;
        };
        let seats = section.seats_remaining;
        let wait = section.waitlist_remaining;
        let seat_alert = item.last_seats_remaining <= 0 && seats > 0;
        let wait_alert = item.notify_on_waitlist && !seat_alert && seats <= 0 && item.last_waitlist_remaining <= 0 && wait > 0;
        if seat_alert || wait_alert {
            alerts.push(WatchlistAlert {
                kind: if seat_alert { AlertKind::Seats } else { AlertKind::Waitlist },
                term_code: item.term_code.clone(),
                crn: item.crn.clone(),
                course_key: item.course_key.clone(),
                course_title: item.course_title.clone(),
                section_name: section_name(section),
                seats_remaining: seats,
                seat_capacity: section.seat_capacity,
                waitlist_remaining: wait,
            });
        }
        let mut updated = item.clone();
        updated.last_seats_remaining = seats;
        updated.last_waitlist_remaining = wait;
        updated.seat_capacity = section.seat_capacity;
        updated.last_checked_at = Some(checked_at);
        if seat_alert || wait_alert { updated.last_notified_at = Some(checked_at); }
        next.push(updated);
    }
    Outcome { items: next, alerts, missing_ids: missing, checked_term_codes }
}
