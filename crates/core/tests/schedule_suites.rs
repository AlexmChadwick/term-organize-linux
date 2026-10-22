#![allow(
    clippy::field_reassign_with_default,
    clippy::cloned_ref_to_slice_refs,
    clippy::manual_is_multiple_of,
    clippy::needless_range_loop,
    clippy::too_many_lines,
    non_snake_case
)]
//! Ports of SchedulePreferencesTests, ScheduleSupportTests, ScheduleGeneratorTests, and ScheduleLogicTests.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::OnceLock;
use termorganize_core::archive::{self, bundled_catalog_path};
use termorganize_core::calendar;
use termorganize_core::index::CatalogIndex;
use termorganize_core::model::*;
use termorganize_core::nl::{
    self, AssistantAvailability, AssistantFailure, DraftBlock, DraftBreaks, DraftFormat,
    DraftLayout, DraftSeats, DraftTime, DraftWeekday, PreferencesDraft,
};
use termorganize_core::offering::OfferingHistory;
use termorganize_core::reorder;
use termorganize_core::schedule::{
    self, Attention, CalendarEvent, ConflictReason, CountProblem, FilterReason, GeneratedSchedule,
    GenerationResult, Limits, NoticeFix, PreferenceRelaxation, Relaxation, ScheduleCourseInput,
    ScheduleSlotMask, ScheduleSort,
};
use termorganize_core::upnext::{self, WidgetMeeting, WidgetScheduleData};
use termorganize_core::CourseEquivalences;
use uuid::Uuid;

fn key(subject: &str, number: &str) -> CourseKey {
    CourseKey {
        subject: subject.into(),
        number: number.into(),
    }
}

fn code(raw: &str) -> CourseKey {
    CourseKey::parse(raw).unwrap_or_else(|| panic!("course {raw}"))
}

struct Extra {
    seats: i32,
    capacity: i32,
    waitlist: i32,
    building: &'static str,
    campus: &'static str,
    method: &'static str,
    instructors: &'static [&'static str],
    start_date: &'static str,
    end_date: &'static str,
}

impl Default for Extra {
    fn default() -> Self {
        Self {
            seats: 10,
            capacity: 40,
            waitlist: 0,
            building: "WVH",
            campus: "Boston",
            method: "Traditional",
            instructors: &[],
            start_date: "2026-09-09",
            end_date: "2026-12-13",
        }
    }
}

fn section(
    crn: &str,
    subject: &str,
    number: &str,
    days: &[i32],
    start: f64,
    end: f64,
    extra: Extra,
) -> CourseSection {
    CourseSection {
        crn: crn.into(),
        subject: subject.into(),
        number: number.into(),
        title: format!("{subject} {number}"),
        section_number: "01".into(),
        part_of_term: "1".into(),
        seat_capacity: extra.capacity,
        seats_remaining: extra.seats,
        enrollment: extra.capacity - extra.seats,
        waitlist_capacity: extra.waitlist,
        waitlist_count: 0,
        waitlist_remaining: extra.waitlist,
        class_type: "Lecture".into(),
        honors: false,
        campus: extra.campus.into(),
        instruction_method: Some(extra.method.into()),
        meetings: vec![Meeting {
            days: DaySet::from_indices(days.iter().copied()),
            start: Some((start * 60.0) as i32),
            end: Some((end * 60.0) as i32),
            building_code: Some(extra.building.into()),
            building: Some(format!("{} Hall", extra.building)),
            room: Some("210".into()),
            campus_code: Some("BOS".into()),
            kind: "Class".into(),
            start_date: Some(extra.start_date.into()),
            end_date: Some(extra.end_date.into()),
        }],
        instructors: Some(
            extra
                .instructors
                .iter()
                .map(|name| Instructor {
                    name: (*name).into(),
                    primary: true,
                    email: None,
                })
                .collect(),
        ),
        cross_list_crns: vec![],
        attributes: vec![],
        description: None,
        prereqs: None,
        coreqs: None,
        credit_hours: None,
        credit_hour_low: None,
        credit_hour_high: None,
        restrictions: None,
    }
}

fn sec(
    crn: &str,
    subject: &str,
    number: &str,
    days: &[i32],
    start: f64,
    end: f64,
) -> CourseSection {
    section(crn, subject, number, days, start, end, Extra::default())
}

fn input(
    subject: &str,
    number: &str,
    required: bool,
    credits: i32,
    sections: Vec<CourseSection>,
) -> ScheduleCourseInput {
    ScheduleCourseInput {
        key: key(subject, number),
        is_required: required,
        sections,
        excluded_crns: HashSet::new(),
        min_credits: credits,
        max_credits: credits,
        unavailable_pin: None,
    }
}

fn req(subject: &str, number: &str, sections: Vec<CourseSection>) -> ScheduleCourseInput {
    input(subject, number, true, 4, sections)
}

fn opt(subject: &str, number: &str, sections: Vec<CourseSection>) -> ScheduleCourseInput {
    input(subject, number, false, 4, sections)
}

fn generate(
    courses: &[ScheduleCourseInput],
    preferences: &SchedulePreferences,
) -> GenerationResult {
    schedule::generate(
        courses,
        &ScheduleFilters::default(),
        preferences,
        ScheduleSort::Best,
        &Limits::default(),
    )
}

fn generate_with(
    courses: &[ScheduleCourseInput],
    filters: &ScheduleFilters,
    preferences: &SchedulePreferences,
    sort: ScheduleSort,
    limits: &Limits,
) -> GenerationResult {
    schedule::generate(courses, filters, preferences, sort, limits)
}

fn crn_sets(result: &GenerationResult) -> HashSet<BTreeSet<String>> {
    result
        .schedules
        .iter()
        .map(|schedule| schedule.crns.iter().cloned().collect())
        .collect()
}

fn expect_sets(result: &GenerationResult, groups: &[&[&str]]) {
    let expected: HashSet<BTreeSet<String>> = groups
        .iter()
        .map(|group| group.iter().map(|crn| (*crn).to_string()).collect())
        .collect();
    assert_eq!(crn_sets(result), expected);
}

fn expect_order(result: &GenerationResult, groups: &[&[&str]]) {
    let actual: Vec<Vec<&str>> = result
        .schedules
        .iter()
        .map(|schedule| schedule.crns.iter().map(String::as_str).collect())
        .collect();
    let expected: Vec<Vec<&str>> = groups.iter().map(|group| group.to_vec()).collect();
    assert_eq!(actual, expected);
}

fn firsts(result: &GenerationResult) -> Vec<String> {
    result
        .schedules
        .iter()
        .map(|schedule| schedule.crns.first().cloned().unwrap_or_default())
        .collect()
}

fn block(label: &str, days: DaySet, start: i32, end: i32) -> BlockedTime {
    BlockedTime::new(label, days, start, end)
}

fn empty_plan(name: &str, term_code: &str) -> SchedulePlan {
    SchedulePlan {
        id: Uuid::new_v4(),
        name: name.into(),
        term_code: term_code.into(),
        courses: vec![],
        selected_crns: vec![],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary: false,
        created_at: 0.0,
        updated_at: 0.0,
    }
}

fn plan_with(codes: &[&str], selected: &[&str]) -> SchedulePlan {
    let mut plan = empty_plan("Plan A", "202710");
    for (offset, code_text) in codes.iter().enumerate() {
        plan.courses.push(PlanCourse {
            key: code(code_text),
            is_required: true,
            excluded_crns: HashSet::new(),
            color_index: offset as i32,

            pinned_crn: None,
            degree_row_id: None,
            credits: None,
        });
    }
    plan.selected_crns = selected.iter().map(|crn| (*crn).to_string()).collect();
    plan
}

fn listed_course(subject: &str, number: &str, credits: f64) -> Course {
    Course {
        subject: subject.into(),
        number: number.into(),
        title: format!("{subject} {number}"),
        description: String::new(),
        min_credits: credits,
        max_credits: credits,
        credit_hours: None,
        attributes: vec![],
        prereqs: None,
        coreqs: None,
        postreqs: vec![],
        special_topics: false,
        college: None,
        department: None,
        details_loaded: true,
    }
}

fn tiny_index(courses: Vec<Course>, sections: Vec<CourseSection>) -> CatalogIndex {
    CatalogIndex::build(TermCatalog {
        version: 1,
        term: Term {
            code: "202710".into(),
            banner_description: "Fall 2026 Semester".into(),
        },
        generated_at: 0.0,
        sections_updated_at: 0.0,
        courses,
        sections,
        subjects: vec![],
        attributes: vec![],
        campuses: vec![],
    })
}

fn clashing_index() -> CatalogIndex {
    tiny_index(
        vec![
            listed_course("CS", "3000", 4.0),
            listed_course("MATH", "2331", 4.0),
            listed_course("ENGW", "1111", 4.0),
        ],
        vec![
            sec("A1", "CS", "3000", &[1, 3], 10.0, 11.5),
            section(
                "B1",
                "MATH",
                "2331",
                &[1],
                11.0,
                12.0,
                Extra {
                    seats: 0,
                    waitlist: 5,
                    ..Extra::default()
                },
            ),
            sec("C1", "ENGW", "1111", &[2, 4], 13.0, 14.5),
        ],
    )
}

fn catalog() -> &'static CatalogIndex {
    static INDEX: OnceLock<CatalogIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        let path = bundled_catalog_path().expect("bundled catalog");
        let catalog = archive::read(&path).expect("read catalog");
        CatalogIndex::build(catalog)
    })
}

fn utc_ymd(unix: f64) -> (i32, i32, i32) {
    let z = unix.floor() as i64;
    let z = z.div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    (year as i32, month as i32, day as i32)
}

fn fully_loaded() -> SchedulePreferences {
    let mut preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::EarlyMorning, TimePreference::Avoid);
    preferences.set_band(TimeBand::Afternoon, TimePreference::Prefer);
    preferences.earliest_start = Some(TimeLimit {
        minutes: 600,
        strength: PreferenceStrength::Preferred,
    });
    preferences.latest_end = Some(TimeLimit {
        minutes: 1_020,
        strength: PreferenceStrength::Required,
    });
    preferences.days_off = DaySet::FRIDAY.union(DaySet::MONDAY);
    preferences.days_off_strength = PreferenceStrength::Required;
    preferences.day_layout = DayLayout::SpreadOut;
    preferences.max_classes_per_day = Some(3);
    preferences.max_minutes_per_day = Some(300);
    preferences.gaps = GapPreference::Compact;
    preferences.breaks = BreakPreference::AtLeast;
    preferences.minimum_break_minutes = 20;
    preferences.minimum_break_strength = PreferenceStrength::Required;
    preferences.lunch = Some(LunchBreak {
        start: 690,
        end: 840,
        minutes: 45,
    });
    let mut work = block("Work", DaySet::MONDAY, 13 * 60, 17 * 60);
    work.strength = PreferenceStrength::Preferred;
    work.is_enabled = false;
    preferences.blocked_times = vec![
        block(
            "Robotics club",
            DaySet::TUESDAY.union(DaySet::THURSDAY),
            18 * 60,
            20 * 60,
        ),
        work,
    ];
    preferences.format = FormatPreference::Hybrid;
    preferences.format_strength = PreferenceStrength::Required;
    preferences.campus = Some("Boston".into());
    preferences.campus_strength = PreferenceStrength::Required;
    preferences.seats = SeatPolicy::WaitlistOk;
    preferences.preferred_instructors = vec!["Smith, Alex".into()];
    preferences.avoided_instructors = vec!["Jones, Sam".into()];
    preferences.credit_target = Some(CreditTarget {
        lower: 16,
        upper: 18,
    });
    preferences
}

fn briefs_example() -> String {
    "No classes before 10, I have robotics club Tuesdays and Thursdays 6\u{2013}8pm, I\u{2019}d like Fridays off and a lunch break".into()
}

fn overeager_draft() -> PreferencesDraft {
    PreferencesDraft {
        earliest_start: Some("10:00".into()),
        time_limits_flexible: Some(true),
        days_off: Some(vec![DraftWeekday::Friday]),
        days_off_required: Some(true),
        layout: Some(DraftLayout::FewerDays),
        max_classes_per_day: Some(3),
        max_hours_per_day: Some(5),
        breaks: Some(DraftBreaks::AvoidBackToBack),
        wants_lunch: Some(true),
        blocked_times: Some(vec![DraftBlock {
            label: "Robotics club".into(),
            days: vec![DraftWeekday::Tuesday, DraftWeekday::Thursday],
            start: "18:00".into(),
            end: "20:00".into(),
        }]),
        format: Some(DraftFormat::Online),
        seats: Some(DraftSeats::OpenSeatsOnly),
        ..PreferencesDraft::default()
    }
}

fn degree_history() -> (DegreePlan, OfferingHistory) {
    let terms = ["202710", "202630", "202610", "202530", "202510"];
    let spring = ["202630", "202530"];
    let fall = ["202710", "202610", "202510"];
    let planned = |raw: &str| DegreePlanCourse::catalog(code(raw), raw, 4.0);
    let mut fall_term = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    fall_term.courses = vec![planned("CS 3386"), planned("CS 3800")];
    let mut spring_term = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    spring_term.courses = vec![planned("CS 4100")];
    let plan = DegreePlan {
        name: "Plan".into(),
        terms: vec![fall_term, spring_term],
        ..DegreePlan::default()
    };
    let mut offered = BTreeMap::new();
    offered.insert(
        code("CS 3386"),
        spring.into_iter().map(str::to_string).collect(),
    );
    offered.insert(
        code("CS 3800"),
        fall.into_iter().map(str::to_string).collect(),
    );
    offered.insert(
        code("CS 4100"),
        fall.into_iter().chain(spring).map(str::to_string).collect(),
    );
    let history = OfferingHistory {
        checked_terms: terms.into_iter().map(str::to_string).collect(),
        offered,
        checked_keys: None,
    };
    (plan, history)
}

fn suggestions_for(
    plan: &DegreePlan,
    origin: Uuid,
    blocking: &[CourseKey],
    history: &OfferingHistory,
) -> Vec<reorder::Suggestion> {
    reorder::suggestions(
        plan,
        origin,
        blocking,
        history,
        |_| Some(None),
        &CourseEquivalences::none(),
        20.0,
    )
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn gen_range(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (hi - lo + 1) as u64;
        lo + (self.next_u64() % span) as i32
    }
    fn gen_bool(&mut self) -> bool {
        self.next_u64() % 2 == 0
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.gen_range(0, items.len() as i32 - 1) as usize]
    }
}

fn too_close_codes(a: &CourseSection, b: &CourseSection, minutes: i32) -> bool {
    for x in a.weekly_meetings() {
        for y in b.weekly_meetings() {
            if x.days.is_disjoint(y.days)
                || !date_windows_overlap(
                    x.start_date.as_deref(),
                    x.end_date.as_deref(),
                    y.start_date.as_deref(),
                    y.end_date.as_deref(),
                )
            {
                continue;
            }
            let (Some(xs), Some(xe), Some(ys), Some(ye)) = (x.start, x.end, y.start, y.end) else {
                continue;
            };
            let gap = if ys >= xe {
                ys - xe
            } else if xs >= ye {
                xs - ye
            } else {
                continue;
            };
            if gap < minutes && x.building_code != y.building_code {
                return true;
            }
        }
    }
    false
}

fn fits_rules(
    section: &CourseSection,
    picked: &[CourseSection],
    preferences: &SchedulePreferences,
) -> bool {
    for other in picked {
        if section.conflicts_with(other) {
            return false;
        }
        if preferences.breaks == BreakPreference::AtLeast
            && preferences.minimum_break_strength == PreferenceStrength::Required
            && too_close_codes(section, other, preferences.minimum_break_minutes)
        {
            return false;
        }
    }
    let mut all = picked.to_vec();
    all.push(section.clone());
    for day in 0..7 {
        let on_day: Vec<&CourseSection> = all
            .iter()
            .filter(|item| item.meeting_days().contains_day(day))
            .collect();
        if let Some(cap) = preferences.max_classes_per_day {
            if on_day.len() as i32 > cap {
                return false;
            }
        }
        if let Some(cap) = preferences.max_minutes_per_day {
            let minutes = on_day
                .iter()
                .map(|item| {
                    let mut seen = HashSet::new();
                    item.weekly_meetings()
                        .into_iter()
                        .filter(|meeting| meeting.days.contains_day(day))
                        .fold(0, |sum, meeting| {
                            let stamp = format!(
                                "{}-{}",
                                meeting.start.unwrap_or(0),
                                meeting.end.unwrap_or(0)
                            );
                            if seen.insert(stamp) {
                                sum + meeting.end.unwrap_or(0) - meeting.start.unwrap_or(0)
                            } else {
                                sum
                            }
                        })
                })
                .sum::<i32>();
            if minutes > cap {
                return false;
            }
        }
    }
    true
}

