//! Free rooms: a timeline of bookings and the gaps between them.

use crate::index::{BuildingSummary, RoomBooking, RoomSchedule};
use crate::model::DaySet;

#[derive(Clone, Debug)]
pub struct FreeStretch {
    pub start: i32,
    pub end: i32,
}

impl FreeStretch {
    pub fn minutes(&self) -> i32 { self.end - self.start }
    pub fn is_brief(&self) -> bool { self.minutes() < 30 }
}

pub fn stretches(bookings: &[RoomBooking], day: i32, iso: &str, day_start: i32, day_end: i32) -> Vec<FreeStretch> {
    let mut used: Vec<(i32, i32)> = bookings.iter().filter(|b| b.occurs_on(iso, day)).map(|b| (b.start, b.end)).collect();
    used.sort_by_key(|p| p.0);
    let mut merged: Vec<(i32, i32)> = Vec::new();
    for (s, e) in used {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 { last.1 = last.1.max(e); continue; }
        }
        merged.push((s, e));
    }
    let mut free = Vec::new();
    let mut cursor = day_start;
    for (s, e) in merged {
        let s = s.max(day_start);
        let e = e.min(day_end);
        if s > cursor { free.push(FreeStretch { start: cursor, end: s }); }
        cursor = cursor.max(e);
    }
    if cursor < day_end { free.push(FreeStretch { start: cursor, end: day_end }); }
    free
}

pub fn free_during_hour(rooms: &[RoomSchedule], day: i32, iso: &str, hour: i32) -> Vec<RoomSchedule> {
    let start = hour * 60;
    let end = start + 60;
    rooms.iter().filter(|room| {
        !room.bookings.iter().any(|b| b.occurs_on(iso, day) && b.start < end && start < b.end)
    }).cloned().collect()
}

pub fn search_rooms(buildings: &[BuildingSummary], query: &str) -> Vec<RoomSchedule> {
    let tokens: Vec<String> = query.split_whitespace().map(|t| t.to_ascii_lowercase()).filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return buildings.iter().flat_map(|b| b.rooms.clone()).collect();
    }
    buildings.iter().flat_map(|b| b.rooms.iter().filter(|room| {
        let hay = format!("{} {} {}", b.name, room.building_code, room.room).to_ascii_lowercase();
        tokens.iter().all(|t| hay.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(t)) || hay.contains(t))
    }).cloned()).collect()
}

pub fn weekday_index(name: &str) -> Option<i32> {
    match name.to_ascii_lowercase().as_str() {
        "sunday" | "sun" | "u" => Some(0),
        "monday" | "mon" | "m" => Some(1),
        "tuesday" | "tue" | "t" => Some(2),
        "wednesday" | "wed" | "w" => Some(3),
        "thursday" | "thu" | "r" => Some(4),
        "friday" | "fri" | "f" => Some(5),
        "saturday" | "sat" | "s" => Some(6),
        _ => None,
    }
}

pub fn day_label(index: i32) -> &'static str {
    DaySet::FULL.get(index as usize).copied().unwrap_or("Day")
}

/// A point in a typical week (`RoomAvailability.Moment`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moment {
    pub weekday: i32,
    pub minutes: i32,
    pub date: Option<&'static str>,
}

impl Moment {
    pub const FIRST_CLASS: i32 = 7 * 60;
    pub const LAST_CLASS: i32 = 22 * 60;

    pub fn new(weekday: i32, minutes: i32) -> Self {
        Self {
            weekday: weekday.rem_euclid(7),
            minutes: minutes.clamp(0, 24 * 60 - 1),
            date: None,
        }
    }

    pub fn on_date(weekday: i32, minutes: i32, date: &'static str) -> Self {
        Self { date: Some(date), ..Self::new(weekday, minutes) }
    }

    pub fn is_within_class_hours(self) -> bool {
        self.minutes >= Self::FIRST_CLASS && self.minutes < Self::LAST_CLASS
    }

    pub fn label(self) -> String {
        let day = DaySet::FULL.get(self.weekday as usize).copied().unwrap_or("Day");
        format!("{day} at {}", crate::model::label_minutes(self.minutes, true))
    }
}

