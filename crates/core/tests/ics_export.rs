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

    assert!(cal.contains("UID:12345-final@termorganize.local"));
    assert!(cal.contains("SUMMARY:CS 3000 Final Exam"));
    assert!(cal.contains("DTSTART;TZID=America/New_York:20261214T080000"));
    assert!(cal.contains("DTEND;TZID=America/New_York:20261214T100000"));
    assert!(cal.contains("LOCATION:Shillman Hall 105"));

    // Verify final exam event does not contain recurrence rule
    let exam_part = cal
        .split("UID:12345-final@termorganize.local")
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

    assert!(cal.contains("UID:12345-final@termorganize.local"));
    assert!(cal.contains("SUMMARY:CS 3000 Final Exam"));
    assert!(cal.contains("DTSTART;VALUE=DATE:20261215"));
    assert!(cal.contains("DTEND;VALUE=DATE:20261216"));

    let exam_part = cal
        .split("UID:12345-final@termorganize.local")
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

    assert!(cal.contains("UID:11111-final@termorganize.local"));
    assert!(cal.contains("SUMMARY:CS 2500 Final Exam"));
    assert!(cal.contains("UID:22222-final@termorganize.local"));
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

    assert!(cal.contains("UID:33333-final@termorganize.local"));
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
