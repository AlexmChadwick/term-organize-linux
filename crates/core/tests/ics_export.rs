// Copyright (C) 2026 Term Organize contributors.
// SPDX-License-Identifier: AGPL-3.0-only

use termorganize_core::model::{CourseSection, DaySet, Instructor, Meeting};
use termorganize_core::schedule;

fn make_section(
    crn: &str,
    subject: &str,
    number: &str,
    title: &str,
    meetings: Vec<Meeting>,
    instructors: Option<Vec<Instructor>>,
) -> CourseSection {
    CourseSection {
        crn: crn.into(),
        subject: subject.into(),
        number: number.into(),
        title: title.into(),
        section_number: "01".into(),
        part_of_term: "1".into(),
        seat_capacity: 30,
        seats_remaining: 5,
        enrollment: 25,
        waitlist_capacity: 10,
        waitlist_count: 0,
        waitlist_remaining: 10,
        class_type: "Lecture".into(),
        honors: false,
        campus: "Boston".into(),
        instruction_method: Some("Traditional".into()),
        meetings,
        instructors,
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

#[test]
fn includes_vtimezone_block_for_america_new_york() {
    let meeting = Meeting {
        days: DaySet::from_indices([2, 4]), // Tue, Thu
        start: Some(590),                   // 9:50 AM
        end: Some(665),                     // 11:05 AM
        building_code: Some("WVH".into()),
        building: Some("West Village H".into()),
        room: Some("108".into()),
        campus_code: Some("BOS".into()),
        kind: "Class".into(),
        start_date: Some("2026-09-08".into()),
        end_date: Some("2026-12-09".into()),
    };
    let section = make_section(
        "12345",
        "CS",
        "3000",
        "Algorithms & Data",
        vec![meeting],
        None,
    );
    let cal = schedule::ics("plan-1", "My Schedule", &[section]);

    assert!(cal.contains("BEGIN:VCALENDAR"));
    assert!(cal.contains("VERSION:2.0"));
    assert!(cal.contains("PRODID:-//Term Organize//Schedule//EN"));
    assert!(cal.contains("X-WR-CALNAME:My Schedule"));
    assert!(cal.contains("X-WR-TIMEZONE:America/New_York"));
    assert!(cal.contains("BEGIN:VTIMEZONE"));
    assert!(cal.contains("TZID:America/New_York"));
    assert!(cal.contains("BEGIN:DAYLIGHT"));
    assert!(cal.contains("TZNAME:EDT"));
    assert!(cal.contains("BEGIN:STANDARD"));
    assert!(cal.contains("TZNAME:EST"));
    assert!(cal.contains("END:VTIMEZONE"));
    assert!(cal.ends_with("END:VCALENDAR\r\n"));
}

#[test]
fn aligns_weekly_meeting_dtstart_to_first_matching_day() {
    // start_date is Monday 2026-09-07, but the section meets on Tuesday (2) and Thursday (4).
    // The first meeting date must advance to Tuesday 2026-09-08 per RFC 5545 section 3.8.2.4.
    let meeting = Meeting {
        days: DaySet::from_indices([2, 4]),
        start: Some(590),
        end: Some(665),
        building_code: Some("WVH".into()),
        building: Some("West Village H".into()),
        room: Some("108".into()),
        campus_code: Some("BOS".into()),
        kind: "Class".into(),
        start_date: Some("2026-09-07".into()), // Monday
        end_date: Some("2026-12-09".into()),
    };
    let section = make_section(
        "12345",
        "CS",
        "3000",
        "Algorithms & Data",
        vec![meeting],
        None,
    );
    let cal = schedule::ics("plan-1", "My Schedule", &[section]);

    assert!(cal.contains("UID:plan-1-12345-0@neuclasssearch"));
    assert!(cal.contains("DTSTART;TZID=America/New_York:20260908T095000"));
    assert!(cal.contains("DTEND;TZID=America/New_York:20260908T110500"));
    assert!(cal.contains("RRULE:FREQ=WEEKLY;BYDAY=TU,TH;UNTIL=20261210T045959Z"));
}

#[test]
fn exports_timed_final_exam_as_distinct_vevent() {
    let class_meeting = Meeting {
        days: DaySet::from_indices([1, 3]), // Mon, Wed
        start: Some(500),
        end: Some(565),
        building_code: Some("RY".into()),
        building: Some("Ryder Hall".into()),
        room: Some("100".into()),
        campus_code: Some("BOS".into()),
        kind: "Class".into(),
        start_date: Some("2026-09-09".into()),
        end_date: Some("2026-12-09".into()),
    };
    let exam_meeting = Meeting {
        days: DaySet(0),
        start: Some(480), // 8:00 AM
        end: Some(600),   // 10:00 AM
        building_code: Some("SH".into()),
        building: Some("Shillman Hall".into()),
        room: Some("105".into()),
        campus_code: Some("BOS".into()),
        kind: "Final Exam".into(),
        start_date: Some("2026-12-14".into()),
        end_date: Some("2026-12-14".into()),
    };
    let instructors = vec![Instructor {
        name: "Prof Smith".into(),
        primary: true,
        email: Some("smith@example.edu".into()),
    }];
    let section = make_section(
        "12345",
        "CS",
        "3000",
        "Algorithms & Data",
        vec![class_meeting, exam_meeting],
        Some(instructors),
    );
    let cal = schedule::ics("plan-1", "Schedule", &[section]);

    assert!(cal.contains("UID:plan-1-12345-final@neuclasssearch"));
    assert!(cal.contains("SUMMARY:CS 3000 Final Exam"));
    assert!(cal.contains("DTSTART;TZID=America/New_York:20261214T080000"));
    assert!(cal.contains("DTEND;TZID=America/New_York:20261214T100000"));
    assert!(cal.contains("LOCATION:Shillman Hall 105"));

    // Verify final exam event does not contain recurrence rule
    let exam_part = cal
        .split("UID:plan-1-12345-final@neuclasssearch")
        .nth(1)
        .expect("final exam VEVENT present");
    let exam_event = exam_part
        .split("END:VEVENT")
        .next()
        .expect("end of final exam VEVENT");
    assert!(!exam_event.contains("RRULE:"));
    assert!(exam_event.contains("DESCRIPTION:Final exam"));
}

#[test]
fn exports_untimed_all_day_final_exam_with_next_day_dtend() {
    let class_meeting = Meeting {
        days: DaySet::from_indices([5]), // Friday
        start: Some(600),
        end: Some(700),
        building_code: None,
        building: None,
        room: None,
        campus_code: None,
        kind: "Class".into(),
        start_date: Some("2026-09-11".into()),
        end_date: Some("2026-12-04".into()),
    };
    let exam_meeting = Meeting {
        days: DaySet(0),
        start: None,
        end: None,
        building_code: None,
        building: None,
        room: None,
        campus_code: None,
        kind: "Final Exam".into(),
        start_date: Some("2026-12-15".into()),
        end_date: Some("2026-12-15".into()),
    };
    let section = make_section(
        "12345",
        "CS",
        "3000",
        "Algorithms & Data",
        vec![class_meeting, exam_meeting],
        None,
    );
    let cal = schedule::ics("plan-1", "Schedule", &[section]);

    assert!(cal.contains("UID:plan-1-12345-final@neuclasssearch"));
    assert!(cal.contains("SUMMARY:CS 3000 Final Exam"));
    assert!(cal.contains("DTSTART;VALUE=DATE:20261215"));
    assert!(cal.contains("DTEND;VALUE=DATE:20261216"));

    let exam_part = cal
        .split("UID:plan-1-12345-final@neuclasssearch")
        .nth(1)
        .expect("final exam VEVENT present");
    let exam_event = exam_part
        .split("END:VEVENT")
        .next()
        .expect("end of final exam VEVENT");
    assert!(!exam_event.contains("RRULE:"));
}

#[test]
fn exports_multiple_sections_each_with_final_exam() {
    let exam1 = Meeting {
        days: DaySet(0),
        start: Some(480),
        end: Some(600),
        building_code: Some("SH".into()),
        building: Some("Shillman Hall".into()),
        room: Some("105".into()),
        campus_code: Some("BOS".into()),
        kind: "Final Exam".into(),
        start_date: Some("2026-12-14".into()),
        end_date: Some("2026-12-14".into()),
    };
    let exam2 = Meeting {
        days: DaySet(0),
        start: None,
        end: None,
        building_code: None,
        building: None,
        room: None,
        campus_code: None,
        kind: "Final Exam".into(),
        start_date: Some("2026-12-16".into()),
        end_date: Some("2026-12-16".into()),
    };
    let s1 = make_section("11111", "CS", "2500", "Fundamentals 1", vec![exam1], None);
    let s2 = make_section("22222", "MATH", "1341", "Calculus 1", vec![exam2], None);
    let cal = schedule::ics("plan-1", "Multi Schedule", &[s1, s2]);

    assert!(cal.contains("UID:plan-1-11111-final@neuclasssearch"));
    assert!(cal.contains("SUMMARY:CS 2500 Final Exam"));
    assert!(cal.contains("UID:plan-1-22222-final@neuclasssearch"));
    assert!(cal.contains("SUMMARY:MATH 1341 Final Exam"));
}

#[test]
fn handles_online_final_exams() {
    let exam = Meeting {
        days: DaySet(0),
        start: Some(480),
        end: Some(600),
        building_code: Some("ONLINE".into()),
        building: None,
        room: None,
        campus_code: Some("VTL".into()),
        kind: "Final Exam".into(),
        start_date: Some("2026-12-17".into()),
        end_date: Some("2026-12-17".into()),
    };
    let section = make_section(
        "33333",
        "CY",
        "2550",
        "Foundations of Cybersecurity",
        vec![exam],
        None,
    );
    let cal = schedule::ics("plan-1", "Online Schedule", &[section]);

    assert!(cal.contains("UID:plan-1-33333-final@neuclasssearch"));
    assert!(cal.contains("LOCATION:Online"));
}

#[test]
fn folds_lines_to_at_most_75_octets() {
    let exam = Meeting {
        days: DaySet(0),
        start: Some(480),
        end: Some(600),
        building_code: Some("WVH".into()),
        building: Some("Extremely Long Building Name For Testing Line Folding".into()),
        room: Some("999".into()),
        campus_code: Some("BOS".into()),
        kind: "Final Exam".into(),
        start_date: Some("2026-12-14".into()),
        end_date: Some("2026-12-14".into()),
    };
    let instructors = vec![Instructor {
        name: "Professor Long Name With Multiple Affiliations And Degrees".into(),
        primary: true,
        email: Some("longname@example.edu".into()),
    }];
    let section = make_section(
        "44444",
        "CS",
        "4999",
        "Extremely Long Course Title Designed To Exceed The RFC 5545 Line Folding Limit By Being Very Verbose",
        vec![exam],
        Some(instructors),
    );
    let cal = schedule::ics("plan-1", "Schedule With Long Names", &[section]);

    for line in cal.split("\r\n") {
        assert!(
            line.len() <= 75,
            "Line exceeds 75 octets: len={}, content={:?}",
            line.len(),
            line
        );
    }
}

#[test]
fn handles_empty_sections_gracefully() {
    let cal = schedule::ics("plan-1", "Empty Schedule", &[]);
    assert!(cal.contains("BEGIN:VCALENDAR"));
    assert!(cal.contains("END:VCALENDAR\r\n"));
    assert!(!cal.contains("BEGIN:VEVENT"));
}

fn class_meeting(end_date: Option<&str>) -> Meeting {
    Meeting {
        days: DaySet::from_indices([1, 3]),
        start: Some(500),
        end: Some(565),
        building_code: None,
        building: None,
        room: None,
        campus_code: None,
        kind: "Class".into(),
        start_date: Some("2026-09-09".into()),
        end_date: end_date.map(Into::into),
    }
}

fn events(cal: &str) -> Vec<&str> {
    cal.split("BEGIN:VEVENT").skip(1).collect()
}

#[test]
fn every_vevent_has_utc_dtstamp_and_unique_uid_per_plan() {
    let exam = Meeting {
        days: DaySet(0),
        start: Some(480),
        end: Some(600),
        kind: "Final Exam".into(),
        start_date: Some("2026-12-14".into()),
        end_date: Some("2026-12-14".into()),
        ..class_meeting(None)
    };
    let section = make_section(
        "12345",
        "CS",
        "3000",
        "Algo",
        vec![class_meeting(Some("2026-12-09")), exam],
        None,
    );
    let a = schedule::ics("plan-a", "S", std::slice::from_ref(&section));
    let b = schedule::ics("plan-b", "S", &[section]);
    let uids = |cal: &str| -> Vec<String> {
        events(cal)
            .iter()
            .map(|e| {
                e.lines()
                    .find(|l| l.starts_with("UID:"))
                    .expect("UID")
                    .to_string()
            })
            .collect()
    };
    for cal in [&a, &b] {
        let evs = events(cal);
        assert_eq!(evs.len(), 2);
        for e in evs {
            let stamp = e
                .lines()
                .find_map(|l| l.strip_prefix("DTSTAMP:"))
                .expect("DTSTAMP");
            assert_eq!(stamp.len(), 16);
            assert!(stamp.ends_with('Z') && stamp.as_bytes()[8] == b'T');
            assert_eq!(
                e.lines().filter(|l| l.starts_with("DTSTAMP:")).count(),
                1,
                "exactly one DTSTAMP per event"
            );
        }
    }
    let (ua, ub) = (uids(&a), uids(&b));
    assert_eq!(ua.len(), 2);
    assert_ne!(ua[0], ua[1]);
    assert!(ua
        .iter()
        .all(|u| u.contains("plan-a") && u.contains("12345")));
    assert!(ua.iter().all(|u| !ub.contains(u)));
}

#[test]
fn weekly_meeting_without_end_date_is_skipped() {
    let section = make_section(
        "777",
        "CS",
        "1000",
        "NoEnd",
        vec![class_meeting(None)],
        None,
    );
    let cal = schedule::ics("plan-1", "S", &[section]);
    assert!(events(&cal).is_empty());
    assert!(!cal.contains("UID:plan-1-777"));
}

#[test]
fn weekly_rrule_always_has_utc_until() {
    let section = make_section(
        "888",
        "CS",
        "1000",
        "Bounded",
        vec![class_meeting(Some("2026-12-09"))],
        None,
    );
    let cal = schedule::ics("plan-1", "S", &[section]);
    assert_eq!(events(&cal).len(), 1);
    for e in events(&cal) {
        let rule = e.lines().find(|l| l.starts_with("RRULE:")).expect("RRULE");
        let until = rule.split("UNTIL=").nth(1).expect("UNTIL");
        assert!(until.ends_with('Z'), "{until}");
        assert!(e.contains("DTSTART;TZID=America/New_York:"));
    }
}

// ---- Date validation, skipping and DTSTART consistency ----

fn weekly(
    days: &[i32],
    start_date: Option<&str>,
    end_date: Option<&str>,
    start: i32,
    end: i32,
) -> Meeting {
    Meeting {
        days: DaySet::from_indices(days.iter().copied()),
        start: Some(start),
        end: Some(end),
        kind: "Class".into(),
        start_date: start_date.map(Into::into),
        end_date: end_date.map(Into::into),
        ..class_meeting(None)
    }
}

/// A property's full value with RFC 5545 folding undone.
fn field(event: &str, prefix: &str) -> String {
    let flat = event.replace("\r\n ", "");
    line_of(&flat, prefix).to_string()
}

fn line_of<'a>(event: &'a str, prefix: &str) -> &'a str {
    event
        .lines()
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("{prefix} missing in {event}"))
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn epoch_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses `YYYYMMDD` or `YYYYMMDDTHHMMSS[Z]` into (days since epoch, seconds into the day).
fn parse_stamp(text: &str) -> (i64, i64) {
    let digits = |r: std::ops::Range<usize>| -> i64 { text[r].parse().expect("digits") };
    let days = epoch_days(digits(0..4), digits(4..6), digits(6..8));
    if text.len() == 8 {
        return (days, 0);
    }
    assert_eq!(text.as_bytes()[8], b'T', "{text}");
    (
        days,
        digits(9..11) * 3600 + digits(11..13) * 60 + digits(13..15),
    )
}

