//! "Your ideal week": hard rules remove schedules; the rest only rank them.

use super::section::{range_label, DaySet};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreferenceStrength {
    Preferred,
    Required,
}

impl PreferenceStrength {
    pub fn title(self) -> &'static str {
        match self {
            Self::Preferred => "If possible",
            Self::Required => "Always",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeBand {
    EarlyMorning,
    Morning,
    Midday,
    Afternoon,
    Evening,
    Night,
}

impl TimeBand {
    pub const ALL: [TimeBand; 6] = [
        Self::EarlyMorning,
        Self::Morning,
        Self::Midday,
        Self::Afternoon,
        Self::Evening,
        Self::Night,
    ];
    pub fn minutes(self) -> (i32, i32) {
        match self {
            Self::EarlyMorning => (0, 9 * 60),
            Self::Morning => (9 * 60, 12 * 60),
            Self::Midday => (12 * 60, 14 * 60),
            Self::Afternoon => (14 * 60, 17 * 60),
            Self::Evening => (17 * 60, 19 * 60),
            Self::Night => (19 * 60, 24 * 60),
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::EarlyMorning => "Early morning",
            Self::Morning => "Morning",
            Self::Midday => "Midday",
            Self::Afternoon => "Afternoon",
            Self::Evening => "Evening",
            Self::Night => "Night",
        }
    }
    pub fn detail(self) -> &'static str {
        match self {
            Self::EarlyMorning => "Before 9 AM",
            Self::Morning => "9 AM – noon",
            Self::Midday => "Noon – 2 PM",
            Self::Afternoon => "2 – 5 PM",
            Self::Evening => "5 – 7 PM",
            Self::Night => "After 7 PM",
        }
    }
    pub fn overlap(self, start: i32, end: i32) -> i32 {
        let (lo, hi) = self.minutes();
        (end.min(hi) - start.max(lo)).max(0)
    }
    fn key(self) -> &'static str {
        match self {
            Self::EarlyMorning => "earlyMorning",
            Self::Morning => "morning",
            Self::Midday => "midday",
            Self::Afternoon => "afternoon",
            Self::Evening => "evening",
            Self::Night => "night",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.key() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimePreference {
    Avoid,
    Okay,
    Prefer,
}

impl TimePreference {
    fn key(self) -> &'static str {
        match self {
            Self::Avoid => "avoid",
            Self::Okay => "okay",
            Self::Prefer => "prefer",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "avoid" => Some(Self::Avoid),
            "okay" => Some(Self::Okay),
            "prefer" => Some(Self::Prefer),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeLimit {
    pub minutes: i32,
    #[serde(default = "required_strength")]
    pub strength: PreferenceStrength,
}

fn required_strength() -> PreferenceStrength {
    PreferenceStrength::Required
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DayLayout {
    NoPreference,
    FewerDays,
    SpreadOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GapPreference {
    Compact,
    NoPreference,
    Fine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BreakPreference {
    Fine,
    AvoidBackToBack,
    AtLeast,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LunchBreak {
    #[serde(default = "lunch_start")]
    pub start: i32,
    #[serde(default = "lunch_end")]
    pub end: i32,
    #[serde(default = "lunch_minutes")]
    pub minutes: i32,
}

fn lunch_start() -> i32 {
    11 * 60 + 30
}
fn lunch_end() -> i32 {
    14 * 60
}
fn lunch_minutes() -> i32 {
    30
}

impl Default for LunchBreak {
    fn default() -> Self {
        Self {
            start: lunch_start(),
            end: lunch_end(),
            minutes: lunch_minutes(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedTime {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub days: DaySet,
    #[serde(default)]
    pub start: i32,
    #[serde(default)]
    pub end: i32,
    #[serde(default = "required_strength")]
    pub strength: PreferenceStrength,
    #[serde(default = "yes")]
    pub is_enabled: bool,
}

fn yes() -> bool {
    true
}

impl BlockedTime {
    pub fn new(label: impl Into<String>, days: DaySet, start: i32, end: i32) -> Self {
        Self {
            id: Uuid::new_v4(),
            label: label.into(),
            days,
            start,
            end,
            strength: PreferenceStrength::Required,
            is_enabled: true,
        }
    }
    pub fn is_valid(&self) -> bool {
        !self.days.is_empty() && self.end > self.start && self.start >= 0 && self.end <= 24 * 60
    }
    pub fn is_active(&self) -> bool {
        self.is_enabled && self.is_valid()
    }
    pub fn overlaps(&self, days: DaySet, start: i32, end: i32) -> bool {
        !self.days.is_disjoint(days) && start < self.end && self.start < end
    }
    pub fn summary(&self) -> String {
        format!(
            "{} {}",
            self.days.short_label(),
            range_label(self.start, self.end)
        )
    }
    pub fn list_detail(&self) -> String {
        if !self.is_valid() {
            return if self.days.is_empty() {
                "Choose the days".into()
            } else {
                "Check the times".into()
            };
        }
        let mut text = self.summary();
        if !self.is_enabled {
            text.push_str(" · Off");
        } else if self.strength == PreferenceStrength::Preferred {
            text.push_str(" · If I must");
        }
        text
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormatPreference {
    NoPreference,
    InPerson,
    Hybrid,
    Online,
}

impl FormatPreference {
    pub fn title(self) -> &'static str {
        match self {
            Self::NoPreference => "No preference",
            Self::InPerson => "In person",
            Self::Hybrid => "Hybrid",
            Self::Online => "Online",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionFormat {
    InPerson,
    Hybrid,
    Online,
    Other,
}

impl SectionFormat {
    pub fn mismatch(self, preference: FormatPreference) -> f64 {
        use FormatPreference as P;
        use SectionFormat as S;
        match (preference, self) {
            (P::NoPreference, _) | (_, S::Other) => 0.0,
            (P::InPerson, S::InPerson) | (P::Online, S::Online) | (P::Hybrid, S::Hybrid) => 0.0,
            (P::InPerson, S::Hybrid)
            | (P::Online, S::Hybrid)
            | (P::Hybrid, S::InPerson)
            | (P::Hybrid, S::Online) => 0.5,
            (P::InPerson, S::Online) | (P::Online, S::InPerson) => 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SeatPolicy {
    Any,
    #[serde(rename = "waitlistOK")]
    WaitlistOk,
    OpenOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditTarget {
    pub lower: i32,
    pub upper: i32,
}

impl CreditTarget {
    pub fn new(lower: i32, upper: i32) -> Self {
        Self {
            lower,
            upper: upper.max(lower),
        }
    }
    pub fn label(&self) -> String {
        if self.lower == self.upper {
            format!("{} credits", self.lower)
        } else {
            format!("{}–{} credits", self.lower, self.upper)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SchedulePreferences {
    pub time_of_day: BTreeMap<TimeBand, TimePreference>,
    pub earliest_start: Option<TimeLimit>,
    pub latest_end: Option<TimeLimit>,
    pub days_off: DaySet,
    pub days_off_strength: PreferenceStrength,
    pub day_layout: DayLayout,
    pub max_classes_per_day: Option<i32>,
    pub max_minutes_per_day: Option<i32>,
    pub gaps: GapPreference,
    pub breaks: BreakPreference,
    pub minimum_break_minutes: i32,
    pub minimum_break_strength: PreferenceStrength,
    pub lunch: Option<LunchBreak>,
    pub blocked_times: Vec<BlockedTime>,
    pub format: FormatPreference,
    pub format_strength: PreferenceStrength,
    pub campus: Option<String>,
    pub campus_strength: PreferenceStrength,
    pub seats: SeatPolicy,
    pub preferred_instructors: Vec<String>,
    pub avoided_instructors: Vec<String>,
    pub credit_target: Option<CreditTarget>,
}

impl Default for SchedulePreferences {
    fn default() -> Self {
        Self {
            time_of_day: BTreeMap::new(),
            earliest_start: None,
            latest_end: None,
            days_off: DaySet(0),
            days_off_strength: PreferenceStrength::Preferred,
            day_layout: DayLayout::NoPreference,
            max_classes_per_day: None,
            max_minutes_per_day: None,
            gaps: GapPreference::NoPreference,
            breaks: BreakPreference::Fine,
            minimum_break_minutes: 15,
            minimum_break_strength: PreferenceStrength::Preferred,
            lunch: None,
            blocked_times: vec![],
            format: FormatPreference::NoPreference,
            format_strength: PreferenceStrength::Preferred,
            campus: None,
            campus_strength: PreferenceStrength::Preferred,
            seats: SeatPolicy::Any,
            preferred_instructors: vec![],
            avoided_instructors: vec![],
            credit_target: None,
        }
    }
}

pub const BACK_TO_BACK_MINUTES: i32 = 30;

impl SchedulePreferences {
    pub fn band(&self, band: TimeBand) -> TimePreference {
        self.time_of_day
            .get(&band)
            .copied()
            .unwrap_or(TimePreference::Okay)
    }
    pub fn set_band(&mut self, band: TimeBand, pref: TimePreference) {
        if pref == TimePreference::Okay {
            self.time_of_day.remove(&band);
        } else {
            self.time_of_day.insert(band, pref);
        }
    }
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn preferred_bands(&self) -> Vec<TimeBand> {
        TimeBand::ALL
            .into_iter()
            .filter(|b| self.band(*b) == TimePreference::Prefer)
            .collect()
    }
    pub fn avoided_bands(&self) -> Vec<TimeBand> {
        TimeBand::ALL
            .into_iter()
            .filter(|b| self.band(*b) == TimePreference::Avoid)
            .collect()
    }
    pub fn has_time_of_day(&self) -> bool {
        !self.time_of_day.is_empty()
    }
    pub fn active_blocked_times(&self) -> Vec<&BlockedTime> {
        self.blocked_times
            .iter()
            .filter(|b| b.is_active())
            .collect()
    }
    pub fn active_count(&self) -> i32 {
        let mut n = 0;
        if self.has_time_of_day() {
            n += 1;
        }
        if self.earliest_start.is_some() {
            n += 1;
        }
        if self.latest_end.is_some() {
            n += 1;
        }
        if !self.days_off.is_empty() {
            n += 1;
        }
        if self.day_layout != DayLayout::NoPreference {
            n += 1;
        }
        if self.max_classes_per_day.is_some() || self.max_minutes_per_day.is_some() {
            n += 1;
        }
        if self.gaps != GapPreference::NoPreference {
            n += 1;
        }
        if self.breaks != BreakPreference::Fine {
            n += 1;
        }
        if self.lunch.is_some() {
            n += 1;
        }
        n += self.active_blocked_times().len() as i32;
        if self.format != FormatPreference::NoPreference {
            n += 1;
        }
        if self.campus.is_some() {
            n += 1;
        }
        if self.seats != SeatPolicy::Any {
            n += 1;
        }
        if !self.preferred_instructors.is_empty() || !self.avoided_instructors.is_empty() {
            n += 1;
        }
        if self.credit_target.is_some() {
            n += 1;
        }
        n
    }
    pub fn has_wishes(&self) -> bool {
        self.active_count() > 0
    }
    pub fn format_of(section: &super::section::CourseSection) -> SectionFormat {
        if section.is_online() {
            return SectionFormat::Online;
        }
        let method = section
            .instruction_method
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase();
        if method.contains("hybrid") {
            return SectionFormat::Hybrid;
        }
        if method.contains("live cast") || method.contains("video") || method.contains("remote") {
            return SectionFormat::Online;
        }
        if method.contains("traditional") {
            return SectionFormat::InPerson;
        }
        SectionFormat::Other
    }
    pub fn normalized(&self) -> Self {
        let mut p = self.clone();
        if let Some(limit) = &mut p.earliest_start {
            limit.minutes = limit.minutes.clamp(0, 23 * 60 + 45);
        }
        if let Some(limit) = &mut p.latest_end {
            limit.minutes = limit.minutes.clamp(30, 24 * 60);
        }
        p.max_classes_per_day = p.max_classes_per_day.filter(|n| (1..=8).contains(n));
        p.max_minutes_per_day = p.max_minutes_per_day.filter(|n| (60..=12 * 60).contains(n));
        p.minimum_break_minutes = p.minimum_break_minutes.clamp(5, 90);
        if let Some(lunch) = &mut p.lunch {
            lunch.start = lunch.start.clamp(9 * 60, 15 * 60);
            lunch.end = lunch.end.clamp(lunch.start + 30, 17 * 60);
            lunch.minutes = lunch.minutes.clamp(15, lunch.end - lunch.start);
        }
        let mut seen = std::collections::HashSet::new();
        p.blocked_times = p
            .blocked_times
            .iter()
            .filter_map(|block| {
                let mut block = block.clone();
                block.label = block.label.trim().to_string();
                if block.label.is_empty() {
                    block.label = "Blocked time".into();
                }
                block.start = block.start.clamp(0, 24 * 60);
                block.end = block.end.clamp(0, 24 * 60);
                if !block.is_valid() {
                    return None;
                }
                let key = format!(
                    "{}|{}|{}|{}",
                    block.label.to_ascii_lowercase(),
                    block.days.0,
                    block.start,
                    block.end
                );
                if seen.insert(key) {
                    Some(block)
                } else {
                    None
                }
            })
            .take(20)
            .collect();
        if let Some(campus) = p.campus.take() {
            let campus = campus.trim().to_string();
            p.campus = if campus.is_empty() {
                None
            } else {
                Some(campus)
            };
        }
        fn cleaned(names: &[String]) -> Vec<String> {
            let mut seen = std::collections::HashSet::new();
            names
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && seen.insert(s.to_ascii_lowercase()))
                .take(30)
                .collect()
        }
        p.preferred_instructors = cleaned(&p.preferred_instructors);
        let preferred: std::collections::HashSet<String> = p
            .preferred_instructors
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect();
        p.avoided_instructors = cleaned(&p.avoided_instructors)
            .into_iter()
            .filter(|s| !preferred.contains(&s.to_ascii_lowercase()))
            .collect();
        if let Some(credits) = &mut p.credit_target {
            credits.lower = credits.lower.clamp(1, 30);
            credits.upper = credits.upper.clamp(credits.lower, 30);
        }
        p
    }

    pub fn summary_parts(&self) -> Vec<String> {
        let mut parts = vec![];
        if let Some(phrase) = time_of_day_phrase(&self.preferred_bands(), &self.avoided_bands()) {
            parts.push(phrase);
        }
        if let Some(limit) = &self.earliest_start {
            parts.push(if limit.strength == PreferenceStrength::Required {
                format!("Nothing before {}", short_time(limit.minutes))
            } else {
                format!("Prefer after {}", short_time(limit.minutes))
            });
        }
        if let Some(limit) = &self.latest_end {
            parts.push(if limit.strength == PreferenceStrength::Required {
                format!("Done by {}", short_time(limit.minutes))
            } else {
                format!("Prefer done by {}", short_time(limit.minutes))
            });
        }
        if !self.days_off.is_empty() {
            parts.push(days_off_phrase(self.days_off));
        }
        match self.day_layout {
            DayLayout::NoPreference => {}
            DayLayout::FewerDays => parts.push("Fewer days".into()),
            DayLayout::SpreadOut => parts.push("Spread out".into()),
        }
        let blocked = self.active_blocked_times().len();
        if blocked > 0 {
            parts.push(if blocked == 1 {
                "1 blocked time".into()
            } else {
                format!("{blocked} blocked times")
            });
        }
        match self.gaps {
            GapPreference::NoPreference => {}
            GapPreference::Compact => parts.push("Short gaps".into()),
            GapPreference::Fine => parts.push("Gaps are fine".into()),
        }
        match self.breaks {
            BreakPreference::Fine => {}
            BreakPreference::AvoidBackToBack => parts.push("No back-to-back".into()),
            BreakPreference::AtLeast => {
                parts.push(format!("{} min to walk", self.minimum_break_minutes))
            }
        }
        if self.lunch.is_some() {
            parts.push("Lunch break".into());
        }
        if let Some(n) = self.max_classes_per_day {
            parts.push(format!("Up to {n} a day"));
        }
        if let Some(n) = self.max_minutes_per_day {
            parts.push(format!("Up to {} a day", hours_label(n)));
        }
        if self.format != FormatPreference::NoPreference {
            parts.push(self.format.title().into());
        }
        if let Some(c) = &self.campus {
            parts.push(c.clone());
        }
        match self.seats {
            SeatPolicy::Any => {}
            SeatPolicy::WaitlistOk => parts.push("Waitlists OK".into()),
            SeatPolicy::OpenOnly => parts.push("Open seats".into()),
        }
        let instructors = self.preferred_instructors.len() + self.avoided_instructors.len();
        if instructors > 0 {
            parts.push(if instructors == 1 {
                "1 instructor".into()
            } else {
                format!("{instructors} instructors")
            });
        }
        if let Some(t) = &self.credit_target {
            parts.push(t.label());
        }
        parts
    }
    pub fn summary(&self, limit: usize) -> String {
        let parts = self.summary_parts();
        if parts.is_empty() {
            return "No preferences yet".into();
        }
        if parts.len() <= limit {
            return parts.join(" · ");
        }
        let mut shown: Vec<String> = parts.iter().take(limit).cloned().collect();
        shown.push(format!("+{}", parts.len() - limit));
        shown.join(" · ")
    }
    pub fn changed_fields(&self, old: &SchedulePreferences) -> Vec<PreferenceField> {
        let mut changed = vec![];
        for band in TimeBand::ALL {
            if self.band(band) != old.band(band) {
                changed.push(PreferenceField::TimeOfDay(band));
            }
        }
        if self.earliest_start != old.earliest_start {
            changed.push(PreferenceField::EarliestStart);
        }
        if self.latest_end != old.latest_end {
            changed.push(PreferenceField::LatestEnd);
        }
        if self.days_off != old.days_off
            || (!self.days_off.is_empty() && self.days_off_strength != old.days_off_strength)
        {
            changed.push(PreferenceField::DaysOff);
        }
        if self.day_layout != old.day_layout {
            changed.push(PreferenceField::DayLayout);
        }
        if self.max_classes_per_day != old.max_classes_per_day
            || self.max_minutes_per_day != old.max_minutes_per_day
        {
            changed.push(PreferenceField::Workload);
        }
        if self.gaps != old.gaps {
            changed.push(PreferenceField::Gaps);
        }
        if self.breaks != old.breaks
            || (self.breaks == BreakPreference::AtLeast
                && (self.minimum_break_minutes != old.minimum_break_minutes
                    || self.minimum_break_strength != old.minimum_break_strength))
        {
            changed.push(PreferenceField::Breaks);
        }
        if self.lunch != old.lunch {
            changed.push(PreferenceField::Lunch);
        }
        for block in &self.blocked_times {
            let prev = old.blocked_times.iter().find(|b| b.id == block.id);
            if prev != Some(block) {
                changed.push(PreferenceField::BlockedTime(block.id));
            }
        }
        if self.format != old.format
            || (self.format != FormatPreference::NoPreference
                && self.format_strength != old.format_strength)
        {
            changed.push(PreferenceField::Format);
        }
        if self.campus != old.campus
            || (self.campus.is_some() && self.campus_strength != old.campus_strength)
        {
            changed.push(PreferenceField::Campus);
        }
        if self.seats != old.seats {
            changed.push(PreferenceField::Seats);
        }
        if self.preferred_instructors != old.preferred_instructors
            || self.avoided_instructors != old.avoided_instructors
        {
            changed.push(PreferenceField::Instructors);
        }
        if self.credit_target != old.credit_target {
            changed.push(PreferenceField::Credits);
        }
        changed
    }
}

pub fn time_of_day_phrase(preferred: &[TimeBand], avoided: &[TimeBand]) -> Option<String> {
    if !preferred.is_empty() {
        let text = band_phrase(preferred);
        let mut chars = text.chars();
        let first = chars.next()?.to_uppercase().to_string();
        return Some(first + chars.as_str());
    }
    if !avoided.is_empty() {
        return Some(format!("No {}", band_phrase(avoided)));
    }
    None
}

pub fn band_phrase(bands: &[TimeBand]) -> String {
    let set: std::collections::HashSet<TimeBand> = bands.iter().copied().collect();
    let mut items = vec![];
    if set.contains(&TimeBand::Morning) {
        items.push("mornings");
    } else if set.contains(&TimeBand::EarlyMorning) {
        items.push("early mornings");
    }
    if set.contains(&TimeBand::Midday) {
        items.push("midday");
    }
    if set.contains(&TimeBand::Afternoon) {
        items.push("afternoons");
    }
    if set.contains(&TimeBand::Evening) {
        items.push("evenings");
    } else if set.contains(&TimeBand::Night) {
        items.push("nights");
    }
    match items.len() {
        0 => String::new(),
        1 => items[0].into(),
        _ => format!(
            "{} & {}",
            items[..items.len() - 1].join(", "),
            items[items.len() - 1]
        ),
    }
}

pub fn days_off_phrase(days: DaySet) -> String {
    let indices = days.indices();
    match indices.len() {
        0 => String::new(),
        1 => format!("{}s off", DaySet::FULL[indices[0] as usize]),
        _ => {
            let names: Vec<&str> = indices.iter().map(|i| DaySet::SHORT[*i as usize]).collect();
            format!(
                "{} & {} off",
                names[..names.len() - 1].join(", "),
                names[names.len() - 1]
            )
        }
    }
}

pub fn short_time(minutes: i32) -> String {
    super::section::label_minutes(minutes, true).replace(":00", "")
}

pub fn hours_label(minutes: i32) -> String {
    let hours = minutes as f64 / 60.0;
    if (hours - hours.round()).abs() < 1e-9 {
        format!("{} h", hours.round() as i32)
    } else {
        format!("{hours:.1} h")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreferenceField {
    TimeOfDay(TimeBand),
    EarliestStart,
    LatestEnd,
    DaysOff,
    DayLayout,
    Workload,
    Gaps,
    Breaks,
    Lunch,
    BlockedTime(Uuid),
    Format,
    Campus,
    Seats,
    Instructors,
    Credits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulePreset {
    MorningPerson,
    NightOwl,
    LongWeekends,
    CompactDays,
}

impl SchedulePreset {
    pub fn title(self) -> &'static str {
        match self {
            Self::MorningPerson => "Morning person",
            Self::NightOwl => "Night owl",
            Self::LongWeekends => "Long weekends",
            Self::CompactDays => "Compact days",
        }
    }
    pub fn is_applied(self, p: &SchedulePreferences) -> bool {
        match self {
            Self::MorningPerson | Self::NightOwl => p.time_of_day == self.bands(),
            Self::LongWeekends => {
                p.days_off.contains(DaySet::FRIDAY) && p.day_layout == DayLayout::FewerDays
            }
            Self::CompactDays => {
                p.day_layout == DayLayout::FewerDays && p.gaps == GapPreference::Compact
            }
        }
    }
    fn bands(self) -> BTreeMap<TimeBand, TimePreference> {
        let mut m = BTreeMap::new();
        match self {
            Self::MorningPerson => {
                m.insert(TimeBand::EarlyMorning, TimePreference::Prefer);
                m.insert(TimeBand::Morning, TimePreference::Prefer);
                m.insert(TimeBand::Evening, TimePreference::Avoid);
                m.insert(TimeBand::Night, TimePreference::Avoid);
            }
            Self::NightOwl => {
                m.insert(TimeBand::EarlyMorning, TimePreference::Avoid);
                m.insert(TimeBand::Morning, TimePreference::Avoid);
                m.insert(TimeBand::Afternoon, TimePreference::Prefer);
                m.insert(TimeBand::Evening, TimePreference::Prefer);
                m.insert(TimeBand::Night, TimePreference::Prefer);
            }
            _ => {}
        }
        m
    }
    pub fn apply(self, p: &mut SchedulePreferences) {
        match self {
            Self::MorningPerson | Self::NightOwl => p.time_of_day = self.bands(),
            Self::LongWeekends => {
                p.days_off.insert(DaySet::FRIDAY);
                p.day_layout = DayLayout::FewerDays;
            }
            Self::CompactDays => {
                p.day_layout = DayLayout::FewerDays;
                p.gaps = GapPreference::Compact;
            }
        }
    }
    pub fn clear(self, p: &mut SchedulePreferences) {
        match self {
            Self::MorningPerson | Self::NightOwl => p.time_of_day.clear(),
            Self::LongWeekends => {
                p.days_off.remove(DaySet::FRIDAY);
                if p.gaps != GapPreference::Compact {
                    p.day_layout = DayLayout::NoPreference;
                }
            }
            Self::CompactDays => {
                p.gaps = GapPreference::NoPreference;
                if !p.days_off.contains(DaySet::FRIDAY) {
                    p.day_layout = DayLayout::NoPreference;
                }
            }
        }
    }
    pub fn toggled(self, p: &SchedulePreferences) -> SchedulePreferences {
        let mut copy = p.clone();
        if self.is_applied(p) {
            self.clear(&mut copy);
        } else {
            self.apply(&mut copy);
        }
        copy
    }
}

impl Serialize for SchedulePreferences {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        if !self.time_of_day.is_empty() {
            let raw: BTreeMap<&str, &str> = self
                .time_of_day
                .iter()
                .map(|(b, p)| (b.key(), p.key()))
                .collect();
            map.serialize_entry("timeOfDay", &raw)?;
        }
        if let Some(v) = &self.earliest_start {
            map.serialize_entry("earliestStart", v)?;
        }
        if let Some(v) = &self.latest_end {
            map.serialize_entry("latestEnd", v)?;
        }
        map.serialize_entry("daysOff", &self.days_off)?;
        map.serialize_entry("daysOffStrength", &self.days_off_strength)?;
        map.serialize_entry("dayLayout", &self.day_layout)?;
        if let Some(v) = self.max_classes_per_day {
            map.serialize_entry("maxClassesPerDay", &v)?;
        }
        if let Some(v) = self.max_minutes_per_day {
            map.serialize_entry("maxMinutesPerDay", &v)?;
        }
        map.serialize_entry("gaps", &self.gaps)?;
        map.serialize_entry("breaks", &self.breaks)?;
        map.serialize_entry("minimumBreakMinutes", &self.minimum_break_minutes)?;
        map.serialize_entry("minimumBreakStrength", &self.minimum_break_strength)?;
        if let Some(v) = &self.lunch {
            map.serialize_entry("lunch", v)?;
        }
        map.serialize_entry("blockedTimes", &self.blocked_times)?;
        map.serialize_entry("format", &self.format)?;
        map.serialize_entry("formatStrength", &self.format_strength)?;
        if let Some(v) = &self.campus {
            map.serialize_entry("campus", v)?;
        }
        map.serialize_entry("campusStrength", &self.campus_strength)?;
        map.serialize_entry("seats", &self.seats)?;
        map.serialize_entry("preferredInstructors", &self.preferred_instructors)?;
        map.serialize_entry("avoidedInstructors", &self.avoided_instructors)?;
        if let Some(v) = &self.credit_target {
            map.serialize_entry("creditTarget", v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for SchedulePreferences {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(Self::from_value(&value))
    }
}

impl SchedulePreferences {
    pub fn from_value(value: &Value) -> Self {
        let mut p = Self::default();
        let Some(obj) = value.as_object() else {
            return p;
        };
        if let Some(map) = obj.get("timeOfDay").and_then(|v| v.as_object()) {
            for (band, pref) in map {
                if let (Some(b), Some(pr)) = (
                    TimeBand::parse(band),
                    pref.as_str().and_then(TimePreference::parse),
                ) {
                    p.set_band(b, pr);
                }
            }
        }
        p.earliest_start = lenient(obj.get("earliestStart"));
        p.latest_end = lenient(obj.get("latestEnd"));
        if let Some(d) = lenient::<DaySet>(obj.get("daysOff")) {
            p.days_off = d;
        }
        if let Some(s) = lenient(obj.get("daysOffStrength")) {
            p.days_off_strength = s;
        }
        if let Some(s) = lenient(obj.get("dayLayout")) {
            p.day_layout = s;
        }
        p.max_classes_per_day = lenient(obj.get("maxClassesPerDay"));
        p.max_minutes_per_day = lenient(obj.get("maxMinutesPerDay"));
        if let Some(s) = lenient(obj.get("gaps")) {
            p.gaps = s;
        }
        if let Some(s) = lenient(obj.get("breaks")) {
            p.breaks = s;
        }
        if let Some(n) = lenient(obj.get("minimumBreakMinutes")) {
            p.minimum_break_minutes = n;
        }
        if let Some(s) = lenient(obj.get("minimumBreakStrength")) {
            p.minimum_break_strength = s;
        }
        p.lunch = lenient(obj.get("lunch"));
        if let Some(arr) = obj.get("blockedTimes").and_then(|v| v.as_array()) {
            p.blocked_times = arr
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
        }
        if let Some(s) = lenient(obj.get("format")) {
            p.format = s;
        }
        if let Some(s) = lenient(obj.get("formatStrength")) {
            p.format_strength = s;
        }
        p.campus = lenient(obj.get("campus"));
        if let Some(s) = lenient(obj.get("campusStrength")) {
            p.campus_strength = s;
        }
        if let Some(s) = lenient(obj.get("seats")) {
            p.seats = s;
        }
        if let Some(v) = lenient(obj.get("preferredInstructors")) {
            p.preferred_instructors = v;
        }
        if let Some(v) = lenient(obj.get("avoidedInstructors")) {
            p.avoided_instructors = v;
        }
        p.credit_target = lenient(obj.get("creditTarget"));
        p
    }
}

fn lenient<T: for<'de> Deserialize<'de>>(value: Option<&Value>) -> Option<T> {
    serde_json::from_value(value?.clone()).ok()
}