fn brute_force(
    courses: &[ScheduleCourseInput],
    filters: &ScheduleFilters,
    preferences: &SchedulePreferences,
) -> HashSet<BTreeSet<String>> {
    let kept: Vec<Vec<&CourseSection>> = courses
        .iter()
        .map(|course| {
            course
                .sections
                .iter()
                .filter(|section| {
                    schedule::rejection_reasons(
                        section,
                        filters,
                        preferences,
                        &course.excluded_crns,
                    ) == 0
                })
                .collect()
        })
        .collect();
    let target = filters.course_count.filter(|count| *count > 0);
    let mut result = HashSet::new();
    fn finish(
        courses: &[ScheduleCourseInput],
        kept: &[Vec<&CourseSection>],
        preferences: &SchedulePreferences,
        target: Option<i32>,
        picks: &[(usize, CourseSection)],
        result: &mut HashSet<BTreeSet<String>>,
    ) {
        if picks.is_empty() {
            return;
        }
        if let Some(target) = target {
            if picks.len() as i32 != target {
                return;
            }
        }
        if let Some(credits) = &preferences.credit_target {
            let low: i32 = picks
                .iter()
                .map(|(index, _)| courses[*index].min_credits)
                .sum();
            let high: i32 = picks
                .iter()
                .map(|(index, _)| courses[*index].min_credits.max(courses[*index].max_credits))
                .sum();
            if low > credits.upper || high < credits.lower {
                return;
            }
        } else if target.is_none() {
            let chosen: HashSet<usize> = picks.iter().map(|(index, _)| *index).collect();
            let sections: Vec<CourseSection> =
                picks.iter().map(|(_, section)| section.clone()).collect();
            for index in 0..courses.len() {
                if chosen.contains(&index) {
                    continue;
                }
                if kept[index]
                    .iter()
                    .any(|section| fits_rules(section, &sections, preferences))
                {
                    return;
                }
            }
        }
        result.insert(
            picks
                .iter()
                .map(|(_, section)| section.crn.clone())
                .collect(),
        );
    }
    fn walk(
        courses: &[ScheduleCourseInput],
        kept: &[Vec<&CourseSection>],
        preferences: &SchedulePreferences,
        target: Option<i32>,
        index: usize,
        picks: &mut Vec<(usize, CourseSection)>,
        result: &mut HashSet<BTreeSet<String>>,
    ) {
        if index == courses.len() {
            finish(courses, kept, preferences, target, picks, result);
            return;
        }
        if !courses[index].is_required {
            walk(courses, kept, preferences, target, index + 1, picks, result);
        }
        let chosen: Vec<CourseSection> = picks.iter().map(|(_, section)| section.clone()).collect();
        for section in &kept[index] {
            if fits_rules(section, &chosen, preferences) {
                picks.push((index, (*section).clone()));
                walk(courses, kept, preferences, target, index + 1, picks, result);
                picks.pop();
            }
        }
    }
    walk(
        courses,
        &kept,
        preferences,
        target,
        0,
        &mut vec![],
        &mut result,
    );
    result
}

fn random_instance(
    rng: &mut SplitMix64,
) -> (
    Vec<ScheduleCourseInput>,
    ScheduleFilters,
    SchedulePreferences,
) {
    let starts = [8.0, 9.5, 11.0, 12.5, 14.0, 15.5, 17.0];
    let buildings = ["RY", "SL", "WVH"];
    let day_choices: [&[i32]; 9] = [
        &[1, 3],
        &[2, 4],
        &[1, 3, 5],
        &[1],
        &[2],
        &[3],
        &[4],
        &[5],
        &[2, 5],
    ];
    let windows = [
        ("2026-09-09", "2026-12-13"),
        ("2026-09-09", "2026-10-23"),
        ("2026-10-26", "2026-12-13"),
    ];
    let lengths = [0.75, 1.25, 1.5, 2.0];
    let course_count = rng.gen_range(2, 5);
    let mut courses = vec![];
    for c in 0..course_count {
        let mut sections = vec![];
        for s in 0..rng.gen_range(1, 4) {
            let start = rng.pick(&starts);
            let window = rng.pick(&windows);
            let days = rng.pick(&day_choices);
            sections.push(section(
                &format!("C{c}S{s}"),
                &format!("C{c}"),
                "1",
                days,
                start,
                start + rng.pick(&lengths),
                Extra {
                    building: rng.pick(&buildings),
                    start_date: window.0,
                    end_date: window.1,
                    ..Extra::default()
                },
            ));
        }
        courses.push(input(
            &format!("C{c}"),
            "1",
            rng.gen_bool() || c == 0,
            rng.gen_range(1, 4),
            sections,
        ));
    }
    let mut filters = ScheduleFilters::default();
    if rng.gen_range(0, 5) == 0 {
        filters.course_count = Some(rng.gen_range(1, course_count));
    }
    let mut preferences = SchedulePreferences::default();
    if rng.gen_bool() {
        preferences.max_classes_per_day = Some(rng.gen_range(1, 3));
    }
    if rng.gen_bool() {
        preferences.max_minutes_per_day = Some(rng.pick(&[90, 150, 210, 300]));
    }
    if rng.gen_bool() {
        preferences.breaks = BreakPreference::AtLeast;
        preferences.minimum_break_minutes = rng.pick(&[10, 20, 40]);
        preferences.minimum_break_strength = PreferenceStrength::Required;
    }
    if rng.gen_range(0, 2) == 0 {
        let lower = rng.gen_range(2, 10);
        preferences.credit_target = Some(CreditTarget {
            lower,
            upper: lower + rng.gen_range(0, 5),
        });
    }
    if rng.gen_range(0, 2) == 0 {
        preferences.blocked_times = vec![block(
            "Block",
            DaySet::from_indices(rng.pick(&day_choices).iter().copied()),
            11 * 60,
            14 * 60,
        )];
    }
    (courses, filters, preferences)
}

fn catalog_inputs(index: &CatalogIndex, codes: &[&str]) -> Vec<ScheduleCourseInput> {
    codes
        .iter()
        .filter_map(|raw| {
            let key = CourseKey::parse(raw)?;
            let course = index.course(&key);
            Some(ScheduleCourseInput {
                key: key.clone(),
                is_required: true,
                sections: index.sections_for(&key),
                excluded_crns: HashSet::new(),
                min_credits: course
                    .map(|course| course.min_credits.ceil() as i32)
                    .unwrap_or(0),
                max_credits: course
                    .map(|course| course.max_credits.ceil() as i32)
                    .unwrap_or(0),
                unavailable_pin: None,
            })
        })
        .collect()
}

fn meeting(days: &[i32], start: i32, end: i32, from: Option<&str>, to: Option<&str>) -> Meeting {
    Meeting {
        days: DaySet::from_indices(days.iter().copied()),
        start: Some(start),
        end: Some(end),
        building_code: Some("RI".into()),
        building: Some("Richards Hall".into()),
        room: Some("300".into()),
        campus_code: Some("BOS".into()),
        kind: "Class".into(),
        start_date: from.map(str::to_string),
        end_date: to.map(str::to_string),
    }
}

// MARK: - The model

#[test]
fn defaultsHaveNoOpinions() {
    let preferences = SchedulePreferences::default();
    assert!(preferences.is_default());
    assert_eq!(preferences.active_count(), 0);
    assert!(preferences.summary_parts().is_empty());
    assert_eq!(preferences.summary(3), "No preferences yet");
}

#[test]
fn settingABandBackToOkayRestoresTheDefault() {
    let mut preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::Morning, TimePreference::Prefer);
    assert!(!preferences.is_default());
    preferences.set_band(TimeBand::Morning, TimePreference::Okay);
    assert!(preferences.is_default());
}

#[test]
fn summaryReadsLikeTheBrief() {
    let mut preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::EarlyMorning, TimePreference::Prefer);
    preferences.set_band(TimeBand::Morning, TimePreference::Prefer);
    preferences.days_off = DaySet::FRIDAY;
    preferences.blocked_times = vec![
        block(
            "Club",
            DaySet::TUESDAY.union(DaySet::THURSDAY),
            18 * 60,
            20 * 60,
        ),
        block("Work", DaySet::MONDAY, 13 * 60, 17 * 60),
    ];
    assert_eq!(
        preferences.summary_parts(),
        vec!["Mornings", "Fridays off", "2 blocked times"]
    );
    assert_eq!(
        preferences.summary(3),
        "Mornings \u{00b7} Fridays off \u{00b7} 2 blocked times"
    );
    preferences.day_layout = DayLayout::FewerDays;
    assert_eq!(
        preferences.summary(3),
        "Mornings \u{00b7} Fridays off \u{00b7} Fewer days \u{00b7} +1"
    );
}

#[test]
fn summaryNamesTheDaysAndTimes() {
    let mut preferences = SchedulePreferences::default();
    preferences.days_off = DaySet::MONDAY.union(DaySet::FRIDAY);
    preferences.earliest_start = Some(TimeLimit {
        minutes: 10 * 60,
        strength: PreferenceStrength::Required,
    });
    preferences.latest_end = Some(TimeLimit {
        minutes: 17 * 60 + 30,
        strength: PreferenceStrength::Preferred,
    });
    assert_eq!(
        preferences.summary_parts(),
        vec![
            "Nothing before 10 AM",
            "Prefer done by 5:30 PM",
            "Mon & Fri off"
        ]
    );
    preferences.set_band(TimeBand::Evening, TimePreference::Avoid);
    preferences.set_band(TimeBand::Night, TimePreference::Avoid);
    assert_eq!(
        preferences.summary_parts().first().map(String::as_str),
        Some("No evenings")
    );
}

#[test]
fn disabledBlocksDontCount() {
    let mut preferences = SchedulePreferences::default();
    let mut disabled = block("Work", DaySet::MONDAY, 600, 700);
    disabled.is_enabled = false;
    preferences.blocked_times = vec![disabled];
    assert!(preferences.active_blocked_times().is_empty());
    assert_eq!(preferences.active_count(), 0);
    assert!(preferences.summary_parts().is_empty());
    assert!(
        !preferences.is_default(),
        "A switched-off block is still the student's data"
    );
}

#[test]
fn everyWishShowsUpInTheSummary() {
    let mut wishes = vec![];
    let mut push = |change: &dyn Fn(&mut SchedulePreferences)| {
        let mut preferences = SchedulePreferences::default();
        change(&mut preferences);
        wishes.push(preferences);
    };
    push(&|p| p.set_band(TimeBand::Night, TimePreference::Avoid));
    push(&|p| {
        p.earliest_start = Some(TimeLimit {
            minutes: 600,
            strength: PreferenceStrength::Required,
        })
    });
    push(&|p| {
        p.latest_end = Some(TimeLimit {
            minutes: 1_020,
            strength: PreferenceStrength::Preferred,
        })
    });
    push(&|p| p.days_off = DaySet::FRIDAY);
    push(&|p| p.day_layout = DayLayout::SpreadOut);
    push(&|p| p.max_classes_per_day = Some(2));
    push(&|p| p.max_minutes_per_day = Some(240));
    push(&|p| p.gaps = GapPreference::Fine);
    push(&|p| p.gaps = GapPreference::Compact);
    push(&|p| p.breaks = BreakPreference::AvoidBackToBack);
    push(&|p| p.breaks = BreakPreference::AtLeast);
    push(&|p| p.lunch = Some(LunchBreak::default()));
    push(&|p| p.blocked_times = vec![block("Work", DaySet::MONDAY, 600, 700)]);
    push(&|p| p.format = FormatPreference::Online);
    push(&|p| p.campus = Some("Boston".into()));
    push(&|p| p.seats = SeatPolicy::WaitlistOk);
    push(&|p| p.preferred_instructors = vec!["Smith, Alex".into()]);
    push(&|p| p.avoided_instructors = vec!["Jones, Sam".into()]);
    push(&|p| {
        p.credit_target = Some(CreditTarget {
            lower: 16,
            upper: 18,
        })
    });
    for preferences in &wishes {
        assert!(preferences.has_wishes());
        assert!(
            !preferences.summary_parts().is_empty(),
            "{preferences:?} has a wish the summary doesn't mention"
        );
        assert!(preferences.active_count() > 0);
    }
    assert!(!SchedulePreferences::default().has_wishes());
}

#[test]
fn presetsToggleAndCombine() {
    let mut preferences = SchedulePreferences::default();
    preferences = SchedulePreset::MorningPerson.toggled(&preferences);
    assert!(SchedulePreset::MorningPerson.is_applied(&preferences));
    assert_eq!(preferences.band(TimeBand::Morning), TimePreference::Prefer);
    assert_eq!(preferences.band(TimeBand::Night), TimePreference::Avoid);

    preferences = SchedulePreset::LongWeekends.toggled(&preferences);
    assert!(SchedulePreset::LongWeekends.is_applied(&preferences));
    assert!(
        SchedulePreset::MorningPerson.is_applied(&preferences),
        "Presets only touch their own wishes"
    );
    assert!(preferences.days_off.contains(DaySet::FRIDAY));
    assert_eq!(preferences.day_layout, DayLayout::FewerDays);

    preferences = SchedulePreset::NightOwl.toggled(&preferences);
    assert!(SchedulePreset::NightOwl.is_applied(&preferences));
    assert!(!SchedulePreset::MorningPerson.is_applied(&preferences));
    assert_eq!(preferences.band(TimeBand::Morning), TimePreference::Avoid);
    assert_eq!(preferences.band(TimeBand::Evening), TimePreference::Prefer);

    preferences = SchedulePreset::NightOwl.toggled(&preferences);
    preferences = SchedulePreset::LongWeekends.toggled(&preferences);
    assert!(
        preferences.is_default(),
        "Switching every preset off leaves nothing behind"
    );
}

#[test]
fn compactDaysSetsLayoutAndGaps() {
    let mut preferences = SchedulePreferences::default();
    preferences = SchedulePreset::CompactDays.toggled(&preferences);
    assert_eq!(preferences.day_layout, DayLayout::FewerDays);
    assert_eq!(preferences.gaps, GapPreference::Compact);
    preferences = SchedulePreset::CompactDays.toggled(&preferences);
    assert!(preferences.is_default());
}

#[test]
fn normalizingDropsWhatCantBeApplied() {
    let mut preferences = SchedulePreferences::default();
    preferences.earliest_start = Some(TimeLimit {
        minutes: 600,
        strength: PreferenceStrength::Required,
    });
    preferences.latest_end = Some(TimeLimit {
        minutes: 540,
        strength: PreferenceStrength::Required,
    });
    preferences.max_classes_per_day = Some(0);
    preferences.max_minutes_per_day = Some(10);
    preferences.minimum_break_minutes = 1_000;
    preferences.lunch = Some(LunchBreak {
        start: 700,
        end: 705,
        minutes: 600,
    });
    preferences.blocked_times = vec![
        block("  ", DaySet::MONDAY, 600, 700),
        block("No days", DaySet(0), 600, 700),
        block("Backwards", DaySet::MONDAY, 700, 600),
        block("Club", DaySet::TUESDAY, 600, 700),
        block("club", DaySet::TUESDAY, 600, 700),
    ];
    preferences.preferred_instructors =
        vec!["Smith, Alex".into(), " smith, alex ".into(), String::new()];
    preferences.avoided_instructors = vec!["Smith, Alex".into(), "Jones, Sam".into()];
    preferences.credit_target = Some(CreditTarget {
        lower: 40,
        upper: 50,
    });
    let normalized = preferences.normalized();
    assert_eq!(
        normalized.latest_end,
        Some(TimeLimit {
            minutes: 540,
            strength: PreferenceStrength::Required
        }),
        "A finish before the start is kept: the generator says no schedule fits"
    );
    assert_eq!(normalized.max_classes_per_day, None);
    assert_eq!(normalized.max_minutes_per_day, None);
    assert_eq!(normalized.minimum_break_minutes, 90);
    assert_eq!(
        normalized
            .lunch
            .map(|lunch| lunch.end - lunch.start >= lunch.minutes),
        Some(true)
    );
    assert_eq!(
        normalized
            .blocked_times
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Blocked time", "Club"]
    );
    assert_eq!(normalized.preferred_instructors, vec!["Smith, Alex"]);
    assert_eq!(
        normalized.avoided_instructors,
        vec!["Jones, Sam"],
        "An instructor can't be both preferred and avoided"
    );
    assert_eq!(
        normalized.credit_target.as_ref().map(|target| target.upper),
        Some(30)
    );
}

#[test]
fn changedFieldsListWhatDiffers() {
    let old = SchedulePreferences::default();
    let mut new = old.clone();
    new.set_band(TimeBand::Morning, TimePreference::Avoid);
    new.earliest_start = Some(TimeLimit {
        minutes: 600,
        strength: PreferenceStrength::Required,
    });
    new.days_off = DaySet::FRIDAY;
    let club = block("Club", DaySet::TUESDAY, 1_080, 1_200);
    new.blocked_times = vec![club.clone()];
    new.lunch = Some(LunchBreak::default());
    let changed: HashSet<PreferenceField> = new.changed_fields(&old).into_iter().collect();
    let expected = HashSet::from([
        PreferenceField::TimeOfDay(TimeBand::Morning),
        PreferenceField::EarliestStart,
        PreferenceField::DaysOff,
        PreferenceField::BlockedTime(club.id),
        PreferenceField::Lunch,
    ]);
    assert_eq!(changed, expected);
    assert!(new.changed_fields(&new).is_empty());
}

// MARK: - Saving