/// Weekday (0 = Sunday) of an eight-digit date.
fn weekday_of(date: &str) -> usize {
    (epoch_days(
        date[0..4].parse().unwrap(),
        date[4..6].parse().unwrap(),
        date[6..8].parse().unwrap(),
    ) + 4)
        .rem_euclid(7) as usize
}

/// Every weekly event must have a well-formed DTSTART on one of its BYDAY weekdays and
/// no UNTIL before it. Returns how many weekly events were checked.
fn check_weekly_events(cal: &str) -> usize {
    let codes = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];
    let mut checked = 0;
    for e in events(cal) {
        let Some(rule) = e.lines().find(|l| l.starts_with("RRULE:")) else {
            continue;
        };
        let start = line_of(e, "DTSTART;TZID=America/New_York:")
            .rsplit(':')
            .next()
            .unwrap();
        assert_eq!(start.len(), 15, "malformed DTSTART {start}");
        assert!(
            start[..8].bytes().all(|b| b.is_ascii_digit())
                && start.as_bytes()[8] == b'T'
                && start[9..].bytes().all(|b| b.is_ascii_digit()),
            "malformed DTSTART {start}"
        );
        let byday = rule
            .split("BYDAY=")
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        assert!(
            byday.split(',').any(|d| d == codes[weekday_of(start)]),
            "DTSTART {start} is not on {byday}"
        );
        let until = rule.split("UNTIL=").nth(1).expect("UNTIL");
        assert!(until.ends_with('Z') && until.len() == 16, "{until}");
        assert!(parse_stamp(until) > parse_stamp(start), "{until} < {start}");
        checked += 1;
    }
    checked
}

