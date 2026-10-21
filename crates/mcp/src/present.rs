// SPDX-License-Identifier: AGPL-3.0-only
//! JSON shapes shared by the catalog tools. Instructor addresses go through
//! the core Northeastern filter before they leave the process.

use serde_json::{json, Value};
use termorganize_core::email::retain_northeastern_addresses;
use termorganize_core::model::{credit_number, label_minutes, Course, CourseSection, Instructor};

pub fn credits_text(min: f64, max: f64) -> String {
    if (min - max).abs() < 1e-9 {
        credit_number(min)
    } else {
        format!("{} to {}", credit_number(min), credit_number(max))
    }
}

pub fn course_json(course: &Course) -> Value {
    json!({
        "subject": course.subject,
        "number": course.number,
        "title": course.title,
        "credits": credits_text(course.min_credits, course.max_credits),
        "min_credits": course.min_credits,
        "max_credits": course.max_credits,
        "attributes": course.attributes,
    })
}

pub fn kept_email(field: &str) -> Option<String> {
    retain_northeastern_addresses(field)
}

pub fn instructor_json(instructor: &Instructor) -> Value {
    let mut value = json!({ "name": instructor.display_name() });
    if let Some(email) = instructor.email.as_deref().and_then(kept_email) {
        value["email"] = json!(email);
    }
    value
}

pub fn times_label(section: &CourseSection) -> String {
    let weekly = section.weekly_meetings();
    if weekly.is_empty() {
        return if section.is_online() {
            "Asynchronous / online".into()
        } else {
            "Times TBA".into()
        };
    }
    weekly
        .iter()
        .map(|meeting| match (meeting.start, meeting.end) {
            (Some(start), Some(end)) => format!(
                "{} to {}",
                label_minutes(start, true),
                label_minutes(end, true)
            ),
            _ => "Time TBA".into(),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn location_label(section: &CourseSection) -> String {
    let weekly = section.weekly_meetings();
    if weekly.is_empty() {
        return if section.is_online() {
            "Online".into()
        } else {
            "Location TBA".into()
        };
    }
    weekly
        .iter()
        .map(|meeting| meeting.location_label())
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn section_json(section: &CourseSection) -> Value {
    let instructors = section
        .instructors
        .as_ref()
        .map(|list| list.iter().map(instructor_json).collect::<Vec<_>>())
        .unwrap_or_default();
    json!({
        "crn": section.crn,
        "section": section.section_number,
        "days": section.meeting_days().compact_label(),
        "times": times_label(section),
        "location": location_label(section),
        "instructors": instructors,
        "seats_remaining": section.seats_remaining,
        "seat_capacity": section.seat_capacity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_address_field_keeps_only_northeastern() {
        let kept = kept_email("person@gmail.com faculty@northeastern.edu other@kent.edu").unwrap();
        assert_eq!(kept, "faculty@northeastern.edu");
        assert!(kept_email("person@gmail.com").is_none());
        assert!(kept_email("lab@mail.neu.edu").is_none());
        assert!(kept_email("albanese.d@husky.neu.edu").is_none());
        assert!(kept_email("viola@ccs.neu.edu").is_none());
    }
}