#[test]
fn roundTrips() {
    let original = fully_loaded();
    let data = serde_json::to_vec(&original).unwrap();
    let decoded: SchedulePreferences = serde_json::from_slice(&data).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn anEmptyObjectDecodesToTheDefaults() {
    let decoded: SchedulePreferences = serde_json::from_str("{}").unwrap();
    assert!(decoded.is_default());
}

#[test]
fn unknownAndDamagedValuesFallBackInsteadOfFailing() {
    let json = r#"{
      "timeOfDay": {"morning": "prefer", "brunch": "prefer", "night": "sometimes"},
      "dayLayout": "teleport",
      "earliestStart": {"minutes": "soon"},
      "daysOff": 32,
      "maxClassesPerDay": 3,
      "blockedTimes": [
        {"label": "Club", "days": 20, "start": 1080, "end": 1200},
        "garbage",
        {"label": 7}
      ],
      "futureField": [1, 2, 3]
    }"#;
    let decoded: SchedulePreferences = serde_json::from_str(json).unwrap();
    assert_eq!(decoded.band(TimeBand::Morning), TimePreference::Prefer);
    assert_eq!(
        decoded.band(TimeBand::Night),
        TimePreference::Okay,
        "A value this version doesn't know is ignored"
    );
    assert_eq!(decoded.day_layout, DayLayout::NoPreference);
    assert_eq!(decoded.earliest_start, None);
    assert_eq!(decoded.days_off, DaySet::FRIDAY);
    assert_eq!(decoded.max_classes_per_day, Some(3));
    assert_eq!(
        decoded
            .blocked_times
            .iter()
            .find(|item| item.label == "Club")
            .map(|item| item.days),
        Some(DaySet(20))
    );
    assert_eq!(
        decoded
            .active_blocked_times()
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Club"],
        "The damaged blocks never become active"
    );
}

#[test]
fn plansSavedBeforePreferencesExistStillLoad() {
    let mut plan = empty_plan("Plan A", "202730");
    plan.courses = vec![PlanCourse {
        key: key("CS", "3000"),
        is_required: true,
        excluded_crns: HashSet::new(),
        color_index: 0,

        pinned_crn: None,
        degree_row_id: None,
        credits: None,
    }];
    let mut value = serde_json::to_value(&plan).unwrap();
    assert!(
        value.get("preferences").is_none(),
        "Plans without preferences don't write the key"
    );
    if let Some(object) = value.as_object_mut() {
        object.remove("preferences");
    }
    let decoded: SchedulePlan = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.name, "Plan A");
    assert!(decoded.preferences.is_none());
    assert_eq!(decoded.courses.len(), 1);
}

#[test]
fn planPreferencesSurviveASaveAndLoad() {
    let preferences = fully_loaded();
    let mut plan = empty_plan("Plan B", "202730");
    plan.preferences = Some(preferences.clone());
    let mut data = UserData::default();
    data.plans = vec![plan];
    let decoded: UserData = serde_json::from_slice(&serde_json::to_vec(&data).unwrap()).unwrap();
    assert_eq!(decoded.plans.len(), 1);
    assert_eq!(
        decoded
            .plans
            .first()
            .and_then(|plan| plan.preferences.clone()),
        Some(preferences)
    );
}

#[test]
fn theStoreSavesPreferencesWithThePlan() {
    let preferences = fully_loaded();
    let mut data = UserData::default();
    let plan = schedule::create_plan(&mut data, "Fall", "202710");
    let other = schedule::create_plan(&mut data, "Other", "202710");
    schedule::update_plan(&mut data, plan.id, |plan| {
        plan.preferences = Some(preferences.clone())
    });
    let reloaded: UserData = serde_json::from_slice(&serde_json::to_vec(&data).unwrap()).unwrap();
    assert_eq!(
        schedule::plan_by_id(&reloaded, plan.id).and_then(|plan| plan.preferences.clone()),
        Some(preferences.clone())
    );
    assert!(
        schedule::plan_by_id(&reloaded, other.id)
            .and_then(|plan| plan.preferences.clone())
            .is_none(),
        "Preferences belong to one schedule"
    );
    let mut saved = reloaded;
    let copy = schedule::duplicate_plan(&mut saved, plan.id).expect("duplicate");
    assert_eq!(
        schedule::plan_by_id(&saved, copy.id).and_then(|plan| plan.preferences.clone()),
        Some(preferences)
    );
}

#[test]
fn preferencesOfAnyShapeNeverTakeTheScheduleDown() {
    let plan = empty_plan("Plan D", "202730");
    let object = serde_json::to_value(&plan).unwrap();
    for garbage in [
        serde_json::json!(7),
        serde_json::json!("text"),
        serde_json::json!([1, 2, 3]),
        serde_json::json!(true),
    ] {
        let mut damaged = object.clone();
        damaged["preferences"] = garbage;
        let decoded: SchedulePlan = serde_json::from_value(damaged).unwrap();
        assert_eq!(decoded.name, "Plan D");
        assert!(decoded
            .preferences
            .as_ref()
            .map(|preferences| preferences.is_default())
            .unwrap_or(true));
    }
    let mut other = object.clone();
    other["preferences"] = serde_json::json!("text");
    let file = serde_json::json!({"version": 1, "plans": [object, other]});
    let decoded: UserData = serde_json::from_value(file).unwrap();
    assert_eq!(decoded.plans.len(), 2);
}

#[test]
fn damagedPreferencesNeverTakeTheScheduleDown() {
    let plan = empty_plan("Plan C", "202730");
    let mut object = serde_json::to_value(&plan).unwrap();
    object["preferences"] =
        serde_json::json!({"timeOfDay": 7, "blockedTimes": "nope", "campus": 3});
    let decoded: SchedulePlan = serde_json::from_value(object).unwrap();
    assert_eq!(decoded.name, "Plan C");
    assert_eq!(
        decoded
            .preferences
            .as_ref()
            .map(|preferences| preferences.is_default()),
        Some(true)
    );
}

// MARK: - Hard rules

#[test]
fn aBlockedTimeRemovesSectionsThatOverlapIt() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("EVE", "CS", "3000", &[2, 4], 18.0, 19.5),
            sec("DAY", "CS", "3000", &[1, 3, 5], 10.0, 11.0),
            sec("LATE", "CS", "3000", &[2, 4], 20.0, 21.0),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.blocked_times = vec![block(
        "Club meeting",
        DaySet::TUESDAY.union(DaySet::THURSDAY),
        18 * 60,
        20 * 60,
    )];
    expect_sets(
        &generate(&[course.clone()], &preferences),
        &[&["DAY"], &["LATE"]],
    );
    assert_eq!(
        generate(&[course], &SchedulePreferences::default()).total_found,
        3
    );
}

#[test]
fn blockedTimesOnlyApplyOnTheirDaysAndHours() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("MON", "CS", "3000", &[1], 18.0, 19.5),
            sec("TUE", "CS", "3000", &[2], 18.0, 19.5),
            sec("EARLY", "CS", "3000", &[2], 16.0, 18.0),
            sec("OVER", "CS", "3000", &[2], 19.5, 21.0),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.blocked_times = vec![block("Club", DaySet::TUESDAY, 18 * 60, 20 * 60)];
    expect_sets(&generate(&[course], &preferences), &[&["MON"], &["EARLY"]]);
}

#[test]
fn aSwitchedOffBlockIsIgnored() {
    let course = req("CS", "3000", vec![sec("A", "CS", "3000", &[2], 18.0, 19.0)]);
    let mut preferences = SchedulePreferences::default();
    let mut club = block("Club", DaySet::TUESDAY, 18 * 60, 20 * 60);
    club.is_enabled = false;
    preferences.blocked_times = vec![club];
    assert_eq!(generate(&[course], &preferences).total_found, 1);
}

#[test]
fn aPreferredBlockOnlyRanksLower() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("CLASH", "CS", "3000", &[2], 18.0, 19.0),
            sec("FREE", "CS", "3000", &[3], 18.0, 19.0),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    let mut club = block("Club", DaySet::TUESDAY, 18 * 60, 20 * 60);
    club.strength = PreferenceStrength::Preferred;
    preferences.blocked_times = vec![club];
    let result = generate(&[course], &preferences);
    expect_order(&result, &[&["FREE"], &["CLASH"]]);
    assert!(result
        .schedules
        .last()
        .unwrap()
        .reasons
        .iter()
        .any(|reason| !reason.benefit && reason.text == "Overlaps Club"));
}

#[test]
fn daysOffAreHardOnlyWhenRequired() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("FRI", "CS", "3000", &[5], 10.0, 11.0),
            sec("TUE", "CS", "3000", &[2], 10.0, 11.0),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.days_off = DaySet::FRIDAY;
    preferences.days_off_strength = PreferenceStrength::Required;
    expect_order(&generate(&[course.clone()], &preferences), &[&["TUE"]]);
    preferences.days_off_strength = PreferenceStrength::Preferred;
    expect_order(&generate(&[course], &preferences), &[&["TUE"], &["FRI"]]);
}

#[test]
fn earliestStartAndLatestEndFilterWhenRequired() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("EARLY", "CS", "3000", &[1], 8.0, 9.5),
            sec("MID", "CS", "3000", &[2], 10.0, 11.5),
            sec("LATE", "CS", "3000", &[3], 16.0, 18.5),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.earliest_start = Some(TimeLimit {
        minutes: 10 * 60,
        strength: PreferenceStrength::Required,
    });
    expect_sets(
        &generate(&[course.clone()], &preferences),
        &[&["MID"], &["LATE"]],
    );
    preferences.latest_end = Some(TimeLimit {
        minutes: 17 * 60,
        strength: PreferenceStrength::Required,
    });
    expect_sets(&generate(&[course], &preferences), &[&["MID"]]);
}

#[test]
fn softLimitsRankByHowFarOutsideTheyAre() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("VERY", "CS", "3000", &[1], 8.0, 9.0),
            sec("SLIGHTLY", "CS", "3000", &[2], 9.75, 11.0),
            sec("OK", "CS", "3000", &[3], 10.0, 11.0),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.earliest_start = Some(TimeLimit {
        minutes: 10 * 60,
        strength: PreferenceStrength::Preferred,
    });
    expect_order(
        &generate(&[course], &preferences),
        &[&["OK"], &["SLIGHTLY"], &["VERY"]],
    );
}

#[test]
fn requiredFormatSeatsAndCampusFilter() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("INPERSON", "CS", "3000", &[1], 9.0, 10.0),
            section(
                "HYBRID",
                "CS",
                "3000",
                &[2],
                9.0,
                10.0,
                Extra {
                    method: "Hybrid",
                    ..Extra::default()
                },
            ),
            section(
                "ONLINE",
                "CS",
                "3000",
                &[3],
                9.0,
                10.0,
                Extra {
                    campus: "Online",
                    method: "Online",
                    ..Extra::default()
                },
            ),
            section(
                "FULL",
                "CS",
                "3000",
                &[4],
                9.0,
                10.0,
                Extra {
                    seats: 0,
                    waitlist: 5,
                    ..Extra::default()
                },
            ),
            section(
                "FULLNOLIST",
                "CS",
                "3000",
                &[5],
                9.0,
                10.0,
                Extra {
                    seats: 0,
                    ..Extra::default()
                },
            ),
            section(
                "OAKLAND",
                "CS",
                "3000",
                &[1],
                12.0,
                13.0,
                Extra {
                    campus: "Oakland, CA",
                    ..Extra::default()
                },
            ),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.format = FormatPreference::InPerson;
    preferences.format_strength = PreferenceStrength::Required;
    expect_sets(
        &generate(&[course.clone()], &preferences),
        &[&["INPERSON"], &["FULL"], &["FULLNOLIST"], &["OAKLAND"]],
    );
    preferences.seats = SeatPolicy::OpenOnly;
    expect_sets(
        &generate(&[course.clone()], &preferences),
        &[&["INPERSON"], &["OAKLAND"]],
    );
    preferences.seats = SeatPolicy::WaitlistOk;
    expect_sets(
        &generate(&[course.clone()], &preferences),
        &[&["INPERSON"], &["FULL"], &["OAKLAND"]],
    );
    preferences.campus = Some("Boston".into());
    preferences.campus_strength = PreferenceStrength::Required;
    expect_sets(
        &generate(&[course], &preferences),
        &[&["INPERSON"], &["FULL"]],
    );
}

#[test]
fn sectionsWithoutASeatLimitAreNeverFull() {
    let course = req(
        "CS",
        "3000",
        vec![section(
            "OPEN",
            "CS",
            "3000",
            &[1],
            9.0,
            10.0,
            Extra {
                seats: 0,
                capacity: 0,
                ..Extra::default()
            },
        )],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.seats = SeatPolicy::OpenOnly;
    assert_eq!(generate(&[course], &preferences).total_found, 1);
}

#[test]
fn maxClassesPerDayLimitsHowManySectionsShareADay() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 8.0, 9.0)]),
        req(
            "B",
            "1",
            vec![
                sec("B1", "B", "1", &[1], 10.0, 11.0),
                sec("B2", "B", "1", &[2], 10.0, 11.0),
            ],
        ),
        req(
            "C",
            "1",
            vec![
                sec("C1", "C", "1", &[1], 12.0, 13.0),
                sec("C2", "C", "1", &[3], 12.0, 13.0),
            ],
        ),
    ];
    assert_eq!(
        generate(&courses, &SchedulePreferences::default()).total_found,
        4
    );
    let mut preferences = SchedulePreferences::default();
    preferences.max_classes_per_day = Some(2);
    expect_sets(
        &generate(&courses, &preferences),
        &[
            &["A1", "B1", "C2"],
            &["A1", "B2", "C1"],
            &["A1", "B2", "C2"],
        ],
    );
    preferences.max_classes_per_day = Some(1);
    expect_sets(&generate(&courses, &preferences), &[&["A1", "B2", "C2"]]);
}

#[test]
fn maxMinutesPerDayLimitsClassTimeOnOneDay() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 11.0)]),
        req(
            "B",
            "1",
            vec![
                sec("B1", "B", "1", &[1], 13.0, 15.0),
                sec("B2", "B", "1", &[2], 13.0, 15.0),
            ],
        ),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.max_minutes_per_day = Some(3 * 60);
    expect_sets(&generate(&courses, &preferences), &[&["A1", "B2"]]);
    preferences.max_minutes_per_day = Some(90);
    let result = generate(&courses, &preferences);
    assert_eq!(
        result.total_found, 0,
        "A 2-hour class can never fit a 90 minute day"
    );
    assert_eq!(
        result
            .diagnostics
            .courses
            .first()
            .and_then(|report| report.rejections.get(&FilterReason::PrefWorkload).copied()),
        Some(1)
    );
}

#[test]
fn requiredWalkingTimeOnlyMattersBetweenBuildings() {
    let courses = vec![
        req(
            "A",
            "1",
            vec![section(
                "A1",
                "A",
                "1",
                &[1],
                9.0,
                10.0,
                Extra {
                    building: "RY",
                    ..Extra::default()
                },
            )],
        ),
        req(
            "B",
            "1",
            vec![
                section(
                    "SAME",
                    "B",
                    "1",
                    &[1],
                    10.1,
                    11.0,
                    Extra {
                        building: "RY",
                        ..Extra::default()
                    },
                ),
                section(
                    "NEAR",
                    "B",
                    "1",
                    &[1],
                    10.1,
                    11.0,
                    Extra {
                        building: "SL",
                        ..Extra::default()
                    },
                ),
                section(
                    "FAR",
                    "B",
                    "1",
                    &[1],
                    10.5,
                    11.5,
                    Extra {
                        building: "SL",
                        ..Extra::default()
                    },
                ),
                section(
                    "OTHERDAY",
                    "B",
                    "1",
                    &[2],
                    10.1,
                    11.0,
                    Extra {
                        building: "SL",
                        ..Extra::default()
                    },
                ),
            ],
        ),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.breaks = BreakPreference::AtLeast;
    preferences.minimum_break_minutes = 15;
    preferences.minimum_break_strength = PreferenceStrength::Required;
    let result = generate(&courses, &preferences);
    expect_sets(
        &result,
        &[&["A1", "SAME"], &["A1", "FAR"], &["A1", "OTHERDAY"]],
    );
    assert!(result.diagnostics.conflicting_pairs.is_empty());
}

#[test]
fn aTightWalkIsReportedWhenItIsTheOnlyProblem() {
    let courses = vec![
        req(
            "A",
            "1",
            vec![section(
                "A1",
                "A",
                "1",
                &[1],
                9.0,
                10.0,
                Extra {
                    building: "RY",
                    ..Extra::default()
                },
            )],
        ),
        req(
            "B",
            "1",
            vec![section(
                "B1",
                "B",
                "1",
                &[1],
                10.1,
                11.0,
                Extra {
                    building: "SL",
                    ..Extra::default()
                },
            )],
        ),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.breaks = BreakPreference::AtLeast;
    preferences.minimum_break_minutes = 15;
    preferences.minimum_break_strength = PreferenceStrength::Required;
    let result = generate(&courses, &preferences);
    assert_eq!(result.total_found, 0);
    assert_eq!(
        result
            .diagnostics
            .conflicting_pairs
            .iter()
            .map(|pair| pair.reason)
            .collect::<Vec<_>>(),
        vec![ConflictReason::TravelTime]
    );
    preferences.minimum_break_strength = PreferenceStrength::Preferred;
    let relaxed = generate(&courses, &preferences);
    assert_eq!(relaxed.total_found, 1);
    assert!(relaxed
        .schedules
        .first()
        .unwrap()
        .reasons
        .iter()
        .any(|reason| !reason.benefit && reason.text == "Tight between buildings"));
}

#[test]
fn halfTermSectionsAtTheSameTimeStillDontConflictWithPreferences() {
    let courses = vec![
        req(
            "A",
            "1",
            vec![section(
                "A1",
                "A",
                "1",
                &[1, 3],
                10.0,
                11.5,
                Extra {
                    start_date: "2026-09-09",
                    end_date: "2026-10-23",
                    ..Extra::default()
                },
            )],
        ),
        req(
            "B",
            "1",
            vec![section(
                "B1",
                "B",
                "1",
                &[1, 3],
                10.0,
                11.5,
                Extra {
                    start_date: "2026-10-26",
                    end_date: "2026-12-13",
                    ..Extra::default()
                },
            )],
        ),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::Morning, TimePreference::Prefer);
    preferences.max_classes_per_day = Some(3);
    preferences.breaks = BreakPreference::AtLeast;
    preferences.minimum_break_strength = PreferenceStrength::Required;
    assert_eq!(generate(&courses, &preferences).total_found, 1);
    assert_eq!(
        generate(&courses, &SchedulePreferences::default()).total_found,
        1
    );
}

#[test]
fn requiredCoursesAreStillRequired() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        req("B", "1", vec![sec("B1", "B", "1", &[1], 9.0, 10.0)]),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::Morning, TimePreference::Prefer);
    assert_eq!(generate(&courses, &preferences).total_found, 0);
}