fn export_one(meetings: Vec<Meeting>) -> schedule::IcsExport {
    let section = make_section("5001", "CS", "1000", "Intro", meetings, None);
    schedule::ics_export("plan-1", "S", &[section], None)
}

#[test]
fn missing_start_date_skips_instead_of_inventing_one() {
    let export = export_one(vec![weekly(&[1, 3], None, Some("2026-12-09"), 500, 565)]);
    assert_eq!(events(&export.text).len(), 0);
    assert_eq!(export.skipped_meetings, 1);
    assert_eq!(export.weekly_meetings, 0);
    assert!(!export.text.contains("20260909"));
}

#[test]
fn missing_end_date_counts_as_skipped() {
    let export = export_one(vec![weekly(&[1, 3], Some("2026-09-09"), None, 500, 565)]);
    assert_eq!(events(&export.text).len(), 0);
    assert_eq!(export.skipped_meetings, 1);
}

#[test]
fn no_weekday_inside_the_window_is_skipped() {
    // 2026-09-07 is a Monday and the window is that single day, but the class meets Tuesday.
    let one_day = export_one(vec![weekly(
        &[2],
        Some("2026-09-07"),
        Some("2026-09-07"),
        500,
        565,
    )]);
    assert_eq!(events(&one_day.text).len(), 0);
    assert_eq!(one_day.skipped_meetings, 1);
    assert!(!one_day.text.contains("DTSTART;TZID"));

    // The end date is before the start date.
    let backwards = export_one(vec![weekly(
        &[1],
        Some("2026-12-09"),
        Some("2026-09-09"),
        500,
        565,
    )]);
    assert_eq!(events(&backwards.text).len(), 0);
    assert_eq!(backwards.skipped_meetings, 1);

    // No weekdays at all.
    let none = export_one(vec![weekly(
        &[],
        Some("2026-09-07"),
        Some("2026-12-09"),
        500,
        565,
    )]);
    assert_eq!(events(&none.text).len(), 0);
}

