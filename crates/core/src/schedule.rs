//! Conflict-free schedule search, scoring, export, and empty-result explanations.
//!
//! Port of `ScheduleGenerator.swift`, `ScheduleScoring.swift`, `SchedulePreferenceRules.swift`,
//! and the calendar half of `ScheduleExporter.swift`.

use crate::model::*;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub const MINUTES_PER_SLOT: i32 = 5;
pub const SLOTS_PER_DAY: i32 = 24 * 60 / 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct ScheduleSlotMask {
    words: [u64; 32],
}

impl ScheduleSlotMask {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn from_section(section: &CourseSection) -> Self {
        Self::from_meetings(&section.weekly_meetings().into_iter().cloned().collect::<Vec<_>>())
    }
    pub fn from_meetings(meetings: &[Meeting]) -> Self {
        let mut mask = Self::new();
        for meeting in meetings {
            mask.insert(meeting);
        }
        mask
    }
    pub fn insert(&mut self, meeting: &Meeting) {
        let (Some(start), Some(end)) = (meeting.start, meeting.end) else { return };
        for day in meeting.days.indices() {
            self.insert_span(day, start, end);
        }
    }
    pub fn insert_span(&mut self, day: i32, start: i32, end: i32) {
        if !(0..7).contains(&day) || end <= start {
            return;
        }
        let first = (start / MINUTES_PER_SLOT).max(0);
        let last = (SLOTS_PER_DAY).min(((end + MINUTES_PER_SLOT - 1) / MINUTES_PER_SLOT).max(first + 1));
        if first >= last {
            return;
        }
        for slot in first..last {
            let bit = day * SLOTS_PER_DAY + slot;
            let word = (bit >> 6) as usize;
            if word < 32 {
                self.words[word] |= 1u64 << (bit & 63);
            }
        }
    }
    pub fn is_empty(self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }
    pub fn conflicts(self, other: Self) -> bool {
        self.words.iter().zip(other.words).any(|(a, b)| a & b != 0)
    }
    pub fn union(self, other: Self) -> Self {
        let mut copy = self;
        for (a, b) in copy.words.iter_mut().zip(other.words) {
            *a |= b;
        }
        copy
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScheduleSort {
    Best,
    FewestDays,
    LatestStart,
    EarliestFinish,
    FewestGaps,
    MostSeats,
}

impl ScheduleSort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Best => "Best overall",
            Self::FewestDays => "Fewest days on campus",
            Self::LatestStart => "Latest start",
            Self::EarliestFinish => "Earliest finish",
            Self::FewestGaps => "Fewest gaps",
            Self::MostSeats => "Most open seats",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterReason {
    ExcludedByUser = 0,
    StartsTooEarly,
    EndsTooLate,
    FreeDay,
    Honors,
    Online,
    Seats,
    Campus,
    BlockedTime,
    PrefStartsTooEarly,
    PrefEndsTooLate,
    PrefDayOff,
    PrefFormat,
    PrefSeats,
    PrefCampus,
    PrefWorkload,
}

impl FilterReason {
    pub const ALL: [FilterReason; 16] = [
        Self::ExcludedByUser, Self::StartsTooEarly, Self::EndsTooLate, Self::FreeDay, Self::Honors, Self::Online, Self::Seats, Self::Campus,
        Self::BlockedTime, Self::PrefStartsTooEarly, Self::PrefEndsTooLate, Self::PrefDayOff, Self::PrefFormat, Self::PrefSeats, Self::PrefCampus, Self::PrefWorkload,
    ];
    pub fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

#[derive(Clone, Debug)]
pub struct ScheduleCourseInput {
    pub key: CourseKey,
    pub is_required: bool,
    pub sections: Vec<CourseSection>,
    pub excluded_crns: HashSet<String>,
    pub min_credits: i32,
    pub max_credits: i32,
}

impl ScheduleCourseInput {
    pub fn new(key: CourseKey, required: bool, sections: Vec<CourseSection>) -> Self {
        let credits = sections.first().map(|s| s.course_key()).map(|_| 0);
        let _ = credits;
        Self { key, is_required: required, sections, excluded_crns: HashSet::new(), min_credits: 0, max_credits: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct ScheduleReason {
    pub benefit: bool,
    pub text: String,
    pub symbol: String,
}

impl ScheduleReason {
    pub fn id(&self) -> String {
        format!("{}{}", if self.benefit { "+" } else { "-" }, self.text)
    }
}

#[derive(Clone, Debug)]
pub struct GeneratedSchedule {
    pub crns: Vec<String>,
    pub sections: Vec<CourseSection>,
    pub days_on_campus: i32,
    pub class_days: i32,
    pub earliest_start: Option<i32>,
    pub latest_end: Option<i32>,
    pub total_gap_minutes: i32,
    pub min_seats: i32,
    pub total_seats: i32,
    pub full_sections: i32,
    pub score: f64,
    pub reasons: Vec<ScheduleReason>,
    pub sorted_crns: Vec<String>,
    pub id: String,
}

impl GeneratedSchedule {
    pub fn from_sections(sections: &[CourseSection], preferences: &SchedulePreferences) -> Self {
        let mut buildings = BuildingTable::default();
        let mut intervals = vec![];
        let mut min_seats = i32::MAX;
        let mut total_seats = 0;
        let mut full = 0;
        let mut points = 0.0;
        for section in sections {
            let own = intervals_for(section, &mut buildings);
            intervals.extend(own.iter().copied());
            if !preferences.is_default() {
                points += section_score(section, &own, preferences);
            }
            let seats = section.seats_remaining.max(0);
            min_seats = min_seats.min(seats);
            total_seats += seats;
            if seats == 0 {
                full += 1;
            }
        }
        let options = measure_options(preferences);
        let raw = measure(&intervals, &options);
        let weights = Weights::from_preferences(preferences);
        let seats_floor = if min_seats == i32::MAX { 0 } else { min_seats };
        Self::assemble(
            sections.to_vec(),
            raw.campus_days,
            raw.class_days,
            if raw.earliest == i32::MAX { None } else { Some(raw.earliest) },
            if raw.class_days == 0 { None } else { Some(raw.latest) },
            raw.gap_minutes,
            seats_floor,
            total_seats,
            full,
            weights.score(&raw, full, seats_floor) + points,
            if preferences.is_default() { vec![] } else { reasons_for(sections, preferences) },
        )
    }
    fn assemble(sections: Vec<CourseSection>, days_on_campus: i32, class_days: i32, earliest_start: Option<i32>, latest_end: Option<i32>, total_gap_minutes: i32, min_seats: i32, total_seats: i32, full_sections: i32, score: f64, reasons: Vec<ScheduleReason>) -> Self {
        let crns: Vec<String> = sections.iter().map(|s| s.crn.clone()).collect();
        let mut sorted_crns = crns.clone();
        sorted_crns.sort();
        let id = sorted_crns.join("-");
        Self { crns, sections, days_on_campus, class_days, earliest_start, latest_end, total_gap_minutes, min_seats, total_seats, full_sections, score, reasons, sorted_crns, id }
    }
}

#[derive(Clone, Debug)]
pub struct CourseReport {
    pub key: CourseKey,
    pub is_required: bool,
    pub total_sections: usize,
    pub remaining_sections: usize,
    pub rejections: HashMap<FilterReason, i32>,
}

impl CourseReport {
    pub fn is_blocked(&self) -> bool {
        self.remaining_sections == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConflictReason {
    Overlap,
    TravelTime,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConflictPair {
    pub first: CourseKey,
    pub second: CourseKey,
    pub reason: ConflictReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountProblem {
    TooManyRequired { required: i32, wanted: i32 },
    TooFewCourses { available: i32, wanted: i32 },
    CreditsOverTarget { required: i32, limit: i32 },
    CreditsUnderTarget { available: i32, wanted: i32 },
}

#[derive(Clone, Debug, Default)]
pub struct ScheduleDiagnostics {
    pub filters: ScheduleFilters,
    pub preferences: SchedulePreferences,
    pub courses: Vec<CourseReport>,
    pub conflicting_pairs: Vec<ConflictPair>,
    pub count_problem: Option<CountProblem>,
}

impl ScheduleDiagnostics {
    pub fn has_no_courses(&self) -> bool {
        self.courses.is_empty()
    }
    pub fn blocked_required(&self) -> Vec<&CourseReport> {
        self.courses.iter().filter(|c| c.is_required && c.is_blocked()).collect()
    }
}

#[derive(Clone, Debug)]
pub struct GenerationResult {
    pub schedules: Vec<GeneratedSchedule>,
    pub total_found: usize,
    pub is_exhaustive: bool,
    pub was_cancelled: bool,
    pub diagnostics: ScheduleDiagnostics,
    pub elapsed: f64,
}

impl GenerationResult {
    pub fn is_truncated(&self) -> bool {
        !self.is_exhaustive || self.schedules.len() < self.total_found
    }
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_results: usize,
    pub max_exhaustive_leaves: usize,
    pub max_nodes: usize,
    pub time_budget: f64,
    pub sample_leaves: usize,
    pub sample_time_budget: f64,
    pub allow_sampling: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_results: 5_000, max_exhaustive_leaves: 2_500_000, max_nodes: 12_000_000, time_budget: 0.9, sample_leaves: 150_000, sample_time_budget: 0.35, allow_sampling: true }
    }
}

impl Limits {
    pub fn probe() -> Self {
        Self { max_results: 1, max_exhaustive_leaves: 300, max_nodes: 300_000, time_budget: 0.25, sample_leaves: 0, sample_time_budget: 0.0, allow_sampling: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PreferenceRelaxation {
    SoftenEarliestStart,
    SoftenLatestEnd,
    SoftenDaysOff,
    SoftenBlockedTime { id: uuid::Uuid, label: String },
    SoftenFormat,
    SoftenCampus,
    AllowFullSections,
    ClearMaxClassesPerDay,
    ClearMaxMinutesPerDay,
    SoftenMinimumBreak,
    ClearCreditTarget,
}

impl PreferenceRelaxation {
    pub fn title(&self) -> String {
        match self {
            Self::SoftenEarliestStart => "Allow earlier classes if needed".into(),
            Self::SoftenLatestEnd => "Allow later classes if needed".into(),
            Self::SoftenDaysOff => "Allow classes on your days off if needed".into(),
            Self::SoftenBlockedTime { label, .. } => format!("Allow classes during “{label}” if needed"),
            Self::SoftenFormat => "Allow other formats if needed".into(),
            Self::SoftenCampus => "Allow other campuses if needed".into(),
            Self::AllowFullSections => "Include full sections".into(),
            Self::ClearMaxClassesPerDay => "Remove the classes-a-day limit".into(),
            Self::ClearMaxMinutesPerDay => "Remove the hours-a-day limit".into(),
            Self::SoftenMinimumBreak => "Don’t require time to walk between classes".into(),
            Self::ClearCreditTarget => "Remove the credit target".into(),
        }
    }
    pub fn apply(&self, p: &mut SchedulePreferences) {
        match self {
            Self::SoftenEarliestStart => {
                if let Some(limit) = &mut p.earliest_start {
                    limit.strength = PreferenceStrength::Preferred;
                }
            }
            Self::SoftenLatestEnd => {
                if let Some(limit) = &mut p.latest_end {
                    limit.strength = PreferenceStrength::Preferred;
                }
            }
            Self::SoftenDaysOff => p.days_off_strength = PreferenceStrength::Preferred,
            Self::SoftenBlockedTime { id, .. } => {
                if let Some(block) = p.blocked_times.iter_mut().find(|b| b.id == *id) {
                    block.strength = PreferenceStrength::Preferred;
                }
            }
            Self::SoftenFormat => p.format_strength = PreferenceStrength::Preferred,
            Self::SoftenCampus => p.campus_strength = PreferenceStrength::Preferred,
            Self::AllowFullSections => p.seats = SeatPolicy::Any,
            Self::ClearMaxClassesPerDay => p.max_classes_per_day = None,
            Self::ClearMaxMinutesPerDay => p.max_minutes_per_day = None,
            Self::SoftenMinimumBreak => p.minimum_break_strength = PreferenceStrength::Preferred,
            Self::ClearCreditTarget => p.credit_target = None,
        }
    }
    pub fn available(p: &SchedulePreferences) -> Vec<Self> {
        let mut result = vec![];
        if p.earliest_start.as_ref().is_some_and(|l| l.strength == PreferenceStrength::Required) {
            result.push(Self::SoftenEarliestStart);
        }
        if p.latest_end.as_ref().is_some_and(|l| l.strength == PreferenceStrength::Required) {
            result.push(Self::SoftenLatestEnd);
        }
        if !p.days_off.is_empty() && p.days_off_strength == PreferenceStrength::Required {
            result.push(Self::SoftenDaysOff);
        }
        for block in p.active_blocked_times() {
            if block.strength == PreferenceStrength::Required {
                result.push(Self::SoftenBlockedTime { id: block.id, label: block.label.clone() });
            }
        }
        if p.format != FormatPreference::NoPreference && p.format_strength == PreferenceStrength::Required {
            result.push(Self::SoftenFormat);
        }
        if p.campus.is_some() && p.campus_strength == PreferenceStrength::Required {
            result.push(Self::SoftenCampus);
        }
        if p.seats != SeatPolicy::Any {
            result.push(Self::AllowFullSections);
        }
        if p.max_classes_per_day.is_some() {
            result.push(Self::ClearMaxClassesPerDay);
        }
        if p.max_minutes_per_day.is_some() {
            result.push(Self::ClearMaxMinutesPerDay);
        }
        if p.breaks == BreakPreference::AtLeast && p.minimum_break_strength == PreferenceStrength::Required {
            result.push(Self::SoftenMinimumBreak);
        }
        if p.credit_target.is_some() {
            result.push(Self::ClearCreditTarget);
        }
        result
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Relaxation {
    ClearEarliestStart,
    ClearLatestEnd,
    ClearFreeDays,
    AllowHonors,
    AllowOnline,
    AllowFullSections,
    ClearCampus,
    ClearCourseCount,
    MakeOptional(CourseKey),
    Preference(PreferenceRelaxation),
}

impl Relaxation {
    pub fn title(&self) -> String {
        match self {
            Self::ClearEarliestStart => "Remove start time limit".into(),
            Self::ClearLatestEnd => "Remove end time limit".into(),
            Self::ClearFreeDays => "Allow classes on every day".into(),
            Self::AllowHonors => "Include honors sections".into(),
            Self::AllowOnline => "Include online sections".into(),
            Self::AllowFullSections => "Include full sections".into(),
            Self::ClearCampus => "Allow any campus".into(),
            Self::ClearCourseCount => "Use any number of courses".into(),
            Self::MakeOptional(key) => format!("Make {key} optional"),
            Self::Preference(p) => p.title(),
        }
    }
    pub fn apply_filters(&self, filters: &mut ScheduleFilters) {
        match self {
            Self::ClearEarliestStart => filters.earliest_start = None,
            Self::ClearLatestEnd => filters.latest_end = None,
            Self::ClearFreeDays => filters.free_days = DaySet(0),
            Self::AllowHonors => filters.include_honors = true,
            Self::AllowOnline => filters.include_online = true,
            Self::AllowFullSections => {
                filters.hide_full_sections = false;
                filters.min_seats = 0;
            }
            Self::ClearCampus => filters.campus = None,
            Self::ClearCourseCount => filters.course_count = None,
            Self::MakeOptional(_) | Self::Preference(_) => {}
        }
    }
    pub fn apply_preferences(&self, preferences: &mut SchedulePreferences) {
        if let Self::Preference(relaxation) = self {
            relaxation.apply(preferences);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Suggestion {
    pub relaxation: Relaxation,
    pub schedule_count: usize,
    pub is_lower_bound: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Interval {
    day: u8,
    in_person: bool,
    start: i32,
    end: i32,
    building: u16,
}

#[derive(Default)]
struct BuildingTable {
    ids: HashMap<String, u16>,
}

impl BuildingTable {
    fn id_for(&mut self, meeting: &Meeting) -> u16 {
        let Some(code) = meeting.building_code.as_ref().or(meeting.building.as_ref()) else { return 0 };
        if code.is_empty() {
            return 0;
        }
        if let Some(known) = self.ids.get(code) {
            return *known;
        }
        let next = (self.ids.len() as u16).saturating_add(1);
        self.ids.insert(code.clone(), next);
        next
    }
}

fn intervals_for(section: &CourseSection, buildings: &mut BuildingTable) -> Vec<Interval> {
    let mut result = vec![];
    let mut seen = HashSet::new();
    for meeting in section.weekly_meetings() {
        let (Some(start), Some(end)) = (meeting.start, meeting.end) else { continue };
        let in_person = !(section.is_online() || meeting.is_online());
        let place = if in_person { buildings.id_for(meeting) } else { 0 };
        for day in meeting.days.indices() {
            let interval = Interval { day: day as u8, in_person, start, end, building: place };
            if seen.insert(interval) {
                result.push(interval);
            }
        }
    }
    result
}

#[derive(Clone, Debug, Default)]
pub struct MeasureOptions {
    pub back_to_back_below: i32,
    pub travel_minutes: i32,
    pub lunch: Option<LunchBreak>,
}

#[derive(Clone, Debug)]
pub struct Measure {
    pub class_days: i32,
    pub campus_days: i32,
    pub earliest: i32,
    pub latest: i32,
    pub gap_minutes: i32,
    pub class_day_mask: u8,
    pub max_day_minutes: i32,
    pub total_minutes: i32,
    pub back_to_back: i32,
    pub tight_transitions: i32,
    pub lunch_misses: i32,
}

impl Default for Measure {
    fn default() -> Self {
        Self { class_days: 0, campus_days: 0, earliest: i32::MAX, latest: i32::MIN, gap_minutes: 0, class_day_mask: 0, max_day_minutes: 0, total_minutes: 0, back_to_back: 0, tight_transitions: 0, lunch_misses: 0 }
    }
}

pub(crate) fn measure(intervals: &[Interval], options: &MeasureOptions) -> Measure {
    let mut items = intervals.to_vec();
    items.sort_by_key(|i| (i.day, i.start, i.end));
    let mut result = Measure::default();
    let mut k = 0;
    while k < items.len() {
        let day = items[k].day;
        let mut run_end = items[k].end;
        let mut on_campus = items[k].in_person;
        let mut day_minutes = items[k].end - items[k].start;
        let mut previous = items[k];
        result.earliest = result.earliest.min(items[k].start);
        result.latest = result.latest.max(items[k].end);
        let mut m = k + 1;
        while m < items.len() && items[m].day == day {
            let current = items[m];
            let gap = current.start - run_end;
            if gap >= 20 {
                result.gap_minutes += gap;
            }
            if gap >= 0 {
                if options.back_to_back_below > 0 && gap < options.back_to_back_below {
                    result.back_to_back += 1;
                }
                if options.travel_minutes > 0 && gap < options.travel_minutes && previous.in_person && current.in_person && (previous.building == 0 || current.building == 0 || previous.building != current.building) {
                    result.tight_transitions += 1;
                }
            }
            run_end = run_end.max(current.end);
            day_minutes += current.end - current.start;
            result.earliest = result.earliest.min(current.start);
            result.latest = result.latest.max(current.end);
            if current.in_person {
                on_campus = true;
            }
            previous = current;
            m += 1;
        }
        if let Some(lunch) = &options.lunch {
            if !has_free_stretch(&items[k..m], lunch) {
                result.lunch_misses += 1;
            }
        }
        result.class_days += 1;
        result.class_day_mask |= 1 << day;
        if on_campus {
            result.campus_days += 1;
        }
        result.max_day_minutes = result.max_day_minutes.max(day_minutes);
        result.total_minutes += day_minutes;
        k = m;
    }
    result
}

fn has_free_stretch(day: &[Interval], lunch: &LunchBreak) -> bool {
    let mut cursor = lunch.start;
    for interval in day {
        if interval.end <= lunch.start || interval.start >= lunch.end {
            continue;
        }
        if interval.start > cursor && interval.start.min(lunch.end) - cursor >= lunch.minutes {
            return true;
        }
        cursor = cursor.max(interval.end.min(lunch.end));
    }
    lunch.end - cursor >= lunch.minutes
}

#[derive(Clone, Debug)]
pub struct Weights {
    pub base: f64,
    pub per_campus_day: f64,
    pub per_online_day: f64,
    pub gap_divisor: f64,
    pub comfort_hours: bool,
    pub per_full_section: f64,
    pub low_seats_penalty: f64,
    pub per_class_day_bonus: f64,
    pub per_imbalance_minute: f64,
    pub per_day_off_missed: f64,
    pub days_off: DaySet,
    pub per_back_to_back: f64,
    pub per_tight_transition: f64,
    pub per_lunch_miss: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self { base: 100.0, per_campus_day: 8.0, per_online_day: 2.0, gap_divisor: 10.0, comfort_hours: true, per_full_section: 6.0, low_seats_penalty: 2.0, per_class_day_bonus: 0.0, per_imbalance_minute: 0.0, per_day_off_missed: 0.0, days_off: DaySet(0), per_back_to_back: 0.0, per_tight_transition: 0.0, per_lunch_miss: 0.0 }
    }
}

impl Weights {
    pub fn from_preferences(p: &SchedulePreferences) -> Self {
        let mut w = Self::default();
        if p.has_time_of_day() || p.earliest_start.is_some() || p.latest_end.is_some() {
            w.comfort_hours = false;
        }
        match p.day_layout {
            DayLayout::NoPreference => {}
            DayLayout::FewerDays => w.per_campus_day = 14.0,
            DayLayout::SpreadOut => {
                w.per_campus_day = 0.0;
                w.per_online_day = 0.0;
                w.per_class_day_bonus = 6.0;
                w.per_imbalance_minute = 0.1;
            }
        }
        match p.gaps {
            GapPreference::NoPreference => {
                if p.lunch.is_some() {
                    w.gap_divisor = 20.0;
                }
            }
            GapPreference::Compact => w.gap_divisor = 10.0 / 3.0,
            GapPreference::Fine => w.gap_divisor = f64::INFINITY,
        }
        if p.seats == SeatPolicy::WaitlistOk {
            w.per_full_section = 3.0;
        }
        if !p.days_off.is_empty() && p.days_off_strength == PreferenceStrength::Preferred {
            w.days_off = p.days_off;
            w.per_day_off_missed = 12.0;
        }
        match p.breaks {
            BreakPreference::Fine => {}
            BreakPreference::AvoidBackToBack => w.per_back_to_back = 10.0,
            BreakPreference::AtLeast => {
                if p.minimum_break_strength == PreferenceStrength::Preferred {
                    w.per_tight_transition = 10.0;
                }
            }
        }
        if p.lunch.is_some() {
            w.per_lunch_miss = 8.0;
        }
        w
    }
    pub fn score(&self, m: &Measure, full_sections: i32, min_seats: i32) -> f64 {
        let mut score = self.base;
        score -= self.per_campus_day * m.campus_days as f64;
        score -= self.per_online_day * (m.class_days - m.campus_days).max(0) as f64;
        score -= m.gap_minutes as f64 / self.gap_divisor;
        if self.comfort_hours {
            if m.earliest != i32::MAX && m.earliest < 9 * 60 {
                score -= (9 * 60 - m.earliest) as f64 / 15.0;
            }
            if m.latest != i32::MIN && m.latest > 17 * 60 {
                score -= (m.latest - 17 * 60) as f64 / 20.0;
            }
        }
        score -= self.per_full_section * full_sections as f64;
        if full_sections == 0 && min_seats <= 3 {
            score -= self.low_seats_penalty;
        }
        if self.per_class_day_bonus != 0.0 {
            score += self.per_class_day_bonus * m.class_days as f64;
        }
        if self.per_imbalance_minute != 0.0 && m.class_days > 0 {
            let average = m.total_minutes as f64 / m.class_days as f64;
            score -= self.per_imbalance_minute * (m.max_day_minutes as f64 - average).max(0.0);
        }
        if self.per_day_off_missed != 0.0 {
            score -= self.per_day_off_missed * (m.class_day_mask & self.days_off.0).count_ones() as f64;
        }
        if self.per_back_to_back != 0.0 {
            score -= self.per_back_to_back * m.back_to_back as f64;
        }
        if self.per_tight_transition != 0.0 {
            score -= self.per_tight_transition * m.tight_transitions as f64;
        }
        if self.per_lunch_miss != 0.0 {
            score -= self.per_lunch_miss * m.lunch_misses as f64;
        }
        score
    }
}

pub const PREFERRED_BAND_PER_HOUR: f64 = 3.0;
pub const AVOIDED_BAND_PER_HOUR: f64 = -6.0;
pub const SOFT_LIMIT_BASE: f64 = 4.0;
pub const SOFT_LIMIT_CAP: f64 = 20.0;
pub const SOFT_BLOCKED: f64 = 15.0;
pub const FORMAT_MISMATCH: f64 = 8.0;
pub const CAMPUS_MISMATCH: f64 = 10.0;
pub const PREFERRED_INSTRUCTOR: f64 = 5.0;
pub const AVOIDED_INSTRUCTOR: f64 = -15.0;

pub fn measure_options(p: &SchedulePreferences) -> MeasureOptions {
    let mut options = MeasureOptions::default();
    match p.breaks {
        BreakPreference::Fine => {}
        BreakPreference::AvoidBackToBack => options.back_to_back_below = BACK_TO_BACK_MINUTES,
        BreakPreference::AtLeast => {
            if p.minimum_break_strength == PreferenceStrength::Preferred {
                options.travel_minutes = p.minimum_break_minutes;
            }
        }
    }
    options.lunch = p.lunch.clone();
    options
}

pub fn preference_is_full(section: &CourseSection) -> bool {
    section.seat_capacity > 0 && section.seats_remaining <= 0
}

pub fn has_no_campus(section: &CourseSection) -> bool {
    section.campus.to_ascii_lowercase().contains("no campus")
}

pub fn rejection_reasons(section: &CourseSection, filters: &ScheduleFilters, preferences: &SchedulePreferences, excluded: &HashSet<String>) -> u32 {
    let mut reasons = 0u32;
    if excluded.contains(&section.crn) {
        reasons |= FilterReason::ExcludedByUser.bit();
    }
    let weekly = section.weekly_meetings();
    if let Some(earliest) = filters.earliest_start {
        if weekly.iter().any(|m| m.start.unwrap_or(i32::MAX) < earliest) {
            reasons |= FilterReason::StartsTooEarly.bit();
        }
    }
    if let Some(latest) = filters.latest_end {
        if weekly.iter().any(|m| m.end.unwrap_or(0) > latest) {
            reasons |= FilterReason::EndsTooLate.bit();
        }
    }
    if !filters.free_days.is_empty() && weekly.iter().any(|m| !m.days.is_disjoint(filters.free_days)) {
        reasons |= FilterReason::FreeDay.bit();
    }
    if !filters.include_honors && section.honors {
        reasons |= FilterReason::Honors.bit();
    }
    if !filters.include_online && section.is_online() {
        reasons |= FilterReason::Online.bit();
    }
    if filters.hide_full_sections && section.seats_remaining <= 0 {
        reasons |= FilterReason::Seats.bit();
    }
    if filters.min_seats > 0 && section.seats_remaining < filters.min_seats {
        reasons |= FilterReason::Seats.bit();
    }
    if let Some(campus) = &filters.campus {
        if !section.is_online() && !has_no_campus(section) && &section.campus != campus {
            reasons |= FilterReason::Campus.bit();
        }
    }
    if !preferences.is_default() {
        reasons |= preference_rejections(section, preferences);
    }
    reasons
}

fn preference_rejections(section: &CourseSection, p: &SchedulePreferences) -> u32 {
    let mut reasons = 0u32;
    let weekly = section.weekly_meetings();
    if let Some(limit) = &p.earliest_start {
        if limit.strength == PreferenceStrength::Required && weekly.iter().any(|m| m.start.unwrap_or(i32::MAX) < limit.minutes) {
            reasons |= FilterReason::PrefStartsTooEarly.bit();
        }
    }
    if let Some(limit) = &p.latest_end {
        if limit.strength == PreferenceStrength::Required && weekly.iter().any(|m| m.end.unwrap_or(0) > limit.minutes) {
            reasons |= FilterReason::PrefEndsTooLate.bit();
        }
    }
    if p.days_off_strength == PreferenceStrength::Required && !p.days_off.is_empty() && weekly.iter().any(|m| !m.days.is_disjoint(p.days_off)) {
        reasons |= FilterReason::PrefDayOff.bit();
    }
    for block in p.active_blocked_times() {
        if block.strength == PreferenceStrength::Required && weekly.iter().any(|m| m.start.zip(m.end).is_some_and(|(s, e)| block.overlaps(m.days, s, e))) {
            reasons |= FilterReason::BlockedTime.bit();
            break;
        }
    }
    if p.format != FormatPreference::NoPreference && p.format_strength == PreferenceStrength::Required && SchedulePreferences::format_of(section).mismatch(p.format) > 0.0 {
        reasons |= FilterReason::PrefFormat.bit();
    }
    if let Some(campus) = &p.campus {
        if p.campus_strength == PreferenceStrength::Required && !section.is_online() && !has_no_campus(section) && &section.campus != campus {
            reasons |= FilterReason::PrefCampus.bit();
        }
    }
    match p.seats {
        SeatPolicy::Any => {}
        SeatPolicy::OpenOnly => {
            if preference_is_full(section) {
                reasons |= FilterReason::PrefSeats.bit();
            }
        }
        SeatPolicy::WaitlistOk => {
            if preference_is_full(section) && section.waitlist_remaining <= 0 {
                reasons |= FilterReason::PrefSeats.bit();
            }
        }
    }
    if let Some(cap) = p.max_minutes_per_day {
        let mut per_day = [0i32; 7];
        for interval in intervals_for(section, &mut BuildingTable::default()) {
            per_day[interval.day as usize] += interval.end - interval.start;
        }
        if per_day.iter().any(|m| *m > cap) {
            reasons |= FilterReason::PrefWorkload.bit();
        }
    }
    reasons
}

pub fn too_close(a: &CourseSection, a_meetings: &[Meeting], b: &CourseSection, b_meetings: &[Meeting], minutes: i32) -> bool {
    if minutes <= 0 || a.is_online() || b.is_online() {
        return false;
    }
    for x in a_meetings.iter().filter(|m| !m.is_online()) {
        let (Some(xs), Some(xe)) = (x.start, x.end) else { continue };
        for y in b_meetings.iter().filter(|m| !m.is_online()) {
            let (Some(ys), Some(ye)) = (y.start, y.end) else { continue };
            if x.days.is_disjoint(y.days) {
                continue;
            }
            let gap = if ys >= xe { ys - xe } else if xs >= ye { xs - ye } else { -1 };
            if gap < 0 || gap >= minutes {
                continue;
            }
            if !date_windows_overlap(x.start_date.as_deref(), x.end_date.as_deref(), y.start_date.as_deref(), y.end_date.as_deref()) {
                continue;
            }
            let bx = x.building_code.as_ref().or(x.building.as_ref());
            let by = y.building_code.as_ref().or(y.building.as_ref());
            if bx.is_some() && bx == by {
                continue;
            }
            return true;
        }
    }
    false
}

fn section_score(section: &CourseSection, intervals: &[Interval], p: &SchedulePreferences) -> f64 {
    let mut score = 0.0;
    if p.has_time_of_day() {
        for interval in intervals {
            for band in TimeBand::ALL {
                let preference = p.band(band);
                if preference == TimePreference::Okay {
                    continue;
                }
                let minutes = band.overlap(interval.start, interval.end);
                if minutes <= 0 {
                    continue;
                }
                let per_hour = if preference == TimePreference::Prefer { PREFERRED_BAND_PER_HOUR } else { AVOIDED_BAND_PER_HOUR };
                score += per_hour * minutes as f64 / 60.0;
            }
        }
    }
    if let Some(limit) = &p.earliest_start {
        if limit.strength == PreferenceStrength::Preferred {
            for interval in intervals {
                if interval.start < limit.minutes {
                    score -= soft_limit(limit.minutes - interval.start);
                }
            }
        }
    }
    if let Some(limit) = &p.latest_end {
        if limit.strength == PreferenceStrength::Preferred {
            for interval in intervals {
                if interval.end > limit.minutes {
                    score -= soft_limit(interval.end - limit.minutes);
                }
            }
        }
    }
    for block in p.active_blocked_times() {
        if block.strength != PreferenceStrength::Preferred {
            continue;
        }
        for interval in intervals {
            if block.overlaps(DaySet::from_indices([interval.day as i32]), interval.start, interval.end) {
                score -= SOFT_BLOCKED;
            }
        }
    }
    if p.format != FormatPreference::NoPreference {
        score -= FORMAT_MISMATCH * SchedulePreferences::format_of(section).mismatch(p.format);
    }
    if let Some(campus) = &p.campus {
        if p.campus_strength == PreferenceStrength::Preferred && !section.is_online() && !has_no_campus(section) && &section.campus != campus {
            score -= CAMPUS_MISMATCH;
        }
    }
    if !p.preferred_instructors.is_empty() || !p.avoided_instructors.is_empty() {
        let names: HashSet<String> = section.instructors.as_ref().map(|list| list.iter().map(|i| i.name.to_ascii_lowercase()).collect()).unwrap_or_default();
        if !names.is_empty() {
            if p.preferred_instructors.iter().any(|n| names.contains(&n.to_ascii_lowercase())) {
                score += PREFERRED_INSTRUCTOR;
            }
            if p.avoided_instructors.iter().any(|n| names.contains(&n.to_ascii_lowercase())) {
                score += AVOIDED_INSTRUCTOR;
            }
        }
    }
    score
}

fn soft_limit(minutes_outside: i32) -> f64 {
    SOFT_LIMIT_CAP.min(SOFT_LIMIT_BASE + minutes_outside as f64 / 10.0)
}

pub fn reasons_for(sections: &[CourseSection], p: &SchedulePreferences) -> Vec<ScheduleReason> {
    if p.is_default() || sections.is_empty() {
        return vec![];
    }
    let mut table = BuildingTable::default();
    let mut intervals = vec![];
    for section in sections {
        intervals.extend(intervals_for(section, &mut table));
    }
    let mut options = MeasureOptions::default();
    if p.breaks == BreakPreference::AvoidBackToBack {
        options.back_to_back_below = BACK_TO_BACK_MINUTES;
    }
    if p.breaks == BreakPreference::AtLeast {
        options.travel_minutes = p.minimum_break_minutes;
    }
    options.lunch = p.lunch.clone();
    let measure = measure(&intervals, &options);
    let has_classes = measure.class_days > 0;
    let mut good = vec![];
    let mut bad = vec![];
    let pro = |good: &mut Vec<ScheduleReason>, text: String, symbol: &str| good.push(ScheduleReason { benefit: true, text, symbol: symbol.into() });
    let con = |bad: &mut Vec<ScheduleReason>, text: String, symbol: &str| bad.push(ScheduleReason { benefit: false, text, symbol: symbol.into() });
    if has_classes && !p.days_off.is_empty() {
        let used = DaySet(measure.class_day_mask & p.days_off.0);
        let free = DaySet(p.days_off.0 & !used.0);
        if !free.is_empty() {
            pro(&mut good, days_off_phrase(free), "calendar.badge.minus");
        }
        if !used.is_empty() && p.days_off_strength == PreferenceStrength::Preferred {
            let names = used.indices().into_iter().map(|i| DaySet::SHORT[i as usize]).collect::<Vec<_>>().join(", ");
            con(&mut bad, format!("Class on {names}"), "calendar.badge.exclamationmark");
        }
    }
    if has_classes {
        if let Some(limit) = &p.earliest_start {
            if measure.earliest >= limit.minutes {
                pro(&mut good, format!("No classes before {}", short_time(limit.minutes)), "sunrise");
            } else if limit.strength == PreferenceStrength::Preferred {
                con(&mut bad, format!("Starts {}", label_minutes(measure.earliest, true)), "sunrise");
            }
        }
        if let Some(limit) = &p.latest_end {
            if measure.latest <= limit.minutes {
                pro(&mut good, format!("Done by {}", short_time(limit.minutes)), "sunset");
            } else if limit.strength == PreferenceStrength::Preferred {
                con(&mut bad, format!("Ends {}", label_minutes(measure.latest, true)), "sunset");
            }
        }
    }
    if has_classes {
        match p.day_layout {
            DayLayout::NoPreference => {}
            DayLayout::FewerDays => {
                let days = if measure.campus_days > 0 { measure.campus_days } else { measure.class_days };
                if days <= 3 {
                    pro(&mut good, if days == 1 { "1 day on campus".into() } else { format!("{days} days on campus") }, "calendar");
                }
            }
            DayLayout::SpreadOut => {
                if measure.class_days >= 4 {
                    pro(&mut good, format!("Spread over {} days", measure.class_days), "calendar");
                }
            }
        }
        if p.gaps == GapPreference::Compact && measure.gap_minutes <= 30 {
            pro(&mut good, "Short gaps".into(), "rectangle.compress.vertical");
        }
    }
    if has_classes && measure.total_minutes > 0 {
        let minutes_in = |bands: &[TimeBand]| intervals.iter().map(|i| bands.iter().map(|b| b.overlap(i.start, i.end)).sum::<i32>()).sum::<i32>();
        let preferred = p.preferred_bands();
        if !preferred.is_empty() {
            let share = minutes_in(&preferred) as f64 / measure.total_minutes as f64;
            let name = band_phrase(&preferred);
            if share >= 0.999 {
                pro(&mut good, format!("All {name}"), "sun.max");
            } else if share >= 0.6 {
                pro(&mut good, format!("Mostly {name}"), "sun.max");
            }
        }
        let avoided = p.avoided_bands();
        if !avoided.is_empty() {
            let name = band_phrase(&avoided);
            if minutes_in(&avoided) == 0 {
                pro(&mut good, format!("No {name}"), "nosign");
            } else {
                con(&mut bad, format!("Includes {name}"), "clock");
            }
        }
    }
    if has_classes {
        if p.lunch.is_some() {
            let free = measure.class_days - measure.lunch_misses;
            if measure.lunch_misses == 0 {
                pro(&mut good, "Lunch free every day".into(), "fork.knife");
            } else if free * 2 >= measure.class_days {
                pro(&mut good, "Lunch free most days".into(), "fork.knife");
            } else {
                con(&mut bad, "Lunch often busy".into(), "fork.knife");
            }
        }
        match p.breaks {
            BreakPreference::Fine => {}
            BreakPreference::AvoidBackToBack => {
                if measure.back_to_back == 0 {
                    pro(&mut good, "No back-to-back classes".into(), "figure.walk");
                } else {
                    con(&mut bad, if measure.back_to_back == 1 { "1 back-to-back".into() } else { format!("{} back-to-back", measure.back_to_back) }, "figure.walk");
                }
            }
            BreakPreference::AtLeast => {
                if measure.tight_transitions == 0 {
                    pro(&mut good, "Time to walk between classes".into(), "figure.walk");
                } else {
                    con(&mut bad, "Tight between buildings".into(), "figure.walk");
                }
            }
        }
    }
    if !p.preferred_instructors.is_empty() || !p.avoided_instructors.is_empty() {
        let names = |wanted: &[String]| {
            let set: HashSet<String> = wanted.iter().map(|s| s.to_ascii_lowercase()).collect();
            let mut result = vec![];
            for section in sections {
                for instructor in section.instructors.as_ref().into_iter().flatten() {
                    if set.contains(&instructor.name.to_ascii_lowercase()) && !result.contains(&instructor.display_name()) {
                        result.push(instructor.display_name());
                    }
                }
            }
            result
        };
        let liked = names(&p.preferred_instructors);
        if liked.len() == 1 {
            pro(&mut good, format!("With {}", liked[0]), "person.fill");
        } else if liked.len() > 1 {
            pro(&mut good, format!("With {} preferred instructors", liked.len()), "person.2.fill");
        }
        let disliked = names(&p.avoided_instructors);
        if let Some(first) = disliked.first() {
            con(&mut bad, if disliked.len() == 1 { format!("With {first}") } else { format!("With {} avoided instructors", disliked.len()) }, "person.fill.xmark");
        }
    }
    if p.format != FormatPreference::NoPreference {
        let formats: Vec<SectionFormat> = sections.iter().map(SchedulePreferences::format_of).collect();
        let deliveries: Vec<_> = formats.into_iter().filter(|f| !matches!(f, SectionFormat::Other)).collect();
        if !deliveries.is_empty() {
            if deliveries.iter().all(|f| f.mismatch(p.format) == 0.0) {
                pro(&mut good, format!("All {}", p.format.title().to_ascii_lowercase()), if p.format == FormatPreference::Online { "wifi" } else { "building.2" });
            } else if p.format_strength == PreferenceStrength::Preferred {
                con(&mut bad, format!("Not all {}", p.format.title().to_ascii_lowercase()), "wifi");
            }
        }
    }
    if let Some(campus) = &p.campus {
        let on_campus: Vec<_> = sections.iter().filter(|s| !s.is_online() && !has_no_campus(s)).collect();
        if !on_campus.is_empty() {
            if on_campus.iter().all(|s| &s.campus == campus) {
                pro(&mut good, format!("{campus} campus"), "mappin.and.ellipse");
            } else if p.campus_strength == PreferenceStrength::Preferred {
                con(&mut bad, "Includes another campus".into(), "mappin.and.ellipse");
            }
        }
    }
    match p.seats {
        SeatPolicy::Any => {}
        SeatPolicy::OpenOnly => pro(&mut good, "All sections open".into(), "chair.lounge"),
        SeatPolicy::WaitlistOk => {
            let waitlisted = sections.iter().filter(|s| preference_is_full(s)).count();
            if waitlisted == 0 {
                pro(&mut good, "All sections open".into(), "chair.lounge");
            } else {
                con(&mut bad, if waitlisted == 1 { "1 waitlisted section".into() } else { format!("{waitlisted} waitlisted sections") }, "chair.lounge");
            }
        }
    }
    for block in p.active_blocked_times() {
        if block.strength != PreferenceStrength::Preferred {
            continue;
        }
        let touches = intervals.iter().any(|i| block.overlaps(DaySet::from_indices([i.day as i32]), i.start, i.end));
        if touches {
            con(&mut bad, format!("Overlaps {}", block.label), "calendar.badge.exclamationmark");
        }
    }
    fn unique(reasons: Vec<ScheduleReason>) -> Vec<ScheduleReason> {
        let mut seen = HashSet::new();
        reasons.into_iter().filter(|r| seen.insert(r.id())).collect()
    }
    let mut out = unique(good);
    out.truncate(4);
    let mut caveats = unique(bad);
    caveats.truncate(2);
    out.extend(caveats);
    out
}

pub fn generate(courses: &[ScheduleCourseInput], filters: &ScheduleFilters, preferences: &SchedulePreferences, sort: ScheduleSort, limits: &Limits) -> GenerationResult {
    let started = Instant::now();
    let mut result = run(courses, filters, &preferences.normalized(), sort, limits);
    result.elapsed = started.elapsed().as_secs_f64();
    result
}

fn run(courses: &[ScheduleCourseInput], filters: &ScheduleFilters, preferences: &SchedulePreferences, sort: ScheduleSort, limits: &Limits) -> GenerationResult {
    let has_preferences = !preferences.is_default();
    let mut kept = vec![];
    let mut reports = vec![];
    for course in courses {
        let mut remaining = vec![];
        let mut rejections = HashMap::new();
        for section in &course.sections {
            let reasons = rejection_reasons(section, filters, preferences, &course.excluded_crns);
            if reasons == 0 {
                remaining.push(section.clone());
            } else {
                for reason in FilterReason::ALL {
                    if reasons & reason.bit() != 0 {
                        *rejections.entry(reason).or_insert(0) += 1;
                    }
                }
            }
        }
        reports.push(CourseReport { key: course.key.clone(), is_required: course.is_required, total_sections: course.sections.len(), remaining_sections: remaining.len(), rejections });
        kept.push(remaining);
    }
    let mut diagnostics = ScheduleDiagnostics { filters: filters.clone(), preferences: preferences.clone(), courses: reports, conflicting_pairs: vec![], count_problem: None };
    let mut empty = GenerationResult { schedules: vec![], total_found: 0, is_exhaustive: true, was_cancelled: false, diagnostics: diagnostics.clone(), elapsed: 0.0 };
    if courses.is_empty() {
        return empty;
    }
    let required: Vec<usize> = courses.iter().enumerate().filter(|(_, c)| c.is_required).map(|(i, _)| i).collect();
    let optional: Vec<usize> = courses.iter().enumerate().filter(|(i, c)| !c.is_required && !kept[*i].is_empty()).map(|(i, _)| i).collect();
    if required.iter().any(|i| kept[*i].is_empty()) {
        empty.diagnostics = diagnostics;
        return empty;
    }
    let target = filters.course_count.filter(|n| *n > 0);
    if let Some(target) = target {
        if required.len() as i32 > target {
            diagnostics.count_problem = Some(CountProblem::TooManyRequired { required: required.len() as i32, wanted: target });
            empty.diagnostics = diagnostics;
            return empty;
        }
        if ((required.len() + optional.len()) as i32) < target {
            diagnostics.count_problem = Some(CountProblem::TooFewCourses { available: (required.len() + optional.len()) as i32, wanted: target });
            empty.diagnostics = diagnostics;
            return empty;
        }
    }
    let credit_window = preferences.credit_target.as_ref().map(|t| (t.lower, t.upper));
    if let Some((lower, upper)) = credit_window {
        let required_credits: i32 = required.iter().map(|i| courses[*i].min_credits).sum();
        if required_credits > upper {
            diagnostics.count_problem = Some(CountProblem::CreditsOverTarget { required: required_credits, limit: upper });
            empty.diagnostics = diagnostics;
            return empty;
        }
        let available: i32 = required.iter().chain(optional.iter()).map(|i| courses[*i].max_credits).sum();
        if available < lower {
            diagnostics.count_problem = Some(CountProblem::CreditsUnderTarget { available, wanted: lower });
            empty.diagnostics = diagnostics;
            return empty;
        }
    }
    let order_indexes = |indexes: &[usize]| {
        let mut v = indexes.to_vec();
        v.sort_by(|a, b| kept[*a].len().cmp(&kept[*b].len()).then(a.cmp(b)));
        v
    };
    let search_order = [order_indexes(&required), order_indexes(&optional)].concat();
    if search_order.is_empty() {
        empty.diagnostics = diagnostics;
        return empty;
    }
    let mut sections = vec![];
    let mut masks = vec![];
    let mut weekly = vec![];
    let mut interval_start = vec![];
    let mut intervals = vec![];
    let mut points = vec![];
    let mut day_classes = vec![];
    let mut day_minutes = vec![];
    let mut slots = vec![];
    let mut slot_min = vec![];
    let mut slot_max = vec![];
    let mut buildings = BuildingTable::default();
    for course_index in &search_order {
        let first = sections.len();
        let mut candidates = vec![];
        for (offset, section) in kept[*course_index].iter().enumerate() {
            let own = intervals_for(section, &mut buildings);
            let score = if has_preferences { section_score(section, &own, preferences) } else { 0.0 };
            candidates.push((offset, section, own, score));
        }
        if has_preferences {
            candidates.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
        }
        for (_, section, own, score) in candidates {
            sections.push(section.clone());
            let meetings: Vec<Meeting> = section.weekly_meetings().into_iter().cloned().collect();
            masks.push(ScheduleSlotMask::from_meetings(&meetings));
            weekly.push(meetings);
            interval_start.push(intervals.len());
            let mut classes = [0i32; 7];
            let mut minutes = [0i32; 7];
            for interval in &own {
                classes[interval.day as usize] = 1;
                minutes[interval.day as usize] += interval.end - interval.start;
            }
            day_classes.push(classes);
            day_minutes.push(minutes);
            intervals.extend(own);
            points.push(score);
        }
        slots.push(Slot { course_index: *course_index, is_required: courses[*course_index].is_required, start: first, size: sections.len() - first });
        slot_min.push(courses[*course_index].min_credits);
        slot_max.push(courses[*course_index].min_credits.max(courses[*course_index].max_credits));
    }
    interval_start.push(intervals.len());
    let travel = if preferences.breaks == BreakPreference::AtLeast && preferences.minimum_break_strength == PreferenceStrength::Required { preferences.minimum_break_minutes } else { 0 };
    let rules = SearchRules {
        max_classes: preferences.max_classes_per_day.unwrap_or(i32::MAX),
        max_minutes: preferences.max_minutes_per_day.unwrap_or(i32::MAX),
        day_classes,
        day_minutes,
        travel,
        credit_window,
        slot_min,
        slot_max,
    };
    let weights = Weights::from_preferences(preferences);
    let options = measure_options(preferences);
    let seed = fnv_seed(&sections);
    let mut search = Search::new(slots, masks, weekly, sections, rules, target, limits, seed);
    search.run_exhaustive(&intervals, &interval_start, &points, &weights, &options, sort);
    if search.hit_limit && limits.allow_sampling && limits.sample_leaves > 0 {
        search.run_sampled(&intervals, &interval_start, &points, &weights, &options, sort);
    }
    let exhaustive = !search.hit_limit;
    let total = search.leaf_count;
    if total == 0 {
        if exhaustive {
            diagnostics.conflicting_pairs = search.conflicting_pairs(courses);
        }
        empty.diagnostics = diagnostics;
        empty.is_exhaustive = exhaustive;
        return empty;
    }
    let schedules = search.materialize(has_preferences, preferences);
    GenerationResult { schedules, total_found: total, is_exhaustive: exhaustive, was_cancelled: false, diagnostics, elapsed: 0.0 }
}

struct Slot {
    course_index: usize,
    is_required: bool,
    start: usize,
    size: usize,
}

struct SearchRules {
    max_classes: i32,
    max_minutes: i32,
    day_classes: Vec<[i32; 7]>,
    day_minutes: Vec<[i32; 7]>,
    travel: i32,
    credit_window: Option<(i32, i32)>,
    slot_min: Vec<i32>,
    slot_max: Vec<i32>,
}

#[derive(Clone)]
struct Leaf {
    picks: Vec<i32>,
    class_days: i32,
    campus_days: i32,
    earliest: i32,
    latest: i32,
    gap: i32,
    min_seats: i32,
    total_seats: i32,
    full: i32,
    score: f64,
    seq: i32,
}

struct Search {
    slots: Vec<Slot>,
    masks: Vec<ScheduleSlotMask>,
    weekly: Vec<Vec<Meeting>>,
    sections: Vec<CourseSection>,
    compat: Vec<Vec<Vec<u64>>>,
    rules: SearchRules,
    target: Option<i32>,
    limits: Limits,
    seed: u64,
    sort: ScheduleSort,
    leaf_count: usize,
    hit_limit: bool,
    collector: Vec<Leaf>,
    seq: i32,
}

impl Search {
    fn new(slots: Vec<Slot>, masks: Vec<ScheduleSlotMask>, weekly: Vec<Vec<Meeting>>, sections: Vec<CourseSection>, rules: SearchRules, target: Option<i32>, limits: &Limits, seed: u64) -> Self {
        let slot_count = slots.len();
        let mut compat = vec![vec![Vec::new(); slot_count]; masks.len()];
        let mut slot_of = vec![0usize; masks.len()];
        for (s, slot) in slots.iter().enumerate() {
            for g in slot.start..slot.start + slot.size {
                slot_of[g] = s;
            }
        }
        for g in 0..masks.len() {
            for s in 0..slot_count {
                if s == slot_of[g] {
                    continue;
                }
                let words = (slots[s].size + 63) / 64;
                let mut bits = vec![0u64; words.max(1)];
                for j in 0..slots[s].size {
                    let h = slots[s].start + j;
                    if masks[g].conflicts(masks[h]) && meetings_conflict(&weekly[g], &weekly[h]) {
                        continue;
                    }
                    if rules.travel > 0 && too_close(&sections[g], &weekly[g], &sections[h], &weekly[h], rules.travel) {
                        continue;
                    }
                    bits[j / 64] |= 1u64 << (j % 64);
                }
                compat[g][s] = bits;
            }
        }
        Self { slots, masks, weekly, sections, compat, rules, target, limits: limits.clone(), seed, sort: ScheduleSort::Best, leaf_count: 0, hit_limit: false, collector: vec![], seq: 0 }
    }

    fn conflicting_pairs(&self, courses: &[ScheduleCourseInput]) -> Vec<ConflictPair> {
        let mut pairs = vec![];
        for a in 0..self.slots.len() {
            for b in (a + 1)..self.slots.len() {
                if !self.slots[a].is_required || !self.slots[b].is_required {
                    continue;
                }
                if !self.has_compatible(a, b) {
                    let reason = if self.rules.travel > 0 && self.has_overlap_free(a, b) { ConflictReason::TravelTime } else { ConflictReason::Overlap };
                    pairs.push(ConflictPair { first: courses[self.slots[a].course_index].key.clone(), second: courses[self.slots[b].course_index].key.clone(), reason });
                }
            }
        }
        pairs
    }

    fn has_compatible(&self, a: usize, b: usize) -> bool {
        for j in 0..self.slots[a].size {
            let g = self.slots[a].start + j;
            if self.compat[g][b].iter().any(|w| *w != 0) {
                return true;
            }
        }
        false
    }

    fn has_overlap_free(&self, a: usize, b: usize) -> bool {
        if self.rules.travel == 0 {
            return self.has_compatible(a, b);
        }
        for i in 0..self.slots[a].size {
            for j in 0..self.slots[b].size {
                let g = self.slots[a].start + i;
                let h = self.slots[b].start + j;
                if !(self.masks[g].conflicts(self.masks[h]) && meetings_conflict(&self.weekly[g], &self.weekly[h])) {
                    return true;
                }
            }
        }
        false
    }

    fn run_exhaustive(&mut self, intervals: &[Interval], interval_start: &[usize], points: &[f64], weights: &Weights, options: &MeasureOptions, sort: ScheduleSort) {
        self.sort = sort;
        self.leaf_count = 0;
        self.hit_limit = false;
        self.collector.clear();
        let domain = self.full_domain();
        let deadline = Instant::now() + std::time::Duration::from_secs_f64(self.limits.time_budget.max(0.05));
        let mut ctx = Walk {
            picks: vec![-1; self.slots.len()],
            chosen: 0,
            nodes: 0,
            load_classes: [0; 7],
            load_minutes: [0; 7],
            credits_min: 0,
            credits_max: 0,
            aborted: false,
        };
        self.descend(0, &domain, &mut ctx, false, self.limits.max_exhaustive_leaves, deadline, intervals, interval_start, points, weights, options, sort, &mut SplitMix::new(self.seed));
    }

    fn run_sampled(&mut self, intervals: &[Interval], interval_start: &[usize], points: &[f64], weights: &Weights, options: &MeasureOptions, sort: ScheduleSort) {
        self.leaf_count = 0;
        let domain = self.full_domain();
        let deadline = Instant::now() + std::time::Duration::from_secs_f64(self.limits.sample_time_budget.max(0.05));
        let mut ctx = Walk { picks: vec![-1; self.slots.len()], chosen: 0, nodes: 0, load_classes: [0; 7], load_minutes: [0; 7], credits_min: 0, credits_max: 0, aborted: false };
        let mut rng = SplitMix::new(self.seed);
        self.descend(0, &domain, &mut ctx, true, self.limits.sample_leaves, deadline, intervals, interval_start, points, weights, options, sort, &mut rng);
    }

    fn full_domain(&self) -> Vec<Vec<u64>> {
        self.slots.iter().map(|slot| {
            let words = (slot.size + 63) / 64;
            let mut bits = vec![0u64; words.max(1)];
            for i in 0..slot.size {
                bits[i / 64] |= 1u64 << (i % 64);
            }
            bits
        }).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn descend(&mut self, depth: usize, domain: &[Vec<u64>], ctx: &mut Walk, sampled: bool, max_leaves: usize, deadline: Instant, intervals: &[Interval], interval_start: &[usize], points: &[f64], weights: &Weights, options: &MeasureOptions, sort: ScheduleSort, rng: &mut SplitMix) {
        if ctx.aborted {
            return;
        }
        ctx.nodes += 1;
        if ctx.nodes & 0x3ff == 0 && (ctx.nodes >= self.limits.max_nodes || Instant::now() >= deadline) {
            self.hit_limit = true;
            ctx.aborted = true;
            return;
        }
        if depth == self.slots.len() {
            self.record(depth, domain, ctx, intervals, interval_start, points, weights, options, sort);
            if self.leaf_count >= max_leaves.max(1) {
                self.hit_limit = true;
                ctx.aborted = true;
            }
            return;
        }
        if let Some(target) = self.target {
            if ctx.chosen == target {
                for s in depth..self.slots.len() {
                    ctx.picks[s] = -1;
                }
                self.record(depth, domain, ctx, intervals, interval_start, points, weights, options, sort);
                if self.leaf_count >= max_leaves.max(1) {
                    self.hit_limit = true;
                    ctx.aborted = true;
                }
                return;
            }
        }
        if !self.feasible(depth, domain, ctx, depth) {
            return;
        }
        let mut options_local = vec![];
        if !self.slots[depth].is_required && self.feasible(depth, domain, ctx, depth + 1) {
            options_local.push(-1);
        }
        let words = domain[depth].clone();
        for (word_index, mut bits) in words.into_iter().enumerate() {
            while bits != 0 {
                let local = word_index * 64 + bits.trailing_zeros() as usize;
                bits &= bits.wrapping_sub(1);
                if local < self.slots[depth].size && self.can_add(self.slots[depth].start + local, depth, ctx) {
                    options_local.push(local as i32);
                }
            }
        }
        if sampled {
            options_local = shuffle(options_local, rng);
            let mut remaining = max_leaves.saturating_sub(self.leaf_count).max(1);
            let mut left = options_local.len();
            for option in options_local {
                if ctx.aborted || remaining == 0 {
                    break;
                }
                let before = self.leaf_count;
                self.try_option(depth, option, domain, ctx, true, remaining, deadline, intervals, interval_start, points, weights, options, sort, rng);
                let got = self.leaf_count.saturating_sub(before);
                remaining = remaining.saturating_sub(got.max(1).min(remaining));
                left = left.saturating_sub(1);
                let _ = left;
            }
            return;
        }
        for option in options_local {
            if ctx.aborted {
                return;
            }
            self.try_option(depth, option, domain, ctx, false, max_leaves, deadline, intervals, interval_start, points, weights, options, sort, rng);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn try_option(&mut self, depth: usize, option: i32, domain: &[Vec<u64>], ctx: &mut Walk, sampled: bool, max_leaves: usize, deadline: Instant, intervals: &[Interval], interval_start: &[usize], points: &[f64], weights: &Weights, options: &MeasureOptions, sort: ScheduleSort, rng: &mut SplitMix) {
        if option < 0 {
            ctx.picks[depth] = -1;
            self.descend(depth + 1, domain, ctx, sampled, max_leaves, deadline, intervals, interval_start, points, weights, options, sort, rng);
            return;
        }
        let local = option as usize;
        let g = self.slots[depth].start + local;
        let Some(next) = self.forward(domain, depth, g) else { return };
        if !self.can_add(g, depth, ctx) {
            return;
        }
        self.take(g, depth, ctx);
        ctx.picks[depth] = option;
        ctx.chosen += 1;
        self.descend(depth + 1, &next, ctx, sampled, max_leaves, deadline, intervals, interval_start, points, weights, options, sort, rng);
        ctx.chosen -= 1;
        ctx.picks[depth] = -1;
        self.release(g, depth, ctx);
    }

    fn forward(&self, domain: &[Vec<u64>], depth: usize, g: usize) -> Option<Vec<Vec<u64>>> {
        let mut next = domain.to_vec();
        for s in 0..self.slots.len() {
            if s == depth {
                continue;
            }
            let compat = &self.compat[g][s];
            let mut any = false;
            for (k, word) in next[s].iter_mut().enumerate() {
                let mask = compat.get(k).copied().unwrap_or(0);
                *word &= mask;
                any |= *word != 0;
            }
            if !any && s > depth && self.slots[s].is_required {
                return None;
            }
        }
        Some(next)
    }

    fn can_add(&self, g: usize, slot: usize, ctx: &Walk) -> bool {
        if self.rules.max_classes != i32::MAX || self.rules.max_minutes != i32::MAX {
            for day in 0..7 {
                if ctx.load_classes[day] + self.rules.day_classes[g][day] > self.rules.max_classes {
                    return false;
                }
                if ctx.load_minutes[day] + self.rules.day_minutes[g][day] > self.rules.max_minutes {
                    return false;
                }
            }
        }
        if let Some((_, upper)) = self.rules.credit_window {
            if ctx.credits_min + self.rules.slot_min[slot] > upper {
                return false;
            }
        }
        true
    }

    fn take(&self, g: usize, slot: usize, ctx: &mut Walk) {
        for day in 0..7 {
            ctx.load_classes[day] += self.rules.day_classes[g][day];
            ctx.load_minutes[day] += self.rules.day_minutes[g][day];
        }
        ctx.credits_min += self.rules.slot_min[slot];
        ctx.credits_max += self.rules.slot_max[slot];
    }

    fn release(&self, g: usize, slot: usize, ctx: &mut Walk) {
        for day in 0..7 {
            ctx.load_classes[day] -= self.rules.day_classes[g][day];
            ctx.load_minutes[day] -= self.rules.day_minutes[g][day];
        }
        ctx.credits_min -= self.rules.slot_min[slot];
        ctx.credits_max -= self.rules.slot_max[slot];
    }

    fn can_still_add(&self, domain: &[Vec<u64>], slot: usize, ctx: &Walk) -> bool {
        if self.rules.max_classes == i32::MAX && self.rules.max_minutes == i32::MAX && self.rules.credit_window.is_none() {
            return domain[slot].iter().any(|w| *w != 0);
        }
        for (word_index, mut bits) in domain[slot].iter().copied().enumerate() {
            while bits != 0 {
                let local = word_index * 64 + bits.trailing_zeros() as usize;
                bits &= bits.wrapping_sub(1);
                if local < self.slots[slot].size && self.can_add(self.slots[slot].start + local, slot, ctx) {
                    return true;
                }
            }
        }
        false
    }

    fn feasible(&self, _depth: usize, domain: &[Vec<u64>], ctx: &Walk, first: usize) -> bool {
        if self.target.is_none() && self.rules.credit_window.is_none() {
            return true;
        }
        let mut reachable = ctx.chosen;
        let mut most = ctx.credits_max;
        let mut least = ctx.credits_min;
        for s in first..self.slots.len() {
            let required = self.slots[s].is_required;
            if required || self.can_still_add(domain, s, ctx) {
                reachable += 1;
                most += self.rules.slot_max[s];
            }
            if required {
                least += self.rules.slot_min[s];
            }
        }
        if let Some(target) = self.target {
            if reachable < target {
                return false;
            }
        }
        if let Some((lower, upper)) = self.rules.credit_window {
            if most < lower || least > upper {
                return false;
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn record(&mut self, depth: usize, domain: &[Vec<u64>], ctx: &Walk, intervals: &[Interval], interval_start: &[usize], points: &[f64], weights: &Weights, options: &MeasureOptions, sort: ScheduleSort) {
        if ctx.chosen == 0 {
            return;
        }
        if let Some(target) = self.target {
            if ctx.chosen != target {
                return;
            }
        }
        if let Some((lower, upper)) = self.rules.credit_window {
            if ctx.credits_min > upper || ctx.credits_max < lower {
                return;
            }
        }
        if self.target.is_none() && self.rules.credit_window.is_none() {
            for s in 0..self.slots.len() {
                if !self.slots[s].is_required && ctx.picks[s] < 0 && self.can_still_add(domain, s, ctx) {
                    return;
                }
            }
        }
        let _ = depth;
        self.offer(ctx, intervals, interval_start, points, weights, options, sort);
        self.leaf_count += 1;
    }

    fn offer(&mut self, ctx: &Walk, intervals: &[Interval], interval_start: &[usize], points: &[f64], weights: &Weights, options: &MeasureOptions, sort: ScheduleSort) {
        let mut own = vec![];
        let mut min_seats = i32::MAX;
        let mut total_seats = 0;
        let mut full = 0;
        let mut extra = 0.0;
        for (s, pick) in ctx.picks.iter().enumerate() {
            if *pick < 0 {
                continue;
            }
            let g = self.slots[s].start + *pick as usize;
            own.extend_from_slice(&intervals[interval_start[g]..interval_start[g + 1]]);
            let seats = self.sections[g].seats_remaining.max(0);
            min_seats = min_seats.min(seats);
            total_seats += seats;
            if seats == 0 {
                full += 1;
            }
            extra += points[g];
        }
        let raw = measure(&own, options);
        let seats_floor = if min_seats == i32::MAX { 0 } else { min_seats };
        let latest = if raw.latest == i32::MIN { 0 } else { raw.latest };
        let leaf = Leaf {
            picks: ctx.picks.clone(),
            class_days: raw.class_days,
            campus_days: raw.campus_days,
            earliest: raw.earliest,
            latest,
            gap: raw.gap_minutes,
            min_seats: seats_floor,
            total_seats,
            full,
            score: weights.score(&raw, full, seats_floor) + extra,
            seq: self.seq,
        };
        self.seq += 1;
        let capacity = self.limits.max_results.max(1);
        if self.collector.len() < capacity {
            self.collector.push(leaf);
            self.sift_up(self.collector.len() - 1, sort);
        } else if precedes(sort, &leaf, &self.collector[0]) {
            self.collector[0] = leaf;
            self.sift_down(0, sort);
        }
    }

    fn worse(&self, a: usize, b: usize, sort: ScheduleSort) -> bool {
        precedes(sort, &self.collector[b], &self.collector[a])
    }

    fn sift_up(&mut self, mut child: usize, sort: ScheduleSort) {
        while child > 0 {
            let parent = (child - 1) / 2;
            if !self.worse(child, parent, sort) {
                return;
            }
            self.collector.swap(child, parent);
            child = parent;
        }
    }

    fn sift_down(&mut self, mut parent: usize, sort: ScheduleSort) {
        loop {
            let left = 2 * parent + 1;
            let right = left + 1;
            let mut worst = parent;
            if left < self.collector.len() && self.worse(left, worst, sort) {
                worst = left;
            }
            if right < self.collector.len() && self.worse(right, worst, sort) {
                worst = right;
            }
            if worst == parent {
                return;
            }
            self.collector.swap(parent, worst);
            parent = worst;
        }
    }

    fn materialize(&self, has_preferences: bool, preferences: &SchedulePreferences) -> Vec<GeneratedSchedule> {
        let mut kept = self.collector.clone();
        let sort = self.sort;
        kept.sort_by(|a, b| {
            let ab = precedes(sort, a, b);
            let ba = precedes(sort, b, a);
            if ab && !ba { std::cmp::Ordering::Less } else if ba && !ab { std::cmp::Ordering::Greater } else { a.seq.cmp(&b.seq) }
        });
        let mut out = vec![];
        for leaf in kept {
            let mut picked = vec![];
            for (s, pick) in leaf.picks.iter().enumerate() {
                if *pick < 0 {
                    continue;
                }
                picked.push((self.slots[s].course_index, self.slots[s].start + *pick as usize));
            }
            picked.sort_by_key(|p| p.0);
            let chosen: Vec<CourseSection> = picked.iter().map(|(_, id)| self.sections[*id].clone()).collect();
            out.push(GeneratedSchedule::assemble(
                chosen.clone(),
                leaf.campus_days,
                leaf.class_days,
                if leaf.earliest == i32::MAX { None } else { Some(leaf.earliest) },
                if leaf.class_days == 0 { None } else { Some(leaf.latest) },
                leaf.gap,
                leaf.min_seats,
                leaf.total_seats,
                leaf.full,
                leaf.score,
                if has_preferences { reasons_for(&chosen, preferences) } else { vec![] },
            ));
        }
        out
    }
}

struct Walk {
    picks: Vec<i32>,
    chosen: i32,
    nodes: usize,
    load_classes: [i32; 7],
    load_minutes: [i32; 7],
    credits_min: i32,
    credits_max: i32,
    aborted: bool,
}

fn precedes(sort: ScheduleSort, a: &Leaf, b: &Leaf) -> bool {
    match sort {
        ScheduleSort::Best => {}
        ScheduleSort::FewestDays => {
            if a.campus_days != b.campus_days { return a.campus_days < b.campus_days; }
            if a.class_days != b.class_days { return a.class_days < b.class_days; }
            if a.gap != b.gap { return a.gap < b.gap; }
        }
        ScheduleSort::LatestStart => {
            if a.earliest != b.earliest { return a.earliest > b.earliest; }
            if a.campus_days != b.campus_days { return a.campus_days < b.campus_days; }
        }
        ScheduleSort::EarliestFinish => {
            if a.latest != b.latest { return a.latest < b.latest; }
            if a.campus_days != b.campus_days { return a.campus_days < b.campus_days; }
        }
        ScheduleSort::FewestGaps => {
            if a.gap != b.gap { return a.gap < b.gap; }
            if a.campus_days != b.campus_days { return a.campus_days < b.campus_days; }
        }
        ScheduleSort::MostSeats => {
            if a.min_seats != b.min_seats { return a.min_seats > b.min_seats; }
            if a.total_seats != b.total_seats { return a.total_seats > b.total_seats; }
        }
    }
    if a.score != b.score {
        return a.score > b.score;
    }
    a.seq < b.seq
}

fn meetings_conflict(a: &[Meeting], b: &[Meeting]) -> bool {
    a.iter().any(|first| b.iter().any(|second| first.overlaps(second)))
}

fn fnv_seed(sections: &[CourseSection]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for section in sections {
        for byte in section.crn.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

struct SplitMix {
    state: u64,
}
impl SplitMix {
    fn new(state: u64) -> Self {
        Self { state }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn shuffle(mut items: Vec<i32>, rng: &mut SplitMix) -> Vec<i32> {
    for i in (1..items.len()).rev() {
        let j = (rng.next_u64() as usize) % (i + 1);
        items.swap(i, j);
    }
    items
}

pub fn suggest_relaxations(courses: &[ScheduleCourseInput], filters: &ScheduleFilters, preferences: &SchedulePreferences, include_courses: bool) -> Vec<Suggestion> {
    let mut candidates: Vec<(Relaxation, ScheduleFilters, SchedulePreferences, Vec<ScheduleCourseInput>)> = vec![];
    let add = |list: &mut Vec<_>, relaxation: Relaxation, active: bool| {
        if !active { return; }
        let mut relaxed = filters.clone();
        relaxation.apply_filters(&mut relaxed);
        list.push((relaxation, relaxed, preferences.clone(), courses.to_vec()));
    };
    add(&mut candidates, Relaxation::ClearEarliestStart, filters.earliest_start.is_some());
    add(&mut candidates, Relaxation::ClearLatestEnd, filters.latest_end.is_some());
    add(&mut candidates, Relaxation::ClearFreeDays, !filters.free_days.is_empty());
    add(&mut candidates, Relaxation::AllowHonors, !filters.include_honors);
    add(&mut candidates, Relaxation::AllowOnline, !filters.include_online);
    add(&mut candidates, Relaxation::AllowFullSections, filters.hide_full_sections || filters.min_seats > 0);
    add(&mut candidates, Relaxation::ClearCampus, filters.campus.is_some());
    add(&mut candidates, Relaxation::ClearCourseCount, filters.course_count.is_some());
    for relaxation in PreferenceRelaxation::available(preferences) {
        let mut relaxed = preferences.clone();
        relaxation.apply(&mut relaxed);
        candidates.push((Relaxation::Preference(relaxation), filters.clone(), relaxed, courses.to_vec()));
    }
    if include_courses && courses.len() > 1 {
        for (index, course) in courses.iter().enumerate() {
            if !course.is_required { continue; }
            let mut relaxed = courses.to_vec();
            relaxed[index].is_required = false;
            candidates.push((Relaxation::MakeOptional(course.key.clone()), filters.clone(), preferences.clone(), relaxed));
        }
    }
    let mut suggestions = vec![];
    let probe = Limits::probe();
    for (relaxation, filters, preferences, courses) in candidates {
        let result = generate(&courses, &filters, &preferences, ScheduleSort::Best, &probe);
        if result.total_found > 0 {
            suggestions.push(Suggestion { relaxation, schedule_count: result.total_found, is_lower_bound: !result.is_exhaustive });
        }
    }
    suggestions.sort_by_key(|suggestion| std::cmp::Reverse(suggestion.schedule_count));
    suggestions
}

pub fn no_results_title(diagnostics: &ScheduleDiagnostics) -> String {
    let required = diagnostics.courses.iter().filter(|c| c.is_required).count();
    if diagnostics.count_problem.is_some() || required <= 1 {
        return "No schedules fit".into();
    }
    if diagnostics.courses.len() > required {
        format!("No schedule fits all {required} required courses")
    } else {
        format!("No schedule fits all {required} courses")
    }
}

pub fn no_results_headline(diagnostics: &ScheduleDiagnostics, has_suggestions: bool) -> String {
    if diagnostics.has_no_courses() {
        return "Add some courses to this schedule first.".into();
    }
    if has_suggestions {
        return "Here’s what would work.".into();
    }
    if diagnostics.count_problem.is_some() || !diagnostics.blocked_required().is_empty() || !diagnostics.conflicting_pairs.is_empty() {
        return "Here’s what’s getting in the way.".into();
    }
    "These courses can’t all be taken together with the current rules.".into()
}

/// `.ics` text for the chosen sections. Dates come from each meeting; times are America/New_York.
pub fn ics(plan_id: &str, name: &str, sections: &[CourseSection]) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//Term Organize//Schedule//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
        format!("X-WR-CALNAME:{}", escape_ics(name)),
        "X-WR-TIMEZONE:America/New_York".into(),
        "BEGIN:VTIMEZONE".into(),
        "TZID:America/New_York".into(),
        "X-LIC-LOCATION:America/New_York".into(),
        "BEGIN:DAYLIGHT".into(),
        "TZOFFSETFROM:-0500".into(),
        "TZOFFSETTO:-0400".into(),
        "TZNAME:EDT".into(),
        "DTSTART:19700308T020000".into(),
        "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU".into(),
        "END:DAYLIGHT".into(),
        "BEGIN:STANDARD".into(),
        "TZOFFSETFROM:-0400".into(),
        "TZOFFSETTO:-0500".into(),
        "TZNAME:EST".into(),
        "DTSTART:19701101T020000".into(),
        "RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU".into(),
        "END:STANDARD".into(),
        "END:VTIMEZONE".into(),
    ];
    let stamp = utc_now();
    let codes = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];
    for section in sections {
        for (n, meeting) in section.weekly_meetings().into_iter().enumerate() {
            let (Some(start), Some(end)) = (meeting.start, meeting.end) else { continue };
            let Some(first) = meeting.start_date.clone().or_else(|| Some("2026-09-09".into())) else { continue };
            let last = meeting.end_date.clone().unwrap_or_else(|| first.clone());
            lines.push("BEGIN:VEVENT".into());
            lines.push(format!("UID:{}-{}-{n}@neuclasssearch", plan_id.to_ascii_lowercase(), section.crn));
            lines.push(format!("DTSTAMP:{stamp}"));
            lines.push(format!("DTSTART;TZID=America/New_York:{}T{}", first.replace('-', ""), hhmm(start)));
            lines.push(format!("DTEND;TZID=America/New_York:{}T{}", first.replace('-', ""), hhmm(end)));
            let days = meeting.days.indices().into_iter().map(|d| codes[d as usize]).collect::<Vec<_>>().join(",");
            lines.push(format!("RRULE:FREQ=WEEKLY;BYDAY={days};UNTIL={}", until_stamp(&last)));
            lines.push(format!("SUMMARY:{}", escape_ics(&format!("{} {}", section.course_key(), section.title))));
            let location = if meeting.is_online() || section.is_online() { Some("Online".into()) } else {
                let label = meeting.long_location_label();
                if label == "Location TBA" { None } else { Some(label) }
            };
            if let Some(location) = location {
                lines.push(format!("LOCATION:{}", escape_ics(&location)));
            }
            lines.push(format!("DESCRIPTION:{}", escape_ics(&format!("CRN {}", section.crn))));
            lines.push("TRANSP:OPAQUE".into());
            lines.push("END:VEVENT".into());
        }
    }
    lines.push("END:VCALENDAR".into());
    lines.into_iter().map(fold_ics).collect::<Vec<_>>().join("\r\n") + "\r\n"
}

fn hhmm(minutes: i32) -> String {
    format!("{:02}{:02}00", minutes.div_euclid(60).rem_euclid(24), minutes.rem_euclid(60))
}
fn escape_ics(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n").replace(',', "\\,").replace(';', "\\;")
}
fn fold_ics(line: String) -> String {
    let bytes = line.as_bytes();
    if bytes.len() <= 75 {
        return line;
    }
    let mut out = String::new();
    let mut start = 0;
    let mut limit = 75;
    while start < bytes.len() {
        let mut end = (start + limit).min(bytes.len());
        while end > start && !line.is_char_boundary(end) {
            end -= 1;
        }
        if start > 0 {
            out.push_str("\r\n ");
        }
        out.push_str(&line[start..end]);
        start = end;
        limit = 74;
    }
    out
}
fn utc_now() -> String {
    "20261013T000000Z".into()
}
fn until_stamp(date: &str) -> String {
    let parts: Vec<i32> = date.split('-').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 3 {
        return format!("{}T235959Z", date.replace('-', ""));
    }
    let (y, m, d) = (parts[0], parts[1], parts[2]);
    let offset = if is_edt(y, m, d) { 4 } else { 5 };
    let hour = 23 + offset;
    let (day, hour) = if hour >= 24 { (d + 1, hour - 24) } else { (d, hour) };
    format!("{y:04}{m:02}{day:02}T{hour:02}5959Z")
}
fn is_edt(year: i32, month: i32, day: i32) -> bool {
    if (4..11).contains(&month) { return true; }
    if month == 3 { return day >= nth_weekday(year, 3, 0, 2); }
    if month == 11 { return day < nth_weekday(year, 11, 0, 1); }
    false
}
fn nth_weekday(year: i32, month: i32, weekday: i32, n: i32) -> i32 {
    let mut day = 1;
    let mut seen = 0;
    while day < 32 {
        if civil_weekday(year, month, day) == weekday {
            seen += 1;
            if seen == n { return day; }
        }
        day += 1;
    }
    1
}
fn civil_weekday(year: i32, month: i32, day: i32) -> i32 {
    let mut y = year;
    let mut m = month;
    if m < 3 { y -= 1; m += 12; }
    let k = y % 100;
    let j = y / 100;
    let h = (day + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    (h + 6) % 7
}

/// A week-grid PNG of the chosen sections. Drawn without GTK so tests can check it.
pub fn week_png(sections: &[CourseSection]) -> Vec<u8> {
    let width = 880u32;
    let height = 520u32;
    let mut pixels = vec![255u8; (width * height * 3) as usize];
    let put = |pixels: &mut [u8], x: i32, y: i32, rgb: [u8; 3]| {
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 { return; }
        let i = ((y as u32 * width + x as u32) * 3) as usize;
        pixels[i..i + 3].copy_from_slice(&rgb);
    };
    for y in 40..height as i32 - 16 {
        for hour in 0..5 {
            let x = 48 + hour * ((width as i32 - 64) / 5);
            for yy in 40..height as i32 - 16 {
                put(&mut pixels, x, yy, [230, 226, 216]);
            }
            let _ = y;
        }
    }
    let palette = [[28, 78, 110], [140, 74, 43], [46, 107, 78], [120, 72, 120], [160, 110, 40], [40, 90, 130], [150, 60, 70], [70, 90, 50], [90, 70, 40], [50, 80, 100]];
    for (index, section) in sections.iter().enumerate() {
        let color = palette[index % palette.len()];
        for meeting in section.weekly_meetings() {
            let (Some(start), Some(end)) = (meeting.start, meeting.end) else { continue };
            for day in meeting.days.indices() {
                if !(1..=5).contains(&day) { continue; }
                let x0 = 48 + (day - 1) * ((width as i32 - 64) / 5) + 4;
                let x1 = x0 + ((width as i32 - 64) / 5) - 8;
                let y0 = 40 + ((start.max(8 * 60) - 8 * 60) * (height as i32 - 56) / (12 * 60));
                let y1 = 40 + ((end.min(20 * 60) - 8 * 60) * (height as i32 - 56) / (12 * 60));
                for y in y0..y1.max(y0 + 2) {
                    for x in x0..x1 {
                        put(&mut pixels, x, y, color);
                    }
                }
            }
        }
    }
    encode_png(width, height, &pixels)
}

fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0);
        let start = (y * width * 3) as usize;
        raw.extend_from_slice(&rgb[start..start + (width * 3) as usize]);
    }
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    use std::io::Write;
    let _ = enc.write_all(&raw);
    let compressed = enc.finish().unwrap_or_default();
    let mut png = Vec::new();
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_chunk(&mut png, b"IHDR", &ihdr);
    write_chunk(&mut png, b"IDAT", &compressed);
    write_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_data = kind.to_vec();
    crc_data.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_data).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

pub fn copy_crns(sections: &[CourseSection]) -> String {
    sections.iter().map(|s| s.crn.clone()).collect::<Vec<_>>().join(", ")
}

/// Courses a degree plan offers for this term: the matching semester, then the closest one.
pub fn autofill_courses(plan: &DegreePlan, term: &Term, index: &crate::index::CatalogIndex, taken: &HashSet<CourseKey>) -> Vec<CourseKey> {
    crate::planlogic::schedule_suggestions(term, std::slice::from_ref(plan), &[], None, taken, Some(index))
        .into_iter()
        .next()
        .map(|s| s.courses)
        .unwrap_or_default()
}