#[test]
fn creditTargetsReplaceTheMaximalRule() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        opt("B", "1", vec![sec("B1", "B", "1", &[2], 9.0, 10.0)]),
        opt("C", "1", vec![sec("C1", "C", "1", &[3], 9.0, 10.0)]),
    ];
    expect_sets(
        &generate(&courses, &SchedulePreferences::default()),
        &[&["A1", "B1", "C1"]],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.credit_target = Some(CreditTarget { lower: 8, upper: 8 });
    expect_sets(
        &generate(&courses, &preferences),
        &[&["A1", "B1"], &["A1", "C1"]],
    );
    preferences.credit_target = Some(CreditTarget { lower: 4, upper: 8 });
    expect_sets(
        &generate(&courses, &preferences),
        &[&["A1"], &["A1", "B1"], &["A1", "C1"]],
    );
    preferences.credit_target = Some(CreditTarget {
        lower: 12,
        upper: 16,
    });
    expect_sets(&generate(&courses, &preferences), &[&["A1", "B1", "C1"]]);
}

#[test]
fn impossibleCreditTargetsAreExplained() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        opt("B", "1", vec![sec("B1", "B", "1", &[2], 9.0, 10.0)]),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.credit_target = Some(CreditTarget { lower: 2, upper: 3 });
    assert_eq!(
        generate(&courses, &preferences).diagnostics.count_problem,
        Some(CountProblem::CreditsOverTarget {
            required: 4,
            limit: 3
        })
    );
    preferences.credit_target = Some(CreditTarget {
        lower: 12,
        upper: 14,
    });
    assert_eq!(
        generate(&courses, &preferences).diagnostics.count_problem,
        Some(CountProblem::CreditsUnderTarget {
            available: 8,
            wanted: 12
        })
    );
}

#[test]
fn diagnosticsNameThePreferenceThatRemovedSections() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("A", "CS", "3000", &[2], 18.0, 19.0),
            sec("B", "CS", "3000", &[4], 18.5, 19.5),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.blocked_times = vec![block(
        "Club",
        DaySet::TUESDAY.union(DaySet::THURSDAY),
        18 * 60,
        20 * 60,
    )];
    let result = generate(&[course], &preferences);
    assert_eq!(result.total_found, 0);
    let report = result.diagnostics.courses.first().unwrap();
    assert!(report.is_blocked());
    assert_eq!(
        report.rejections.get(&FilterReason::BlockedTime).copied(),
        Some(2)
    );
}

#[test]
fn relaxationsOfferToSoftenWhatIsInTheWay() {
    let course = req("CS", "3000", vec![sec("A", "CS", "3000", &[2], 18.0, 19.0)]);
    let mut preferences = SchedulePreferences::default();
    let club = block("Club", DaySet::TUESDAY, 18 * 60, 20 * 60);
    preferences.blocked_times = vec![club.clone()];
    preferences.earliest_start = Some(TimeLimit {
        minutes: 20 * 60,
        strength: PreferenceStrength::Required,
    });
    let suggestions = schedule::suggest_relaxations(
        &[course.clone()],
        &ScheduleFilters::default(),
        &preferences,
        true,
    );
    assert!(suggestions.is_empty());
    preferences.earliest_start = None;
    let single = schedule::suggest_relaxations(
        &[course.clone()],
        &ScheduleFilters::default(),
        &preferences,
        true,
    );
    assert_eq!(
        single
            .iter()
            .map(|suggestion| suggestion.relaxation.clone())
            .collect::<Vec<_>>(),
        vec![Relaxation::Preference(
            PreferenceRelaxation::SoftenBlockedTime {
                id: club.id,
                label: "Club".into()
            }
        )]
    );
    let mut relaxed = preferences;
    single[0].relaxation.apply_preferences(&mut relaxed);
    assert_eq!(
        relaxed.blocked_times.first().map(|item| item.strength),
        Some(PreferenceStrength::Preferred)
    );
    assert_eq!(generate(&[course], &relaxed).total_found, 1);
}

fn morning_or_night() -> ScheduleCourseInput {
    req(
        "CS",
        "3000",
        vec![
            sec("NIGHT", "CS", "3000", &[2, 4], 19.0, 20.5),
            sec("MORNING", "CS", "3000", &[2, 4], 9.5, 11.0),
            sec("AFTERNOON", "CS", "3000", &[2, 4], 14.5, 16.0),
        ],
    )
}

#[test]
fn withoutPreferencesTheDefaultOrderIsUnchanged() {
    let result = generate(&[morning_or_night()], &SchedulePreferences::default());
    assert_eq!(firsts(&result).last().map(String::as_str), Some("NIGHT"));
    assert!(result
        .schedules
        .iter()
        .all(|schedule| schedule.reasons.is_empty()));
}

#[test]
fn aMorningPersonSeesMorningsFirstAndANightOwlSeesNightsFirst() {
    let morning = SchedulePreset::MorningPerson.toggled(&SchedulePreferences::default());
    assert_eq!(
        firsts(&generate(&[morning_or_night()], &morning)),
        vec![
            "MORNING".to_string(),
            "AFTERNOON".to_string(),
            "NIGHT".to_string()
        ]
    );
    let night = SchedulePreset::NightOwl.toggled(&SchedulePreferences::default());
    let order = firsts(&generate(&[morning_or_night()], &night));
    assert_eq!(order.first().map(String::as_str), Some("NIGHT"));
    assert_eq!(order.last().map(String::as_str), Some("MORNING"));
}

#[test]
fn avoidingEarlyMorningsPutsThemLast() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("EIGHT", "CS", "3000", &[1, 3, 5], 8.0, 9.0),
            sec("NOON", "CS", "3000", &[1, 3, 5], 12.0, 13.0),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::EarlyMorning, TimePreference::Avoid);
    assert_eq!(
        firsts(&generate(&[course], &preferences)),
        vec!["NOON".to_string(), "EIGHT".to_string()]
    );
    let eight_only = req(
        "CS",
        "3000",
        vec![
            sec("EIGHT", "CS", "3000", &[2], 8.0, 9.0),
            sec("NINEFIFTEEN", "CS", "3000", &[2, 4, 5], 9.25, 10.25),
        ],
    );
    preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::EarlyMorning, TimePreference::Prefer);
    assert_eq!(
        generate(&[eight_only], &preferences)
            .schedules
            .first()
            .map(|schedule| schedule.crns.clone()),
        Some(vec!["EIGHT".into()])
    );
}

#[test]
fn aWishedForDayOffBeatsEverythingElseTheDefaultRankingLikes() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("ONEDAY", "CS", "3000", &[5], 9.0, 10.0),
            sec("TWODAYS", "CS", "3000", &[2, 4], 9.0, 10.0),
        ],
    );
    assert_eq!(
        generate(&[course.clone()], &SchedulePreferences::default())
            .schedules
            .first()
            .map(|schedule| schedule.crns.clone()),
        Some(vec!["ONEDAY".into()]),
        "The default ranking prefers fewer days"
    );
    let mut preferences = SchedulePreferences::default();
    preferences.days_off = DaySet::FRIDAY;
    assert_eq!(
        generate(&[course], &preferences)
            .schedules
            .first()
            .map(|schedule| schedule.crns.clone()),
        Some(vec!["TWODAYS".into()])
    );
}

#[test]
fn fewerDaysAndSpreadOutPullInOppositeDirections() {
    let course = req(
        "CS",
        "3000",
        vec![
            sec("TWO", "CS", "3000", &[2, 4], 9.0, 11.0),
            sec("FIVE", "CS", "3000", &[1, 2, 3, 4, 5], 9.0, 9.8),
        ],
    );
    let mut few = SchedulePreferences::default();
    few.day_layout = DayLayout::FewerDays;
    assert_eq!(
        generate(&[course.clone()], &few)
            .schedules
            .first()
            .map(|schedule| schedule.crns.clone()),
        Some(vec!["TWO".into()])
    );
    let mut spread = SchedulePreferences::default();
    spread.day_layout = DayLayout::SpreadOut;
    assert_eq!(
        generate(&[course], &spread)
            .schedules
            .first()
            .map(|schedule| schedule.crns.clone()),
        Some(vec!["FIVE".into()])
    );
}

#[test]
fn protectingLunchRanksFreeLunchesFirst() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1, 3], 9.0, 10.5)]),
        req(
            "B",
            "1",
            vec![
                sec("BUSY", "B", "1", &[1, 3], 11.75, 13.75),
                sec("FREE", "B", "1", &[1, 3], 14.0, 15.5),
            ],
        ),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.lunch = Some(LunchBreak {
        start: 11 * 60 + 30,
        end: 14 * 60,
        minutes: 45,
    });
    let result = generate(&courses, &preferences);
    assert_eq!(
        result.schedules.first().map(|schedule| schedule
            .crns
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()),
        Some(["A1", "FREE"].into_iter().map(str::to_string).collect())
    );
    assert!(result
        .schedules
        .first()
        .unwrap()
        .reasons
        .iter()
        .any(|reason| reason.text == "Lunch free every day"));
    assert!(result
        .schedules
        .last()
        .unwrap()
        .reasons
        .iter()
        .any(|reason| reason.text == "Lunch often busy"));
}

#[test]
fn avoidingBackToBackPrefersRealBreaks() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        req(
            "B",
            "1",
            vec![
                sec("TIGHT", "B", "1", &[1], 10.1, 11.0),
                sec("SPACED", "B", "1", &[1], 11.0, 12.0),
            ],
        ),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.breaks = BreakPreference::AvoidBackToBack;
    assert_eq!(
        generate(&courses, &preferences)
            .schedules
            .first()
            .map(|schedule| schedule.crns.iter().cloned().collect::<BTreeSet<_>>()),
        Some(["A1", "SPACED"].into_iter().map(str::to_string).collect())
    );
}

#[test]
fn keepingGapsShortPrefersCompactDays() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        req(
            "B",
            "1",
            vec![
                sec("GAP", "B", "1", &[1], 13.0, 14.0),
                sec("NOGAP", "B", "1", &[1], 10.25, 11.25),
            ],
        ),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.gaps = GapPreference::Compact;
    assert_eq!(
        generate(&courses, &preferences)
            .schedules
            .first()
            .map(|schedule| schedule.crns.iter().cloned().collect::<BTreeSet<_>>()),
        Some(["A1", "NOGAP"].into_iter().map(str::to_string).collect())
    );
    preferences.gaps = GapPreference::Fine;
    assert_eq!(generate(&courses, &preferences).total_found, 2);
}

#[test]
fn preferredAndAvoidedInstructorsMoveSchedules() {
    let course = req(
        "CS",
        "3000",
        vec![
            section(
                "NEUTRAL",
                "CS",
                "3000",
                &[1],
                9.0,
                10.0,
                Extra {
                    instructors: &["Lee, Kim"],
                    ..Extra::default()
                },
            ),
            section(
                "LOVED",
                "CS",
                "3000",
                &[2],
                9.0,
                10.0,
                Extra {
                    instructors: &["Smith, Alex"],
                    ..Extra::default()
                },
            ),
            section(
                "AVOIDED",
                "CS",
                "3000",
                &[3],
                9.0,
                10.0,
                Extra {
                    instructors: &["Jones, Sam"],
                    ..Extra::default()
                },
            ),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.preferred_instructors = vec!["Smith, Alex".into()];
    preferences.avoided_instructors = vec!["jones, sam".into()];
    let result = generate(&[course], &preferences);
    expect_order(&result, &[&["LOVED"], &["NEUTRAL"], &["AVOIDED"]]);
    assert!(result
        .schedules
        .first()
        .unwrap()
        .reasons
        .iter()
        .any(|reason| reason.text == "With Alex Smith"));
}

#[test]
fn preferredFormatAndCampusRankWithoutRemoving() {
    let course = req(
        "CS",
        "3000",
        vec![
            section(
                "ONLINE",
                "CS",
                "3000",
                &[1],
                9.0,
                10.0,
                Extra {
                    campus: "Online",
                    method: "Online",
                    ..Extra::default()
                },
            ),
            sec("INPERSON", "CS", "3000", &[2], 9.0, 10.0),
            section(
                "OAKLAND",
                "CS",
                "3000",
                &[3],
                9.0,
                10.0,
                Extra {
                    campus: "Oakland, CA",
                    ..Extra::default()
                },
            ),
        ],
    );
    let mut preferences = SchedulePreferences::default();
    preferences.format = FormatPreference::InPerson;
    assert_eq!(
        generate(&[course.clone()], &preferences)
            .schedules
            .first()
            .map(|schedule| schedule.crns.clone()),
        Some(vec!["INPERSON".into()])
    );
    assert_eq!(generate(&[course], &preferences).total_found, 3);
    let in_person = req(
        "CS",
        "3000",
        vec![
            sec("BOSTON", "CS", "3000", &[2], 9.0, 10.0),
            section(
                "OAKLAND",
                "CS",
                "3000",
                &[3],
                9.0,
                10.0,
                Extra {
                    campus: "Oakland, CA",
                    ..Extra::default()
                },
            ),
        ],
    );
    let mut campus = SchedulePreferences::default();
    campus.campus = Some("Oakland, CA".into());
    expect_order(
        &generate(&[in_person.clone()], &campus),
        &[&["OAKLAND"], &["BOSTON"]],
    );
    campus.campus = Some("Boston".into());
    expect_order(
        &generate(&[in_person], &campus),
        &[&["BOSTON"], &["OAKLAND"]],
    );
}

#[test]
fn aSearchThatRunsOutOfLeavesStillMeetsTheBestSectionsFirst() {
    let night = |crn: &str, course: &str, day: i32| sec(crn, course, "1", &[day], 19.0, 20.0);
    let a = req(
        "A",
        "1",
        (1..=8)
            .map(|index| night(&format!("A{index}"), "A", 1))
            .chain([sec("AMORNING", "A", "1", &[1], 9.0, 10.0)])
            .collect(),
    );
    let b = req(
        "B",
        "1",
        (1..=8)
            .map(|index| night(&format!("B{index}"), "B", 2))
            .chain([sec("BMORNING", "B", "1", &[2], 9.0, 10.0)])
            .collect(),
    );
    let mut preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::Morning, TimePreference::Prefer);
    preferences.set_band(TimeBand::Night, TimePreference::Avoid);
    let tiny = Limits {
        max_results: 100,
        max_exhaustive_leaves: 3,
        max_nodes: 1_000_000,
        time_budget: 5.0,
        sample_leaves: 0,
        sample_time_budget: 0.0,
        allow_sampling: false,
    };
    let result = generate_with(
        &[a, b],
        &ScheduleFilters::default(),
        &preferences,
        ScheduleSort::Best,
        &tiny,
    );
    assert!(
        !result.is_exhaustive,
        "Only three of the 81 schedules were looked at"
    );
    assert_eq!(
        result.schedules.first().map(|schedule| schedule
            .crns
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()),
        Some(
            ["AMORNING", "BMORNING"]
                .into_iter()
                .map(str::to_string)
                .collect()
        )
    );
}

#[test]
fn otherSortOrdersStillWorkWithPreferences() {
    let mut preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::Morning, TimePreference::Prefer);
    let result = generate_with(
        &[morning_or_night()],
        &ScheduleFilters::default(),
        &preferences,
        ScheduleSort::LatestStart,
        &Limits::default(),
    );
    assert_eq!(
        result
            .schedules
            .first()
            .map(|schedule| schedule.crns.clone()),
        Some(vec!["NIGHT".into()]),
        "Latest start still sorts by start time first"
    );
}