#[test]
fn window_ending_on_the_first_matching_day_is_kept() {
    let export = export_one(vec![weekly(
        &[2],
        Some("2026-09-07"),
        Some("2026-09-08"),
        500,
        565,
    )]);
    assert_eq!(events(&export.text).len(), 1);
    assert_eq!(export.weekly_meetings, 1);
    assert_eq!(export.skipped_meetings, 0);
    assert!(export
        .text
        .contains("DTSTART;TZID=America/New_York:20260908T082000"));
    assert_eq!(check_weekly_events(&export.text), 1);
}

#[test]
fn unparseable_or_impossible_dates_never_reach_the_file() {
    let bad = [
        ("bad", "2026-12-09"),
        ("2026-09-09", "not-a-date"),
        ("2026-02-30", "2026-12-09"),
        ("2026-09-09", "2026-02-30"),
        ("2026-02-29", "2026-12-09"),
        ("2026-13-01", "2026-12-09"),
        ("2026-09-31", "2026-12-09"),
        ("2026-00-10", "2026-12-09"),
        ("20260230", "20261209"),
        ("2026-09-09 ", "2026-12-09x"),
        ("", "2026-12-09"),
    ];
    for (from, to) in bad {
        let export = export_one(vec![weekly(&[1, 3], Some(from), Some(to), 500, 565)]);
        assert_eq!(events(&export.text).len(), 0, "{from} to {to}");
        assert_eq!(export.skipped_meetings, 1, "{from} to {to}");
        assert!(!export.text.contains("bad"), "{from} to {to}");
        assert!(!export.text.contains("DTSTART;TZID"), "{from} to {to}");
    }
}

