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
    Outcome { items: next, alerts, missing_ids: missing }
}