#[test]
fn theScoreOfADefaultScheduleMatchesTheOriginalFormula() {
    let sections = vec![
        sec("A1", "A", "1", &[1, 3], 8.0, 9.5),
        sec("B1", "B", "1", &[2, 4], 13.0, 14.5),
        section(
            "C1",
            "C",
            "1",
            &[1],
            17.0,
            18.5,
            Extra {
                seats: 0,
                ..Extra::default()
            },
        ),
    ];
    let schedule = GeneratedSchedule::from_sections(&sections, &SchedulePreferences::default());
    let measure = schedule::measure_sections(&sections);
    let mut expected = 100.0;
    expected -= 8.0 * f64::from(measure.campus_days);
    expected -= 2.0 * f64::from((measure.class_days - measure.campus_days).max(0));
    expected -= f64::from(measure.gap_minutes) / 10.0;
    if measure.earliest < 9 * 60 {
        expected -= f64::from(9 * 60 - measure.earliest) / 15.0;
    }
    if measure.latest > 17 * 60 {
        expected -= f64::from(measure.latest - 17 * 60) / 20.0;
    }
    expected -= 6.0;
    assert_eq!(schedule.score, expected);
    assert_eq!(schedule::Weights::default().score(&measure, 1, 0), expected);
}

#[test]
fn generatingWithEmptyPreferencesMatchesGeneratingWithout() {
    let courses = vec![
        req(
            "A",
            "1",
            vec![
                sec("A1", "A", "1", &[1, 3], 9.0, 10.0),
                sec("A2", "A", "1", &[2, 4], 11.0, 12.0),
            ],
        ),
        req(
            "B",
            "1",
            vec![
                sec("B1", "B", "1", &[1, 3], 9.5, 11.0),
                sec("B2", "B", "1", &[5], 14.0, 17.0),
            ],
        ),
    ];
    let plain = generate_with(
        &courses,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        ScheduleSort::Best,
        &Limits::default(),
    );
    let with_empty = generate(&courses, &SchedulePreferences::default());
    assert_eq!(
        plain
            .schedules
            .iter()
            .map(|schedule| schedule.crns.clone())
            .collect::<Vec<_>>(),
        with_empty
            .schedules
            .iter()
            .map(|schedule| schedule.crns.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        plain
            .schedules
            .iter()
            .map(|schedule| schedule.score)
            .collect::<Vec<_>>(),
        with_empty
            .schedules
            .iter()
            .map(|schedule| schedule.score)
            .collect::<Vec<_>>()
    );
}

#[test]
fn reasonsAreEmptyWithoutPreferences() {
    let sections = vec![sec("A", "A", "1", &[1], 9.0, 10.0)];
    assert!(schedule::reasons_for(&sections, &SchedulePreferences::default()).is_empty());
}

#[test]
fn aScheduleThatMeetsTheWishesSaysSo() {
    let sections = vec![
        sec("A", "A", "1", &[2, 4], 10.5, 12.0),
        sec("B", "B", "1", &[1, 3], 14.0, 15.5),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.days_off = DaySet::FRIDAY;
    preferences.earliest_start = Some(TimeLimit {
        minutes: 10 * 60,
        strength: PreferenceStrength::Required,
    });
    preferences.lunch = Some(LunchBreak::default());
    let reasons = schedule::reasons_for(&sections, &preferences);
    assert_eq!(
        reasons
            .iter()
            .map(|reason| reason.text.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Fridays off",
            "No classes before 10 AM",
            "Lunch free every day"
        ]
    );
    assert!(reasons.iter().all(|reason| reason.benefit));
}

#[test]
fn wishesItMissesShowUpAsCaveats() {
    let sections = vec![sec("A", "A", "1", &[5], 8.0, 9.0)];
    let mut preferences = SchedulePreferences::default();
    preferences.days_off = DaySet::FRIDAY;
    preferences.earliest_start = Some(TimeLimit {
        minutes: 10 * 60,
        strength: PreferenceStrength::Preferred,
    });
    preferences.set_band(TimeBand::EarlyMorning, TimePreference::Avoid);
    let reasons = schedule::reasons_for(&sections, &preferences);
    assert!(reasons
        .iter()
        .filter(|reason| reason.benefit)
        .collect::<Vec<_>>()
        .is_empty());
    assert_eq!(
        reasons
            .iter()
            .filter(|reason| !reason.benefit)
            .map(|reason| reason.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Class on Fri", "Starts 8:00 AM"]
    );
    let all = schedule::preference_reasons(&sections, &preferences, 4, 5);
    assert_eq!(
        all.iter()
            .filter(|reason| !reason.benefit)
            .map(|reason| reason.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Class on Fri", "Starts 8:00 AM", "Includes early mornings"]
    );
}

#[test]
fn morningsAndFormatsAreDescribed() {
    let sections = vec![
        sec("A", "A", "1", &[1, 3], 9.5, 11.0),
        sec("B", "B", "1", &[2, 4], 10.0, 11.5),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.set_band(TimeBand::Morning, TimePreference::Prefer);
    preferences.set_band(TimeBand::Evening, TimePreference::Avoid);
    preferences.format = FormatPreference::InPerson;
    preferences.day_layout = DayLayout::FewerDays;
    let reasons: Vec<String> = schedule::reasons_for(&sections, &preferences)
        .into_iter()
        .map(|reason| reason.text)
        .collect();
    assert!(reasons.iter().any(|text| text == "All mornings"));
    assert!(reasons.iter().any(|text| text == "No evenings"));
    assert!(reasons.iter().any(|text| text == "All in person"));
}

#[test]
fn favoritesGetTheSameTreatment() {
    let sections = vec![sec("A", "A", "1", &[2, 4], 11.0, 12.5)];
    let mut preferences = SchedulePreferences::default();
    preferences.days_off = DaySet::FRIDAY;
    let schedule = GeneratedSchedule::from_sections(&sections, &preferences);
    assert_eq!(
        schedule
            .reasons
            .iter()
            .map(|reason| reason.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Fridays off"]
    );
    assert!(
        GeneratedSchedule::from_sections(&sections, &SchedulePreferences::default())
            .reasons
            .is_empty()
    );
}

#[test]
fn theSameThingIsNeverSaidTwice() {
    let sections = vec![sec("A", "A", "1", &[2, 4], 18.0, 19.5)];
    let mut preferences = SchedulePreferences::default();
    let mut tuesday = block("Club", DaySet::TUESDAY, 17 * 60, 19 * 60);
    tuesday.strength = PreferenceStrength::Preferred;
    let mut thursday = block("Club", DaySet::THURSDAY, 17 * 60, 19 * 60);
    thursday.strength = PreferenceStrength::Preferred;
    preferences.blocked_times = vec![tuesday, thursday];
    let reasons = schedule::reasons_for(&sections, &preferences);
    assert_eq!(
        reasons
            .iter()
            .map(|reason| reason.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Overlaps Club"]
    );
    assert_eq!(
        reasons
            .iter()
            .map(|reason| reason.id())
            .collect::<HashSet<_>>()
            .len(),
        reasons.len()
    );
}

#[test]
fn campusesAreNamedPlainly() {
    let sections = vec![section(
        "A",
        "A",
        "1",
        &[1],
        9.0,
        10.0,
        Extra {
            campus: "Oakland, CA",
            ..Extra::default()
        },
    )];
    let mut preferences = SchedulePreferences::default();
    preferences.campus = Some("Oakland, CA".into());
    assert_eq!(
        schedule::reasons_for(&sections, &preferences)
            .iter()
            .map(|reason| reason.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Oakland, CA campus"]
    );
}

#[test]
fn cardTagsAreCapped() {
    let sections = vec![sec("A", "A", "1", &[2, 4], 11.0, 12.5)];
    let mut preferences = SchedulePreferences::default();
    preferences.days_off = DaySet::FRIDAY;
    preferences.earliest_start = Some(TimeLimit {
        minutes: 600,
        strength: PreferenceStrength::Required,
    });
    preferences.latest_end = Some(TimeLimit {
        minutes: 1_020,
        strength: PreferenceStrength::Required,
    });
    preferences.day_layout = DayLayout::FewerDays;
    preferences.set_band(TimeBand::Morning, TimePreference::Prefer);
    preferences.lunch = Some(LunchBreak::default());
    preferences.format = FormatPreference::InPerson;
    let reasons = schedule::reasons_for(&sections, &preferences);
    assert_eq!(reasons.iter().filter(|reason| reason.benefit).count(), 4);
}

#[test]
fn timesAreReadTheWayAModelWritesThem() {
    let parse = nl::minutes_from_clock;
    assert_eq!(parse("18:00"), Some(18 * 60));
    assert_eq!(parse("6:30 PM"), Some(18 * 60 + 30));
    assert_eq!(parse("6pm"), Some(18 * 60));
    assert_eq!(parse("6 p.m."), Some(18 * 60));
    assert_eq!(parse("10"), Some(10 * 60));
    assert_eq!(parse("1800"), Some(18 * 60));
    assert_eq!(parse("930"), Some(9 * 60 + 30));
    assert_eq!(parse("12am"), Some(0));
    assert_eq!(parse("12pm"), Some(12 * 60));
    assert_eq!(parse("noon"), Some(12 * 60));
    assert_eq!(parse("midnight"), Some(24 * 60));
    assert_eq!(parse("24:00"), Some(24 * 60));
    assert_eq!(parse("25:00"), None);
    assert_eq!(parse("13pm"), None);
    assert_eq!(parse("soon"), None);
    assert_eq!(parse(""), None);
    assert_eq!(parse("7:75"), None);
}

#[test]
fn theBriefsExampleBecomesTheExpectedSurvey() {
    let draft = PreferencesDraft {
        earliest_start: Some("10:00".into()),
        days_off: Some(vec![DraftWeekday::Friday]),
        wants_lunch: Some(true),
        blocked_times: Some(vec![DraftBlock {
            label: "Robotics club".into(),
            days: vec![DraftWeekday::Tuesday, DraftWeekday::Thursday],
            start: "18:00".into(),
            end: "20:00".into(),
        }]),
        ..PreferencesDraft::default()
    };
    let applied = draft.applied(&SchedulePreferences::default());
    assert_eq!(
        applied.earliest_start,
        Some(TimeLimit {
            minutes: 600,
            strength: PreferenceStrength::Required
        })
    );
    assert_eq!(applied.days_off, DaySet::FRIDAY);
    assert_eq!(applied.days_off_strength, PreferenceStrength::Preferred);
    assert!(applied.lunch.is_some());
    assert_eq!(applied.blocked_times.len(), 1);
    let block = applied.blocked_times.first().unwrap();
    assert_eq!(block.label, "Robotics club");
    assert_eq!(block.days, DaySet::TUESDAY.union(DaySet::THURSDAY));
    assert_eq!((block.start, block.end), (18 * 60, 20 * 60));
    assert_eq!(block.strength, PreferenceStrength::Required);
    assert_eq!(
        applied.summary(4),
        "Nothing before 10 AM \u{00b7} Fridays off \u{00b7} 1 blocked time \u{00b7} Lunch break"
    );
}

#[test]
fn bandsAndLayoutsMap() {
    let draft = PreferencesDraft {
        preferred_times: Some(vec![DraftTime::Morning, DraftTime::EarlyMorning]),
        avoided_times: Some(vec![DraftTime::Night, DraftTime::Morning]),
        layout: Some(DraftLayout::SpreadOut),
        max_classes_per_day: Some(3),
        max_hours_per_day: Some(5),
        breaks: Some(DraftBreaks::TimeToWalk),
        format: Some(DraftFormat::Hybrid),
        seats: Some(DraftSeats::WaitlistOk),
        ..PreferencesDraft::default()
    };
    let preferences = draft.applied(&SchedulePreferences::default());
    assert_eq!(
        preferences.band(TimeBand::EarlyMorning),
        TimePreference::Prefer
    );
    assert_eq!(
        preferences.band(TimeBand::Morning),
        TimePreference::Avoid,
        "An avoided band wins when it is listed both ways"
    );
    assert_eq!(preferences.band(TimeBand::Night), TimePreference::Avoid);
    assert_eq!(preferences.day_layout, DayLayout::SpreadOut);
    assert_eq!(preferences.max_classes_per_day, Some(3));
    assert_eq!(preferences.max_minutes_per_day, Some(300));
    assert_eq!(preferences.breaks, BreakPreference::AtLeast);
    assert_eq!(preferences.minimum_break_minutes, 15);
    assert_eq!(
        preferences.minimum_break_strength,
        PreferenceStrength::Preferred
    );
    assert_eq!(preferences.format, FormatPreference::Hybrid);
    assert_eq!(preferences.seats, SeatPolicy::WaitlistOk);
}

#[test]
fn flexibleLimitsAndRequiredDaysOff() {
    let draft = PreferencesDraft {
        earliest_start: Some("9:30".into()),
        latest_end: Some("5pm".into()),
        time_limits_flexible: Some(true),
        days_off: Some(vec![DraftWeekday::Monday, DraftWeekday::Friday]),
        days_off_required: Some(true),
        ..PreferencesDraft::default()
    };
    let preferences = draft.applied(&SchedulePreferences::default());
    assert_eq!(
        preferences.earliest_start,
        Some(TimeLimit {
            minutes: 9 * 60 + 30,
            strength: PreferenceStrength::Preferred
        })
    );
    assert_eq!(
        preferences.latest_end,
        Some(TimeLimit {
            minutes: 17 * 60,
            strength: PreferenceStrength::Preferred
        })
    );
    assert_eq!(preferences.days_off, DaySet::MONDAY.union(DaySet::FRIDAY));
    assert_eq!(preferences.days_off_strength, PreferenceStrength::Required);
}

#[test]
fn anEndBeforeTheStartIsRepairedAsTheAfternoonOrEvening() {
    let draft = PreferencesDraft {
        blocked_times: Some(vec![
            DraftBlock {
                label: "Lunch".into(),
                days: vec![DraftWeekday::Monday],
                start: "11:00".into(),
                end: "01:00".into(),
            },
            DraftBlock {
                label: "Choir".into(),
                days: vec![DraftWeekday::Wednesday],
                start: "18:00".into(),
                end: "08:00".into(),
            },
            DraftBlock {
                label: "Run".into(),
                days: vec![DraftWeekday::Friday],
                start: "06:00".into(),
                end: "08:00".into(),
            },
        ]),
        ..PreferencesDraft::default()
    };
    let blocks = draft.applied(&SchedulePreferences::default()).blocked_times;
    assert_eq!(
        blocks
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Lunch", "Choir", "Run"]
    );
    assert_eq!((blocks[0].start, blocks[0].end), (11 * 60, 13 * 60));
    assert_eq!((blocks[1].start, blocks[1].end), (18 * 60, 20 * 60));
    assert_eq!((blocks[2].start, blocks[2].end), (6 * 60, 8 * 60));
}

#[test]
fn aFinishBeforeTheStartIsNotBelieved() {
    let draft = PreferencesDraft {
        earliest_start: Some("10:00".into()),
        latest_end: Some("09:00".into()),
        ..PreferencesDraft::default()
    };
    let preferences = draft.applied(&SchedulePreferences::default());
    assert_eq!(
        preferences.earliest_start.map(|limit| limit.minutes),
        Some(10 * 60)
    );
    assert_eq!(preferences.latest_end, None);
    let mut current = SchedulePreferences::default();
    current.latest_end = Some(TimeLimit {
        minutes: 15 * 60,
        strength: PreferenceStrength::Required,
    });
    assert_eq!(
        PreferencesDraft {
            earliest_start: Some("16:00".into()),
            ..PreferencesDraft::default()
        }
        .applied(&current)
        .latest_end
        .map(|limit| limit.minutes),
        Some(15 * 60),
        "Only what the description says is judged"
    );
}

#[test]
fn unusableAnswersAreDroppedNotGuessedAt() {
    let draft = PreferencesDraft {
        earliest_start: Some("whenever".into()),
        latest_end: Some("26:00".into()),
        max_classes_per_day: Some(40),
        max_hours_per_day: Some(0),
        blocked_times: Some(vec![
            DraftBlock {
                label: "No days".into(),
                days: vec![],
                start: "10:00".into(),
                end: "11:00".into(),
            },
            DraftBlock {
                label: "No time".into(),
                days: vec![DraftWeekday::Monday],
                start: "later".into(),
                end: "11:00".into(),
            },
            DraftBlock {
                label: "Backwards".into(),
                days: vec![DraftWeekday::Monday],
                start: "17:00".into(),
                end: "16:00".into(),
            },
        ]),
        ..PreferencesDraft::default()
    };
    assert!(draft.applied(&SchedulePreferences::default()).is_default());
}

#[test]
fn aDescriptionAddsToTheSurveyInsteadOfReplacingIt() {
    let mut current = SchedulePreferences::default();
    current.set_band(TimeBand::Afternoon, TimePreference::Prefer);
    current.days_off = DaySet::MONDAY;
    current.blocked_times = vec![block("Work", DaySet::WEDNESDAY, 13 * 60, 17 * 60)];
    let draft = PreferencesDraft {
        days_off: Some(vec![DraftWeekday::Friday]),
        blocked_times: Some(vec![
            DraftBlock {
                label: "work".into(),
                days: vec![DraftWeekday::Wednesday],
                start: "13:00".into(),
                end: "17:00".into(),
            },
            DraftBlock {
                label: "Choir".into(),
                days: vec![DraftWeekday::Thursday],
                start: "18:00".into(),
                end: "19:30".into(),
            },
        ]),
        ..PreferencesDraft::default()
    };
    let merged = draft.applied(&current);
    assert_eq!(
        merged.band(TimeBand::Afternoon),
        TimePreference::Prefer,
        "What the description doesn't mention stays"
    );
    assert_eq!(
        merged.days_off,
        DaySet::FRIDAY,
        "A new list of days off replaces the old one"
    );
    assert_eq!(
        merged
            .blocked_times
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Work", "Choir"],
        "A block it repeats isn't added twice"
    );
    let changed = merged.changed_fields(&current);
    assert!(changed.contains(&PreferenceField::DaysOff));
    assert!(!changed.contains(&PreferenceField::TimeOfDay(TimeBand::Afternoon)));
    assert_eq!(changed.len(), 2);
}

#[test]
fn anEmptyDraftIsEmpty() {
    assert!(PreferencesDraft::default().is_empty());
    assert!(PreferencesDraft {
        wants_lunch: Some(false),
        ..PreferencesDraft::default()
    }
    .is_empty());
    assert!(!PreferencesDraft {
        wants_lunch: Some(true),
        ..PreferencesDraft::default()
    }
    .is_empty());
    assert!(PreferencesDraft::default()
        .applied(&SchedulePreferences::default())
        .is_default());
    assert_eq!(
        PreferencesDraft {
            preferred_times: Some(vec![DraftTime::Morning]),
            days_off: Some(vec![DraftWeekday::Friday]),
            ..PreferencesDraft::default()
        }
        .filled_count(),
        2
    );
}

#[test]
fn answersTheStudentNeverMentionedAreDropped() {
    let grounded = overeager_draft().grounded(&briefs_example());
    assert_eq!(grounded.format, None);
    assert_eq!(grounded.seats, None);
    assert_eq!(grounded.breaks, None);
    assert_eq!(grounded.layout, None);
    assert_eq!(grounded.max_classes_per_day, None);
    assert_eq!(grounded.max_hours_per_day, None);
    assert_eq!(grounded.earliest_start.as_deref(), Some("10:00"));
    assert_eq!(grounded.wants_lunch, Some(true));
    assert_eq!(grounded.days_off, Some(vec![DraftWeekday::Friday]));
    assert_eq!(grounded.blocked_times.as_ref().map(Vec::len), Some(1));
}

#[test]
fn firmnessFollowsTheWordsNotTheModel() {
    let applied = overeager_draft()
        .grounded(&briefs_example())
        .applied(&SchedulePreferences::default());
    assert_eq!(
        applied.earliest_start,
        Some(TimeLimit {
            minutes: 600,
            strength: PreferenceStrength::Required
        })
    );
    assert_eq!(applied.days_off_strength, PreferenceStrength::Preferred);
    assert_eq!(
        applied.summary(4),
        "Nothing before 10 AM \u{00b7} Fridays off \u{00b7} 1 blocked time \u{00b7} Lunch break"
    );
}

#[test]
fn commitmentsAndSoftenersAreRecognized() {
    let work = PreferencesDraft {
        days_off: Some(vec![DraftWeekday::Friday]),
        days_off_required: Some(false),
        ..PreferencesDraft::default()
    }
    .grounded("I work Fridays so I need them off");
    assert_eq!(work.days_off_required, Some(true));
    let never = PreferencesDraft {
        days_off: Some(vec![DraftWeekday::Monday]),
        ..PreferencesDraft::default()
    }
    .grounded("No classes on Mondays please");
    assert_eq!(never.days_off_required, Some(true));
    let wish = PreferencesDraft {
        days_off: Some(vec![DraftWeekday::Monday]),
        days_off_required: Some(true),
        ..PreferencesDraft::default()
    }
    .grounded("Mondays off would be great");
    assert_eq!(wish.days_off_required, Some(false));
    let soft = PreferencesDraft {
        earliest_start: Some("10:00".into()),
        time_limits_flexible: Some(false),
        ..PreferencesDraft::default()
    }
    .grounded("Ideally nothing before 10 in the morning");
    assert_eq!(soft.time_limits_flexible, Some(true));
    let firm = PreferencesDraft {
        earliest_start: Some("10:00".into()),
        time_limits_flexible: Some(true),
        ..PreferencesDraft::default()
    }
    .grounded("I can\u{2019}t do classes before 10");
    assert_eq!(firm.time_limits_flexible, Some(false));
}

#[test]
fn limitsNeedANumberAndADay() {
    let draft = PreferencesDraft {
        max_classes_per_day: Some(3),
        max_hours_per_day: Some(5),
        ..PreferencesDraft::default()
    };
    assert_eq!(
        draft
            .grounded("I don\u{2019}t want more than three classes a day")
            .max_classes_per_day,
        Some(3)
    );
    assert_eq!(
        draft
            .grounded("I don\u{2019}t want too many classes")
            .max_classes_per_day,
        None
    );
    assert_eq!(
        draft
            .grounded("No more than 5 hours of class each day")
            .max_hours_per_day,
        Some(5)
    );
}

#[test]
fn otherLanguagesAreLeftToTheModel() {
    let draft = PreferencesDraft {
        time_limits_flexible: Some(true),
        days_off: Some(vec![DraftWeekday::Friday]),
        days_off_required: Some(true),
        format: Some(DraftFormat::Online),
        ..PreferencesDraft::default()
    };
    let text = "Quiero las mañanas libres y los viernes sin clase, porque trabajo por las tardes";
    assert!(!nl::is_english(text));
    assert_eq!(draft.grounded(text), draft);
    assert!(nl::is_english(&briefs_example()));
}

#[test]
fn tooManyBlocksAreCapped() {
    let blocks = (0..20)
        .map(|index| DraftBlock {
            label: format!("Block {index}"),
            days: vec![DraftWeekday::Monday],
            start: format!("{:02}:00", 8 + index % 10),
            end: format!("{:02}:00", 9 + index % 10),
        })
        .collect();
    let preferences = PreferencesDraft {
        blocked_times: Some(blocks),
        ..PreferencesDraft::default()
    }
    .applied(&SchedulePreferences::default());
    assert!(preferences.blocked_times.len() <= PreferencesDraft::MAX_BLOCKS);
}

#[test]
fn failuresHaveHumanMessages() {
    let failures = [
        AssistantFailure::EmptyDescription,
        AssistantFailure::DescriptionTooLong,
        AssistantFailure::Guardrail,
        AssistantFailure::ContextTooLarge,
        AssistantFailure::UnsupportedLanguage,
        AssistantFailure::Busy,
        AssistantFailure::TimedOut,
        AssistantFailure::NothingFound,
        AssistantFailure::Failed("x".into()),
        AssistantFailure::Unavailable(AssistantAvailability::NotEnabled),
    ];
    assert!(failures.iter().all(|failure| !failure.message().is_empty()));
    assert!(AssistantAvailability::DeviceNotEligible
        .note()
        .unwrap()
        .contains("survey"));
    assert!(AssistantAvailability::NotEnabled
        .note()
        .unwrap()
        .contains("Settings"));
    assert!(AssistantAvailability::NotReady
        .note()
        .unwrap()
        .contains("later"));
    assert!(AssistantAvailability::Available.note().is_none());
}

#[test]
fn findsTheCourseThatAlwaysClashes() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        req("B", "1", vec![sec("B1", "B", "1", &[1], 9.5, 10.5)]),
        req("C", "1", vec![sec("C1", "C", "1", &[2], 9.0, 10.0)]),
    ];
    let result = generate(&courses, &SchedulePreferences::default());
    assert_eq!(result.total_found, 0);
    let analysis = schedule::analyze_blockers(
        &courses,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        Some(&result.diagnostics),
        1.5,
        60,
    );
    assert_eq!(analysis.required_count, 3);
    let options: HashSet<Vec<String>> = analysis
        .options
        .iter()
        .map(|option| {
            option
                .courses
                .iter()
                .map(|course| course.to_string())
                .collect()
        })
        .collect();
    assert_eq!(
        options,
        HashSet::from([vec!["A 1".into()], vec!["B 1".into()]])
    );
    let blocking: HashSet<String> = analysis
        .all_blocking_courses()
        .into_iter()
        .map(|course| course.to_string())
        .collect();
    assert_eq!(blocking, HashSet::from(["A 1".into(), "B 1".into()]));
    assert_eq!(analysis.clashes(&key("A", "1")), vec![key("B", "1")]);
    assert!(analysis
        .options
        .iter()
        .all(|option| option.schedule_count == 1));
}

#[test]
fn aCourseWithNoSectionsAlwaysHasToGo() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        req("B", "1", vec![]),
        req("C", "1", vec![sec("C1", "C", "1", &[2], 9.0, 10.0)]),
    ];
    let analysis = schedule::analyze_blockers(
        &courses,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        None,
        1.5,
        60,
    );
    assert_eq!(
        analysis
            .not_offered
            .iter()
            .map(|course| course.to_string())
            .collect::<Vec<_>>(),
        vec!["B 1".to_string()]
    );
    assert_eq!(
        analysis
            .options
            .iter()
            .map(|option| option
                .courses
                .iter()
                .map(|course| course.to_string())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![vec!["B 1".to_string()]]
    );
}

#[test]
fn coursesTheRulesRuleOutAreReportedSeparately() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[5], 9.0, 10.0)]),
        req("B", "1", vec![sec("B1", "B", "1", &[2], 9.0, 10.0)]),
    ];
    let mut preferences = SchedulePreferences::default();
    preferences.days_off = DaySet::FRIDAY;
    preferences.days_off_strength = PreferenceStrength::Required;
    let analysis = schedule::analyze_blockers(
        &courses,
        &ScheduleFilters::default(),
        &preferences,
        None,
        1.5,
        60,
    );
    assert_eq!(
        analysis
            .filtered_out
            .iter()
            .map(|course| course.to_string())
            .collect::<Vec<_>>(),
        vec!["A 1".to_string()]
    );
    assert!(analysis.not_offered.is_empty());
    assert_eq!(
        analysis
            .options
            .iter()
            .map(|option| option
                .courses
                .iter()
                .map(|course| course.to_string())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![vec!["A 1".to_string()]]
    );
}

#[test]
fn looksAtPairsWhenNoSingleCourseIsToBlame() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        req("B", "1", vec![sec("B1", "B", "1", &[1], 9.0, 10.0)]),
        req("C", "1", vec![sec("C1", "C", "1", &[2], 9.0, 10.0)]),
        req("D", "1", vec![sec("D1", "D", "1", &[2], 9.0, 10.0)]),
    ];
    let analysis = schedule::analyze_blockers(
        &courses,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        None,
        1.5,
        60,
    );
    assert_eq!(analysis.options.len(), 4);
    assert!(analysis
        .options
        .iter()
        .all(|option| option.courses.len() == 2));
    let sets: HashSet<BTreeSet<String>> = analysis
        .options
        .iter()
        .map(|option| {
            option
                .courses
                .iter()
                .map(|course| course.to_string())
                .collect()
        })
        .collect();
    let expected: HashSet<BTreeSet<String>> = [
        ["A 1", "C 1"],
        ["A 1", "D 1"],
        ["B 1", "C 1"],
        ["B 1", "D 1"],
    ]
    .into_iter()
    .map(|group| group.into_iter().map(str::to_string).collect())
    .collect();
    assert_eq!(sets, expected);
}