#[test]
fn real_leap_day_and_compact_dates_are_accepted() {
    // 2028-02-29 is a Tuesday.
    let leap = export_one(vec![weekly(
        &[2],
        Some("2028-02-29"),
        Some("2028-04-25"),
        500,
        565,
    )]);
    assert_eq!(events(&leap.text).len(), 1);
    assert!(leap
        .text
        .contains("DTSTART;TZID=America/New_York:20280229T082000"));
    assert_eq!(check_weekly_events(&leap.text), 1);

    let compact = export_one(vec![weekly(
        &[2],
        Some("20260908"),
        Some("20261209"),
        500,
        565,
    )]);
    assert_eq!(events(&compact.text).len(), 1);
    assert!(compact
        .text
        .contains("DTSTART;TZID=America/New_York:20260908T082000"));
}

#[test]
fn kept_and_skipped_meetings_are_counted_per_meeting() {
    let export = export_one(vec![
        weekly(&[1, 3], Some("2026-09-09"), Some("2026-12-09"), 500, 565),
        weekly(&[5], None, Some("2026-12-09"), 500, 565),
        weekly(&[4], Some("2026-02-30"), Some("2026-12-09"), 500, 565),
    ]);
    assert_eq!(events(&export.text).len(), 1);
    assert_eq!(export.weekly_meetings, 1);
    assert_eq!(export.skipped_meetings, 2);
    assert_eq!(check_weekly_events(&export.text), 1);
}

