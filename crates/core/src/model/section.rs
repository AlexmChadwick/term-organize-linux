use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CourseKey {
    pub subject: String,
    pub number: String,
}

impl CourseKey {
    pub fn parse(raw: &str) -> Option<Self> {
        let cleaned = raw.to_ascii_uppercase().replace('-', " ");
        let letters: String = cleaned.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let rest = cleaned[letters.len()..].trim();
        if letters.is_empty() || rest.is_empty() || !rest.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        Some(Self { subject: letters, number: rest.to_string() })
    }

    pub fn level(&self) -> Option<i32> {
        self.number.chars().next().and_then(|c| c.to_digit(10)).map(|d| d as i32)
    }

    pub fn numeric_value(&self) -> i32 {
        self.number.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
    }
}

impl fmt::Display for CourseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.subject, self.number)
    }
}

impl PartialOrd for CourseKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CourseKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.subject
            .cmp(&other.subject)
            .then(self.numeric_value().cmp(&other.numeric_value()))
            .then(self.number.cmp(&other.number))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct DaySet(pub u8);

impl DaySet {
    pub const SUNDAY: Self = Self(1 << 0);
    pub const MONDAY: Self = Self(1 << 1);
    pub const TUESDAY: Self = Self(1 << 2);
    pub const WEDNESDAY: Self = Self(1 << 3);
    pub const THURSDAY: Self = Self(1 << 4);
    pub const FRIDAY: Self = Self(1 << 5);
    pub const SATURDAY: Self = Self(1 << 6);
    pub const WEEKDAYS: Self = Self(0b0111110);
    pub const LETTERS: [&str; 7] = ["U", "M", "T", "W", "R", "F", "S"];
    pub const SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    pub const FULL: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

    pub fn new(raw: u8) -> Self {
        Self(raw & 0x7f)
    }
    pub fn from_indices(indices: impl IntoIterator<Item = i32>) -> Self {
        let mut raw = 0u8;
        for i in indices {
            if (0..=6).contains(&i) {
                raw |= 1 << i;
            }
        }
        Self(raw)
    }
    pub fn contains_day(self, index: i32) -> bool {
        (0..=6).contains(&index) && self.0 & (1 << index) != 0
    }
    pub fn indices(self) -> Vec<i32> {
        (0..=6).filter(|&i| self.contains_day(i)).collect()
    }
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
    pub fn is_disjoint(self, other: Self) -> bool {
        self.0 & other.0 == 0
    }
    pub fn is_subset(self, other: Self) -> bool {
        self.0 & other.0 == self.0
    }
    pub fn compact_label(self) -> String {
        self.indices().into_iter().map(|i| Self::LETTERS[i as usize]).collect()
    }
    pub fn short_label(self) -> String {
        self.indices().into_iter().map(|i| Self::SHORT[i as usize].to_string()).collect::<Vec<_>>().join(", ")
    }
    pub fn insert(&mut self, day: Self) {
        self.0 |= day.0;
    }
    pub fn remove(&mut self, day: Self) {
        self.0 &= !day.0;
    }
    pub fn contains(self, day: Self) -> bool {
        self.0 & day.0 == day.0 && day.0 != 0
    }
}

impl Serialize for DaySet {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.0 & 0x7f)
    }
}
impl<'de> Deserialize<'de> for DaySet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(u8::deserialize(deserializer)? & 0x7f))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    #[serde(default)]
    pub days: DaySet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub building_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub building: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campus_code: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
}

impl Meeting {
    pub fn is_final_exam(&self) -> bool {
        self.kind.to_ascii_lowercase().contains("final")
    }
    pub fn has_time(&self) -> bool {
        self.start.is_some() && self.end.is_some() && !self.days.is_empty()
    }
    pub fn time_label(&self) -> String {
        match (self.start, self.end) {
            (Some(s), Some(e)) => range_label(s, e),
            _ => "Time TBA".into(),
        }
    }
    pub fn location_label(&self) -> String {
        let b = self.building_code.as_deref().or(self.building.as_deref());
        match (b, self.room.as_deref()) {
            (Some(b), Some(r)) => format!("{b} {r}"),
            (Some(b), None) => b.to_string(),
            _ => "Location TBA".into(),
        }
    }
    pub fn long_location_label(&self) -> String {
        let b = self.building.as_deref().or(self.building_code.as_deref());
        match (b, self.room.as_deref()) {
            (Some(b), Some(r)) => format!("{b} {r}"),
            (Some(b), None) => b.to_string(),
            _ => "Location TBA".into(),
        }
    }
    pub fn is_online(&self) -> bool {
        let b = format!("{}{}", self.building.as_deref().unwrap_or(""), self.building_code.as_deref().unwrap_or(""));
        b.to_ascii_lowercase().contains("online") || self.campus_code.as_deref() == Some("VTL")
    }
    pub fn overlaps(&self, other: &Meeting) -> bool {
        let (Some(start), Some(end), Some(os), Some(oe)) = (self.start, self.end, other.start, other.end) else {
            return false;
        };
        if self.days.intersection(other.days).is_empty() || !(start < oe && os < end) {
            return false;
        }
        date_windows_overlap(self.start_date.as_deref(), self.end_date.as_deref(), other.start_date.as_deref(), other.end_date.as_deref())
    }
}