#[test]
fn countProblemsAreNotCourseProblems() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        req("B", "1", vec![sec("B1", "B", "1", &[2], 9.0, 10.0)]),
    ];
    let mut filters = ScheduleFilters::default();
    filters.course_count = Some(1);
    let result = generate_with(
        &courses,
        &filters,
        &SchedulePreferences::default(),
        ScheduleSort::Best,
        &Limits::default(),
    );
    let analysis = schedule::analyze_blockers(
        &courses,
        &filters,
        &SchedulePreferences::default(),
        Some(&result.diagnostics),
        1.5,
        60,
    );
    assert!(analysis.options.is_empty());
}

#[test]
fn makingTheBlockersOptionalProducesSchedules() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        req("B", "1", vec![sec("B1", "B", "1", &[1], 9.5, 10.5)]),
    ];
    let analysis = schedule::analyze_blockers(
        &courses,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        None,
        1.5,
        60,
    );
    let drop: HashSet<CourseKey> = analysis.blocking_courses().into_iter().collect();
    let relaxed: Vec<ScheduleCourseInput> = courses
        .into_iter()
        .map(|mut course| {
            if drop.contains(&course.key) {
                course.is_required = false;
            }
            course
        })
        .collect();
    assert!(generate(&relaxed, &SchedulePreferences::default()).total_found > 0);
}

#[test]
fn noResultsAnalysisCombinesBlockersAndRelaxations() {
    let courses = vec![
        req("A", "1", vec![sec("A1", "A", "1", &[1], 9.0, 10.0)]),
        req("B", "1", vec![sec("B1", "B", "1", &[1], 9.5, 10.5)]),
    ];
    let result = generate(&courses, &SchedulePreferences::default());
    let analysis = schedule::analyze_empty_result(
        &courses,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        &result.diagnostics,
        true,
    );
    assert!(!analysis.blockers.options.is_empty());
    assert!(
        analysis.suggestions.is_empty(),
        "No filter is in the way, so there is nothing to loosen"
    );
}

#[test]
fn campusesInstructorsAndCreditsComeFromThePlansCourses() {
    let index = catalog();
    let mut plan = empty_plan("Plan", "202710");
    plan.courses = vec![
        PlanCourse {
            key: key("CS", "3000"),
            is_required: true,
            excluded_crns: HashSet::new(),
            color_index: 0,

            pinned_crn: None,
            degree_row_id: None,
            credits: None,
        },
        PlanCourse {
            key: key("MATH", "2331"),
            is_required: false,
            excluded_crns: HashSet::new(),
            color_index: 1,

            pinned_crn: None,
            degree_row_id: None,
            credits: None,
        },
    ];
    let context = schedule::preferences_context(&plan, Some(index));
    assert!(context.campus_names.iter().any(|name| name == "Boston"));
    assert!(context.campus_names.iter().all(|name| {
        let lower = name.to_ascii_lowercase();
        !lower.contains("online") && !lower.contains("no campus")
    }));
    assert_eq!(context.optional_course_count, 1);
    assert!(context.required_credits > 0);
    assert!(context.total_credits > context.required_credits);
    assert!(!context.instructors.is_empty());
    assert!(context
        .instructors
        .iter()
        .all(|instructor| !instructor.courses.is_empty()));
    let names: Vec<String> = context
        .instructors
        .iter()
        .map(|instructor| instructor.name.clone())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_by_key(|name| name.to_ascii_lowercase());
    assert_eq!(names, sorted);
    assert!(schedule::preferences_context(&plan, None)
        .instructors
        .is_empty());
}

#[test]
fn excludedSectionsDontOfferTheirInstructors() {
    let index = catalog();
    let course_key = key("CS", "3000");
    let mut plan = empty_plan("Plan", "202710");
    plan.courses = vec![PlanCourse {
        key: course_key.clone(),
        is_required: true,
        excluded_crns: HashSet::new(),
        color_index: 0,

        pinned_crn: None,
        degree_row_id: None,
        credits: None,
    }];
    let everyone = schedule::preferences_context(&plan, Some(index)).instructors;
    plan.courses[0].excluded_crns = index
        .sections_for(&course_key)
        .into_iter()
        .map(|section| section.crn)
        .collect();
    assert!(schedule::preferences_context(&plan, Some(index))
        .instructors
        .is_empty());
    assert!(!everyone.is_empty());
}

#[test]
fn aSuggestionBecomesAnOptionThatMovesTheCourseOutOfTheSchedule() {
    let (plan, history) = degree_history();
    let origin = plan.terms[0].id;
    let found = suggestions_for(&plan, origin, &[code("CS 3386")], &history);
    let single = found
        .iter()
        .find(|suggestion| suggestion.moves.len() == 1)
        .expect("one-course move");
    let option = schedule::ReorderOption::from_suggestion(single, origin);
    assert_eq!(option.title, "Move CS 3386 to Spring 2027");
    assert_eq!(option.applied_text(), "Moved CS 3386 to Spring 2027");
    assert_eq!(option.moved_away, vec![code("CS 3386")]);
    assert!(option.moved_in.is_empty());
    assert_eq!(
        option.schedule_effect(),
        "CS 3386 comes out of this schedule."
    );
    assert_eq!(
        option.reasons.first().map(String::as_str),
        Some("Offered every spring")
    );
    assert!(!option.adds_semester);
    let mut changed = plan.clone();
    option.apply_to(&mut changed);
    assert_eq!(
        changed.terms[0]
            .courses
            .iter()
            .filter_map(|course| course.key.clone())
            .collect::<Vec<_>>(),
        vec![code("CS 3800")]
    );
    assert_eq!(
        changed.terms[1]
            .courses
            .iter()
            .filter_map(|course| course.key.clone())
            .collect::<Vec<_>>(),
        vec![code("CS 4100"), code("CS 3386")]
    );
    assert_eq!(
        plan.terms[0].courses.len(),
        2,
        "Applying edits the copy it is given"
    );
}