#[test]
fn every_exported_weekly_dtstart_lands_on_a_listed_weekday() {
    let export = export_one(vec![
        weekly(&[1], Some("2026-09-09"), Some("2026-12-09"), 500, 565), // Wed start, Mon class
        weekly(&[0], Some("2026-09-09"), Some("2026-12-09"), 500, 565),
        weekly(&[6, 2], Some("2026-09-11"), Some("2026-12-09"), 500, 565),
        weekly(&[3], Some("2026-09-09"), Some("2026-12-09"), 500, 565),
    ]);
    assert_eq!(events(&export.text).len(), 4);
    assert_eq!(check_weekly_events(&export.text), 4);
}

#[test]
fn final_exam_with_impossible_date_is_skipped() {
    let exam = |date: &str| Meeting {
        days: DaySet(0),
        start: Some(480),
        end: Some(600),
        kind: "Final Exam".into(),
        start_date: Some(date.into()),
        end_date: Some(date.into()),
        ..class_meeting(None)
    };
    for date in ["2026-02-30", "2026-12-32", "junk", "2026-04-31"] {
        let export = export_one(vec![exam(date)]);
        assert_eq!(events(&export.text).len(), 0, "{date}");
        assert_eq!(export.final_exams, 0, "{date}");
        assert_eq!(export.skipped_finals, 1, "{date}");
        assert!(!export.text.contains("junk"));
    }
    let good = export_one(vec![exam("2026-12-14")]);
    assert_eq!(events(&good.text).len(), 1);
    assert_eq!(good.final_exams, 1);
    assert_eq!(good.skipped_finals, 0);
}

// ---- Event text ----

fn catalog_index(course_title: &str) -> termorganize_core::index::CatalogIndex {
    use termorganize_core::model::{Course, Term, TermCatalog};
    termorganize_core::index::CatalogIndex::build(TermCatalog {
        version: 1,
        term: Term {
            code: "202710".into(),
            banner_description: "Fall 2026 Semester".into(),
        },
        generated_at: 0.0,
        sections_updated_at: 0.0,
        courses: vec![Course {
            subject: "CS".into(),
            number: "3000".into(),
            title: course_title.into(),
            description: String::new(),
            min_credits: 4.0,
            max_credits: 4.0,
            credit_hours: None,
            attributes: vec![],
            prereqs: None,
            coreqs: None,
            postreqs: vec![],
            special_topics: false,
            college: None,
            department: None,
            details_loaded: true,
        }],
        sections: vec![],
        subjects: vec![],
        attributes: vec![],
        campuses: vec![],
    })
}

fn titled_section() -> CourseSection {
    let exam = Meeting {
        days: DaySet(0),
        start: Some(480),
        end: Some(600),
        kind: "Final Exam".into(),
        start_date: Some("2026-12-14".into()),
        end_date: Some("2026-12-14".into()),
        ..class_meeting(None)
    };
    let instructors = vec![Instructor {
        name: "Ada Lovelace".into(),
        primary: true,
        email: None,
    }];
    make_section(
        "12345",
        "CS",
        "3000",
        "Section Title Differs",
        vec![class_meeting(Some("2026-12-09")), exam],
        Some(instructors),
    )
}

#[test]
fn weekly_summary_is_course_code_and_class_type() {
    let cal = schedule::ics("plan-1", "S", &[titled_section()]);
    let evs = events(&cal);
    assert_eq!(evs.len(), 2);
    assert_eq!(line_of(evs[0], "SUMMARY:"), "SUMMARY:CS 3000 Lecture");
    assert_eq!(line_of(evs[1], "SUMMARY:"), "SUMMARY:CS 3000 Final Exam");
}