/// Whether a classroom is free at one moment (`RoomAvailability.Status`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Free { until: Option<i32> },
    Busy { until: i32, free_until: Option<i32>, course: crate::model::CourseKey },
}

impl Availability {
    pub fn is_free(&self) -> bool {
        matches!(self, Self::Free { .. })
    }

    pub fn minutes_remaining(&self, moment: Moment) -> i32 {
        match self {
            Self::Free { until } => until.unwrap_or(24 * 60) - moment.minutes,
            Self::Busy { until, .. } => until - moment.minutes,
        }
    }

    pub fn is_free_only_briefly(&self, moment: Moment) -> bool {
        self.is_free() && self.minutes_remaining(moment) < 30
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Free { until: Some(until) } => format!("Free until {}", crate::model::label_minutes(*until, true)),
            Self::Free { until: None } => "Free for the rest of the day".into(),
            Self::Busy { until, .. } => format!("In use until {}", crate::model::label_minutes(*until, true)),
        }
    }

    pub fn detailed_summary(&self) -> String {
        match self {
            Self::Free { .. } => self.summary(),
            Self::Busy { until, course, .. } => format!("In use: {course} until {}", crate::model::label_minutes(*until, true)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RoomEntry {
    pub room: RoomSchedule,
    pub status: Availability,
}

#[derive(Clone, Debug)]
pub struct BuildingAvailability {
    pub building: BuildingSummary,
    pub free: Vec<RoomEntry>,
    pub busy: Vec<RoomEntry>,
    pub free_count: usize,
    pub room_count: usize,
}

pub fn bookings_of(room: &RoomSchedule, weekday: i32, date: Option<&str>) -> Vec<RoomBooking> {
    let mut day: Vec<RoomBooking> = room
        .bookings
        .iter()
        .filter(|booking| booking.days.contains_day(weekday) && booking.end > booking.start)
        .filter(|booking| match date {
            Some(iso) => booking.active_on(iso),
            None => !booking.is_short_session(),
        })
        .cloned()
        .collect();
    day.sort_by_key(|booking| (booking.start, booking.end));
    day
}

pub fn status_of_bookings(day: &[RoomBooking], minutes: i32) -> Availability {
    let Some(current) = day.iter().find(|booking| booking.start <= minutes && minutes < booking.end) else {
        return Availability::Free { until: day.iter().find(|booking| booking.start > minutes).map(|booking| booking.start) };
    };
    let mut until = current.end;
    for booking in day {
        if booking.start <= until && booking.end > until {
            until = booking.end;
        }
    }
    Availability::Busy {
        until,
        free_until: day.iter().find(|booking| booking.start >= until).map(|booking| booking.start),
        course: current.course.clone(),
    }
}

pub fn status_of(room: &RoomSchedule, moment: Moment) -> Availability {
    status_of_bookings(&bookings_of(room, moment.weekday, moment.date), moment.minutes)
}

/// Gaps between classes from `start` up to but not including `end` (`RoomAvailability.freeStretches`).
pub fn free_stretches(day: &[RoomBooking], start: i32, end: i32) -> Vec<std::ops::Range<i32>> {
    if start >= end {
        return Vec::new();
    }
    let mut ordered = day.to_vec();
    ordered.sort_by_key(|booking| (booking.start, booking.end));
    let mut stretches = Vec::new();
    let mut cursor = start;
    for booking in ordered {
        if booking.start >= end {
            break;
        }
        if booking.start > cursor {
            stretches.push(cursor..booking.start);
        }
        cursor = cursor.max(booking.end);
        if cursor >= end {
            return stretches;
        }
    }
    if cursor < end {
        stretches.push(cursor..end);
    }
    stretches
}

/// Rooms with no class during each whole hour from 7 AM to 10 PM (`RoomAvailability.freeRoomCounts`).
pub fn free_room_counts(rooms: &[RoomSchedule], weekday: i32) -> Vec<i32> {
    let hours = (Moment::LAST_CLASS - Moment::FIRST_CLASS) / 60;
    let mut counts = vec![0; hours as usize];
    for room in rooms {
        let day = bookings_of(room, weekday, None);
        for hour in 0..hours {
            let start = Moment::FIRST_CLASS + hour * 60;
            let end = start + 60;
            if !day.iter().any(|booking| booking.start < end && booking.end > start) {
                counts[hour as usize] += 1;
            }
        }
    }
    counts
}

pub fn availability_of(buildings: &[BuildingSummary], moment: Moment) -> Vec<BuildingAvailability> {
    buildings.iter().map(|building| availability_of_one(building, moment)).collect()
}

fn availability_of_one(building: &BuildingSummary, moment: Moment) -> BuildingAvailability {
    let mut free: Vec<(RoomEntry, usize)> = Vec::new();
    let mut busy: Vec<(RoomEntry, usize)> = Vec::new();
    for (order, room) in building.rooms.iter().enumerate() {
        let entry = RoomEntry { room: room.clone(), status: status_of(room, moment) };
        if entry.status.is_free() { free.push((entry, order)); } else { busy.push((entry, order)); }
    }
    free.sort_by(|left, right| {
        let remain = right.0.status.minutes_remaining(moment).cmp(&left.0.status.minutes_remaining(moment));
        remain.then(left.1.cmp(&right.1))
    });
    busy.sort_by(|left, right| {
        left.0.status.minutes_remaining(moment).cmp(&right.0.status.minutes_remaining(moment)).then(left.1.cmp(&right.1))
    });
    let free_count = free.len();
    BuildingAvailability {
        building: building.clone(),
        free: free.into_iter().map(|(entry, _)| entry).collect(),
        busy: busy.into_iter().map(|(entry, _)| entry).collect(),
        free_count,
        room_count: building.rooms.len(),
    }
}

fn fold_token(text: &str) -> String {
    text.chars().flat_map(|c| c.to_lowercase()).collect()
}

/// Keeps buildings and rooms whose words match `query` (`RoomAvailability.filter`).
pub fn filter_availability(items: &[BuildingAvailability], query: &str) -> Vec<BuildingAvailability> {
    let tokens: Vec<String> = query.split_whitespace().map(fold_token).filter(|token| !token.is_empty()).collect();
    if tokens.is_empty() {
        return items.to_vec();
    }
    items.iter().filter_map(|item| {
        let name = fold_token(&item.building.name);
        let code = fold_token(&item.building.code);
        let name_words: Vec<String> = name.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).map(|word| word.to_string()).collect();
        let building_hits: Vec<bool> = tokens.iter().map(|token| {
            code.starts_with(token) || name_words.iter().any(|word| word.starts_with(token)) || (token.len() >= 3 && name.contains(token))
        }).collect();
        if building_hits.iter().all(|hit| *hit) {
            return Some(item.clone());
        }
        let matches = |entry: &RoomEntry| {
            let number = fold_token(&entry.room.room);
            tokens.iter().enumerate().all(|(i, token)| building_hits[i] || number.starts_with(token) || (token.len() >= 2 && number.contains(token)))
        };
        let free: Vec<_> = item.free.iter().filter(|entry| matches(entry)).cloned().collect();
        let busy: Vec<_> = item.busy.iter().filter(|entry| matches(entry)).cloned().collect();
        if free.is_empty() && busy.is_empty() {
            None
        } else {
            Some(BuildingAvailability { free, busy, ..item.clone() })
        }
    }).collect()
}

/// What happens next in a room (`RoomsText.followUpNote`).
pub fn follow_up_note(status: &Availability, today: &[RoomBooking]) -> String {
    match status {
        Availability::Free { until: Some(until) } => {
            if let Some(next) = today.iter().find(|booking| booking.start == *until) {
                format!("Next: {}, {}", next.course, crate::model::range_label(next.start, next.end))
            } else if today.is_empty() {
                "No classes are scheduled here today.".into()
            } else {
                "No more classes today.".into()
            }
        }
        Availability::Free { until: None } => {
            if today.is_empty() { "No classes are scheduled here today.".into() } else { "No more classes today.".into() }
        }
        Availability::Busy { free_until: Some(free_until), .. } => format!("Then free until {}", crate::model::label_minutes(*free_until, true)),
        Availability::Busy { free_until: None, .. } => "Then free for the rest of the day".into(),
    }
}

pub fn free_summary(free: i32, total: i32) -> String {
    let noun = if total == 1 { "room" } else { "rooms" };
    format!("{free} of {total} {noun} free")
}
