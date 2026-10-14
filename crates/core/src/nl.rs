//! Deterministic reading of a schedule description. This replaces Apple Intelligence:
//! the same draft, grounding, and clock-time rules, filled by a parser instead of a model.

use crate::model::*;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftWeekday {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl DraftWeekday {
    pub fn day(self) -> DaySet {
        match self {
            Self::Sunday => DaySet::SUNDAY,
            Self::Monday => DaySet::MONDAY,
            Self::Tuesday => DaySet::TUESDAY,
            Self::Wednesday => DaySet::WEDNESDAY,
            Self::Thursday => DaySet::THURSDAY,
            Self::Friday => DaySet::FRIDAY,
            Self::Saturday => DaySet::SATURDAY,
        }
    }
    pub fn parse(word: &str) -> Option<Self> {
        let w = word.to_ascii_lowercase();
        Some(match w.as_str() {
            "sunday" | "sun" => Self::Sunday,
            "monday" | "mon" | "mondays" => Self::Monday,
            "tuesday" | "tue" | "tues" | "tuesdays" => Self::Tuesday,
            "wednesday" | "wed" | "wednesdays" => Self::Wednesday,
            "thursday" | "thu" | "thur" | "thurs" | "thursdays" => Self::Thursday,
            "friday" | "fri" | "fridays" => Self::Friday,
            "saturday" | "sat" | "saturdays" => Self::Saturday,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftTime {
    EarlyMorning,
    Morning,
    Midday,
    Afternoon,
    Evening,
    Night,
}

impl DraftTime {
    fn band(self) -> TimeBand {
        match self {
            Self::EarlyMorning => TimeBand::EarlyMorning,
            Self::Morning => TimeBand::Morning,
            Self::Midday => TimeBand::Midday,
            Self::Afternoon => TimeBand::Afternoon,
            Self::Evening => TimeBand::Evening,
            Self::Night => TimeBand::Night,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftLayout {
    FewerDays,
    SpreadOut,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftBreaks {
    AvoidBackToBack,
    TimeToWalk,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftFormat {
    InPerson,
    Online,
    Hybrid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftSeats {
    OpenSeatsOnly,
    WaitlistOk,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftBlock {
    pub label: String,
    pub days: Vec<DraftWeekday>,
    pub start: String,
    pub end: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreferencesDraft {
    pub preferred_times: Option<Vec<DraftTime>>,
    pub avoided_times: Option<Vec<DraftTime>>,
    pub earliest_start: Option<String>,
    pub latest_end: Option<String>,
    pub time_limits_flexible: Option<bool>,
    pub days_off: Option<Vec<DraftWeekday>>,
    pub days_off_required: Option<bool>,
    pub layout: Option<DraftLayout>,
    pub max_classes_per_day: Option<i32>,
    pub max_hours_per_day: Option<i32>,
    pub breaks: Option<DraftBreaks>,
    pub wants_lunch: Option<bool>,
    pub blocked_times: Option<Vec<DraftBlock>>,
    pub format: Option<DraftFormat>,
    pub seats: Option<DraftSeats>,
}

impl PreferencesDraft {
    pub const MAX_BLOCKS: usize = 8;
    pub fn is_empty(&self) -> bool {
        self.preferred_times
            .as_ref()
            .map(|v| v.is_empty())
            .unwrap_or(true)
            && self
                .avoided_times
                .as_ref()
                .map(|v| v.is_empty())
                .unwrap_or(true)
            && self.earliest_start.is_none()
            && self.latest_end.is_none()
            && self.days_off.as_ref().map(|v| v.is_empty()).unwrap_or(true)
            && self.layout.is_none()
            && self.max_classes_per_day.is_none()
            && self.max_hours_per_day.is_none()
            && self.breaks.is_none()
            && self.wants_lunch != Some(true)
            && self
                .blocked_times
                .as_ref()
                .map(|v| v.is_empty())
                .unwrap_or(true)
            && self.format.is_none()
            && self.seats.is_none()
    }
    pub fn filled_count(&self) -> i32 {
        let mut n = 0;
        if self.preferred_times.as_ref().is_some_and(|v| !v.is_empty()) {
            n += 1;
        }
        if self.avoided_times.as_ref().is_some_and(|v| !v.is_empty()) {
            n += 1;
        }
        if self.earliest_start.is_some() {
            n += 1;
        }
        if self.latest_end.is_some() {
            n += 1;
        }
        if self.days_off.as_ref().is_some_and(|v| !v.is_empty()) {
            n += 1;
        }
        if self.layout.is_some() {
            n += 1;
        }
        if self.max_classes_per_day.is_some() || self.max_hours_per_day.is_some() {
            n += 1;
        }
        if self.breaks.is_some() {
            n += 1;
        }
        if self.wants_lunch == Some(true) {
            n += 1;
        }
        n += self.blocked_times.as_ref().map(|v| v.len()).unwrap_or(0) as i32;
        if self.format.is_some() {
            n += 1;
        }
        if self.seats.is_some() {
            n += 1;
        }
        n
    }
    pub fn applied(&self, base: &SchedulePreferences) -> SchedulePreferences {
        let mut p = base.clone();
        for time in self.preferred_times.as_deref().unwrap_or(&[]) {
            p.set_band(time.band(), TimePreference::Prefer);
        }
        for time in self.avoided_times.as_deref().unwrap_or(&[]) {
            p.set_band(time.band(), TimePreference::Avoid);
        }
        let strength = if self.time_limits_flexible == Some(true) {
            PreferenceStrength::Preferred
        } else {
            PreferenceStrength::Required
        };
        if let Some(minutes) = self.earliest_start.as_deref().and_then(minutes_from_clock) {
            if minutes < 24 * 60 {
                p.earliest_start = Some(TimeLimit { minutes, strength });
            }
        }
        if let Some(minutes) = self.latest_end.as_deref().and_then(minutes_from_clock) {
            if minutes > p.earliest_start.as_ref().map(|l| l.minutes).unwrap_or(0) {
                p.latest_end = Some(TimeLimit { minutes, strength });
            }
        }
        if let Some(wanted) = &self.days_off {
            if !wanted.is_empty() {
                p.days_off = wanted.iter().fold(DaySet(0), |a, d| a.union(d.day()));
                p.days_off_strength = if self.days_off_required == Some(true) {
                    PreferenceStrength::Required
                } else {
                    PreferenceStrength::Preferred
                };
            }
        }
        match self.layout {
            Some(DraftLayout::FewerDays) => p.day_layout = DayLayout::FewerDays,
            Some(DraftLayout::SpreadOut) => p.day_layout = DayLayout::SpreadOut,
            None => {}
        }
        if let Some(count) = self.max_classes_per_day {
            if (1..=8).contains(&count) {
                p.max_classes_per_day = Some(count);
            }
        }
        if let Some(hours) = self.max_hours_per_day {
            if (1..=12).contains(&hours) {
                p.max_minutes_per_day = Some(hours * 60);
            }
        }
        match self.breaks {
            Some(DraftBreaks::AvoidBackToBack) => p.breaks = BreakPreference::AvoidBackToBack,
            Some(DraftBreaks::TimeToWalk) => {
                p.breaks = BreakPreference::AtLeast;
                p.minimum_break_minutes = 15;
                p.minimum_break_strength = PreferenceStrength::Preferred;
            }
            None => {}
        }
        if self.wants_lunch == Some(true) && p.lunch.is_none() {
            p.lunch = Some(LunchBreak::default());
        }
        for block in self
            .blocked_times
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .take(Self::MAX_BLOCKS)
        {
            let Some(start) = minutes_from_clock(&block.start) else {
                continue;
            };
            let Some(mut end) = minutes_from_clock(&block.end) else {
                continue;
            };
            if end <= start && end < 12 * 60 && end + 12 * 60 <= 24 * 60 {
                end += 12 * 60;
            }
            let days = block.days.iter().fold(DaySet(0), |a, d| a.union(d.day()));
            let candidate = BlockedTime {
                id: Uuid::new_v4(),
                label: block.label.clone(),
                days,
                start,
                end,
                strength: PreferenceStrength::Required,
                is_enabled: true,
            };
            if !candidate.is_valid() {
                continue;
            }
            let label = candidate.label.trim().to_ascii_lowercase();
            let dup = p.blocked_times.iter().any(|b| {
                b.label.to_ascii_lowercase() == label
                    && b.days == candidate.days
                    && b.start == candidate.start
                    && b.end == candidate.end
            });
            if !dup {
                p.blocked_times.push(candidate);
            }
        }
        match self.format {
            Some(DraftFormat::InPerson) => p.format = FormatPreference::InPerson,
            Some(DraftFormat::Online) => p.format = FormatPreference::Online,
            Some(DraftFormat::Hybrid) => p.format = FormatPreference::Hybrid,
            None => {}
        }
        match self.seats {
            Some(DraftSeats::OpenSeatsOnly) => p.seats = SeatPolicy::OpenOnly,
            Some(DraftSeats::WaitlistOk) => p.seats = SeatPolicy::WaitlistOk,
            None => {}
        }
        p.normalized()
    }
    pub fn grounded(&self, description: &str) -> Self {
        if !is_english(description) {
            return self.clone();
        }
        let text = format!(" {} ", description.to_ascii_lowercase());
        let mentions = |cues: &[&str]| cues.iter().any(|c| text.contains(c));
        let mut copy = self.clone();
        if !mentions(&[
            "online",
            "remote",
            "virtual",
            "zoom",
            "in person",
            "in-person",
            "on campus",
            "on-campus",
            "hybrid",
            "face to face",
            "face-to-face",
        ]) {
            copy.format = None;
        }
        if !mentions(&[
            "seat",
            "waitlist",
            "wait list",
            "wait-list",
            "capacity",
            "full class",
            "full section",
            "open section",
            "open class",
            "closed class",
        ]) {
            copy.seats = None;
        }
        if !mentions(&[
            "back to back",
            "back-to-back",
            "walk",
            "between class",
            "between building",
            "break between",
            "gap",
            "rush",
            "far apart",
            "time to get",
            "travel",
        ]) {
            copy.breaks = None;
        }
        if !mentions(&["lunch", "meal", " eat "]) {
            copy.wants_lunch = None;
        }
        if !mentions(&[
            "few days",
            "fewer days",
            "fewest days",
            "spread",
            "evenly",
            "compact",
            "long weekend",
            "bunch",
            "pack ",
            "cram",
            "consolidat",
            "every day",
            "each day",
            "all week",
            "days a week",
            "days per week",
            "days on campus",
        ]) {
            copy.layout = None;
        }
        let per_day = mentions(&[
            "per day",
            "a day",
            "each day",
            "every day",
            "in a day",
            "in one day",
            "daily",
            "same day",
        ]) && has_quantity(&text);
        if !(per_day && text.contains("class")) {
            copy.max_classes_per_day = None;
        }
        if !(per_day && text.contains("hour")) {
            copy.max_hours_per_day = None;
        }
        let clauses = clauses_of(&text);
        let has = |cues: &[&str], clause: &str| cues.iter().any(|c| clause.contains(c));
        let time_limit = [
            "before",
            "after",
            "earlier than",
            "later than",
            "start",
            "finish",
            "end by",
            "done by",
            "out by",
            "no classes",
            "nothing",
        ];
        let weekday = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
        let off = [
            " off",
            "free",
            "no class",
            "without class",
            "skip",
            "nothing on",
        ];
        let no_classes = ["no class on", "no classes on", "nothing on"];
        let firm = [
            "must",
            "have to",
            "has to",
            "need to",
            "i need",
            "can't",
            "can’t",
            "cannot",
            "can not",
            "no way",
            "never",
            "always",
            "required",
            "i work",
            "my job",
            "my shift",
            "shifts",
            "only available",
        ];
        let soft = [
            "ideally",
            "prefer",
            "if possible",
            "if i can",
            "would like",
            "i'd like",
            "i’d like",
            "id like",
            "rather",
            "hopefully",
            "try to",
            "nice if",
            "would be nice",
            "wish",
        ];
        let limit_clauses: Vec<_> = clauses
            .iter()
            .filter(|c| has(&time_limit, c))
            .cloned()
            .collect();
        copy.time_limits_flexible = Some(
            !limit_clauses.is_empty()
                && limit_clauses
                    .iter()
                    .all(|c| has(&soft, c) && !has(&firm, c)),
        );
        let off_clauses: Vec<_> = clauses
            .iter()
            .filter(|c| has(&weekday, c) && has(&off, c))
            .cloned()
            .collect();
        copy.days_off_required = Some(
            off_clauses
                .iter()
                .any(|c| has(&firm, c) || has(&no_classes, c)),
        );
        copy
    }
}

fn has_quantity(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
        || [
            "one",
            "two",
            "three",
            "four",
            "five",
            "six",
            "seven",
            "eight",
            "nine",
            "ten",
            "twelve",
            "couple",
            "no more than",
            "at most",
            "max",
        ]
        .iter()
        .any(|w| text.contains(w))
}

fn clauses_of(text: &str) -> Vec<String> {
    let mut parts = vec![text.to_string()];
    for sep in [",", ";", ".", "\n", " and ", " but "] {
        parts = parts
            .into_iter()
            .flat_map(|p| p.split(sep).map(|s| s.to_string()).collect::<Vec<_>>())
            .collect();
    }
    parts
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn is_english(text: &str) -> bool {
    let letters: String = text
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || c.is_whitespace())
        .collect();
    let words: Vec<&str> = letters.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    let english = [
        "the", "a", "i", "my", "no", "classes", "class", "before", "after", "off", "and", "have",
        "on", "with", "friday", "fridays", "monday", "like", "lunch", "work",
    ];
    let hits = words
        .iter()
        .filter(|w| english.contains(&w.to_ascii_lowercase().as_str()))
        .count();
    hits * 5 >= words.len() || hits >= 3
}

pub fn minutes_from_clock(text: &str) -> Option<i32> {
    let mut s: String = text
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '.')
        .collect();
    match s.as_str() {
        "noon" => return Some(12 * 60),
        "midnight" => return Some(24 * 60),
        _ => {}
    }
    let mut meridiem = None;
    if s.ends_with("am") {
        meridiem = Some(false);
        s.truncate(s.len() - 2);
    } else if s.ends_with("pm") {
        meridiem = Some(true);
        s.truncate(s.len() - 2);
    }
    if s.is_empty() {
        return None;
    }
    let (mut hour, minute) = if let Some((h, m)) = s.split_once(':') {
        (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?)
    } else if (3..=4).contains(&s.len()) && s.chars().all(|c| c.is_ascii_digit()) {
        let number: i32 = s.parse().ok()?;
        (number / 100, number % 100)
    } else {
        (s.parse::<i32>().ok()?, 0)
    };
    if !(0..=24).contains(&hour) || !(0..60).contains(&minute) {
        return None;
    }
    if let Some(is_pm) = meridiem {
        if !(1..=12).contains(&hour) {
            return None;
        }
        if is_pm && hour < 12 {
            hour += 12;
        }
        if !is_pm && hour == 12 {
            hour = 0;
        }
    }
    let total = hour * 60 + minute;
    if total <= 24 * 60 {
        Some(total)
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub enum InterpretError {
    Empty,
    TooLong,
    NothingFound,
}

impl InterpretError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Empty => "Describe the schedule you want first.",
            Self::TooLong => "That’s a lot to read at once. Try describing the most important things in a few sentences.",
            Self::NothingFound => "Couldn’t find any schedule wishes in that. Try mentioning times, days or commitments, like “no classes before 10” or “work Tuesdays 1–5”.",
        }
    }
}

pub const MAX_DESCRIPTION: usize = 1500;

/// Reads English (and a few set phrases) into the same draft the survey applies.
pub fn interpret(text: &str) -> Result<PreferencesDraft, InterpretError> {
    let description = text.trim();
    if description.is_empty() {
        return Err(InterpretError::Empty);
    }
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(InterpretError::TooLong);
    }
    let mut draft = extract(description);
    draft = draft.grounded(description);
    if draft.is_empty() {
        return Err(InterpretError::NothingFound);
    }
    Ok(draft)
}

fn extract(text: &str) -> PreferencesDraft {
    let lower = text.to_ascii_lowercase();
    let mut draft = PreferencesDraft::default();
    if let Some(mins) = find_time_after(
        &lower,
        &[
            "before",
            "no classes before",
            "nothing before",
            "not before",
        ],
    ) {
        draft.earliest_start = Some(clock_string(mins));
    }
    if let Some(mins) = find_time_after(
        &lower,
        &[
            "done by",
            "out by",
            "end by",
            "finish by",
            "before the end",
            "no classes after",
        ],
    ) {
        draft.latest_end = Some(clock_string(mins));
    }
    let mut days = Vec::new();
    for (word, day) in [
        ("monday", DraftWeekday::Monday),
        ("tuesday", DraftWeekday::Tuesday),
        ("wednesday", DraftWeekday::Wednesday),
        ("thursday", DraftWeekday::Thursday),
        ("friday", DraftWeekday::Friday),
        ("saturday", DraftWeekday::Saturday),
        ("sunday", DraftWeekday::Sunday),
    ] {
        if lower.contains(word)
            && (lower.contains("off") || lower.contains("free") || lower.contains("no class"))
            && !days.contains(&day)
        {
            days.push(day);
        }
    }
    if !days.is_empty() {
        draft.days_off = Some(days);
    }
    if lower.contains("lunch") || lower.contains("meal") {
        draft.wants_lunch = Some(true);
    }
    if lower.contains("morning person") || lower.contains("mornings") {
        draft.preferred_times = Some(vec![DraftTime::EarlyMorning, DraftTime::Morning]);
    }
    if lower.contains("night owl") || lower.contains("evenings") {
        draft.preferred_times = Some(vec![
            DraftTime::Afternoon,
            DraftTime::Evening,
            DraftTime::Night,
        ]);
    }
    if lower.contains("fewer days") || lower.contains("long weekend") {
        draft.layout = Some(DraftLayout::FewerDays);
    }
    if lower.contains("spread") {
        draft.layout = Some(DraftLayout::SpreadOut);
    }
    if lower.contains("back to back") || lower.contains("back-to-back") {
        draft.breaks = Some(DraftBreaks::AvoidBackToBack);
    }
    if lower.contains("walk") {
        draft.breaks = Some(DraftBreaks::TimeToWalk);
    }
    if lower.contains("online") {
        draft.format = Some(DraftFormat::Online);
    } else if lower.contains("hybrid") {
        draft.format = Some(DraftFormat::Hybrid);
    } else if lower.contains("in person") || lower.contains("in-person") {
        draft.format = Some(DraftFormat::InPerson);
    }
    if lower.contains("waitlist") {
        draft.seats = Some(DraftSeats::WaitlistOk);
    } else if lower.contains("open seat") {
        draft.seats = Some(DraftSeats::OpenSeatsOnly);
    }
    draft.blocked_times = Some(find_blocks(text));
    if draft.blocked_times.as_ref().is_some_and(|b| b.is_empty()) {
        draft.blocked_times = None;
    }
    draft
}

fn clock_string(minutes: i32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn find_time_after(text: &str, cues: &[&str]) -> Option<i32> {
    for cue in cues {
        if let Some(idx) = text.find(cue) {
            let rest = &text[idx + cue.len()..];
            let token: String = rest
                .trim_start()
                .chars()
                .take_while(|c| {
                    c.is_ascii_digit() || *c == ':' || c.is_ascii_alphabetic() || *c == ' '
                })
                .take(12)
                .collect();
            if let Some(m) = minutes_from_clock(token.trim()) {
                return Some(m);
            }
            let word = token.split_whitespace().next().unwrap_or("");
            if let Some(m) = minutes_from_clock(word) {
                return Some(m);
            }
        }
    }
    None
}

fn find_blocks(text: &str) -> Vec<DraftBlock> {
    let lower = text.to_ascii_lowercase();
    let day_re = regex::Regex::new(
        r"(?i)(mondays?|tuesdays?|wednesdays?|thursdays?|fridays?|saturdays?|sundays?)",
    )
    .unwrap();
    let mut blocks = Vec::new();
    // "robotics club Tuesdays and Thursdays 6–8pm"
    let range = regex::Regex::new(r"(?i)(\d{1,2}(?::\d{2})?\s*(?:[ap]\.?m\.?)?)\s*[–—-]\s*(\d{1,2}(?::\d{2})?\s*(?:[ap]\.?m\.?)?)").unwrap();
    if let Some(m) = range.captures(&lower) {
        let mut days = Vec::new();
        let start = m.get(0).map(|c| c.start()).unwrap_or(0);
        let window = lower[..start]
            .chars()
            .rev()
            .take(80)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        for cap in day_re.captures_iter(&window) {
            if let Some(day) = DraftWeekday::parse(&cap[1]) {
                if !days.contains(&day) {
                    days.push(day);
                }
            }
        }
        if !days.is_empty() {
            let label = label_before(&window);
            blocks.push(DraftBlock {
                label,
                days,
                start: normalize_clock(&m[1]),
                end: normalize_clock(&m[2]),
            });
        }
    }
    blocks
}

fn label_before(window: &str) -> String {
    let words: Vec<&str> = window.split_whitespace().collect();
    let skip = ["and", "on", "the", "a", "i", "have", "my"];
    let kept: Vec<&str> = words
        .into_iter()
        .rev()
        .take(4)
        .filter(|w| {
            !skip.contains(&w.to_ascii_lowercase().as_str()) && DraftWeekday::parse(w).is_none()
        })
        .collect();
    let label = kept.into_iter().rev().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        "Blocked time".into()
    } else {
        label
    }
}

fn normalize_clock(text: &str) -> String {
    minutes_from_clock(text)
        .map(clock_string)
        .unwrap_or_else(|| text.trim().to_string())
}

/// Whether a description can be read by the on-device model. The survey still works either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantAvailability {
    Available,
    DeviceNotEligible,
    NotEnabled,
    NotReady,
    UnsupportedLanguage,
    Unavailable,
}

impl AssistantAvailability {
    pub fn is_available(self) -> bool {
        self == Self::Available
    }
    pub fn note(self) -> Option<&'static str> {
        match self {
            Self::Available => None,
            Self::DeviceNotEligible => Some("Apple Intelligence isn’t available on this device, but the survey below still works."),
            Self::NotEnabled => Some("Turn on Apple Intelligence in Settings to describe your week in your own words. The survey below works without it."),
            Self::NotReady => Some("Apple Intelligence isn’t ready yet. Try again later; the survey below works in the meantime."),
            Self::UnsupportedLanguage => Some("Apple Intelligence doesn’t support your language yet, but the survey below still works."),
            Self::Unavailable => Some("Apple Intelligence isn’t available right now, but the survey below still works."),
        }
    }
}

/// Why a description could not be turned into survey answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantFailure {
    Unavailable(AssistantAvailability),
    EmptyDescription,
    DescriptionTooLong,
    Guardrail,
    ContextTooLarge,
    UnsupportedLanguage,
    Busy,
    TimedOut,
    NothingFound,
    Failed(String),
}

impl AssistantFailure {
    pub fn message(&self) -> String {
        match self {
            Self::Unavailable(availability) => availability.note().unwrap_or("Apple Intelligence isn’t available right now.").to_string(),
            Self::EmptyDescription => "Describe the schedule you want first.".into(),
            Self::DescriptionTooLong => "That’s a lot to read at once. Try describing the most important things in a few sentences.".into(),
            Self::Guardrail => "Apple Intelligence couldn’t use that description. Try rephrasing it, or fill in the survey yourself.".into(),
            Self::ContextTooLarge => "That description is too long for Apple Intelligence. Shorten it, or fill in the survey yourself.".into(),
            Self::UnsupportedLanguage => "Apple Intelligence can’t read that language yet. Try English, or fill in the survey yourself.".into(),
            Self::Busy => "Apple Intelligence is busy right now. Try again in a moment.".into(),
            Self::TimedOut => "Apple Intelligence took too long. Try a shorter description, or fill in the survey yourself.".into(),
            Self::NothingFound => "Couldn’t find any schedule wishes in that. Try mentioning times, days or commitments, like “no classes before 10” or “work Tuesdays 1–5”.".into(),
            Self::Failed(_) => "Something went wrong reading that. Try again, or fill in the survey yourself.".into(),
        }
    }
}