#[test]
fn notes_use_the_catalog_course_title_for_weekly_and_final_events() {
    let index = catalog_index("Algorithms, Data & Design");
    let export = schedule::ics_export("plan-1", "S", &[titled_section()], Some(&index));
    let evs = events(&export.text);
    assert_eq!(evs.len(), 2);
    let notes = "Algorithms\\, Data & Design\\nCRN 12345 \u{b7} Lecture 01\\nInstructor: Ada Lovelace\\nAdded from Term Organize";
    assert_eq!(
        field(evs[0], "DESCRIPTION:"),
        format!("DESCRIPTION:{notes}")
    );
    assert_eq!(
        field(evs[1], "DESCRIPTION:"),
        format!("DESCRIPTION:Final exam\\n{notes}")
    );
    assert!(!export.text.contains("Section Title Differs"));
}

#[test]
fn notes_fall_back_to_the_section_title_without_a_catalog_title() {
    let expected = "Section Title Differs\\nCRN 12345 \u{b7} Lecture 01\\nInstructor: Ada Lovelace\\nAdded from Term Organize";
    let cal = schedule::ics("plan-1", "S", &[titled_section()]);
    let evs = events(&cal);
    assert_eq!(
        field(evs[0], "DESCRIPTION:"),
        format!("DESCRIPTION:{expected}")
    );
    assert_eq!(
        field(evs[1], "DESCRIPTION:"),
        format!("DESCRIPTION:Final exam\\n{expected}")
    );
    // An empty catalog title falls back too.
    let index = catalog_index("");
    let export = schedule::ics_export("plan-1", "S", &[titled_section()], Some(&index));
    assert!(export.text.contains("DESCRIPTION:Section Title Differs\\n"));
}

#[test]
fn calendar_and_file_names_are_plan_hyphen_term() {
    let name = schedule::calendar_name("My Plan", "Fall 2026");
    assert_eq!(name, "My Plan - Fall 2026");
    assert!(name.is_ascii());
    assert_eq!(
        schedule::export_file_name(&name, "ics"),
        "My Plan - Fall 2026.ics"
    );
    let cal = schedule::ics("plan-1", &name, &[]);
    assert!(cal.contains("X-WR-CALNAME:My Plan - Fall 2026\r\n"));
    assert_ne!(
        schedule::calendar_name("Plan", "Fall 2026"),
        schedule::calendar_name("Plan", "Spring 2027")
    );
    assert_eq!(
        schedule::export_file_name("a/b:c*d?e\"f<g>h|i\\j - Fall", "ics"),
        "a-b-c-d-e-f-g-h-i-j - Fall.ics"
    );
    assert_eq!(schedule::export_file_name("  ", "ics"), "Schedule.ics");
}

// ---- Line endings and folding ----

/// No bare CR or LF anywhere: every CR is followed by LF and every LF follows a CR.
fn assert_crlf_only(text: &str) {
    let bytes = text.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\r' {
            assert_eq!(bytes.get(i + 1), Some(&b'\n'), "bare CR at {i}");
        }
        if *b == b'\n' {
            assert!(i > 0 && bytes[i - 1] == b'\r', "bare LF at {i}");
        }
    }
}

#[test]
fn lone_cr_and_crlf_in_text_values_are_escaped() {
    let mut section = make_section(
        "6001",
        "CS",
        "1000",
        "Line\rOne\r\nTwo\nThree",
        vec![
            Meeting {
                building: Some("Hall\rA".into()),
                room: Some("1\r\n2".into()),
                ..class_meeting(Some("2026-12-09"))
            },
            Meeting {
                days: DaySet(0),
                start: Some(480),
                end: Some(600),
                kind: "Final Exam".into(),
                start_date: Some("2026-12-14".into()),
                end_date: Some("2026-12-14".into()),
                building: Some("Gym\rB".into()),
                room: Some("9".into()),
                ..class_meeting(None)
            },
        ],
        Some(vec![Instructor {
            name: "Pro\rf".into(),
            primary: true,
            email: None,
        }]),
    );
    section.class_type = "Lec\rture".into();
    let cal = schedule::ics("plan-1", "Plan\rName\r\nTwo\nThree", &[section]);
    assert_crlf_only(&cal);
    assert!(cal.contains("X-WR-CALNAME:Plan\\nName\\nTwo\\nThree\r\n"));
    assert!(cal.contains("Line\\nOne\\nTwo\\nThree"));
    assert!(cal.contains("Lec\\nture"));
    assert!(cal.contains("Pro\\nf"));
    assert_eq!(events(&cal).len(), 2);
}

