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
