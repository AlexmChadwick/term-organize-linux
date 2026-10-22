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
        Self::from_meetings(
            &section
                .weekly_meetings()
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
        )
    }
    pub fn from_meetings(meetings: &[Meeting]) -> Self {
        let mut mask = Self::new();
        for meeting in meetings {
            mask.insert(meeting);
        }
        mask
    }
    pub fn insert(&mut self, meeting: &Meeting) {
        let (Some(start), Some(end)) = (meeting.start, meeting.end) else {
            return;
        };
        for day in meeting.days.indices() {
            self.insert_span(day, start, end);
        }
    }
    pub fn insert_span(&mut self, day: i32, start: i32, end: i32) {
        if !(0..7).contains(&day) || end <= start {
            return;
        }
        let first = (start / MINUTES_PER_SLOT).max(0);
        let last =
            (SLOTS_PER_DAY).min(((end + MINUTES_PER_SLOT - 1) / MINUTES_PER_SLOT).max(first + 1));
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
        Self::ExcludedByUser,
        Self::StartsTooEarly,
        Self::EndsTooLate,
        Self::FreeDay,
        Self::Honors,
        Self::Online,
        Self::Seats,
        Self::Campus,
        Self::BlockedTime,
        Self::PrefStartsTooEarly,
        Self::PrefEndsTooLate,
        Self::PrefDayOff,
        Self::PrefFormat,
        Self::PrefSeats,
        Self::PrefCampus,
        Self::PrefWorkload,
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
    /// Set when this row is locked to a CRN the catalog does not have for this
    /// course. `sections` is then empty. Generation reports the row as
    /// unavailable and does not substitute another section of the same number.
    pub unavailable_pin: Option<String>,
}