/// Joins RFC 5545 folded lines back together.
fn unfold(text: &str) -> String {
    text.replace("\r\n ", "")
}

#[test]
fn folding_keeps_unicode_intact_and_octet_limited() {
    let title = "日本語の授業\u{1F4DA}\u{e9}\u{e8}".repeat(12);
    let name = format!("Plan \u{1F393} {title}");
    let section = make_section(
        "7001",
        "CS",
        "1000",
        &title,
        vec![class_meeting(Some("2026-12-09"))],
        None,
    );
    let cal = schedule::ics("plan-1", &name, &[section]);
    assert_crlf_only(&cal);
    for line in cal.split("\r\n") {
        assert!(line.len() <= 75, "{} octets: {line:?}", line.len());
        assert!(
            std::str::from_utf8(line.as_bytes()).is_ok(),
            "split inside a character"
        );
    }
    assert!(
        cal.split("\r\n").any(|l| l.starts_with(' ')),
        "nothing folded"
    );
    let flat = unfold(&cal);
    assert!(flat.contains(&format!("X-WR-CALNAME:{}", name.replace(',', "\\,"))));
    assert!(flat.contains(&format!("DESCRIPTION:{title}\\nCRN 7001")));
    // The folded bytes also decode as UTF-8 end to end.
    assert!(String::from_utf8(cal.clone().into_bytes()).is_ok());
}

// ---- DST change days ----

fn until_for(last_date: &str, days: &[i32], first: &str) -> String {
    let export = export_one(vec![weekly(days, Some(first), Some(last_date), 600, 660)]);
    assert_eq!(events(&export.text).len(), 1, "{last_date}");
    assert_eq!(check_weekly_events(&export.text), 1);
    let rule = line_of(events(&export.text)[0], "RRULE:");
    rule.split("UNTIL=").nth(1).unwrap().to_string()
}

#[test]
fn until_is_end_of_new_york_day_on_both_dst_change_days() {
    // Spring forward is Sunday 2026-03-08 at 02:00. The day itself ends in EDT (UTC-4).
    assert_eq!(
        until_for("2026-03-07", &[0], "2026-03-01"),
        "20260308T045959Z"
    );
    assert_eq!(
        until_for("2026-03-08", &[0], "2026-03-01"),
        "20260309T035959Z"
    );
    assert_eq!(
        until_for("2026-03-09", &[0], "2026-03-01"),
        "20260310T035959Z"
    );
    // Fall back is Sunday 2026-11-01 at 02:00. The day itself ends in EST (UTC-5).
    assert_eq!(
        until_for("2026-10-31", &[0], "2026-10-04"),
        "20261101T035959Z"
    );
    assert_eq!(
        until_for("2026-11-01", &[0], "2026-10-04"),
        "20261102T045959Z"
    );
    assert_eq!(
        until_for("2026-11-02", &[0], "2026-10-04"),
        "20261103T045959Z"
    );
}

// ---- Clock and event counts ----

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[test]
fn dtstamp_is_the_current_utc_time_on_every_event() {
    let exam = Meeting {
        days: DaySet(0),
        start: Some(480),
        end: Some(600),
        kind: "Final Exam".into(),
        start_date: Some("2026-12-14".into()),
        end_date: Some("2026-12-14".into()),
        ..class_meeting(None)
    };
    let section = make_section(
        "8001",
        "CS",
        "1000",
        "Clock",
        vec![class_meeting(Some("2026-12-09")), exam],
        None,
    );
    let before = now_epoch();
    let cal = schedule::ics("plan-1", "S", &[section]);
    let after = now_epoch();
    let evs = events(&cal);
    assert_eq!(evs.len(), 2);
    for e in evs {
        let stamps: Vec<&str> = e
            .lines()
            .filter_map(|l| l.strip_prefix("DTSTAMP:"))
            .collect();
        assert_eq!(stamps.len(), 1, "exactly one DTSTAMP per event");
        assert!(stamps[0].ends_with('Z'));
        let (days, secs) = parse_stamp(stamps[0]);
        let at = days * 86_400 + secs;
        assert!(
            (before..=after).contains(&at),
            "{} not within {before}..={after}",
            stamps[0]
        );
    }
}
