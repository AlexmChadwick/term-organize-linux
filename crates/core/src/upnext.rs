//! The class in progress, or the next one today.

use crate::model::*;

#[derive(Clone, Debug)]
pub struct UpcomingClass {
    pub crn: String,
    pub course: CourseKey,
    pub title: String,
    pub start: i32,
    pub end: i32,
    pub location: String,
    pub in_progress: bool,
}

pub fn occurrences(sections: &[CourseSection], weekday: i32, minutes: i32, iso: &str) -> Vec<UpcomingClass> {
    let mut items = Vec::new();
    for section in sections {
        for meeting in section.weekly_meetings() {
            if !meeting.days.contains_day(weekday) { continue; }
            let (Some(start), Some(end)) = (meeting.start, meeting.end) else { continue };
            if let Some(s) = &meeting.start_date {
                if iso < s.as_str() { continue; }
            }
            if let Some(e) = &meeting.end_date {
                if iso > e.as_str() { continue; }
            }
            if minutes >= end { continue; }
            items.push(UpcomingClass {
                crn: section.crn.clone(),
                course: section.course_key(),
                title: section.title.clone(),
                start,
                end,
                location: meeting.location_label(),
                in_progress: minutes >= start && minutes < end,
            });
        }
    }
    items.sort_by_key(|i| (i.start, i.course.clone()));
    items
}

pub fn up_next(items: &[UpcomingClass]) -> Option<&UpcomingClass> {
    items.iter().find(|i| i.in_progress).or_else(|| items.first())
}

pub fn summary(items: &[UpcomingClass]) -> String {
    match up_next(items) {
        Some(item) if item.in_progress => format!("{} until {}", item.course, super::model::label_minutes(item.end, true)),
        Some(item) => format!("{} at {}", item.course, super::model::label_minutes(item.start, true)),
        None => "No more classes today".into(),
    }
}