#[test]
fn aSwapAlsoBringsACourseIn() {
    let (mut plan, history) = degree_history();
    plan.terms[1].courses = vec![DegreePlanCourse::catalog(code("CS 4100"), "CS 4100", 4.0)];
    let origin = plan.terms[0].id;
    let found = suggestions_for(&plan, origin, &[code("CS 3386")], &history);
    let swap = found
        .iter()
        .find(|suggestion| suggestion.moves.len() == 2)
        .expect("swap");
    let option = schedule::ReorderOption::from_suggestion(swap, origin);
    assert_eq!(option.moved_away, vec![code("CS 3386")]);
    assert_eq!(option.moved_in, vec![code("CS 4100")]);
    assert_eq!(option.schedule_effect(), "CS 3386 comes out of this schedule. CS 4100 will be added to it if it\u{2019}s offered this term.");
}

#[test]
fn aNewSemesterIsFlagged() {
    let (mut plan, history) = degree_history();
    let mut only = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    only.courses = vec![DegreePlanCourse::catalog(code("CS 3386"), "CS 3386", 4.0)];
    plan.terms = vec![only];
    let origin = plan.terms[0].id;
    let found = suggestions_for(&plan, origin, &[code("CS 3386")], &history);
    let option =
        schedule::ReorderOption::from_suggestion(found.first().expect("suggestion"), origin);
    assert!(option.adds_semester);
    let mut changed = plan;
    option.apply_to(&mut changed);
    assert_eq!(changed.terms.len(), 2);
    assert_eq!(
        changed
            .terms
            .last()
            .unwrap()
            .courses
            .iter()
            .filter_map(|course| course.key.clone())
            .collect::<Vec<_>>(),
        vec![code("CS 3386")]
    );
}

#[test]
fn nothingIsLookedUpWhenNoBlockingCourseIsInTheSemester() {
    let (plan, _) = degree_history();
    let fall = Term {
        code: "202710".into(),
        banner_description: "Fall 2026 Semester".into(),
    };
    assert!(schedule::reorder_load_offline(&[code("CS 9001")], &plan, &fall).is_empty());
    let summer = Term {
        code: "202750".into(),
        banner_description: "Summer Full 2027 Semester".into(),
    };
    assert!(schedule::reorder_load_offline(&[code("CS 3386")], &plan, &summer).is_empty());
}

#[test]
fn titlesThatAreNotMovesStillReadSensibly() {
    let option = schedule::ReorderOption::custom("Something else");
    assert_eq!(option.applied_text(), "Updated your degree plan");
    assert!(option.schedule_effect().is_empty());
}

#[test]
fn randomPlansMatchAnObviouslyCorrectEnumeration() {
    let mut rng = SplitMix64 { state: 20_261_004 };
    let mut non_trivial = 0;
    for round in 0..400 {
        let (courses, filters, preferences) = random_instance(&mut rng);
        let expected = brute_force(&courses, &filters, &preferences);
        let result = generate_with(
            &courses,
            &filters,
            &preferences,
            ScheduleSort::Best,
            &Limits::default(),
        );
        assert!(result.is_exhaustive);
        let actual = crn_sets(&result);
        if actual != expected {
            let missing: Vec<_> = expected.difference(&actual).take(3).cloned().collect();
            let extra: Vec<_> = actual.difference(&expected).take(3).cloned().collect();
            panic!("Round {round}: generator found {} schedules, enumeration found {}. Missing: {missing:?}, extra: {extra:?}", actual.len(), expected.len());
        }
        assert_eq!(result.total_found, expected.len());
        if !expected.is_empty() {
            non_trivial += 1;
        }
    }
    assert!(
        non_trivial > 100,
        "The random plans should usually have schedules, or the comparison proves little"
    );
}

#[test]
fn preferencesRankAndFilterARealPlanQuickly() {
    let index = catalog();
    let courses = catalog_inputs(index, &["CS 3000", "CS 3001", "MATH 2331", "PHYS 1151"]);
    let plain = generate(&courses, &SchedulePreferences::default());
    assert!(plain.total_found > 0);
    let mut preferences = SchedulePreferences::default();
    preferences = SchedulePreset::LongWeekends.toggled(&preferences);
    preferences = SchedulePreset::MorningPerson.toggled(&preferences);
    preferences.blocked_times = vec![block(
        "Club",
        DaySet::TUESDAY.union(DaySet::THURSDAY),
        18 * 60,
        20 * 60,
    )];
    preferences.lunch = Some(LunchBreak::default());
    preferences.max_classes_per_day = Some(3);
    let ranked = generate(&courses, &preferences);
    assert!(ranked.total_found <= plain.total_found);
    assert!(
        ranked.elapsed < 10.0,
        "Preferences must not make generation slow"
    );
    if let Some(best) = ranked.schedules.first() {
        for section in &best.sections {
            for meeting in section.weekly_meetings() {
                let overlaps_club = !meeting
                    .days
                    .is_disjoint(DaySet::TUESDAY.union(DaySet::THURSDAY))
                    && meeting.start.unwrap_or(0) < 20 * 60
                    && meeting.end.unwrap_or(0) > 18 * 60;
                assert!(!overlaps_club);
            }
        }
        assert!(!best.reasons.is_empty());
    }
    let scores: Vec<f64> = ranked
        .schedules
        .iter()
        .map(|schedule| schedule.score)
        .collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert_eq!(scores, sorted);
}

#[test]
fn slotMasksDetectOverlaps() {
    let a = ScheduleSlotMask::from_section(&sec("1", "CS", "1", &[1, 3], 10.0, 11.0));
    let b = ScheduleSlotMask::from_section(&sec("2", "CS", "2", &[3], 10.5, 12.0));
    let c = ScheduleSlotMask::from_section(&sec("3", "CS", "3", &[1, 3], 11.0, 12.0));
    assert!(a.conflicts(b));
    assert!(!a.conflicts(c), "Back-to-back classes don't conflict");
    assert!(!a.conflicts(ScheduleSlotMask::from_section(&sec(
        "4",
        "CS",
        "4",
        &[2, 4],
        10.0,
        11.0
    ))));
}

#[test]
fn generatesOnlyConflictFreeCombinations() {
    let courses = vec![
        req0_fixed(
            "CS",
            "3000",
            vec![
                sec("A1", "CS", "3000", &[1, 3, 4], 9.0, 10.0),
                sec("A2", "CS", "3000", &[1, 3, 4], 13.0, 14.0),
            ],
        ),
        req0_fixed(
            "MATH",
            "2331",
            vec![
                sec("B1", "MATH", "2331", &[1, 3, 4], 9.5, 10.5),
                sec("B2", "MATH", "2331", &[2, 5], 9.0, 10.5),
            ],
        ),
    ];
    let result = generate(&courses, &SchedulePreferences::default());
    assert_eq!(result.total_found, 3);
    assert!(!result.schedules.iter().any(|schedule| schedule
        .crns
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        == ["A1", "B1"].into_iter().map(str::to_string).collect()));
    for schedule in &result.schedules {
        let masks: Vec<ScheduleSlotMask> = schedule
            .sections
            .iter()
            .map(ScheduleSlotMask::from_section)
            .collect();
        for i in 0..masks.len() {
            for j in (i + 1)..masks.len() {
                assert!(!masks[i].conflicts(masks[j]));
            }
        }
    }
}

fn req0_fixed(subject: &str, number: &str, sections: Vec<CourseSection>) -> ScheduleCourseInput {
    input(subject, number, true, 0, sections)
}

#[test]
fn respectsFilters() {
    let courses = vec![req0_fixed(
        "CS",
        "3000",
        vec![
            sec("EARLY", "CS", "3000", &[1, 3], 8.0, 9.0),
            sec("FRI", "CS", "3000", &[5], 12.0, 15.0),
            sec("OK", "CS", "3000", &[2, 4], 11.0, 12.0),
        ],
    )];
    let mut filters = ScheduleFilters::default();
    filters.earliest_start = Some(9 * 60);
    filters.free_days = DaySet::FRIDAY;
    let result = generate_with(
        &courses,
        &filters,
        &SchedulePreferences::default(),
        ScheduleSort::Best,
        &Limits::default(),
    );
    expect_order(&result, &[&["OK"]]);
}

#[test]
fn optionalCoursesFillInWhenTheyFit() {
    let courses = vec![
        req0_fixed(
            "CS",
            "3000",
            vec![sec("R", "CS", "3000", &[1, 3], 10.0, 11.0)],
        ),
        input(
            "ARTF",
            "1220",
            false,
            0,
            vec![
                sec("O1", "ARTF", "1220", &[1], 10.5, 12.0),
                sec("O2", "ARTF", "1220", &[2], 10.0, 12.0),
            ],
        ),
    ];
    expect_sets(
        &generate(&courses, &SchedulePreferences::default()),
        &[&["R", "O2"]],
    );
    let mut exactly_one = ScheduleFilters::default();
    exactly_one.course_count = Some(1);
    let single = generate_with(
        &courses,
        &exactly_one,
        &SchedulePreferences::default(),
        ScheduleSort::Best,
        &Limits::default(),
    );
    expect_order(&single, &[&["R"]]);
}

#[test]
fn realCatalogPlanGenerates() {
    let index = catalog();
    let courses = catalog_inputs(index, &["CS 3000", "CS 3001", "MATH 2331"]);
    let result = generate(&courses, &SchedulePreferences::default());
    assert!(result.total_found > 0);
    assert!(result
        .schedules
        .iter()
        .all(|schedule| schedule.crns.len() == 3));
}

#[test]
fn warningsListWhatNeedsAttention() {
    let index = clashing_index();
    let resolved = schedule::ResolvedPlan::resolve(
        &plan_with(&["CS 3000", "MATH 2331", "ENGW 1111"], &["A1", "B1"]),
        Some(&index),
    );
    let texts: Vec<String> = resolved
        .warnings()
        .into_iter()
        .map(|warning| warning.text)
        .collect();
    assert_eq!(
        texts,
        vec!["1 conflict", "1 full section", "1 course needs a section"]
    );
    assert_eq!(
        resolved.warnings().first().map(|warning| warning.severity),
        Some(Attention::Error)
    );
    assert_eq!(
        resolved.warnings().last().map(|warning| warning.severity),
        Some(Attention::Neutral)
    );
    assert!(
        schedule::ResolvedPlan::resolve(&plan_with(&["CS 3000"], &[]), None)
            .warnings()
            .is_empty()
    );
}

#[test]
fn noticesExplainConflictsAndFullSections() {
    let index = clashing_index();
    let resolved = schedule::ResolvedPlan::resolve(
        &plan_with(&["CS 3000", "MATH 2331"], &["A1", "B1"]),
        Some(&index),
    );
    let notices = schedule::make_notices(&resolved, "Fall 2026", false);
    assert_eq!(
        notices
            .iter()
            .map(|notice| notice.id.as_str())
            .collect::<Vec<_>>(),
        vec!["conflicts", "full"]
    );
    assert_eq!(notices[0].severity, Attention::Error);
    assert_eq!(notices[0].title, "1 time conflict");
    assert!(notices[0]
        .lines
        .first()
        .unwrap()
        .contains("CS 3000 overlaps MATH 2331"));
    assert_eq!(
        notices[1].lines,
        vec!["MATH 2331 Lecture 01 is full \u{00b7} 5 waitlist spots left"]
    );
    assert!(notices.iter().all(|notice| notice.fix.is_none()));
}

#[test]
fn noticesOfferToForgetSectionsTheCatalogDoesNotList() {
    let index = clashing_index();
    let resolved = schedule::ResolvedPlan::resolve(
        &plan_with(&["CS 3000"], &["A1", "99999", "C1"]),
        Some(&index),
    );
    let notices = schedule::make_notices(&resolved, "Fall 2026", false);
    let ids: Vec<&str> = notices.iter().map(|notice| notice.id.as_str()).collect();
    assert!(ids.contains(&"unlisted"));
    assert!(ids.contains(&"orphaned"));
    assert_eq!(
        notices
            .iter()
            .find(|notice| notice.id == "unlisted")
            .and_then(|notice| notice.fix.clone()),
        Some(NoticeFix::RemoveSections(vec!["99999".into()]))
    );
    assert_eq!(
        notices
            .iter()
            .find(|notice| notice.id == "orphaned")
            .and_then(|notice| notice.fix.clone()),
        Some(NoticeFix::RemoveSections(vec!["C1".into()]))
    );
    assert_eq!(
        notices
            .iter()
            .find(|notice| notice.id == "orphaned")
            .and_then(|notice| notice.fix_title()),
        Some("Remove")
    );
}

#[test]
fn aMissingCatalogIsReportedOnceItIsKnownToBeMissing() {
    let resolved = schedule::ResolvedPlan::resolve(&plan_with(&["CS 3000"], &[]), None);
    assert!(schedule::make_notices(&resolved, "Spring 2027", false).is_empty());
    let notices = schedule::make_notices(&resolved, "Spring 2027", true);
    assert_eq!(
        notices
            .iter()
            .map(|notice| notice.id.as_str())
            .collect::<Vec<_>>(),
        vec!["catalog"]
    );
    assert_eq!(
        notices[0].title,
        "Course data for Spring 2027 isn\u{2019}t on this device"
    );
    assert!(schedule::make_notices(
        &schedule::ResolvedPlan::resolve(&plan_with(&[], &[]), None),
        "Spring 2027",
        true
    )
    .is_empty());
}

#[test]
fn favoritesAreMeasuredAndSkipWhenTheyCanNoLongerBeApplied() {
    let index = clashing_index();
    let mut plan = plan_with(&["CS 3000", "ENGW 1111"], &[]);
    plan.favorite_combinations = vec![
        vec!["A1".into(), "C1".into()],
        vec!["A1".into(), "GONE".into()],
        vec!["B1".into(), "C1".into()],
    ];
    let favorites = schedule::favorites_of(&plan, &index, &SchedulePreferences::default());
    assert_eq!(
        favorites
            .iter()
            .map(|schedule| schedule.crns.clone())
            .collect::<Vec<_>>(),
        vec![vec!["A1".to_string(), "C1".to_string()]]
    );
    assert_eq!(
        favorites
            .first()
            .map(|schedule| schedule.sorted_crns.clone()),
        Some(vec!["A1".into(), "C1".into()])
    );
}

#[test]
fn favoritesComeNewestFirstInPlanOrder() {
    let index = clashing_index();
    let mut plan = plan_with(&["ENGW 1111", "CS 3000"], &[]);
    plan.favorite_combinations = vec![vec!["A1".into()], vec!["C1".into(), "A1".into()]];
    let favorites = schedule::favorites_of(&plan, &index, &SchedulePreferences::default());
    let sets: Vec<BTreeSet<String>> = favorites
        .iter()
        .map(|schedule| schedule.crns.iter().cloned().collect())
        .collect();
    assert_eq!(
        sets,
        vec![
            ["A1", "C1"].into_iter().map(str::to_string).collect(),
            ["A1"].into_iter().map(str::to_string).collect()
        ]
    );
    assert_eq!(
        favorites.first().map(|schedule| schedule.crns.clone()),
        Some(vec!["C1".into(), "A1".into()])
    );
}

#[test]
fn courseInputsRoundCreditsUpAndKeepExclusions() {
    let index = tiny_index(
        vec![
            listed_course("CS", "3000", 3.5),
            listed_course("MATH", "2331", 4.0),
        ],
        vec![
            sec("A1", "CS", "3000", &[1], 9.0, 10.0),
            sec("A2", "CS", "3000", &[2], 9.0, 10.0),
        ],
    );
    let mut plan = plan_with(&["CS 3000", "MATH 2331"], &[]);
    plan.courses[0].excluded_crns = HashSet::from(["A2".into()]);
    plan.courses[1].is_required = false;
    let inputs = schedule::inputs_for(&plan, &index);
    assert_eq!(
        inputs
            .iter()
            .map(|course| course.key.to_string())
            .collect::<Vec<_>>(),
        vec!["CS 3000".to_string(), "MATH 2331".to_string()]
    );
    assert_eq!(inputs[0].min_credits, 4);
    assert_eq!(inputs[0].sections.len(), 2);
    assert_eq!(inputs[0].excluded_crns, HashSet::from(["A2".into()]));
    assert!(!inputs[1].is_required);
    assert!(inputs[1].sections.is_empty());
}

#[test]
fn looseningARuleEditsThePlanOrItsPreferences() {
    let mut plan = plan_with(&["CS 3000", "MATH 2331"], &[]);
    plan.filters.earliest_start = Some(10 * 60);
    plan.filters.free_days = DaySet::FRIDAY;
    let mut preferences = SchedulePreferences::default();
    preferences.days_off = DaySet::FRIDAY;
    preferences.days_off_strength = PreferenceStrength::Required;
    plan.preferences = Some(preferences);
    schedule::apply_relaxation(&mut plan, &Relaxation::ClearEarliestStart);
    assert_eq!(plan.filters.earliest_start, None);
    assert_eq!(plan.filters.free_days, DaySet::FRIDAY);
    schedule::apply_relaxation(&mut plan, &Relaxation::MakeOptional(key("CS", "3000")));
    assert!(!plan.courses[0].is_required);
    assert!(plan.courses[1].is_required);
    schedule::apply_relaxation(
        &mut plan,
        &Relaxation::Preference(PreferenceRelaxation::SoftenDaysOff),
    );
    assert_eq!(
        plan.preferences
            .as_ref()
            .map(|preferences| preferences.days_off_strength),
        Some(PreferenceStrength::Preferred)
    );
    let mut only_block = plan_with(&["CS 3000"], &[]);
    let mut block_preferences = SchedulePreferences::default();
    block_preferences.max_classes_per_day = Some(2);
    only_block.preferences = Some(block_preferences);
    schedule::apply_relaxation(
        &mut only_block,
        &Relaxation::Preference(PreferenceRelaxation::ClearMaxClassesPerDay),
    );
    assert!(only_block.preferences.is_none());
    schedule::make_optional(&mut plan, &[key("CS", "3000"), key("MATH", "2331")]);
    assert!(plan.courses.iter().all(|course| !course.is_required));
}