pub fn date_windows_overlap(a_start: Option<&str>, a_end: Option<&str>, b_start: Option<&str>, b_end: Option<&str>) -> bool {
    let as_ = a_start.unwrap_or("0000-00-00");
    let ae = a_end.unwrap_or("9999-99-99");
    let bs = b_start.unwrap_or("0000-00-00");
    let be = b_end.unwrap_or("9999-99-99");
    as_ <= be && bs <= ae
}

pub fn label_minutes(minutes: i32, include_period: bool) -> String {
    let h24 = minutes.div_euclid(60).rem_euclid(24);
    let m = minutes.rem_euclid(60);
    let h12 = if h24 % 12 == 0 { 12 } else { h24 % 12 };
    let period = if h24 < 12 { "AM" } else { "PM" };
    let base = format!("{h12}:{m:02}");
    if include_period { format!("{base} {period}") } else { base }
}

pub fn range_label(start: i32, end: i32) -> String {
    let same = (start / 60 < 12) == (end / 60 < 12);
    format!("{} – {}", label_minutes(start, !same), label_minutes(end, true))
}

pub fn hour_axis_label(hour: i32) -> String {
    let h12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    let suffix = if hour < 12 || hour == 24 { "a" } else { "p" };
    format!("{h12}{suffix}")
}

pub fn minutes_from_hhmm(value: i32) -> i32 {
    (value / 100) * 60 + value % 100
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instructor {
    pub name: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

impl Instructor {
    pub fn display_name(&self) -> String {
        display_name(&self.name)
    }
}

pub fn display_name(raw: &str) -> String {
    let parts: Vec<&str> = raw.splitn(2, ',').map(str::trim).collect();
    if parts.len() == 2 && !parts[1].is_empty() {
        format!("{} {}", parts[1], parts[0])
    } else {
        raw.trim().to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseSection {
    pub crn: String,
    pub subject: String,
    pub number: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub section_number: String,
    #[serde(default)]
    pub part_of_term: String,
    #[serde(default)]
    pub seat_capacity: i32,
    #[serde(default)]
    pub seats_remaining: i32,
    #[serde(default)]
    pub enrollment: i32,
    #[serde(default)]
    pub waitlist_capacity: i32,
    #[serde(default)]
    pub waitlist_count: i32,
    #[serde(default)]
    pub waitlist_remaining: i32,
    #[serde(default)]
    pub class_type: String,
    #[serde(default)]
    pub honors: bool,
    #[serde(default)]
    pub campus: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_method: Option<String>,
    #[serde(default)]
    pub meetings: Vec<Meeting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructors: Option<Vec<Instructor>>,
    #[serde(default)]
    pub cross_list_crns: Vec<String>,
    #[serde(default)]
    pub attributes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereqs: Option<super::requisite::Requisite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coreqs: Option<super::requisite::Requisite>,
}

impl CourseSection {
    pub fn course_key(&self) -> CourseKey {
        CourseKey { subject: self.subject.clone(), number: self.number.clone() }
    }
    pub fn is_open(&self) -> bool {
        self.seats_remaining > 0
    }
    pub fn is_full(&self) -> bool {
        self.seats_remaining <= 0
    }
    pub fn is_online(&self) -> bool {
        self.campus.to_ascii_lowercase().contains("online")
            || self.instruction_method.as_deref().unwrap_or("").to_ascii_lowercase().contains("online")
    }
    pub fn weekly_meetings(&self) -> Vec<&Meeting> {
        self.meetings.iter().filter(|m| !m.is_final_exam() && m.has_time()).collect()
    }
    pub fn final_exam(&self) -> Option<&Meeting> {
        self.meetings.iter().find(|m| m.is_final_exam())
    }
    pub fn meeting_days(&self) -> DaySet {
        self.weekly_meetings().iter().fold(DaySet(0), |a, m| a.union(m.days))
    }
    pub fn instructor_label(&self) -> String {
        let Some(instructors) = &self.instructors else { return "Instructor unavailable".into() };
        if instructors.is_empty() {
            return "TBA".into();
        }
        let mut list = instructors.clone();
        list.sort_by_key(|instructor| std::cmp::Reverse(instructor.primary));
        list.iter().map(|i| i.display_name()).collect::<Vec<_>>().join(", ")
    }
    pub fn schedule_summary(&self) -> String {
        let weekly = self.weekly_meetings();
        if weekly.is_empty() {
            return if self.is_online() { "Asynchronous / online".into() } else { "Times TBA".into() };
        }
        weekly.iter().map(|m| format!("{} {}", m.days.compact_label(), m.time_label())).collect::<Vec<_>>().join(" · ")
    }
    pub fn seats_label(&self) -> String {
        format!("{} of {} seats", self.seats_remaining.max(0), self.seat_capacity)
    }
    pub fn conflicts_with(&self, other: &CourseSection) -> bool {
        let mine: Vec<&Meeting> = self.weekly_meetings();
        let theirs: Vec<&Meeting> = other.weekly_meetings();
        mine.iter().any(|a| theirs.iter().any(|b| a.overlaps(b)))
    }
    pub fn has_no_campus(&self) -> bool {
        self.campus.to_ascii_lowercase().contains("no campus")
    }
}