impl ScheduleCourseInput {
    pub fn new(key: CourseKey, required: bool, sections: Vec<CourseSection>) -> Self {
        let credits = sections.first().map(|s| s.course_key()).map(|_| 0);
        let _ = credits;
        Self {
            key,
            is_required: required,
            sections,
            excluded_crns: HashSet::new(),
            min_credits: 0,
            max_credits: 0,
            unavailable_pin: None,
        }
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
            if raw.earliest == i32::MAX {
                None
            } else {
                Some(raw.earliest)
            },
            if raw.class_days == 0 {
                None
            } else {
                Some(raw.latest)
            },
            raw.gap_minutes,
            seats_floor,
            total_seats,
            full,
            weights.score(&raw, full, seats_floor) + points,
            if preferences.is_default() {
                vec![]
            } else {
                reasons_for(sections, preferences)
            },
        )
    }
    fn assemble(
        sections: Vec<CourseSection>,
        days_on_campus: i32,
        class_days: i32,
        earliest_start: Option<i32>,
        latest_end: Option<i32>,
        total_gap_minutes: i32,
        min_seats: i32,
        total_seats: i32,
        full_sections: i32,
        score: f64,
        reasons: Vec<ScheduleReason>,
    ) -> Self {
        let crns: Vec<String> = sections.iter().map(|s| s.crn.clone()).collect();
        let mut sorted_crns = crns.clone();
        sorted_crns.sort();
        let id = sorted_crns.join("-");
        Self {
            crns,
            sections,
            days_on_campus,
            class_days,
            earliest_start,
            latest_end,
            total_gap_minutes,
            min_seats,
            total_seats,
            full_sections,
            score,
            reasons,
            sorted_crns,
            id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CourseReport {
    pub key: CourseKey,
    pub is_required: bool,
    pub total_sections: usize,
    pub remaining_sections: usize,
    pub rejections: HashMap<FilterReason, i32>,
    /// The CRN this row is locked to when the catalog no longer has it.
    /// Empty `total_sections` then means unavailable, not "not offered".
    pub unavailable_pin: Option<String>,
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
        self.courses
            .iter()
            .filter(|c| c.is_required && c.is_blocked())
            .collect()
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
        Self {
            max_results: 5_000,
            max_exhaustive_leaves: 2_500_000,
            max_nodes: 12_000_000,
            time_budget: 0.9,
            sample_leaves: 150_000,
            sample_time_budget: 0.35,
            allow_sampling: true,
        }
    }
}

impl Limits {
    pub fn probe() -> Self {
        Self {
            max_results: 1,
            max_exhaustive_leaves: 300,
            max_nodes: 300_000,
            time_budget: 0.25,
            sample_leaves: 0,
            sample_time_budget: 0.0,
            allow_sampling: false,
        }
    }
    /// Cheap existence checks: a thousand leaves is proof enough, and sampling stays off.
    pub fn blocker_probe() -> Self {
        Self {
            max_results: 1,
            max_exhaustive_leaves: 1_000,
            max_nodes: 250_000,
            time_budget: 0.15,
            sample_leaves: 0,
            sample_time_budget: 0.0,
            allow_sampling: false,
        }
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
            Self::SoftenBlockedTime { label, .. } => {
                format!("Allow classes during “{label}” if needed")
            }
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
        if p.earliest_start
            .as_ref()
            .is_some_and(|l| l.strength == PreferenceStrength::Required)
        {
            result.push(Self::SoftenEarliestStart);
        }
        if p.latest_end
            .as_ref()
            .is_some_and(|l| l.strength == PreferenceStrength::Required)
        {
            result.push(Self::SoftenLatestEnd);
        }
        if !p.days_off.is_empty() && p.days_off_strength == PreferenceStrength::Required {
            result.push(Self::SoftenDaysOff);
        }
        for block in p.active_blocked_times() {
            if block.strength == PreferenceStrength::Required {
                result.push(Self::SoftenBlockedTime {
                    id: block.id,
                    label: block.label.clone(),
                });
            }
        }
        if p.format != FormatPreference::NoPreference
            && p.format_strength == PreferenceStrength::Required
        {
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
        if p.breaks == BreakPreference::AtLeast
            && p.minimum_break_strength == PreferenceStrength::Required
        {
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
        let Some(code) = meeting.building_code.as_ref().or(meeting.building.as_ref()) else {
            return 0;
        };
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
        let (Some(start), Some(end)) = (meeting.start, meeting.end) else {
            continue;
        };
        let in_person = !(section.is_online() || meeting.is_online());
        let place = if in_person {
            buildings.id_for(meeting)
        } else {
            0
        };
        for day in meeting.days.indices() {
            let interval = Interval {
                day: day as u8,
                in_person,
                start,
                end,
                building: place,
            };
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
        Self {
            class_days: 0,
            campus_days: 0,
            earliest: i32::MAX,
            latest: i32::MIN,
            gap_minutes: 0,
            class_day_mask: 0,
            max_day_minutes: 0,
            total_minutes: 0,
            back_to_back: 0,
            tight_transitions: 0,
            lunch_misses: 0,
        }
    }
}

pub fn measure_sections(sections: &[CourseSection]) -> Measure {
    let mut buildings = BuildingTable::default();
    let mut intervals = vec![];
    for section in sections {
        intervals.extend(intervals_for(section, &mut buildings));
    }
    measure(&intervals, &MeasureOptions::default())
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
                if options.travel_minutes > 0
                    && gap < options.travel_minutes
                    && previous.in_person
                    && current.in_person
                    && (previous.building == 0
                        || current.building == 0
                        || previous.building != current.building)
                {
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
        Self {
            base: 100.0,
            per_campus_day: 8.0,
            per_online_day: 2.0,
            gap_divisor: 10.0,
            comfort_hours: true,
            per_full_section: 6.0,
            low_seats_penalty: 2.0,
            per_class_day_bonus: 0.0,
            per_imbalance_minute: 0.0,
            per_day_off_missed: 0.0,
            days_off: DaySet(0),
            per_back_to_back: 0.0,
            per_tight_transition: 0.0,
            per_lunch_miss: 0.0,
        }
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
            score -=
                self.per_day_off_missed * (m.class_day_mask & self.days_off.0).count_ones() as f64;
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

pub fn rejection_reasons(
    section: &CourseSection,
    filters: &ScheduleFilters,
    preferences: &SchedulePreferences,
    excluded: &HashSet<String>,
) -> u32 {
    let mut reasons = 0u32;
    if excluded.contains(&section.crn) {
        reasons |= FilterReason::ExcludedByUser.bit();
    }
    let weekly = section.weekly_meetings();
    if let Some(earliest) = filters.earliest_start {
        if weekly
            .iter()
            .any(|m| m.start.unwrap_or(i32::MAX) < earliest)
        {
            reasons |= FilterReason::StartsTooEarly.bit();
        }
    }
    if let Some(latest) = filters.latest_end {
        if weekly.iter().any(|m| m.end.unwrap_or(0) > latest) {
            reasons |= FilterReason::EndsTooLate.bit();
        }
    }
    if !filters.free_days.is_empty()
        && weekly
            .iter()
            .any(|m| !m.days.is_disjoint(filters.free_days))
    {
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
        if limit.strength == PreferenceStrength::Required
            && weekly
                .iter()
                .any(|m| m.start.unwrap_or(i32::MAX) < limit.minutes)
        {
            reasons |= FilterReason::PrefStartsTooEarly.bit();
        }
    }
    if let Some(limit) = &p.latest_end {
        if limit.strength == PreferenceStrength::Required
            && weekly.iter().any(|m| m.end.unwrap_or(0) > limit.minutes)
        {
            reasons |= FilterReason::PrefEndsTooLate.bit();
        }
    }
    if p.days_off_strength == PreferenceStrength::Required
        && !p.days_off.is_empty()
        && weekly.iter().any(|m| !m.days.is_disjoint(p.days_off))
    {
        reasons |= FilterReason::PrefDayOff.bit();
    }
    for block in p.active_blocked_times() {
        if block.strength == PreferenceStrength::Required
            && weekly.iter().any(|m| {
                m.start
                    .zip(m.end)
                    .is_some_and(|(s, e)| block.overlaps(m.days, s, e))
            })
        {
            reasons |= FilterReason::BlockedTime.bit();
            break;
        }
    }
    if p.format != FormatPreference::NoPreference
        && p.format_strength == PreferenceStrength::Required
        && SchedulePreferences::format_of(section).mismatch(p.format) > 0.0
    {
        reasons |= FilterReason::PrefFormat.bit();
    }
    if let Some(campus) = &p.campus {
        if p.campus_strength == PreferenceStrength::Required
            && !section.is_online()
            && !has_no_campus(section)
            && &section.campus != campus
        {
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

pub fn too_close(
    a: &CourseSection,
    a_meetings: &[Meeting],
    b: &CourseSection,
    b_meetings: &[Meeting],
    minutes: i32,
) -> bool {
    if minutes <= 0 || a.is_online() || b.is_online() {
        return false;
    }
    for x in a_meetings.iter().filter(|m| !m.is_online()) {
        let (Some(xs), Some(xe)) = (x.start, x.end) else {
            continue;
        };
        for y in b_meetings.iter().filter(|m| !m.is_online()) {
            let (Some(ys), Some(ye)) = (y.start, y.end) else {
                continue;
            };
            if x.days.is_disjoint(y.days) {
                continue;
            }
            let gap = if ys >= xe {
                ys - xe
            } else if xs >= ye {
                xs - ye
            } else {
                -1
            };
            if gap < 0 || gap >= minutes {
                continue;
            }
            if !date_windows_overlap(
                x.start_date.as_deref(),
                x.end_date.as_deref(),
                y.start_date.as_deref(),
                y.end_date.as_deref(),
            ) {
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
                let per_hour = if preference == TimePreference::Prefer {
                    PREFERRED_BAND_PER_HOUR
                } else {
                    AVOIDED_BAND_PER_HOUR
                };
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
            if block.overlaps(
                DaySet::from_indices([interval.day as i32]),
                interval.start,
                interval.end,
            ) {
                score -= SOFT_BLOCKED;
            }
        }
    }
    if p.format != FormatPreference::NoPreference {
        score -= FORMAT_MISMATCH * SchedulePreferences::format_of(section).mismatch(p.format);
    }
    if let Some(campus) = &p.campus {
        if p.campus_strength == PreferenceStrength::Preferred
            && !section.is_online()
            && !has_no_campus(section)
            && &section.campus != campus
        {
            score -= CAMPUS_MISMATCH;
        }
    }
    if !p.preferred_instructors.is_empty() || !p.avoided_instructors.is_empty() {
        let names: HashSet<String> = section
            .instructors
            .as_ref()
            .map(|list| list.iter().map(|i| i.name.to_ascii_lowercase()).collect())
            .unwrap_or_default();
        if !names.is_empty() {
            if p.preferred_instructors
                .iter()
                .any(|n| names.contains(&n.to_ascii_lowercase()))
            {
                score += PREFERRED_INSTRUCTOR;
            }
            if p.avoided_instructors
                .iter()
                .any(|n| names.contains(&n.to_ascii_lowercase()))
            {
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
    preference_reasons(sections, p, 4, 2)
}

pub fn preference_reasons(
    sections: &[CourseSection],
    p: &SchedulePreferences,
    max_benefits: usize,
    max_caveats: usize,
) -> Vec<ScheduleReason> {
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
    let pro = |good: &mut Vec<ScheduleReason>, text: String, symbol: &str| {
        good.push(ScheduleReason {
            benefit: true,
            text,
            symbol: symbol.into(),
        })
    };
    let con = |bad: &mut Vec<ScheduleReason>, text: String, symbol: &str| {
        bad.push(ScheduleReason {
            benefit: false,
            text,
            symbol: symbol.into(),
        })
    };
    if has_classes && !p.days_off.is_empty() {
        let used = DaySet(measure.class_day_mask & p.days_off.0);
        let free = DaySet(p.days_off.0 & !used.0);
        if !free.is_empty() {
            pro(&mut good, days_off_phrase(free), "calendar.badge.minus");
        }
        if !used.is_empty() && p.days_off_strength == PreferenceStrength::Preferred {
            let names = used
                .indices()
                .into_iter()
                .map(|i| DaySet::SHORT[i as usize])
                .collect::<Vec<_>>()
                .join(", ");
            con(
                &mut bad,
                format!("Class on {names}"),
                "calendar.badge.exclamationmark",
            );
        }
    }
    if has_classes {
        if let Some(limit) = &p.earliest_start {
            if measure.earliest >= limit.minutes {
                pro(
                    &mut good,
                    format!("No classes before {}", short_time(limit.minutes)),
                    "sunrise",
                );
            } else if limit.strength == PreferenceStrength::Preferred {
                con(
                    &mut bad,
                    format!("Starts {}", label_minutes(measure.earliest, true)),
                    "sunrise",
                );
            }
        }
        if let Some(limit) = &p.latest_end {
            if measure.latest <= limit.minutes {
                pro(
                    &mut good,
                    format!("Done by {}", short_time(limit.minutes)),
                    "sunset",
                );
            } else if limit.strength == PreferenceStrength::Preferred {
                con(
                    &mut bad,
                    format!("Ends {}", label_minutes(measure.latest, true)),
                    "sunset",
                );
            }
        }
    }
    if has_classes {
        match p.day_layout {
            DayLayout::NoPreference => {}
            DayLayout::FewerDays => {
                let days = if measure.campus_days > 0 {
                    measure.campus_days
                } else {
                    measure.class_days
                };
                if days <= 3 {
                    pro(
                        &mut good,
                        if days == 1 {
                            "1 day on campus".into()
                        } else {
                            format!("{days} days on campus")
                        },
                        "calendar",
                    );
                }
            }
            DayLayout::SpreadOut => {
                if measure.class_days >= 4 {
                    pro(
                        &mut good,
                        format!("Spread over {} days", measure.class_days),
                        "calendar",
                    );
                }
            }
        }
        if p.gaps == GapPreference::Compact && measure.gap_minutes <= 30 {
            pro(
                &mut good,
                "Short gaps".into(),
                "rectangle.compress.vertical",
            );
        }
    }
    if has_classes && measure.total_minutes > 0 {
        let minutes_in = |bands: &[TimeBand]| {
            intervals
                .iter()
                .map(|i| bands.iter().map(|b| b.overlap(i.start, i.end)).sum::<i32>())
                .sum::<i32>()
        };
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
                    con(
                        &mut bad,
                        if measure.back_to_back == 1 {
                            "1 back-to-back".into()
                        } else {
                            format!("{} back-to-back", measure.back_to_back)
                        },
                        "figure.walk",
                    );
                }
            }
            BreakPreference::AtLeast => {
                if measure.tight_transitions == 0 {
                    pro(
                        &mut good,
                        "Time to walk between classes".into(),
                        "figure.walk",
                    );
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
                    if set.contains(&instructor.name.to_ascii_lowercase())
                        && !result.contains(&instructor.display_name())
                    {
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
            pro(
                &mut good,
                format!("With {} preferred instructors", liked.len()),
                "person.2.fill",
            );
        }
        let disliked = names(&p.avoided_instructors);
        if let Some(first) = disliked.first() {
            con(
                &mut bad,
                if disliked.len() == 1 {
                    format!("With {first}")
                } else {
                    format!("With {} avoided instructors", disliked.len())
                },
                "person.fill.xmark",
            );
        }
    }
    if p.format != FormatPreference::NoPreference {
        let formats: Vec<SectionFormat> = sections
            .iter()
            .map(SchedulePreferences::format_of)
            .collect();
        let deliveries: Vec<_> = formats
            .into_iter()
            .filter(|f| !matches!(f, SectionFormat::Other))
            .collect();
        if !deliveries.is_empty() {
            if deliveries.iter().all(|f| f.mismatch(p.format) == 0.0) {
                pro(
                    &mut good,
                    format!("All {}", p.format.title().to_ascii_lowercase()),
                    if p.format == FormatPreference::Online {
                        "wifi"
                    } else {
                        "building.2"
                    },
                );
            } else if p.format_strength == PreferenceStrength::Preferred {
                con(
                    &mut bad,
                    format!("Not all {}", p.format.title().to_ascii_lowercase()),
                    "wifi",
                );
            }
        }
    }
    if let Some(campus) = &p.campus {
        let on_campus: Vec<_> = sections
            .iter()
            .filter(|s| !s.is_online() && !has_no_campus(s))
            .collect();
        if !on_campus.is_empty() {
            if on_campus.iter().all(|s| &s.campus == campus) {
                pro(&mut good, format!("{campus} campus"), "mappin.and.ellipse");
            } else if p.campus_strength == PreferenceStrength::Preferred {
                con(
                    &mut bad,
                    "Includes another campus".into(),
                    "mappin.and.ellipse",
                );
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
                con(
                    &mut bad,
                    if waitlisted == 1 {
                        "1 waitlisted section".into()
                    } else {
                        format!("{waitlisted} waitlisted sections")
                    },
                    "chair.lounge",
                );
            }
        }
    }
    for block in p.active_blocked_times() {
        if block.strength != PreferenceStrength::Preferred {
            continue;
        }
        let touches = intervals
            .iter()
            .any(|i| block.overlaps(DaySet::from_indices([i.day as i32]), i.start, i.end));
        if touches {
            con(
                &mut bad,
                format!("Overlaps {}", block.label),
                "calendar.badge.exclamationmark",
            );
        }
    }
    fn unique(reasons: Vec<ScheduleReason>) -> Vec<ScheduleReason> {
        let mut seen = HashSet::new();
        reasons
            .into_iter()
            .filter(|r| seen.insert(r.id()))
            .collect()
    }
    let mut out = unique(good);
    out.truncate(max_benefits);
    let mut caveats = unique(bad);
    caveats.truncate(max_caveats);
    out.extend(caveats);
    out
}

pub fn generate(
    courses: &[ScheduleCourseInput],
    filters: &ScheduleFilters,
    preferences: &SchedulePreferences,
    sort: ScheduleSort,
    limits: &Limits,
) -> GenerationResult {
    let started = Instant::now();
    let mut result = run(courses, filters, &preferences.normalized(), sort, limits);
    result.elapsed = started.elapsed().as_secs_f64();
    result
}

fn run(
    courses: &[ScheduleCourseInput],
    filters: &ScheduleFilters,
    preferences: &SchedulePreferences,
    sort: ScheduleSort,
    limits: &Limits,
) -> GenerationResult {
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
        reports.push(CourseReport {
            key: course.key.clone(),
            is_required: course.is_required,
            total_sections: course.sections.len(),
            remaining_sections: remaining.len(),
            rejections,
            unavailable_pin: course.unavailable_pin.clone(),
        });
        kept.push(remaining);
    }
    let mut diagnostics = ScheduleDiagnostics {
        filters: filters.clone(),
        preferences: preferences.clone(),
        courses: reports,
        conflicting_pairs: vec![],
        count_problem: None,
    };
    let mut empty = GenerationResult {
        schedules: vec![],
        total_found: 0,
        is_exhaustive: true,
        was_cancelled: false,
        diagnostics: diagnostics.clone(),
        elapsed: 0.0,
    };
    if courses.is_empty() {
        return empty;
    }
    let required: Vec<usize> = courses
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is_required)
        .map(|(i, _)| i)
        .collect();
    let optional: Vec<usize> = courses
        .iter()
        .enumerate()
        .filter(|(i, c)| !c.is_required && !kept[*i].is_empty())
        .map(|(i, _)| i)
        .collect();
    if required.iter().any(|i| kept[*i].is_empty()) {
        empty.diagnostics = diagnostics;
        return empty;
    }
    let target = filters.course_count.filter(|n| *n > 0);
    if let Some(target) = target {
        if required.len() as i32 > target {
            diagnostics.count_problem = Some(CountProblem::TooManyRequired {
                required: required.len() as i32,
                wanted: target,
            });
            empty.diagnostics = diagnostics;
            return empty;
        }
        if ((required.len() + optional.len()) as i32) < target {
            diagnostics.count_problem = Some(CountProblem::TooFewCourses {
                available: (required.len() + optional.len()) as i32,
                wanted: target,
            });
            empty.diagnostics = diagnostics;
            return empty;
        }
    }
    let credit_window = preferences
        .credit_target
        .as_ref()
        .map(|t| (t.lower, t.upper));
    if let Some((lower, upper)) = credit_window {
        let required_credits: i32 = required.iter().map(|i| courses[*i].min_credits).sum();
        if required_credits > upper {
            diagnostics.count_problem = Some(CountProblem::CreditsOverTarget {
                required: required_credits,
                limit: upper,
            });
            empty.diagnostics = diagnostics;
            return empty;
        }
        let available: i32 = required
            .iter()
            .chain(optional.iter())
            .map(|i| courses[*i].max_credits)
            .sum();
        if available < lower {
            diagnostics.count_problem = Some(CountProblem::CreditsUnderTarget {
                available,
                wanted: lower,
            });
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
            let score = if has_preferences {
                section_score(section, &own, preferences)
            } else {
                0.0
            };
            candidates.push((offset, section, own, score));
        }
        if has_preferences {
            candidates.sort_by(|a, b| {
                b.3.partial_cmp(&a.3)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.0.cmp(&b.0))
            });
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
        slots.push(Slot {
            course_index: *course_index,
            is_required: courses[*course_index].is_required,
            start: first,
            size: sections.len() - first,
        });
        slot_min.push(courses[*course_index].min_credits);
        slot_max.push(
            courses[*course_index]
                .min_credits
                .max(courses[*course_index].max_credits),
        );
    }
    interval_start.push(intervals.len());
    let travel = if preferences.breaks == BreakPreference::AtLeast
        && preferences.minimum_break_strength == PreferenceStrength::Required
    {
        preferences.minimum_break_minutes
    } else {
        0
    };
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
    search.run_exhaustive(
        &intervals,
        &interval_start,
        &points,
        &weights,
        &options,
        sort,
    );
    if search.hit_limit && limits.allow_sampling && limits.sample_leaves > 0 {
        search.run_sampled(
            &intervals,
            &interval_start,
            &points,
            &weights,
            &options,
            sort,
        );
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
    GenerationResult {
        schedules,
        total_found: total,
        is_exhaustive: exhaustive,
        was_cancelled: false,
        diagnostics,
        elapsed: 0.0,
    }
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
    fn new(
        slots: Vec<Slot>,
        masks: Vec<ScheduleSlotMask>,
        weekly: Vec<Vec<Meeting>>,
        sections: Vec<CourseSection>,
        rules: SearchRules,
        target: Option<i32>,
        limits: &Limits,
        seed: u64,
    ) -> Self {
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
                    if rules.travel > 0
                        && too_close(
                            &sections[g],
                            &weekly[g],
                            &sections[h],
                            &weekly[h],
                            rules.travel,
                        )
                    {
                        continue;
                    }
                    bits[j / 64] |= 1u64 << (j % 64);
                }
                compat[g][s] = bits;
            }
        }
        Self {
            slots,
            masks,
            weekly,
            sections,
            compat,
            rules,
            target,
            limits: limits.clone(),
            seed,
            sort: ScheduleSort::Best,
            leaf_count: 0,
            hit_limit: false,
            collector: vec![],
            seq: 0,
        }
    }

    fn conflicting_pairs(&self, courses: &[ScheduleCourseInput]) -> Vec<ConflictPair> {
        let mut pairs = vec![];
        for a in 0..self.slots.len() {
            for b in (a + 1)..self.slots.len() {
                if !self.slots[a].is_required || !self.slots[b].is_required {
                    continue;
                }
                if !self.has_compatible(a, b) {
                    let reason = if self.rules.travel > 0 && self.has_overlap_free(a, b) {
                        ConflictReason::TravelTime
                    } else {
                        ConflictReason::Overlap
                    };
                    pairs.push(ConflictPair {
                        first: courses[self.slots[a].course_index].key.clone(),
                        second: courses[self.slots[b].course_index].key.clone(),
                        reason,
                    });
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
                if !(self.masks[g].conflicts(self.masks[h])
                    && meetings_conflict(&self.weekly[g], &self.weekly[h]))
                {
                    return true;
                }
            }
        }
        false
    }

    fn run_exhaustive(
        &mut self,
        intervals: &[Interval],
        interval_start: &[usize],
        points: &[f64],
        weights: &Weights,
        options: &MeasureOptions,
        sort: ScheduleSort,
    ) {
        self.sort = sort;
        self.leaf_count = 0;
        self.hit_limit = false;
        self.collector.clear();
        let domain = self.full_domain();
        let deadline =
            Instant::now() + std::time::Duration::from_secs_f64(self.limits.time_budget.max(0.05));
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
        self.descend(
            0,
            &domain,
            &mut ctx,
            false,
            self.limits.max_exhaustive_leaves,
            deadline,
            intervals,
            interval_start,
            points,
            weights,
            options,
            sort,
            &mut SplitMix::new(self.seed),
        );
    }

    fn run_sampled(
        &mut self,
        intervals: &[Interval],
        interval_start: &[usize],
        points: &[f64],
        weights: &Weights,
        options: &MeasureOptions,
        sort: ScheduleSort,
    ) {
        self.leaf_count = 0;
        let domain = self.full_domain();
        let deadline = Instant::now()
            + std::time::Duration::from_secs_f64(self.limits.sample_time_budget.max(0.05));
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
        let mut rng = SplitMix::new(self.seed);
        self.descend(
            0,
            &domain,
            &mut ctx,
            true,
            self.limits.sample_leaves,
            deadline,
            intervals,
            interval_start,
            points,
            weights,
            options,
            sort,
            &mut rng,
        );
    }

    fn full_domain(&self) -> Vec<Vec<u64>> {
        self.slots
            .iter()
            .map(|slot| {
                let words = (slot.size + 63) / 64;
                let mut bits = vec![0u64; words.max(1)];
                for i in 0..slot.size {
                    bits[i / 64] |= 1u64 << (i % 64);
                }
                bits
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn descend(
        &mut self,
        depth: usize,
        domain: &[Vec<u64>],
        ctx: &mut Walk,
        sampled: bool,
        max_leaves: usize,
        deadline: Instant,
        intervals: &[Interval],
        interval_start: &[usize],
        points: &[f64],
        weights: &Weights,
        options: &MeasureOptions,
        sort: ScheduleSort,
        rng: &mut SplitMix,
    ) {
        if ctx.aborted {
            return;
        }
        ctx.nodes += 1;
        if ctx.nodes & 0x3ff == 0
            && (ctx.nodes >= self.limits.max_nodes || Instant::now() >= deadline)
        {
            self.hit_limit = true;
            ctx.aborted = true;
            return;
        }
        if depth == self.slots.len() {
            self.record(
                depth,
                domain,
                ctx,
                intervals,
                interval_start,
                points,
                weights,
                options,
                sort,
            );
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
                self.record(
                    depth,
                    domain,
                    ctx,
                    intervals,
                    interval_start,
                    points,
                    weights,
                    options,
                    sort,
                );
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
                if local < self.slots[depth].size
                    && self.can_add(self.slots[depth].start + local, depth, ctx)
                {
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
                self.try_option(
                    depth,
                    option,
                    domain,
                    ctx,
                    true,
                    remaining,
                    deadline,
                    intervals,
                    interval_start,
                    points,
                    weights,
                    options,
                    sort,
                    rng,
                );
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
            self.try_option(
                depth,
                option,
                domain,
                ctx,
                false,
                max_leaves,
                deadline,
                intervals,
                interval_start,
                points,
                weights,
                options,
                sort,
                rng,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn try_option(
        &mut self,
        depth: usize,
        option: i32,
        domain: &[Vec<u64>],
        ctx: &mut Walk,
        sampled: bool,
        max_leaves: usize,
        deadline: Instant,
        intervals: &[Interval],
        interval_start: &[usize],
        points: &[f64],
        weights: &Weights,
        options: &MeasureOptions,
        sort: ScheduleSort,
        rng: &mut SplitMix,
    ) {
        if option < 0 {
            ctx.picks[depth] = -1;
            self.descend(
                depth + 1,
                domain,
                ctx,
                sampled,
                max_leaves,
                deadline,
                intervals,
                interval_start,
                points,
                weights,
                options,
                sort,
                rng,
            );
            return;
        }
        let local = option as usize;
        let g = self.slots[depth].start + local;
        let Some(next) = self.forward(domain, depth, g) else {
            return;
        };
        if !self.can_add(g, depth, ctx) {
            return;
        }
        self.take(g, depth, ctx);
        ctx.picks[depth] = option;
        ctx.chosen += 1;
        self.descend(
            depth + 1,
            &next,
            ctx,
            sampled,
            max_leaves,
            deadline,
            intervals,
            interval_start,
            points,
            weights,
            options,
            sort,
            rng,
        );
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
        if self.rules.max_classes == i32::MAX
            && self.rules.max_minutes == i32::MAX
            && self.rules.credit_window.is_none()
        {
            return domain[slot].iter().any(|w| *w != 0);
        }
        for (word_index, mut bits) in domain[slot].iter().copied().enumerate() {
            while bits != 0 {
                let local = word_index * 64 + bits.trailing_zeros() as usize;
                bits &= bits.wrapping_sub(1);
                if local < self.slots[slot].size
                    && self.can_add(self.slots[slot].start + local, slot, ctx)
                {
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
    fn record(
        &mut self,
        depth: usize,
        domain: &[Vec<u64>],
        ctx: &Walk,
        intervals: &[Interval],
        interval_start: &[usize],
        points: &[f64],
        weights: &Weights,
        options: &MeasureOptions,
        sort: ScheduleSort,
    ) {
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
                if !self.slots[s].is_required
                    && ctx.picks[s] < 0
                    && self.can_still_add(domain, s, ctx)
                {
                    return;
                }
            }
        }
        let _ = depth;
        self.offer(
            ctx,
            intervals,
            interval_start,
            points,
            weights,
            options,
            sort,
        );
        self.leaf_count += 1;
    }

    fn offer(
        &mut self,
        ctx: &Walk,
        intervals: &[Interval],
        interval_start: &[usize],
        points: &[f64],
        weights: &Weights,
        options: &MeasureOptions,
        sort: ScheduleSort,
    ) {
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
        let latest = if raw.latest == i32::MIN {
            0
        } else {
            raw.latest
        };
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

    fn materialize(
        &self,
        has_preferences: bool,
        preferences: &SchedulePreferences,
    ) -> Vec<GeneratedSchedule> {
        let mut kept = self.collector.clone();
        let sort = self.sort;
        kept.sort_by(|a, b| {
            let ab = precedes(sort, a, b);
            let ba = precedes(sort, b, a);
            if ab && !ba {
                std::cmp::Ordering::Less
            } else if ba && !ab {
                std::cmp::Ordering::Greater
            } else {
                a.seq.cmp(&b.seq)
            }
        });
        let mut out = vec![];
        for leaf in kept {
            let mut picked = vec![];
            for (s, pick) in leaf.picks.iter().enumerate() {
                if *pick < 0 {
                    continue;
                }
                picked.push((
                    self.slots[s].course_index,
                    self.slots[s].start + *pick as usize,
                ));
            }
            picked.sort_by_key(|p| p.0);
            let chosen: Vec<CourseSection> = picked
                .iter()
                .map(|(_, id)| self.sections[*id].clone())
                .collect();
            out.push(GeneratedSchedule::assemble(
                chosen.clone(),
                leaf.campus_days,
                leaf.class_days,
                if leaf.earliest == i32::MAX {
                    None
                } else {
                    Some(leaf.earliest)
                },
                if leaf.class_days == 0 {
                    None
                } else {
                    Some(leaf.latest)
                },
                leaf.gap,
                leaf.min_seats,
                leaf.total_seats,
                leaf.full,
                leaf.score,
                if has_preferences {
                    reasons_for(&chosen, preferences)
                } else {
                    vec![]
                },
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
            if a.campus_days != b.campus_days {
                return a.campus_days < b.campus_days;
            }
            if a.class_days != b.class_days {
                return a.class_days < b.class_days;
            }
            if a.gap != b.gap {
                return a.gap < b.gap;
            }
        }
        ScheduleSort::LatestStart => {
            if a.earliest != b.earliest {
                return a.earliest > b.earliest;
            }
            if a.campus_days != b.campus_days {
                return a.campus_days < b.campus_days;
            }
        }
        ScheduleSort::EarliestFinish => {
            if a.latest != b.latest {
                return a.latest < b.latest;
            }
            if a.campus_days != b.campus_days {
                return a.campus_days < b.campus_days;
            }
        }
        ScheduleSort::FewestGaps => {
            if a.gap != b.gap {
                return a.gap < b.gap;
            }
            if a.campus_days != b.campus_days {
                return a.campus_days < b.campus_days;
            }
        }
        ScheduleSort::MostSeats => {
            if a.min_seats != b.min_seats {
                return a.min_seats > b.min_seats;
            }
            if a.total_seats != b.total_seats {
                return a.total_seats > b.total_seats;
            }
        }
    }
    if a.score != b.score {
        return a.score > b.score;
    }
    a.seq < b.seq
}

fn meetings_conflict(a: &[Meeting], b: &[Meeting]) -> bool {
    a.iter()
        .any(|first| b.iter().any(|second| first.overlaps(second)))
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

pub fn suggest_relaxations(
    courses: &[ScheduleCourseInput],
    filters: &ScheduleFilters,
    preferences: &SchedulePreferences,
    include_courses: bool,
) -> Vec<Suggestion> {
    let mut candidates: Vec<(
        Relaxation,
        ScheduleFilters,
        SchedulePreferences,
        Vec<ScheduleCourseInput>,
    )> = vec![];
    let add = |list: &mut Vec<_>, relaxation: Relaxation, active: bool| {
        if !active {
            return;
        }
        let mut relaxed = filters.clone();
        relaxation.apply_filters(&mut relaxed);
        list.push((relaxation, relaxed, preferences.clone(), courses.to_vec()));
    };
    add(
        &mut candidates,
        Relaxation::ClearEarliestStart,
        filters.earliest_start.is_some(),
    );
    add(
        &mut candidates,
        Relaxation::ClearLatestEnd,
        filters.latest_end.is_some(),
    );
    add(
        &mut candidates,
        Relaxation::ClearFreeDays,
        !filters.free_days.is_empty(),
    );
    add(
        &mut candidates,
        Relaxation::AllowHonors,
        !filters.include_honors,
    );
    add(
        &mut candidates,
        Relaxation::AllowOnline,
        !filters.include_online,
    );
    add(
        &mut candidates,
        Relaxation::AllowFullSections,
        filters.hide_full_sections || filters.min_seats > 0,
    );
    add(
        &mut candidates,
        Relaxation::ClearCampus,
        filters.campus.is_some(),
    );
    add(
        &mut candidates,
        Relaxation::ClearCourseCount,
        filters.course_count.is_some(),
    );
    for relaxation in PreferenceRelaxation::available(preferences) {
        let mut relaxed = preferences.clone();
        relaxation.apply(&mut relaxed);
        candidates.push((
            Relaxation::Preference(relaxation),
            filters.clone(),
            relaxed,
            courses.to_vec(),
        ));
    }
    if include_courses && courses.len() > 1 {
        for (index, course) in courses.iter().enumerate() {
            if !course.is_required {
                continue;
            }
            let mut relaxed = courses.to_vec();
            relaxed[index].is_required = false;
            candidates.push((
                Relaxation::MakeOptional(course.key.clone()),
                filters.clone(),
                preferences.clone(),
                relaxed,
            ));
        }
    }
    let mut suggestions = vec![];
    let probe = Limits::probe();
    for (relaxation, filters, preferences, courses) in candidates {
        let result = generate(&courses, &filters, &preferences, ScheduleSort::Best, &probe);
        if result.total_found > 0 {
            suggestions.push(Suggestion {
                relaxation,
                schedule_count: result.total_found,
                is_lower_bound: !result.is_exhaustive,
            });
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
    if diagnostics.count_problem.is_some()
        || !diagnostics.blocked_required().is_empty()
        || !diagnostics.conflicting_pairs.is_empty()
    {
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
            let (Some(start), Some(end)) = (meeting.start, meeting.end) else {
                continue;
            };
            let Some(first) = meeting
                .start_date
                .clone()
                .or_else(|| Some("2026-09-09".into()))
            else {
                continue;
            };
            let last = meeting.end_date.clone().unwrap_or_else(|| first.clone());
            lines.push("BEGIN:VEVENT".into());
            lines.push(format!(
                "UID:{}-{}-{n}@neuclasssearch",
                plan_id.to_ascii_lowercase(),
                section.crn
            ));
            lines.push(format!("DTSTAMP:{stamp}"));
            lines.push(format!(
                "DTSTART;TZID=America/New_York:{}T{}",
                first.replace('-', ""),
                hhmm(start)
            ));
            lines.push(format!(
                "DTEND;TZID=America/New_York:{}T{}",
                first.replace('-', ""),
                hhmm(end)
            ));
            let days = meeting
                .days
                .indices()
                .into_iter()
                .map(|d| codes[d as usize])
                .collect::<Vec<_>>()
                .join(",");
            lines.push(format!(
                "RRULE:FREQ=WEEKLY;BYDAY={days};UNTIL={}",
                until_stamp(&last)
            ));
            lines.push(format!(
                "SUMMARY:{}",
                escape_ics(&format!("{} {}", section.course_key(), section.title))
            ));
            let location = if meeting.is_online() || section.is_online() {
                Some("Online".into())
            } else {
                let label = meeting.long_location_label();
                if label == "Location TBA" {
                    None
                } else {
                    Some(label)
                }
            };
            if let Some(location) = location {
                lines.push(format!("LOCATION:{}", escape_ics(&location)));
            }
            lines.push(format!(
                "DESCRIPTION:{}",
                escape_ics(&format!("CRN {}", section.crn))
            ));
            lines.push("TRANSP:OPAQUE".into());
            lines.push("END:VEVENT".into());
        }
    }
    lines.push("END:VCALENDAR".into());
    lines
        .into_iter()
        .map(fold_ics)
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

fn hhmm(minutes: i32) -> String {
    format!(
        "{:02}{:02}00",
        minutes.div_euclid(60).rem_euclid(24),
        minutes.rem_euclid(60)
    )
}
fn escape_ics(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
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
    let (day, hour) = if hour >= 24 {
        (d + 1, hour - 24)
    } else {
        (d, hour)
    };
    format!("{y:04}{m:02}{day:02}T{hour:02}5959Z")
}
fn is_edt(year: i32, month: i32, day: i32) -> bool {
    if (4..11).contains(&month) {
        return true;
    }
    if month == 3 {
        return day >= nth_weekday(year, 3, 0, 2);
    }
    if month == 11 {
        return day < nth_weekday(year, 11, 0, 1);
    }
    false
}
fn nth_weekday(year: i32, month: i32, weekday: i32, n: i32) -> i32 {
    let mut day = 1;
    let mut seen = 0;
    while day < 32 {
        if civil_weekday(year, month, day) == weekday {
            seen += 1;
            if seen == n {
                return day;
            }
        }
        day += 1;
    }
    1
}
fn civil_weekday(year: i32, month: i32, day: i32) -> i32 {
    let mut y = year;
    let mut m = month;
    if m < 3 {
        y -= 1;
        m += 12;
    }
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
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
            return;
        }
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
    let palette = [
        [28, 78, 110],
        [140, 74, 43],
        [46, 107, 78],
        [120, 72, 120],
        [160, 110, 40],
        [40, 90, 130],
        [150, 60, 70],
        [70, 90, 50],
        [90, 70, 40],
        [50, 80, 100],
    ];
    for (index, section) in sections.iter().enumerate() {
        let color = palette[index % palette.len()];
        for meeting in section.weekly_meetings() {
            let (Some(start), Some(end)) = (meeting.start, meeting.end) else {
                continue;
            };
            for day in meeting.days.indices() {
                if !(1..=5).contains(&day) {
                    continue;
                }
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
    sections
        .iter()
        .map(|s| s.crn.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Courses a degree plan offers for this term: the matching semester, then the closest one.
pub fn autofill_courses(
    plan: &DegreePlan,
    term: &Term,
    index: &crate::index::CatalogIndex,
    taken: &HashSet<CourseKey>,
) -> Vec<CourseKey> {
    crate::planlogic::schedule_suggestions(
        term,
        std::slice::from_ref(plan),
        &[],
        None,
        taken,
        Some(index),
    )
    .into_iter()
    .next()
    .map(|s| s.courses)
    .unwrap_or_default()
}

// MARK: - What a plan resolves to, and the words around an empty search

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    Error,
    Warning,
    Neutral,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleWarning {
    pub text: String,
    pub symbol: String,
    pub severity: Attention,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeFix {
    RemoveSections(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleNotice {
    pub id: String,
    pub severity: Attention,
    pub title: String,
    pub lines: Vec<String>,
    pub fix: Option<NoticeFix>,
}

impl ScheduleNotice {
    pub fn fix_title(&self) -> Option<&'static str> {
        self.fix.as_ref().map(|_| "Remove")
    }
}

#[derive(Clone, Debug)]
pub struct PlanEntry {
    pub plan_course: PlanCourse,
    pub course: Option<Course>,
    pub all_sections: Vec<CourseSection>,
    pub selected: Vec<CourseSection>,
    pub catalog_available: bool,
}

impl PlanEntry {
    pub fn key(&self) -> &CourseKey {
        &self.plan_course.key
    }
    pub fn is_missing(&self) -> bool {
        self.catalog_available && self.course.is_none() && self.all_sections.is_empty()
    }
    pub fn has_no_sections(&self) -> bool {
        self.catalog_available && self.course.is_some() && self.all_sections.is_empty()
    }
    pub fn course_crns(&self) -> HashSet<String> {
        self.all_sections.iter().map(|s| s.crn.clone()).collect()
    }
    pub fn counts_towards_credits(&self) -> bool {
        self.plan_course.is_required || !self.selected.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct ScheduleConflict {
    pub first: CourseSection,
    pub second: CourseSection,
    pub days: DaySet,
    pub start: i32,
    pub end: i32,
}

impl ScheduleConflict {
    pub fn summary(&self) -> String {
        format!(
            "{} overlaps {} · {} {}",
            self.first.course_key(),
            self.second.course_key(),
            self.days.short_label(),
            range_label(self.start, self.end)
        )
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedPlan {
    pub plan: SchedulePlan,
    pub has_catalog: bool,
    pub entries: Vec<PlanEntry>,
    pub unlisted_crns: Vec<String>,
    pub orphaned_crns: Vec<String>,
    pub conflicts: Vec<ScheduleConflict>,
    pub full_sections: Vec<CourseSection>,
    pub missing_courses: Vec<CourseKey>,
    pub credits_min: f64,
    pub credits_max: f64,
}

impl ResolvedPlan {
    pub fn resolve(plan: &SchedulePlan, index: Option<&crate::index::CatalogIndex>) -> Self {
        let mut by_section = HashMap::new();
        if let Some(index) = index {
            for crn in &plan.selected_crns {
                if let Some(section) = index.section_crn(crn) {
                    by_section.insert(crn.clone(), section.clone());
                }
            }
        }
        let pins = pinned_sections(plan, index);
        let mut entries = vec![];
        let mut claimed = HashSet::new();
        for (plan_course, selected) in plan.courses.iter().zip(pins.iter()) {
            for section in selected {
                claimed.insert(section.crn.clone());
            }
            if let Some(crn) = plan_course.pinned_crn.as_deref() {
                claimed.insert(crn.to_string());
            }
            let all = index
                .map(|index| index.sections_for(&plan_course.key))
                .unwrap_or_default();
            entries.push(PlanEntry {
                plan_course: plan_course.clone(),
                course: index.and_then(|index| index.course(&plan_course.key).cloned()),
                all_sections: all,
                selected: selected.clone(),
                catalog_available: index.is_some(),
            });
        }
        let unclaimed: Vec<String> = if index.is_none() {
            vec![]
        } else {
            plan.selected_crns
                .iter()
                .filter(|crn| !claimed.contains(*crn))
                .cloned()
                .collect()
        };
        let unlisted_crns = unclaimed
            .iter()
            .filter(|crn| !by_section.contains_key(*crn))
            .cloned()
            .collect();
        let orphaned_crns = unclaimed
            .iter()
            .filter(|crn| by_section.contains_key(*crn))
            .cloned()
            .collect();
        let sections: Vec<CourseSection> =
            entries.iter().flat_map(|e| e.selected.clone()).collect();
        let mut full_sections = vec![];
        for entry in &entries {
            for section in &entry.selected {
                if section.is_full() {
                    full_sections.push(section.clone());
                }
            }
        }
        let conflicts = find_conflicts(&sections);
        let missing_courses = entries
            .iter()
            .filter(|e| e.is_missing() || e.has_no_sections())
            .map(|e| e.key().clone())
            .collect();
        let mut credits_min = 0.0;
        let mut credits_max = 0.0;
        for entry in &entries {
            if !entry.counts_towards_credits() {
                continue;
            }
            if let Some(course) = &entry.course {
                let copies = if course.special_topics && !entry.selected.is_empty() {
                    entry.selected.len() as f64
                } else {
                    1.0
                };
                credits_min += course.min_credits * copies;
                credits_max += course.max_credits * copies;
            }
        }
        Self {
            plan: plan.clone(),
            has_catalog: index.is_some(),
            entries,
            unlisted_crns,
            orphaned_crns,
            conflicts,
            full_sections,
            missing_courses,
            credits_min,
            credits_max,
        }
    }

    pub fn courses_without_section(&self) -> Vec<&PlanEntry> {
        self.entries
            .iter()
            .filter(|e| e.selected.is_empty() && !e.is_missing() && !e.has_no_sections())
            .collect()
    }

    pub fn is_over_credit_limit(&self) -> bool {
        self.credits_min > 19.0
    }

    pub fn warnings(&self) -> Vec<ScheduleWarning> {
        if !self.has_catalog {
            return vec![];
        }
        let mut items = vec![];
        if !self.conflicts.is_empty() {
            let n = self.conflicts.len();
            items.push(ScheduleWarning {
                text: if n == 1 {
                    "1 conflict".into()
                } else {
                    format!("{n} conflicts")
                },
                symbol: "exclamationmark.triangle.fill".into(),
                severity: Attention::Error,
            });
        }
        if !self.full_sections.is_empty() {
            let n = self.full_sections.len();
            items.push(ScheduleWarning {
                text: if n == 1 {
                    "1 full section".into()
                } else {
                    format!("{n} full sections")
                },
                symbol: "chair".into(),
                severity: Attention::Warning,
            });
        }
        if !self.missing_courses.is_empty() {
            let n = self.missing_courses.len();
            items.push(ScheduleWarning {
                text: if n == 1 {
                    "1 course not offered".into()
                } else {
                    format!("{n} courses not offered")
                },
                symbol: "questionmark.folder".into(),
                severity: Attention::Warning,
            });
        }
        if self.is_over_credit_limit() {
            items.push(ScheduleWarning {
                text: "Over 19 credits".into(),
                symbol: "exclamationmark.circle".into(),
                severity: Attention::Warning,
            });
        }
        let missing_sections = self
            .courses_without_section()
            .iter()
            .filter(|e| e.plan_course.is_required)
            .count();
        if missing_sections > 0 && !self.plan.courses.is_empty() {
            items.push(ScheduleWarning {
                text: if missing_sections == 1 {
                    "1 course needs a section".into()
                } else {
                    format!("{missing_sections} courses need sections")
                },
                symbol: "circle.dashed".into(),
                severity: Attention::Neutral,
            });
        }
        items
    }
}

pub fn section_title(section: &CourseSection) -> String {
    if section.section_number.is_empty() {
        section.class_type.clone()
    } else {
        format!("{} {}", section.class_type, section.section_number)
    }
}

fn find_conflicts(sections: &[CourseSection]) -> Vec<ScheduleConflict> {
    let mut result = vec![];
    for i in 0..sections.len() {
        for j in (i + 1)..sections.len() {
            if let Some((days, start, end)) = section_overlap(&sections[i], &sections[j]) {
                result.push(ScheduleConflict {
                    first: sections[i].clone(),
                    second: sections[j].clone(),
                    days,
                    start,
                    end,
                });
            }
        }
    }
    result
}

fn section_overlap(a: &CourseSection, b: &CourseSection) -> Option<(DaySet, i32, i32)> {
    let mut days = DaySet(0);
    let mut window: Option<(i32, i32)> = None;
    for m1 in a.weekly_meetings() {
        for m2 in b.weekly_meetings() {
            if !m1.overlaps(m2) {
                continue;
            }
            let (Some(s1), Some(e1), Some(s2), Some(e2)) = (m1.start, m1.end, m2.start, m2.end)
            else {
                continue;
            };
            days = days.union(m1.days.intersection(m2.days));
            let start = s1.max(s2);
            let end = e1.min(e2);
            if window.is_none_or(|current| start < current.0) {
                window = Some((start, end));
            }
        }
    }
    window.map(|(start, end)| (days, start, end))
}

pub fn make_notices(
    resolved: &ResolvedPlan,
    term_name: &str,
    catalog_unavailable: bool,
) -> Vec<ScheduleNotice> {
    let mut items = vec![];
    if !resolved.has_catalog && catalog_unavailable && !resolved.plan.courses.is_empty() {
        items.push(ScheduleNotice {
            id: "catalog".into(),
            severity: Attention::Warning,
            title: format!("Course data for {term_name} isn’t on this device"),
            lines: vec![
                "Switch to that term and download its catalog to see sections, times and seats."
                    .into(),
            ],
            fix: None,
        });
    }
    if !resolved.conflicts.is_empty() {
        let count = resolved.conflicts.len();
        let mut lines: Vec<String> = resolved
            .conflicts
            .iter()
            .take(3)
            .map(|c| c.summary())
            .collect();
        if count > 3 {
            lines.push(format!("and {} more", count - 3));
        }
        items.push(ScheduleNotice {
            id: "conflicts".into(),
            severity: Attention::Error,
            title: if count == 1 {
                "1 time conflict".into()
            } else {
                format!("{count} time conflicts")
            },
            lines,
            fix: None,
        });
    }
    if !resolved.full_sections.is_empty() {
        let lines = resolved
            .full_sections
            .iter()
            .map(|section| {
                let waitlist = if section.waitlist_capacity > 0 {
                    format!(" · {} waitlist spots left", section.waitlist_remaining)
                } else {
                    String::new()
                };
                format!(
                    "{} {} is full{waitlist}",
                    section.course_key(),
                    section_title(section)
                )
            })
            .collect();
        let n = resolved.full_sections.len();
        items.push(ScheduleNotice {
            id: "full".into(),
            severity: Attention::Warning,
            title: if n == 1 {
                "1 section is full".into()
            } else {
                format!("{n} sections are full")
            },
            lines,
            fix: None,
        });
    }
    if !resolved.missing_courses.is_empty() {
        let n = resolved.missing_courses.len();
        items.push(ScheduleNotice {
            id: "missing".into(),
            severity: Attention::Warning,
            title: if n == 1 {
                format!("1 course isn’t offered in {term_name}")
            } else {
                format!("{n} courses aren’t offered in {term_name}")
            },
            lines: resolved
                .missing_courses
                .iter()
                .map(|key| format!("{key} has no sections this term."))
                .collect(),
            fix: None,
        });
    }
    if resolved.is_over_credit_limit() {
        items.push(ScheduleNotice {
            id: "credits".into(),
            severity: Attention::Warning,
            title: format!(
                "{} is a heavy load",
                credit_span(resolved.credits_min, resolved.credits_max)
            ),
            lines: vec![
                "Above 19 credits you may need approval from your advisor or college.".into(),
            ],
            fix: None,
        });
    }
    if !resolved.unlisted_crns.is_empty() {
        let unlisted = resolved.unlisted_crns.clone();
        let n = unlisted.len();
        items.push(ScheduleNotice {
            id: "unlisted".into(),
            severity: Attention::Info,
            title: if n == 1 {
                "A saved section is no longer listed".into()
            } else {
                format!("{n} saved sections are no longer listed")
            },
            lines: vec![format!(
                "CRN {} isn’t in the current {term_name} catalog.",
                unlisted.join(", ")
            )],
            fix: Some(NoticeFix::RemoveSections(unlisted)),
        });
    }
    if !resolved.orphaned_crns.is_empty() {
        let orphaned = resolved.orphaned_crns.clone();
        let n = orphaned.len();
        items.push(ScheduleNotice {
            id: "orphaned".into(),
            severity: Attention::Info,
            title: if n == 1 {
                "A section from a removed course is still selected".into()
            } else {
                format!("{n} sections from removed courses are still selected")
            },
            lines: vec![format!(
                "CRN {} doesn’t belong to any course in this schedule.",
                orphaned.join(", ")
            )],
            fix: Some(NoticeFix::RemoveSections(orphaned)),
        });
    }
    items
}

fn credit_span(min: f64, max: f64) -> String {
    if (min - max).abs() < 1e-9 {
        format!("{} credits", credit_number(min))
    } else {
        format!("{}–{} credits", credit_number(min), credit_number(max))
    }
}

fn credit_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// Sections generation is allowed to use for each plan row.
///
/// A chosen CRN is the only candidate for that row. Two special-topics
/// sections that share a course number stay two candidates, and a CRN is
/// given to at most one row.
pub fn pinned_sections(
    plan: &SchedulePlan,
    index: Option<&crate::index::CatalogIndex>,
) -> Vec<Vec<CourseSection>> {
    let Some(index) = index else {
        return vec![Vec::new(); plan.courses.len()];
    };
    let mut claimed = HashSet::new();
    let mut rows = Vec::with_capacity(plan.courses.len());
    for (row_index, course) in plan.courses.iter().enumerate() {
        if let Some(crn) = crate::planlogic::chosen_crn(course.pinned_crn.as_deref()) {
            let mut selected = Vec::new();
            if claimed.insert(crn.to_string()) {
                if let Some(section) = index.section_crn(crn) {
                    if section.course_key() == course.key {
                        selected.push(section.clone());
                    }
                }
            }
            rows.push(selected);
            continue;
        }
        let later = plan
            .courses
            .iter()
            .skip(row_index + 1)
            .filter(|other| other.key == course.key && other.pinned_crn.is_none())
            .count();
        let mut selected = Vec::new();
        for crn in &plan.selected_crns {
            if claimed.contains(crn) {
                continue;
            }
            let Some(section) = index.section_crn(crn) else {
                continue;
            };
            if section.course_key() != course.key {
                continue;
            }
            claimed.insert(crn.clone());
            selected.push(section.clone());
            if later > 0 {
                break;
            }
        }
        rows.push(selected);
    }
    rows
}

fn input_for(
    course: &PlanCourse,
    sections: Vec<CourseSection>,
    index: &crate::index::CatalogIndex,
) -> ScheduleCourseInput {
    let listed = index.course(&course.key);
    ScheduleCourseInput {
        key: course.key.clone(),
        is_required: course.is_required,
        sections,
        excluded_crns: course.excluded_crns.clone(),
        min_credits: listed.map(|c| c.min_credits.ceil() as i32).unwrap_or(0),
        max_credits: listed.map(|c| c.max_credits.ceil() as i32).unwrap_or(0),
        unavailable_pin: None,
    }
}

fn locked_input(
    course: &PlanCourse,
    index: &crate::index::CatalogIndex,
    pin: &str,
) -> ScheduleCourseInput {
    let mut input = input_for(course, Vec::new(), index);
    input.unavailable_pin = Some(pin.to_string());
    input
}

pub fn inputs_for(
    plan: &SchedulePlan,
    index: &crate::index::CatalogIndex,
) -> Vec<ScheduleCourseInput> {
    let pins = pinned_sections(plan, Some(index));
    let claimed: HashSet<String> = pins
        .iter()
        .flatten()
        .map(|section| section.crn.clone())
        .collect();
    let mut inputs = Vec::new();
    for (row_index, (course, selected)) in plan.courses.iter().zip(pins).enumerate() {
        let topics = index
            .course(&course.key)
            .is_some_and(|listed| listed.special_topics);
        if let Some(crn) = crate::planlogic::chosen_crn(course.pinned_crn.as_deref()) {
            if selected.is_empty() {
                let listed = index.section_crn(crn);
                let missing = listed.is_none_or(|section| section.course_key() != course.key);
                if missing {
                    inputs.push(locked_input(course, index, crn));
                    continue;
                }
            }
        } else if selected.is_empty() {
            let siblings =
                plan.courses.iter().enumerate().any(|(other_index, other)| {
                    other_index != row_index && other.key == course.key
                });
            if !siblings {
                let listed_elsewhere = plan.selected_crns.iter().any(|crn| {
                    index
                        .section_crn(crn)
                        .is_some_and(|section| section.course_key() != course.key)
                });
                if !listed_elsewhere {
                    if let Some(missing) = plan
                        .selected_crns
                        .iter()
                        .find(|crn| index.section_crn(crn).is_none())
                    {
                        inputs.push(locked_input(course, index, missing));
                        continue;
                    }
                }
            }
        }
        if !selected.is_empty() {
            if selected.len() > 1 {
                for (offset, section) in selected.into_iter().enumerate() {
                    let mut input = input_for(course, vec![section], index);
                    if !topics && offset > 0 {
                        input.min_credits = 0;
                        input.max_credits = 0;
                    }
                    inputs.push(input);
                }
            } else {
                inputs.push(input_for(course, selected, index));
            }
            continue;
        }
        let sections = index
            .sections_for(&course.key)
            .into_iter()
            .filter(|section| !claimed.contains(&section.crn))
            .collect();
        inputs.push(input_for(course, sections, index));
    }
    inputs
}

pub fn favorites_of(
    plan: &SchedulePlan,
    index: &crate::index::CatalogIndex,
    preferences: &SchedulePreferences,
) -> Vec<GeneratedSchedule> {
    let order: HashMap<CourseKey, usize> = plan
        .courses
        .iter()
        .enumerate()
        .map(|(i, course)| (course.key.clone(), i))
        .collect();
    let mut out = vec![];
    for crns in plan.favorite_combinations.iter().rev() {
        let sections: Vec<CourseSection> = crns
            .iter()
            .filter_map(|crn| index.section_crn(crn).cloned())
            .collect();
        if sections.len() != crns.len()
            || sections.is_empty()
            || sections
                .iter()
                .any(|s| !order.contains_key(&s.course_key()))
        {
            continue;
        }
        let mut ordered = sections;
        ordered.sort_by_key(|section| {
            order
                .get(&section.course_key())
                .copied()
                .unwrap_or(usize::MAX)
        });
        out.push(GeneratedSchedule::from_sections(&ordered, preferences));
    }
    out
}

pub fn apply_relaxation(plan: &mut SchedulePlan, relaxation: &Relaxation) {
    match relaxation {
        Relaxation::MakeOptional(key) => set_required(plan, key, false),
        Relaxation::Preference(_) => {
            let mut preferences = plan.preferences.clone().unwrap_or_default();
            relaxation.apply_preferences(&mut preferences);
            plan.preferences = if preferences.is_default() {
                None
            } else {
                Some(preferences)
            };
        }
        other => other.apply_filters(&mut plan.filters),
    }
}

pub fn make_optional(plan: &mut SchedulePlan, keys: &[CourseKey]) {
    for key in keys {
        set_required(plan, key, false);
    }
}

fn set_required(plan: &mut SchedulePlan, key: &CourseKey, required: bool) {
    if let Some(course) = plan.courses.iter_mut().find(|course| &course.key == key) {
        course.is_required = required;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockerOption {
    pub courses: Vec<CourseKey>,
    pub schedule_count: usize,
    pub is_lower_bound: bool,
}

impl BlockerOption {
    pub fn id(&self) -> String {
        self.courses
            .iter()
            .map(|key| key.to_string())
            .collect::<Vec<_>>()
            .join("+")
    }
}

#[derive(Clone, Debug, Default)]
pub struct BlockerAnalysis {
    pub required_count: usize,
    pub not_offered: Vec<CourseKey>,
    /// Required courses locked to a CRN the catalog no longer has.
    pub unavailable: Vec<CourseKey>,
    pub filtered_out: Vec<CourseKey>,
    pub options: Vec<BlockerOption>,
    pub conflicting_pairs: Vec<ConflictPair>,
    pub is_incomplete: bool,
}

impl BlockerAnalysis {
    pub fn blocking_courses(&self) -> Vec<CourseKey> {
        self.options
            .first()
            .map(|option| option.courses.clone())
            .unwrap_or_default()
    }
    pub fn all_blocking_courses(&self) -> Vec<CourseKey> {
        let mut seen = HashSet::new();
        let mut out = vec![];
        for option in &self.options {
            for course in &option.courses {
                if seen.insert(course.clone()) {
                    out.push(course.clone());
                }
            }
        }
        out
    }
    pub fn clashes(&self, key: &CourseKey) -> Vec<CourseKey> {
        self.conflicting_pairs
            .iter()
            .filter_map(|pair| {
                if &pair.first == key {
                    Some(pair.second.clone())
                } else if &pair.second == key {
                    Some(pair.first.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

pub fn analyze_blockers(
    courses: &[ScheduleCourseInput],
    filters: &ScheduleFilters,
    preferences: &SchedulePreferences,
    diagnostics: Option<&ScheduleDiagnostics>,
    time_budget: f64,
    max_probes: usize,
) -> BlockerAnalysis {
    let started = Instant::now();
    let mut analysis = BlockerAnalysis::default();
    let required: Vec<usize> = courses
        .iter()
        .enumerate()
        .filter(|(_, course)| course.is_required)
        .map(|(i, _)| i)
        .collect();
    analysis.required_count = required.len();
    if required.is_empty() {
        return analysis;
    }
    let owned;
    let base = if let Some(diagnostics) = diagnostics {
        diagnostics
    } else {
        owned = generate(
            courses,
            filters,
            preferences,
            ScheduleSort::Best,
            &Limits::probe(),
        )
        .diagnostics;
        &owned
    };
    analysis.conflicting_pairs = base.conflicting_pairs.clone();
    if base.count_problem.is_some() || base.courses.len() != courses.len() {
        return analysis;
    }
    let mut dead = vec![];
    for index in &required {
        if base.courses[*index].is_blocked() {
            dead.push(*index);
            if base.courses[*index].unavailable_pin.is_some() {
                analysis.unavailable.push(courses[*index].key.clone());
            } else if base.courses[*index].total_sections == 0 {
                analysis.not_offered.push(courses[*index].key.clone());
            } else {
                analysis.filtered_out.push(courses[*index].key.clone());
            }
        }
    }
    let others: Vec<usize> = required
        .iter()
        .copied()
        .filter(|index| !dead.contains(index))
        .collect();
    let mut probes = 0usize;
    let over_budget =
        |probes: usize| probes >= max_probes || started.elapsed().as_secs_f64() > time_budget;
    let probe = |drop: &[usize], probes: &mut usize| -> Option<BlockerOption> {
        *probes += 1;
        let mut relaxed = courses.to_vec();
        for index in drop {
            relaxed[*index].is_required = false;
        }
        let result = generate(
            &relaxed,
            filters,
            preferences,
            ScheduleSort::Best,
            &Limits::blocker_probe(),
        );
        if result.total_found == 0 {
            return None;
        }
        let mut indexes = drop.to_vec();
        indexes.sort_unstable();
        Some(BlockerOption {
            courses: indexes
                .iter()
                .map(|index| courses[*index].key.clone())
                .collect(),
            schedule_count: result.total_found,
            is_lower_bound: !result.is_exhaustive,
        })
    };
    let mut found = vec![];
    if !dead.is_empty() {
        if let Some(option) = probe(&dead, &mut probes) {
            found.push(option);
        }
    }
    if found.is_empty() {
        for index in &others {
            if over_budget(probes) {
                analysis.is_incomplete = true;
                break;
            }
            let mut drop = dead.clone();
            drop.push(*index);
            if let Some(option) = probe(&drop, &mut probes) {
                found.push(option);
            }
        }
    }
    if found.is_empty() && !analysis.is_incomplete && others.len() >= 2 {
        'search: for i in 0..others.len() {
            for j in (i + 1)..others.len() {
                if over_budget(probes) {
                    analysis.is_incomplete = true;
                    break 'search;
                }
                let mut drop = dead.clone();
                drop.push(others[i]);
                drop.push(others[j]);
                if let Some(option) = probe(&drop, &mut probes) {
                    found.push(option);
                }
            }
        }
    }
    found.sort_by(|a, b| {
        a.courses
            .len()
            .cmp(&b.courses.len())
            .then(b.schedule_count.cmp(&a.schedule_count))
    });
    analysis.options = found;
    analysis
}

#[derive(Clone, Debug)]
pub struct EmptyResult {
    pub diagnostics: ScheduleDiagnostics,
    pub suggestions: Vec<Suggestion>,
    pub blockers: BlockerAnalysis,
}

pub fn analyze_empty_result(
    courses: &[ScheduleCourseInput],
    filters: &ScheduleFilters,
    preferences: &SchedulePreferences,
    diagnostics: &ScheduleDiagnostics,
    include_blockers: bool,
) -> EmptyResult {
    let blockers = if include_blockers {
        analyze_blockers(courses, filters, preferences, Some(diagnostics), 1.5, 60)
    } else {
        BlockerAnalysis::default()
    };
    let suggestions = suggest_relaxations(courses, filters, preferences, !include_blockers);
    EmptyResult {
        diagnostics: diagnostics.clone(),
        suggestions,
        blockers,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoResultReason {
    pub symbol: String,
    pub title: String,
    pub detail: Option<String>,
}

pub fn course_list(keys: &[CourseKey]) -> String {
    let names: Vec<String> = keys.iter().map(|key| key.to_string()).collect();
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        _ => format!(
            "{} and {}",
            names[..names.len() - 1].join(", "),
            names[names.len() - 1]
        ),
    }
}

pub fn no_results_headline_for(
    diagnostics: &ScheduleDiagnostics,
    blockers: Option<&BlockerAnalysis>,
) -> String {
    if diagnostics.has_no_courses() {
        return "Add some courses to this schedule first.".into();
    }
    if blockers.is_some_and(|blockers| !blockers.options.is_empty()) {
        return "Here’s what would work.".into();
    }
    if diagnostics.count_problem.is_some()
        || !diagnostics.blocked_required().is_empty()
        || !diagnostics.conflicting_pairs.is_empty()
    {
        return "Here’s what’s getting in the way.".into();
    }
    "These courses can’t all be taken together with the current rules.".into()
}

pub fn blocker_detail(
    option: &BlockerOption,
    blockers: &BlockerAnalysis,
    term_name: &str,
) -> String {
    let noun = if option.schedule_count == 1 && !option.is_lower_bound {
        "schedule"
    } else {
        "schedules"
    };
    let mut parts = vec![if option.is_lower_bound {
        format!("{}+ {noun}", option.schedule_count)
    } else {
        format!("{} {noun}", option.schedule_count)
    }];
    for key in &option.courses {
        if blockers.unavailable.contains(key) {
            parts.push(format!("{key}'s chosen section is unavailable"));
        } else if blockers.not_offered.contains(key) {
            parts.push(format!("{key} isn’t offered in {term_name}"));
        } else if blockers.filtered_out.contains(key) {
            parts.push(format!("your rules rule out every {key} section"));
        }
    }
    if option.courses.len() == 1 {
        if let Some(key) = option.courses.first() {
            let clashes = blockers.clashes(key);
            if !clashes.is_empty() {
                parts.push(format!("overlaps {}", course_list(&clashes)));
            }
        }
    }
    parts.join(" · ")
}

pub fn is_uncertain(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains("unknown") || lower.contains("couldn’t")
}

pub fn no_results_reasons(
    diagnostics: &ScheduleDiagnostics,
    term_name: &str,
) -> Vec<NoResultReason> {
    let mut result = vec![];
    for report in diagnostics
        .courses
        .iter()
        .filter(|report| report.is_blocked() && report.is_required)
    {
        if let Some(pin) = &report.unavailable_pin {
            result.push(NoResultReason {
                symbol: "lock".into(),
                title: format!("{} is unavailable", report.key),
                detail: Some(format!(
                    "CRN {pin} is no longer in the {term_name} catalog. Another section was not substituted."
                )),
            });
        } else if report.total_sections == 0 {
            result.push(NoResultReason {
                symbol: "questionmark.folder".into(),
                title: format!("{} isn’t offered in {term_name}", report.key),
                detail: Some("Remove it from the schedule or make it optional.".into()),
            });
        } else {
            result.push(NoResultReason {
                symbol: "xmark.circle".into(),
                title: format!("No {} sections left", report.key),
                detail: Some(rejection_text(report, diagnostics)),
            });
        }
    }
    for pair in &diagnostics.conflicting_pairs {
        match pair.reason {
            ConflictReason::Overlap => result.push(NoResultReason {
                symbol: "rectangle.on.rectangle.slash".into(),
                title: format!("{} and {} always overlap", pair.first, pair.second),
                detail: Some("Every remaining section of one meets while a section of the other does. Make one optional or pick different sections.".into()),
            }),
            ConflictReason::TravelTime => result.push(NoResultReason {
                symbol: "figure.walk".into(),
                title: format!("{} and {} are always too close together", pair.first, pair.second),
                detail: Some(format!("Every combination leaves less than {} minutes between them in different buildings. Lower the walking time or make one optional.", diagnostics.preferences.minimum_break_minutes)),
            }),
        }
    }
    match &diagnostics.count_problem {
        Some(CountProblem::TooManyRequired { required, wanted }) => result.push(NoResultReason {
            symbol: "number.circle".into(),
            title: format!(
                "{required} courses are required, but schedules are limited to {wanted}"
            ),
            detail: Some("Raise the number of courses or make some courses optional.".into()),
        }),
        Some(CountProblem::TooFewCourses { available, wanted }) => result.push(NoResultReason {
            symbol: "number.circle".into(),
            title: format!(
                "Only {available} {} sections left, but {wanted} are wanted",
                if *available == 1 {
                    "course has"
                } else {
                    "courses have"
                }
            ),
            detail: Some("Lower the number of courses or relax a filter.".into()),
        }),
        Some(CountProblem::CreditsOverTarget { required, limit }) => result.push(NoResultReason {
            symbol: "number.circle".into(),
            title: format!(
                "The required courses are {required} credits, but your target stops at {limit}"
            ),
            detail: Some("Raise the credit target or make a course optional.".into()),
        }),
        Some(CountProblem::CreditsUnderTarget { available, wanted }) => {
            result.push(NoResultReason {
                symbol: "number.circle".into(),
                title: format!("Your courses add up to {available} credits, but you want {wanted}"),
                detail: Some("Lower the credit target or add a course.".into()),
            })
        }
        None => {}
    }
    for report in diagnostics
        .courses
        .iter()
        .filter(|report| report.is_blocked() && !report.is_required && report.total_sections > 0)
    {
        result.push(NoResultReason {
            symbol: "minus.circle".into(),
            title: format!("{} (optional) can’t be added", report.key),
            detail: Some(rejection_text(report, diagnostics)),
        });
    }
    result
}

fn rejection_text(report: &CourseReport, diagnostics: &ScheduleDiagnostics) -> String {
    let total = report.total_sections;
    let mut ordered: Vec<(FilterReason, i32)> = report
        .rejections
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(reason, count)| (*reason, *count))
        .collect();
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then((a.0 as u32).cmp(&(b.0 as u32))));
    if ordered.is_empty() {
        return if total == 1 {
            "Its only section was filtered out.".into()
        } else {
            format!("All {total} sections were filtered out.")
        };
    }
    if ordered.len() == 1 && ordered[0].1 as usize == total {
        let phrase = rejection_phrase(ordered[0].0, total != 1, diagnostics);
        return if total == 1 {
            format!("Its only section {phrase}.")
        } else {
            format!("All {total} sections {phrase}.")
        };
    }
    let parts: Vec<String> = ordered
        .iter()
        .map(|(reason, count)| {
            format!(
                "{count} {}",
                rejection_phrase(*reason, *count != 1, diagnostics)
            )
        })
        .collect();
    format!("Of {total} sections: {}.", parts.join(", "))
}

fn rejection_phrase(
    reason: FilterReason,
    plural: bool,
    diagnostics: &ScheduleDiagnostics,
) -> String {
    let filters = &diagnostics.filters;
    let preferences = &diagnostics.preferences;
    let clock = |minutes: i32| label_minutes(minutes, true);
    match reason {
        FilterReason::ExcludedByUser => if plural {
            "are skipped by you"
        } else {
            "is skipped by you"
        }
        .into(),
        FilterReason::StartsTooEarly => format!(
            "{} before {}",
            if plural { "start" } else { "starts" },
            filters
                .earliest_start
                .map(clock)
                .unwrap_or_else(|| "your limit".into())
        ),
        FilterReason::EndsTooLate => format!(
            "{} after {}",
            if plural { "end" } else { "ends" },
            filters
                .latest_end
                .map(clock)
                .unwrap_or_else(|| "your limit".into())
        ),
        FilterReason::FreeDay => format!(
            "{} on a day you want free",
            if plural { "meet" } else { "meets" }
        ),
        FilterReason::Honors => if plural {
            "are honors sections"
        } else {
            "is an honors section"
        }
        .into(),
        FilterReason::Online => if plural { "are online" } else { "is online" }.into(),
        FilterReason::Seats => if plural {
            "are full or low on seats"
        } else {
            "is full or low on seats"
        }
        .into(),
        FilterReason::Campus => if plural {
            "are on another campus"
        } else {
            "is on another campus"
        }
        .into(),
        FilterReason::BlockedTime => if plural {
            "overlap a blocked time"
        } else {
            "overlaps a blocked time"
        }
        .into(),
        FilterReason::PrefStartsTooEarly => format!(
            "{} before {}",
            if plural { "start" } else { "starts" },
            preferences
                .earliest_start
                .as_ref()
                .map(|l| clock(l.minutes))
                .unwrap_or_else(|| "your earliest time".into())
        ),
        FilterReason::PrefEndsTooLate => format!(
            "{} after {}",
            if plural { "end" } else { "ends" },
            preferences
                .latest_end
                .as_ref()
                .map(|l| clock(l.minutes))
                .unwrap_or_else(|| "your latest time".into())
        ),
        FilterReason::PrefDayOff => format!(
            "{} on a day you want off",
            if plural { "meet" } else { "meets" }
        ),
        FilterReason::PrefFormat => format!(
            "{}{}",
            if plural { "aren’t " } else { "isn’t " },
            preferences.format.title().to_ascii_lowercase()
        ),
        FilterReason::PrefSeats => if plural {
            "have no open seats"
        } else {
            "has no open seats"
        }
        .into(),
        FilterReason::PrefCampus => if plural {
            "are on another campus"
        } else {
            "is on another campus"
        }
        .into(),
        FilterReason::PrefWorkload => if plural {
            "run longer in a day than your limit"
        } else {
            "runs longer in a day than your limit"
        }
        .into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarEvent {
    pub id: String,
    pub day: i32,
    pub start: i32,
    pub end: i32,
    pub is_conflict: bool,
    pub is_ghost: bool,
    pub payload: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

pub fn marking_conflicts(events: &[CalendarEvent]) -> Vec<CalendarEvent> {
    let mut result = events.to_vec();
    for i in 0..result.len() {
        for j in 0..result.len() {
            if i == j || result[i].is_ghost || result[j].is_ghost {
                continue;
            }
            let (a_day, a_start, a_end, a_payload, a_from, a_to) = (
                result[i].day,
                result[i].start,
                result[i].end,
                result[i].payload.clone(),
                result[i].start_date.clone(),
                result[i].end_date.clone(),
            );
            let b = &result[j];
            if a_day == b.day
                && a_start < b.end
                && b.start < a_end
                && a_payload != b.payload
                && date_windows_overlap(
                    a_from.as_deref(),
                    a_to.as_deref(),
                    b.start_date.as_deref(),
                    b.end_date.as_deref(),
                )
            {
                result[i].is_conflict = true;
            }
        }
    }
    result
}

pub struct CarouselLayout {
    pub hours: std::ops::RangeInclusive<i32>,
    pub days: Vec<i32>,
}

impl CarouselLayout {
    pub fn from_schedules(schedules: &[GeneratedSchedule], preferences: &AppPreferences) -> Self {
        let mut earliest = i32::MAX;
        let mut latest = 0;
        let mut weekdays = HashSet::new();
        for schedule in schedules {
            if let Some(start) = schedule.earliest_start {
                earliest = earliest.min(start);
            }
            if let Some(end) = schedule.latest_end {
                latest = latest.max(end);
            }
            for section in &schedule.sections {
                for meeting in section.weekly_meetings() {
                    for day in meeting.days.indices() {
                        weekdays.insert(day);
                    }
                }
            }
        }
        let hours = if earliest == i32::MAX {
            let start = preferences.calendar_start_hour.max(0);
            let end = (start + 5).max(preferences.calendar_end_hour.min(24));
            start..=end
        } else {
            let lower = (earliest / 60).max(0);
            let upper = ((latest as f64 / 60.0).ceil() as i32)
                .max(lower + 5)
                .min(24);
            lower..=upper
        };
        let mut days = vec![1, 2, 3, 4, 5];
        if preferences.show_weekends || weekdays.contains(&6) {
            days.push(6);
        }
        if preferences.show_weekends || weekdays.contains(&0) {
            days.insert(0, 0);
        }
        Self { hours, days }
    }
}

#[derive(Clone, Debug)]
pub struct AutofillSuggestion {
    pub id: String,
    pub plan_name: String,
    pub semester: Option<String>,
    pub title: String,
    pub detail: String,
    pub is_exact_term: bool,
    pub courses: Vec<CourseKey>,
    pub unavailable: Vec<CourseKey>,
    pub from_schedule: bool,
}

pub fn autofill_suggestions(
    plan: &SchedulePlan,
    terms: &[Term],
    degree_plans: &[DegreePlan],
    schedule_plans: &[SchedulePlan],
    index: Option<&crate::index::CatalogIndex>,
) -> Vec<AutofillSuggestion> {
    let has_siblings = schedule_plans.iter().any(|other| {
        other.term_code == plan.term_code && other.id != plan.id && !other.courses.is_empty()
    });
    if degree_plans.is_empty() && !has_siblings {
        return vec![];
    }
    let term = resolve_term(&plan.term_code, terms, index);
    let already: HashSet<CourseKey> = plan
        .courses
        .iter()
        .map(|course| course.key.clone())
        .collect();
    crate::planlogic::schedule_suggestions(
        &term,
        degree_plans,
        schedule_plans,
        Some(plan.id),
        &already,
        index,
    )
    .into_iter()
    .map(|suggestion| {
        let from_schedule = suggestion.semester.is_none();
        let id = if from_schedule {
            schedule_plans
                .iter()
                .find(|other| other.name == suggestion.plan_name && other.term_code == term.code)
                .map(|other| format!("schedule-{}", other.id))
                .unwrap_or_else(|| format!("schedule-{}", suggestion.plan_name))
        } else {
            degree_plans
                .iter()
                .find(|other| other.name == suggestion.plan_name)
                .map(|other| format!("degree-{}", other.id))
                .unwrap_or_else(|| format!("degree-{}", suggestion.plan_name))
        };
        AutofillSuggestion {
            id,
            plan_name: suggestion.plan_name,
            semester: suggestion.semester,
            title: suggestion.title,
            detail: suggestion.detail,
            is_exact_term: suggestion.is_exact_term,
            courses: suggestion.courses,
            unavailable: suggestion.unavailable,
            from_schedule,
        }
    })
    .collect()
}

pub fn autofill_source_name(suggestion: &AutofillSuggestion) -> String {
    match &suggestion.semester {
        Some(term) => format!("“{}” · {term}", suggestion.plan_name),
        None => format!("Copy “{}”", suggestion.plan_name),
    }
}

pub fn course_count_label(count: usize) -> String {
    if count == 1 {
        "1 course".into()
    } else {
        format!("{count} courses")
    }
}

pub fn dismissal_key(plan_id: uuid::Uuid, suggestion_id: &str) -> String {
    format!("{plan_id}|{suggestion_id}")
}

pub fn dismissal_contains(key: &str, raw: &str) -> bool {
    raw.split('\n').any(|line| line == key)
}

pub fn dismissal_adding(key: &str, raw: &str, limit: usize) -> String {
    let mut keys: Vec<String> = raw
        .split('\n')
        .filter(|line| !line.is_empty() && *line != key)
        .map(str::to_string)
        .collect();
    keys.push(key.to_string());
    let start = keys.len().saturating_sub(limit);
    keys[start..].join("\n")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedCourse {
    pub key: CourseKey,
    pub crn: Option<String>,
}

pub fn parse_dropped(items: &[String]) -> Vec<DroppedCourse> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    for item in items {
        for line in item.split(['\n', '\r']) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }
            let Some(key) = CourseKey::parse(&format!("{} {}", parts[0], parts[1])) else {
                continue;
            };
            if !key.number.chars().any(|c| c.is_ascii_digit()) {
                continue;
            }
            let crn = parts
                .get(2)
                .filter(|text| text.len() == 5 && text.chars().all(|c| c.is_ascii_digit()))
                .map(|text| (*text).to_string());
            let dropped = DroppedCourse { key, crn };
            if seen.insert((dropped.key.clone(), dropped.crn.clone())) {
                out.push(dropped);
            }
        }
    }
    out
}

/// Minutes after midnight, clamped to a single day. Time pickers round-trip through this.
pub fn clock_minutes(minutes: i32) -> i32 {
    minutes.clamp(0, 24 * 60 - 1)
}

#[derive(Clone, Debug)]
pub struct InstructorOption {
    pub name: String,
    pub display_name: String,
    pub courses: Vec<CourseKey>,
}

#[derive(Clone, Debug, Default)]
pub struct PreferencesContext {
    pub campus_names: Vec<String>,
    pub instructors: Vec<InstructorOption>,
    pub optional_course_count: i32,
    pub required_credits: i32,
    pub total_credits: i32,
}

pub fn preferences_context(
    plan: &SchedulePlan,
    index: Option<&crate::index::CatalogIndex>,
) -> PreferencesContext {
    let Some(index) = index else {
        return PreferencesContext::default();
    };
    let campus_names = index
        .campus_names
        .iter()
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            !lower.contains("online") && !lower.contains("no campus")
        })
        .cloned()
        .collect();
    let mut by_name: HashMap<String, InstructorOption> = HashMap::new();
    let mut optional_course_count = 0;
    let mut required_credits = 0;
    let mut total_credits = 0;
    for plan_course in &plan.courses {
        let course = index.course(&plan_course.key);
        let least = course.map(|c| c.min_credits.ceil() as i32).unwrap_or(0);
        let most = course.map(|c| c.max_credits.ceil() as i32).unwrap_or(0);
        if plan_course.is_required {
            required_credits += least;
        } else {
            optional_course_count += 1;
        }
        total_credits += least.max(most);
        for section in index.sections_for(&plan_course.key) {
            if plan_course.excluded_crns.contains(&section.crn) {
                continue;
            }
            for instructor in section.instructors.as_ref().into_iter().flatten() {
                let option =
                    by_name
                        .entry(instructor.name.clone())
                        .or_insert_with(|| InstructorOption {
                            name: instructor.name.clone(),
                            display_name: instructor.display_name(),
                            courses: vec![],
                        });
                if !option.courses.contains(&plan_course.key) {
                    option.courses.push(plan_course.key.clone());
                }
            }
        }
    }
    let mut instructors: Vec<_> = by_name.into_values().collect();
    instructors.sort_by_key(|instructor| instructor.name.to_ascii_lowercase());
    PreferencesContext {
        campus_names,
        instructors,
        optional_course_count,
        required_credits,
        total_credits,
    }
}

pub fn create_plan(data: &mut UserData, name: &str, term_code: &str) -> SchedulePlan {
    let plan = SchedulePlan {
        id: uuid::Uuid::new_v4(),
        name: name.to_string(),
        term_code: term_code.to_string(),
        courses: vec![],
        selected_crns: vec![],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: !data.plans.iter().any(|plan| plan.term_code == term_code),
        created_at: 0.0,
        updated_at: 0.0,
    };
    data.plans.push(plan.clone());
    plan
}

pub fn update_plan(data: &mut UserData, id: uuid::Uuid, change: impl FnOnce(&mut SchedulePlan)) {
    if let Some(plan) = data.plans.iter_mut().find(|plan| plan.id == id) {
        change(plan);
    }
}

pub fn duplicate_plan(data: &mut UserData, id: uuid::Uuid) -> Option<SchedulePlan> {
    let mut copy = data.plans.iter().find(|plan| plan.id == id)?.clone();
    copy.id = uuid::Uuid::new_v4();
    copy.name = format!("{} Copy", copy.name);
    copy.is_primary = false;
    data.plans.push(copy.clone());
    Some(copy)
}

pub fn plan_by_id(data: &UserData, id: uuid::Uuid) -> Option<&SchedulePlan> {
    data.plans.iter().find(|plan| plan.id == id)
}

pub fn add_course(
    data: &mut UserData,
    plan_id: uuid::Uuid,
    key: CourseKey,
    required: bool,
) -> bool {
    let Some(plan) = data.plans.iter_mut().find(|plan| plan.id == plan_id) else {
        return false;
    };
    if plan.contains(&key) {
        return false;
    }
    let color = plan.next_color_index();
    plan.courses.push(PlanCourse {
        key,
        is_required: required,
        excluded_crns: HashSet::new(),
        color_index: color,
        pinned_crn: None,
        degree_row_id: None,
    });
    true
}

pub fn remove_course(
    data: &mut UserData,
    plan_id: uuid::Uuid,
    key: &CourseKey,
    sections_of_course: &[String],
) {
    let Some(plan) = data.plans.iter_mut().find(|plan| plan.id == plan_id) else {
        return;
    };
    let Some(position) = plan.courses.iter().position(|course| &course.key == key) else {
        return;
    };
    let removed = plan.courses.remove(position);
    let another = plan.courses.iter().any(|course| &course.key == key);
    let mut crn_list: Vec<&str> = sections_of_course.iter().map(String::as_str).collect();
    if another {
        crn_list.clear();
        if let Some(pin) = removed.pinned_crn.as_deref() {
            crn_list.push(pin);
        }
    }
    let crns: HashSet<&str> = crn_list.into_iter().collect();
    plan.selected_crns
        .retain(|crn| !crns.contains(crn.as_str()));
    plan.favorite_combinations
        .retain(|combination| !combination.iter().any(|crn| crns.contains(crn.as_str())));
}

#[derive(Clone, Debug)]
pub struct CourseRemoval {
    pub course: PlanCourse,
    pub index: usize,
    pub selected_crns: Vec<String>,
    pub favorite_combinations: Vec<Vec<String>>,
}

pub fn remove_plan_course(
    data: &mut UserData,
    plan_id: uuid::Uuid,
    key: &CourseKey,
    index: Option<&crate::index::CatalogIndex>,
) -> Option<CourseRemoval> {
    let plan = data.plans.iter().find(|plan| plan.id == plan_id)?.clone();
    let resolved = ResolvedPlan::resolve(&plan, index);
    let entry = resolved.entries.iter().find(|entry| entry.key() == key)?;
    let topics = entry
        .course
        .as_ref()
        .is_some_and(|course| course.special_topics);
    let rows = plan
        .courses
        .iter()
        .filter(|course| &course.key == key)
        .count();
    let mut crns = HashSet::new();
    if topics || rows > 1 {
        for section in &entry.selected {
            crns.insert(section.crn.clone());
        }
        if let Some(pin) = &entry.plan_course.pinned_crn {
            crns.insert(pin.clone());
        }
    } else {
        crns = entry.course_crns();
        for section in &entry.selected {
            crns.insert(section.crn.clone());
        }
        if let Some(index) = index {
            for section in index.sections_for(key) {
                crns.insert(section.crn);
            }
        }
    }
    let removal = CourseRemoval {
        course: entry.plan_course.clone(),
        index: plan
            .courses
            .iter()
            .position(|course| &course.key == key)
            .unwrap_or(plan.courses.len()),
        selected_crns: plan
            .selected_crns
            .iter()
            .filter(|crn| crns.contains(*crn))
            .cloned()
            .collect(),
        favorite_combinations: plan
            .favorite_combinations
            .iter()
            .filter(|combination| combination.iter().any(|crn| crns.contains(crn)))
            .cloned()
            .collect(),
    };
    remove_course(data, plan_id, key, &crns.into_iter().collect::<Vec<_>>());
    Some(removal)
}

pub fn restore_removal(plan: &mut SchedulePlan, removal: &CourseRemoval) {
    if plan.contains(&removal.course.key) {
        return;
    }
    let index = removal.index.min(plan.courses.len());
    plan.courses.insert(index, removal.course.clone());
    for crn in &removal.selected_crns {
        if !plan.selected_crns.contains(crn) {
            plan.selected_crns.push(crn.clone());
        }
    }
    for combination in &removal.favorite_combinations {
        if !plan.favorite_combinations.contains(combination) {
            plan.favorite_combinations.push(combination.clone());
        }
    }
}

pub fn add_autofill_courses(
    data: &mut UserData,
    plan_id: uuid::Uuid,
    courses: &[CourseKey],
) -> Vec<CourseKey> {
    let mut added = vec![];
    for key in courses {
        if add_course(data, plan_id, key.clone(), true) {
            added.push(key.clone());
        }
    }
    added
}

pub fn undo_autofill(
    data: &mut UserData,
    plan_id: uuid::Uuid,
    keys: &[CourseKey],
    index: Option<&crate::index::CatalogIndex>,
) {
    for key in keys {
        let crns: Vec<String> = index
            .map(|index| {
                index
                    .sections_for(key)
                    .into_iter()
                    .map(|section| section.crn)
                    .collect()
            })
            .unwrap_or_default();
        remove_course(data, plan_id, key, &crns);
    }
}

pub fn active_primary_plan(plans: &[SchedulePlan], now: f64) -> Option<&SchedulePlan> {
    let candidates: Vec<(&SchedulePlan, (f64, f64))> = plans
        .iter()
        .filter_map(|plan| {
            if !plan.is_primary || plan.selected_crns.is_empty() {
                return None;
            }
            Some((plan, crate::calendar::approximate_window(&plan.term_code)?))
        })
        .collect();
    if let Some((plan, _)) = candidates
        .iter()
        .filter(|(_, window)| now >= window.0 && now <= window.1)
        .max_by_key(|(plan, _)| plan.selected_crns.len())
    {
        return Some(*plan);
    }
    if let Some((plan, _)) = candidates
        .iter()
        .filter(|(_, window)| window.0 > now)
        .min_by(|a, b| {
            a.1 .0
                .partial_cmp(&b.1 .0)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    {
        return Some(*plan);
    }
    let cutoff = now - 14.0 * 24.0 * 3600.0;
    candidates
        .iter()
        .filter(|(_, window)| window.1 >= cutoff)
        .max_by(|a, b| {
            a.1 .1
                .partial_cmp(&b.1 .1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(plan, _)| *plan)
}

pub fn reorder_semester<'a>(
    plan: &SchedulePlan,
    degree: Option<&'a DegreePlan>,
    terms: &[Term],
    index: Option<&crate::index::CatalogIndex>,
) -> Option<(&'a DegreePlan, Term)> {
    let degree = degree?;
    let term = resolve_term(&plan.term_code, terms, index);
    degree.term_matching(&term)?;
    Some((degree, term))
}

fn resolve_term(code: &str, terms: &[Term], index: Option<&crate::index::CatalogIndex>) -> Term {
    if let Some(known) = terms.iter().find(|term| term.code == code) {
        return known.clone();
    }
    if let Some(index) = index {
        if index.catalog.term.code == code {
            return index.catalog.term.clone();
        }
    }
    Term {
        code: code.to_string(),
        banner_description: String::new(),
    }
}

#[derive(Clone, Debug)]
pub struct ReorderOption {
    pub id: String,
    pub title: String,
    pub reasons: Vec<String>,
    pub adds_semester: bool,
    pub moved_away: Vec<CourseKey>,
    pub moved_in: Vec<CourseKey>,
    suggestion: Option<crate::reorder::Suggestion>,
}

impl ReorderOption {
    pub fn from_suggestion(suggestion: &crate::reorder::Suggestion, origin: uuid::Uuid) -> Self {
        let (moved_away, moved_in, adds_semester, _) =
            crate::reorder::schedule_option(suggestion, origin);
        Self {
            id: suggestion.id.clone(),
            title: suggestion.title.clone(),
            reasons: suggestion.reasons.clone(),
            adds_semester,
            moved_away,
            moved_in,
            suggestion: Some(suggestion.clone()),
        }
    }
    pub fn custom(title: impl Into<String>) -> Self {
        Self {
            id: "x".into(),
            title: title.into(),
            reasons: vec![],
            adds_semester: false,
            moved_away: vec![],
            moved_in: vec![],
            suggestion: None,
        }
    }
    pub fn applied_text(&self) -> String {
        self.title
            .strip_prefix("Move ")
            .map(|rest| format!("Moved {rest}"))
            .unwrap_or_else(|| "Updated your degree plan".into())
    }
    pub fn schedule_effect(&self) -> String {
        let mut parts = vec![];
        if !self.moved_away.is_empty() {
            let verb = if self.moved_away.len() == 1 {
                "comes"
            } else {
                "come"
            };
            parts.push(format!(
                "{} {verb} out of this schedule.",
                course_list(&self.moved_away)
            ));
        }
        if !self.moved_in.is_empty() {
            let verb = if self.moved_in.len() == 1 {
                "it’s"
            } else {
                "they’re"
            };
            parts.push(format!(
                "{} will be added to it if {verb} offered this term.",
                course_list(&self.moved_in)
            ));
        }
        parts.join(" ")
    }
    pub fn apply_to(&self, plan: &mut DegreePlan) {
        if let Some(suggestion) = &self.suggestion {
            crate::reorder::apply(suggestion, plan);
        }
    }
}

/// The offline early return: nothing is looked up when the blocking course is not in the semester.
pub fn reorder_load_offline(
    blocking: &[CourseKey],
    degree_plan: &DegreePlan,
    term: &Term,
) -> Vec<ReorderOption> {
    let mut plan = degree_plan.clone();
    plan.sort_terms();
    let Some(origin) = plan
        .terms
        .iter()
        .find(|semester| semester.matches_term(term))
        .cloned()
    else {
        return vec![];
    };
    let in_semester: HashSet<CourseKey> = origin
        .courses
        .iter()
        .filter_map(|course| course.key.clone())
        .collect();
    let candidates: Vec<CourseKey> = blocking
        .iter()
        .filter(|key| in_semester.contains(*key))
        .cloned()
        .collect();
    if candidates.is_empty() {
        return vec![];
    }
    let history = crate::offering::OfferingHistory::empty();
    crate::reorder::suggestions(
        &plan,
        origin.id,
        &candidates,
        &history,
        |_| None,
        &crate::CourseEquivalences::none(),
        20.0,
    )
    .iter()
    .map(|suggestion| ReorderOption::from_suggestion(suggestion, origin.id))
    .collect()
}