#[test]
fn noResultsWordingNamesWhatBlocksAndWhatWouldWork() {
    let index = clashing_index();
    let plan = plan_with(&["CS 3000", "MATH 2331"], &[]);
    let courses = schedule::inputs_for(&plan, &index);
    let result = generate(&courses, &SchedulePreferences::default());
    assert!(result.schedules.is_empty());
    assert_eq!(
        schedule::no_results_title(&result.diagnostics),
        "No schedule fits all 2 courses"
    );
    let analysis = schedule::analyze_empty_result(
        &courses,
        &ScheduleFilters::default(),
        &SchedulePreferences::default(),
        &result.diagnostics,
        true,
    );
    assert_eq!(
        schedule::no_results_headline_for(&result.diagnostics, Some(&analysis.blockers)),
        "Here\u{2019}s what would work."
    );
    let reasons = schedule::no_results_reasons(&result.diagnostics, "Fall 2026");
    assert_eq!(
        reasons
            .iter()
            .map(|reason| reason.title.as_str())
            .collect::<Vec<_>>(),
        vec!["CS 3000 and MATH 2331 always overlap"]
    );
    let option = analysis.blockers.options.first().unwrap();
    assert!(
        schedule::blocker_detail(option, &analysis.blockers, "Fall 2026").starts_with("1 schedule")
    );
    assert_eq!(
        schedule::course_list(&[key("CS", "3000"), key("MATH", "2331"), key("PHYS", "1151")]),
        "CS 3000, MATH 2331 and PHYS 1151"
    );
}

#[test]
fn noResultsWordingSaysWhyEverySectionWasRuledOut() {
    let index = clashing_index();
    let plan = plan_with(&["CS 3000"], &[]);
    let mut filters = ScheduleFilters::default();
    filters.earliest_start = Some(20 * 60);
    let result = generate_with(
        &schedule::inputs_for(&plan, &index),
        &filters,
        &SchedulePreferences::default(),
        ScheduleSort::Best,
        &Limits::default(),
    );
    let reasons = schedule::no_results_reasons(&result.diagnostics, "Fall 2026");
    assert_eq!(
        reasons
            .iter()
            .map(|reason| reason.title.as_str())
            .collect::<Vec<_>>(),
        vec!["No CS 3000 sections left"]
    );
    assert_eq!(
        reasons.first().and_then(|reason| reason.detail.as_deref()),
        Some("Its only section starts before 8:00 PM.")
    );
}

#[test]
fn uncertainReorderReasonsAreNotGoodNews() {
    assert!(schedule::is_uncertain("Offering history unknown"));
    assert!(schedule::is_uncertain("Couldn\u{2019}t check the catalog"));
    assert!(!schedule::is_uncertain("Offered every spring"));
}

#[test]
fn blockedTimesReadAsTheyAreConfigured() {
    let mut item = block(
        "Club",
        DaySet::TUESDAY.union(DaySet::THURSDAY),
        18 * 60,
        20 * 60,
    );
    assert_eq!(item.list_detail(), "Tue, Thu 6:00 \u{2013} 8:00 PM");
    item.strength = PreferenceStrength::Preferred;
    assert_eq!(
        item.list_detail(),
        "Tue, Thu 6:00 \u{2013} 8:00 PM \u{00b7} If I must"
    );
    item.is_enabled = false;
    assert_eq!(
        item.list_detail(),
        "Tue, Thu 6:00 \u{2013} 8:00 PM \u{00b7} Off"
    );
    assert_eq!(
        BlockedTime {
            id: Uuid::new_v4(),
            label: String::new(),
            days: DaySet(0),
            start: 600,
            end: 660,
            strength: PreferenceStrength::Required,
            is_enabled: true
        }
        .list_detail(),
        "Choose the days"
    );
    assert_eq!(
        BlockedTime {
            id: Uuid::new_v4(),
            label: String::new(),
            days: DaySet::MONDAY,
            start: 700,
            end: 660,
            strength: PreferenceStrength::Required,
            is_enabled: true
        }
        .list_detail(),
        "Check the times"
    );
}

#[test]
fn calendarLayoutFitsEverySchedule() {
    let early = GeneratedSchedule::from_sections(
        &[sec("A", "CS", "1", &[1], 8.0, 9.0)],
        &SchedulePreferences::default(),
    );
    let late = GeneratedSchedule::from_sections(
        &[sec("B", "CS", "2", &[6], 17.0, 20.0)],
        &SchedulePreferences::default(),
    );
    let layout =
        schedule::CarouselLayout::from_schedules(&[early, late], &AppPreferences::default());
    assert_eq!(layout.hours, 8..=20);
    assert_eq!(layout.days, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(
        schedule::CarouselLayout::from_schedules(&[], &AppPreferences::default()).days,
        vec![1, 2, 3, 4, 5]
    );
}

#[test]
fn autofillSuggestionsComeFromTheDegreePlanAndOtherSchedules() {
    let index = clashing_index();
    let target = plan_with(&[], &[]);
    let mut sibling = plan_with(&["CS 3000", "ENGW 1111"], &[]);
    sibling.name = "Plan B".into();
    let suggestions =
        schedule::autofill_suggestions(&target, &[], &[], &[sibling, target.clone()], Some(&index));
    assert_eq!(suggestions.len(), 1);
    assert_eq!(
        suggestions
            .first()
            .unwrap()
            .courses
            .iter()
            .map(|course| course.to_string())
            .collect::<Vec<_>>(),
        vec!["CS 3000".to_string(), "ENGW 1111".to_string()]
    );
    assert_eq!(
        schedule::autofill_source_name(&suggestions[0]),
        "Copy \u{201c}Plan B\u{201d}"
    );
    assert!(
        schedule::autofill_suggestions(&target, &[], &[], &[target.clone()], Some(&index))
            .is_empty()
    );
    assert_eq!(schedule::course_count_label(1), "1 course");
    assert_eq!(schedule::course_count_label(3), "3 courses");
    let dismissal = schedule::dismissal_key(target.id, &suggestions[0].id);
    let stored = schedule::dismissal_adding(&dismissal, "", 200);
    assert!(schedule::dismissal_contains(&dismissal, &stored));
    assert!(!schedule::dismissal_contains("other", &stored));
}

#[test]
fn draggedTextNamesCoursesAndSections() {
    let parsed = schedule::parse_dropped(&[
        "CS 3000".into(),
        "math 2331 12345\nPHYS 1151 99\nCS 3000".into(),
        "Dr. Smith".into(),
        "John Smith".into(),
    ]);
    assert_eq!(
        parsed
            .iter()
            .map(|item| item.key.to_string())
            .collect::<Vec<_>>(),
        vec![
            "CS 3000".to_string(),
            "MATH 2331".to_string(),
            "PHYS 1151".to_string()
        ]
    );
    assert_eq!(
        parsed
            .iter()
            .map(|item| item.crn.clone())
            .collect::<Vec<_>>(),
        vec![None, Some("12345".into()), None]
    );
    assert!(schedule::parse_dropped(&[]).is_empty());
    assert!(schedule::parse_dropped(&["hello".into()]).is_empty());
}

#[test]
fn clockConvertsMinutesAndPickerDates() {
    for minutes in [0, 9 * 60 + 15, 23 * 60 + 59] {
        assert_eq!(schedule::clock_minutes(minutes), minutes);
    }
    assert_eq!(schedule::clock_minutes(24 * 60), 24 * 60 - 1);
    assert_eq!(schedule::clock_minutes(-5), 0);
}

#[test]
fn removingACourseTakesItsSectionsAndFavoritesAndUndoPutsThemBack() {
    let index = clashing_index();
    let mut data = UserData::default();
    let plan = schedule::create_plan(&mut data, "Plan A", "202710");
    for raw in ["CS 3000", "MATH 2331", "ENGW 1111"] {
        schedule::add_course(&mut data, plan.id, code(raw), true);
    }
    schedule::update_plan(&mut data, plan.id, |plan| {
        plan.selected_crns = vec!["A1".into(), "C1".into()];
        plan.favorite_combinations = vec![
            vec!["A1".into(), "C1".into()],
            vec!["B1".into(), "C1".into()],
        ];
    });
    let removal =
        schedule::remove_plan_course(&mut data, plan.id, &code("CS 3000"), Some(&index)).unwrap();
    let after = schedule::plan_by_id(&data, plan.id).unwrap();
    assert_eq!(
        after
            .courses
            .iter()
            .map(|course| course.key.to_string())
            .collect::<Vec<_>>(),
        vec!["MATH 2331".to_string(), "ENGW 1111".to_string()]
    );
    assert_eq!(after.selected_crns, vec!["C1".to_string()]);
    assert_eq!(
        after.favorite_combinations,
        vec![vec!["B1".to_string(), "C1".to_string()]]
    );
    assert_eq!(removal.index, 0);
    assert_eq!(removal.selected_crns, vec!["A1".to_string()]);
    schedule::update_plan(&mut data, plan.id, |plan| {
        schedule::restore_removal(plan, &removal)
    });
    let restored = schedule::plan_by_id(&data, plan.id).unwrap();
    assert_eq!(
        restored
            .courses
            .iter()
            .map(|course| course.key.to_string())
            .collect::<Vec<_>>(),
        vec![
            "CS 3000".to_string(),
            "MATH 2331".to_string(),
            "ENGW 1111".to_string()
        ]
    );
    assert_eq!(
        restored
            .selected_crns
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        ["A1", "C1"].into_iter().map(str::to_string).collect()
    );
    assert!(restored
        .favorite_combinations
        .iter()
        .any(|combination| combination == &vec!["A1".to_string(), "C1".to_string()]));
}

#[test]
fn autofillAddsRequiredCoursesOnceAndUndoTakesThemOut() {
    let index = clashing_index();
    let mut data = UserData::default();
    let plan = schedule::create_plan(&mut data, "Plan A", "202710");
    schedule::add_course(&mut data, plan.id, code("CS 3000"), true);
    let added =
        schedule::add_autofill_courses(&mut data, plan.id, &[code("CS 3000"), code("ENGW 1111")]);
    assert_eq!(
        added
            .iter()
            .map(|course| course.to_string())
            .collect::<Vec<_>>(),
        vec!["ENGW 1111".to_string()]
    );
    assert_eq!(
        schedule::plan_by_id(&data, plan.id).unwrap().courses.len(),
        2
    );
    assert_eq!(
        schedule::plan_by_id(&data, plan.id)
            .unwrap()
            .courses
            .iter()
            .find(|course| course.key == code("ENGW 1111"))
            .map(|course| course.is_required),
        Some(true)
    );
    schedule::undo_autofill(&mut data, plan.id, &added, Some(&index));
    assert_eq!(
        schedule::plan_by_id(&data, plan.id)
            .unwrap()
            .courses
            .iter()
            .map(|course| course.key.to_string())
            .collect::<Vec<_>>(),
        vec!["CS 3000".to_string()]
    );
}

#[test]
fn reorderingTheDegreePlanNeedsASemesterThatMatchesTheTerm() {
    let plan = plan_with(&["CS 3000"], &[]);
    let mut degree = DegreePlan {
        name: "Plan".into(),
        ..DegreePlan::default()
    };
    let mut fall = DegreePlanTerm::new(PlanSeason::Fall, 2026);
    fall.courses = vec![DegreePlanCourse::catalog(
        code("CS 3000"),
        "Algorithms",
        4.0,
    )];
    degree.terms = vec![fall];
    let terms = vec![Term {
        code: "202710".into(),
        banner_description: "Fall 2026 Semester".into(),
    }];
    let found = schedule::reorder_semester(&plan, Some(&degree), &terms, None);
    assert_eq!(
        found.as_ref().map(|(_, term)| term.code.as_str()),
        Some("202710")
    );
    assert_eq!(found.as_ref().map(|(plan, _)| plan.id), Some(degree.id));
    let mut spring = plan.clone();
    spring.term_code = "202730".into();
    assert!(schedule::reorder_semester(&spring, Some(&degree), &terms, None).is_none());
    assert!(schedule::reorder_semester(&plan, None, &terms, None).is_none());
}

#[test]
fn halfTermSectionsDoNotConflict() {
    let first_half = meeting(&[1, 3], 600, 665, Some("2026-09-09"), Some("2026-10-27"));
    let second_half = meeting(&[1, 3], 600, 665, Some("2026-10-28"), Some("2026-12-13"));
    let full_term = meeting(&[3], 630, 700, Some("2026-09-09"), Some("2026-12-13"));
    assert!(!first_half.overlaps(&second_half));
    assert!(first_half.overlaps(&full_term));
    assert!(second_half.overlaps(&full_term));
    assert!(first_half.overlaps(&meeting(&[1], 640, 700, None, None)));
}

#[test]
fn calendarConflictsRespectDates() {
    let event =
        |id: &str, payload: &str, start: i32, end: i32, from: Option<&str>, to: Option<&str>| {
            CalendarEvent {
                id: id.into(),
                day: 1,
                start,
                end,
                is_conflict: false,
                is_ghost: false,
                payload: Some(payload.into()),
                start_date: from.map(str::to_string),
                end_date: to.map(str::to_string),
            }
        };
    let a = event("a", "1", 600, 665, Some("2026-09-09"), Some("2026-10-27"));
    let b = event("b", "2", 600, 665, Some("2026-10-28"), Some("2026-12-13"));
    let c = event("c", "3", 630, 700, None, None);
    assert!(schedule::marking_conflicts(&[a.clone(), b.clone(), c])
        .iter()
        .all(|item| item.is_conflict));
    assert!(!schedule::marking_conflicts(&[a, b])
        .iter()
        .any(|item| item.is_conflict));
}

#[test]
fn widgetMeetingsHonorDateWindows() {
    let now = upnext::eastern_to_unix(2026, 11, 2, 9, 0);
    let early = WidgetMeeting {
        id: "a".into(),
        crn: "1".into(),
        course_code: "ARTF 1220".into(),
        title: "A".into(),
        location: "RY 200".into(),
        class_type: "Lecture".into(),
        weekday: 1,
        start: 600,
        end: 665,
        color_index: 0,
        start_date: Some("2026-09-09".into()),
        end_date: Some("2026-10-27".into()),
    };
    let late = WidgetMeeting {
        id: "b".into(),
        crn: "2".into(),
        course_code: "ARTF 1221".into(),
        title: "B".into(),
        location: "RY 200".into(),
        class_type: "Lecture".into(),
        weekday: 1,
        start: 600,
        end: 665,
        color_index: 1,
        start_date: Some("2026-10-28".into()),
        end_date: Some("2026-12-13".into()),
    };
    let data = WidgetScheduleData {
        generated_at: now,
        term_code: "202710".into(),
        term_name: "Fall 2026".into(),
        plan_name: "Plan".into(),
        classes_start: None,
        classes_end: None,
        meetings: vec![early, late],
    };
    assert_eq!(
        data.classes_on(now)
            .iter()
            .map(|item| item.meeting.course_code.as_str())
            .collect::<Vec<_>>(),
        vec!["ARTF 1221"]
    );
    assert_eq!(
        data.occurrences(now, 1)
            .first()
            .map(|item| item.meeting.course_code.as_str()),
        Some("ARTF 1221")
    );
}

#[test]
fn termWindowsAreSensible() {
    let fall = calendar::approximate_window("202710").expect("fall window");
    let spring = calendar::approximate_window("202730").expect("spring window");
    let (fall_year, fall_month, _) = utc_ymd(fall.0);
    let (spring_year, _, _) = utc_ymd(spring.0);
    assert_eq!(fall_year, 2026);
    assert_eq!(fall_month, 9);
    assert_eq!(spring_year, 2027);
    assert!(fall.1 < spring.0);
}

#[test]
fn activePlanFollowsTheTermInSession() {
    let mut data = UserData::default();
    let fall = schedule::create_plan(&mut data, "Fall", "202710");
    let spring = schedule::create_plan(&mut data, "Spring", "202730");
    schedule::update_plan(&mut data, fall.id, |plan| {
        plan.selected_crns = vec!["10001".into()]
    });
    schedule::update_plan(&mut data, spring.id, |plan| {
        plan.selected_crns = vec!["20001".into()]
    });
    let mid_fall = upnext::eastern_to_unix(2026, 10, 15, 0, 0);
    let winter_break = upnext::eastern_to_unix(2026, 12, 28, 0, 0);
    let long_after = upnext::eastern_to_unix(2027, 7, 1, 0, 0);
    assert_eq!(
        schedule::active_primary_plan(&data.plans, mid_fall).map(|plan| plan.id),
        Some(fall.id)
    );
    assert_eq!(
        schedule::active_primary_plan(&data.plans, winter_break).map(|plan| plan.id),
        Some(spring.id)
    );
    assert!(schedule::active_primary_plan(&data.plans, long_after).is_none());
}
