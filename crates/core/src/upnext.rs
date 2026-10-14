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

pub fn occurrences(
    sections: &[CourseSection],
    weekday: i32,
    minutes: i32,
    iso: &str,
) -> Vec<UpcomingClass> {
    let mut items = Vec::new();
    for section in sections {
        for meeting in section.weekly_meetings() {
            if !meeting.days.contains_day(weekday) {
                continue;
            }
            let (Some(start), Some(end)) = (meeting.start, meeting.end) else {
                continue;
            };
            if let Some(s) = &meeting.start_date {
                if iso < s.as_str() {
                    continue;
                }
            }
            if let Some(e) = &meeting.end_date {
                if iso > e.as_str() {
                    continue;
                }
            }
            if minutes >= end {
                continue;
            }
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
    items
        .iter()
        .find(|i| i.in_progress)
        .or_else(|| items.first())
}

pub fn summary(items: &[UpcomingClass]) -> String {
    match up_next(items) {
        Some(item) if item.in_progress => format!(
            "{} until {}",
            item.course,
            super::model::label_minutes(item.end, true)
        ),
        Some(item) => format!(
            "{} at {}",
            item.course,
            super::model::label_minutes(item.start, true)
        ),
        None => "No more classes today".into(),
    }
}

/// One weekly meeting in the widget snapshot (`WidgetScheduleData.Meeting`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidgetMeeting {
    pub id: String,
    pub crn: String,
    pub course_code: String,
    pub title: String,
    pub location: String,
    pub class_type: String,
    pub weekday: i32,
    pub start: i32,
    pub end: i32,
    pub color_index: i32,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

impl WidgetMeeting {
    fn is_active(&self, iso: &str) -> bool {
        if self.start_date.is_none() && self.end_date.is_none() {
            return true;
        }
        if let Some(start) = &self.start_date {
            if iso < start.as_str() {
                return false;
            }
        }
        if let Some(end) = &self.end_date {
            if iso > end.as_str() {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetOccurrence {
    pub meeting: WidgetMeeting,
    pub start: f64,
    pub end: f64,
}

/// Primary schedule flattened for the up-next widget (`WidgetScheduleData`).
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetScheduleData {
    pub generated_at: f64,
    pub term_code: String,
    pub term_name: String,
    pub plan_name: String,
    pub classes_start: Option<f64>,
    pub classes_end: Option<f64>,
    pub meetings: Vec<WidgetMeeting>,
}

impl WidgetScheduleData {
    /// Upcoming or in-progress classes over the next 14 days in America/New_York.
    pub fn occurrences(&self, from: f64, limit: usize) -> Vec<WidgetOccurrence> {
        if self.meetings.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut results = Vec::new();
        let start_of_today = start_of_eastern_day(from);
        for day_offset in 0..14 {
            let day = start_of_today + day_offset as f64 * 86_400.0;
            if let Some(end) = self.classes_end {
                if day > start_of_eastern_day(end) {
                    break;
                }
            }
            if let Some(start) = self.classes_start {
                if day < start_of_eastern_day(start) {
                    continue;
                }
            }
            let iso = eastern_iso_day(day);
            let weekday = eastern_weekday(day);
            let mut todays: Vec<&WidgetMeeting> = self
                .meetings
                .iter()
                .filter(|meeting| meeting.weekday == weekday && meeting.is_active(&iso))
                .collect();
            todays.sort_by_key(|meeting| meeting.start);
            for meeting in todays {
                let start = day + f64::from(meeting.start) * 60.0;
                let end = day + f64::from(meeting.end) * 60.0;
                if end <= from {
                    continue;
                }
                results.push(WidgetOccurrence {
                    meeting: meeting.clone(),
                    start,
                    end,
                });
                if results.len() >= limit {
                    return results;
                }
            }
        }
        results
    }

    pub fn classes_on(&self, date: f64) -> Vec<WidgetOccurrence> {
        let day = start_of_eastern_day(date);
        if let Some(start) = self.classes_start {
            if day < start_of_eastern_day(start) {
                return Vec::new();
            }
        }
        if let Some(end) = self.classes_end {
            if day > start_of_eastern_day(end) {
                return Vec::new();
            }
        }
        let iso = eastern_iso_day(day);
        let weekday = eastern_weekday(day);
        let mut todays: Vec<&WidgetMeeting> = self
            .meetings
            .iter()
            .filter(|meeting| meeting.weekday == weekday && meeting.is_active(&iso))
            .collect();
        todays.sort_by_key(|meeting| meeting.start);
        todays
            .into_iter()
            .map(|meeting| WidgetOccurrence {
                start: day + f64::from(meeting.start) * 60.0,
                end: day + f64::from(meeting.end) * 60.0,
                meeting: meeting.clone(),
            })
            .collect()
    }
}

fn eastern_weekday(unix: f64) -> i32 {
    let (year, month, day, _, _) = eastern_parts(unix);
    ((days_from_civil(year, month, day) + 4).rem_euclid(7)) as i32
}

fn eastern_iso_day(unix: f64) -> String {
    let (year, month, day, _, _) = eastern_parts(unix);
    format!("{year:04}-{month:02}-{day:02}")
}

fn start_of_eastern_day(unix: f64) -> f64 {
    let (year, month, day, _, _) = eastern_parts(unix);
    eastern_to_unix(year, month, day, 0, 0)
}

fn eastern_parts(unix: f64) -> (i32, i32, i32, i32, i32) {
    let secs = unix.floor() as i64;
    for offset_hours in [-4_i64, -5] {
        let local = secs + offset_hours * 3600;
        let (year, month, day, hour, minute, _) = civil_hms(local);
        if eastern_offset_hours(year, month, day, hour, minute) == offset_hours {
            return (year, month, day, hour, minute);
        }
    }
    let (year, month, day, hour, minute, _) = civil_hms(secs - 4 * 3600);
    (year, month, day, hour, minute)
}

pub fn eastern_to_unix(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> f64 {
    let offset = eastern_offset_hours(year, month, day, hour, minute);
    (days_from_civil(year, month, day) * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60
        - offset * 3600) as f64
}

fn eastern_offset_hours(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> i64 {
    let local = (month, day, hour, minute);
    let spring = nth_weekday(year, 3, 0, 2);
    let fall = nth_weekday(year, 11, 0, 1);
    let dst = local >= (3, spring, 3, 0) && local < (11, fall, 2, 0);
    if dst {
        -4
    } else {
        -5
    }
}

fn nth_weekday(year: i32, month: i32, weekday: i32, n: i32) -> i32 {
    let first = ((days_from_civil(year, month, 1) + 4).rem_euclid(7)) as i32;
    let delta = (weekday - first).rem_euclid(7);
    1 + delta + (n - 1) * 7
}

fn civil_hms(seconds: i64) -> (i32, i32, i32, i32, i32, i32) {
    let day = seconds.div_euclid(86_400);
    let tod = seconds.rem_euclid(86_400);
    let (year, month, date) = civil_from_days(day);
    let hour = (tod / 3600) as i32;
    let minute = ((tod % 3600) / 60) as i32;
    let second = (tod % 60) as i32;
    (year, month, date, hour, minute, second)
}

fn days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let mut year = i64::from(year);
    let month = month as u32;
    let day = day as u32;
    year -= i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = (year - era * 400) as u64;
    let doy = (153 * u64::from(if month > 2 { month - 3 } else { month + 9 }) + 2) / 5
        + u64::from(day)
        - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

fn civil_from_days(z: i64) -> (i32, i32, i32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as i32, day as i32)
}
