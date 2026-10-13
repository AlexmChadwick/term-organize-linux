#![allow(
    clippy::field_reassign_with_default,
    clippy::cloned_ref_to_slice_refs,
    clippy::redundant_closure,
    clippy::unnecessary_get_then_check,
    non_snake_case
)]
//! Ports of CloudSyncTests, ShellSupportTests, CatalogSupportTests, BannerParserTests,
//! CoreModelTests, and SearchEngineTests.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;
use termorganize_core::archive::{self, bundled_catalog_path};
use termorganize_core::banner::{self, SubjectLookup};
use termorganize_core::html;
use termorganize_core::index::{BuildingSummary, CatalogIndex, RoomBooking, RoomSchedule};
use termorganize_core::legal;
use termorganize_core::model::*;
use termorganize_core::professors;
use termorganize_core::rooms::{self, Availability, Moment};
use termorganize_core::search::{self, HomeQuickSearch, InstructionFilter, SearchDayPreset, SearchFilters, SearchQuery};
use termorganize_core::store;
use termorganize_core::sync::{self, CloudSyncKey, Entity, IncomingRecord, Metadata, RecordState, RecordType, RemoteValue, Snapshot, SyncedPreferences, SyncPhase, UnavailableReason};
use termorganize_core::upnext::{self, WidgetMeeting, WidgetScheduleData};
use termorganize_core::watchlist::{self, AlertKind, WatchStatus};
use uuid::Uuid;

const DATE: f64 = 1_790_000_000.0;

fn cs3000() -> CourseKey {
    CourseKey { subject: "CS".into(), number: "3000".into() }
}
fn cs3500() -> CourseKey {
    CourseKey { subject: "CS".into(), number: "3500".into() }
}
fn key(subject: &str, number: &str) -> CourseKey {
    CourseKey { subject: subject.into(), number: number.into() }
}

fn schedule_plan(name: &str, id: Uuid, term_code: &str, is_primary: bool, updated_at: f64) -> SchedulePlan {
    SchedulePlan {
        id,
        name: name.into(),
        term_code: term_code.into(),
        courses: vec![PlanCourse {
            key: cs3000(),
            is_required: true,
            excluded_crns: ["10001", "10002", "10003"].into_iter().map(str::to_string).collect(),
            color_index: 0,
        }],
        selected_crns: vec!["10004".into()],
        filters: ScheduleFilters::default(),
        preferences: None,
        favorite_combinations: vec![],
        is_primary,
        created_at: DATE,
        updated_at,
    }
}

fn degree_plan(id: Uuid, updated_at: f64) -> DegreePlan {
    let mut term = DegreePlanTerm::new(PlanSeason::Spring, 2027);
    term.courses = vec![DegreePlanCourse::catalog(cs3500(), "OOD", 4.0)];
    let mut plan = DegreePlan::default();
    plan.id = id;
    plan.name = "BSCS".into();
    plan.program = Some("CS-B-CSCI-BSCS".into());
    plan.student_name = Some("SAMPLE STUDENT".into());
    plan.student_id = Some("000000000".into());
    plan.completed = vec![DegreePlanCourse::catalog(cs3000(), "Algorithms and Data", 4.0)];
    plan.terms = vec![term];
    plan.is_primary = true;
    plan.created_at = Some(sync::unix_to_iso8601(DATE));
    plan.updated_at = Some(sync::unix_to_iso8601(updated_at));
    plan
}

fn watch_item(crn: &str, added_at: f64) -> WatchItem {
    WatchItem {
        term_code: "202710".into(),
        crn: crn.into(),
        course_key: cs3000(),
        course_title: "Algorithms and Data".into(),
        section_label: "Lecture · Section 01".into(),
        added_at,
        last_seats_remaining: 0,
        last_waitlist_remaining: 2,
        seat_capacity: 40,
        last_checked_at: Some(DATE),
        notify_on_waitlist: false,
        last_notified_at: None,
    }
}

fn saved_course(course: CourseKey, saved_at: f64) -> SavedCourse {
    SavedCourse { term_code: "202710".into(), key: course, saved_at }
}

fn synced_metadata(data: &UserData) -> Metadata {
    let snapshot = Snapshot::from_data(data);
    let mut metadata = Metadata::default();
    metadata.observe_local_data(&snapshot, DATE);
    for (key, item) in &snapshot.items {
        metadata.record_saved(key, Some(&item.fingerprint), Some(&[1]));
    }
    metadata
}

fn incoming(entity: &Entity, modified_at: f64, schema_version: i32, preference_changed_at: &BTreeMap<String, f64>) -> IncomingRecord {
    IncomingRecord {
        key: entity.key(),
        schema_version,
        modified_at,
        payload: Some(sync::payload_for(entity, preference_changed_at).unwrap()),
        system_fields: Some(vec![2]),
    }
}

fn empty_times() -> BTreeMap<String, f64> {
    BTreeMap::new()
}

/// Applies a newer remote rename with no entitlement blob. Linux stand-in shared by the CloudKit checks.
fn renamed_without_entitlement() -> String {
    let original = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![original.clone()];
    let mut metadata = synced_metadata(&data);
    let mut remote = original;
    remote.name = "Renamed elsewhere".into();
    remote.updated_at = DATE + 60.0;
    let mut record = incoming(&Entity::SchedulePlan(remote), DATE + 60.0, sync::SCHEMA_VERSION, &empty_times());
    record.system_fields = None;
    let key = record.key.clone();
    let result = sync::merge(&[record], &[], &Snapshot::from_data(&data), &mut metadata, DATE + 120.0);
    assert!(metadata.records.get(&key).unwrap().system_fields.is_none());
    let Entity::SchedulePlan(plan) = &result.saves[0] else { panic!("expected a schedule plan") };
    plan.name.clone()
}

fn bundled_index() -> &'static CatalogIndex {
    static INDEX: OnceLock<CatalogIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        let path = bundled_catalog_path().expect("bundled catalog");
        CatalogIndex::build(archive::read(&path).expect("catalog"))
    })
}

fn top_codes(text: &str, filters: SearchFilters) -> Vec<String> {
    search::search(&SearchQuery { text: text.into(), filters, sort: search::SearchSort::Relevance }, bundled_index())
        .results
        .into_iter()
        .take(5)
        .map(|result| result.course.code())
        .collect()
}

fn shell_section(crn: &str, seats: i32, waitlist: i32, capacity: i32) -> CourseSection {
    CourseSection {
        crn: crn.into(),
        subject: "CS".into(),
        number: "3000".into(),
        title: "Algorithms and Data".into(),
        section_number: "02".into(),
        part_of_term: "1".into(),
        seat_capacity: capacity,
        seats_remaining: seats,
        enrollment: capacity - seats,
        waitlist_capacity: 10,
        waitlist_count: 10 - waitlist,
        waitlist_remaining: waitlist,
        class_type: "Lecture".into(),
        honors: false,
        campus: "Boston".into(),
        instruction_method: Some("Traditional".into()),
        meetings: vec![],
        instructors: Some(vec![]),
        cross_list_crns: vec![],
        attributes: vec![],
        description: None,
        prereqs: None,
        coreqs: None,
    }
}

fn shell_watch(crn: &str, seats: i32, waitlist: i32, capacity: i32, notify: bool, term_code: &str) -> WatchItem {
    WatchItem {
        term_code: term_code.into(),
        crn: crn.into(),
        course_key: cs3000(),
        course_title: "Algorithms and Data".into(),
        section_label: "Lecture · Section 02".into(),
        added_at: 0.0,
        last_seats_remaining: seats,
        last_waitlist_remaining: waitlist,
        seat_capacity: capacity,
        last_checked_at: None,
        notify_on_waitlist: notify,
        last_notified_at: None,
    }
}

fn booking(code: &str, start: i32, end: i32, days: &[i32], from: Option<&str>, to: Option<&str>) -> RoomBooking {
    RoomBooking {
        crn: format!("1{start}"),
        course: CourseKey::parse(code).unwrap_or_else(|| cs3000()),
        days: DaySet::from_indices(days.iter().copied()),
        start,
        end,
        start_date: from.map(str::to_string),
        end_date: to.map(str::to_string),
    }
}

fn room(bookings: Vec<RoomBooking>, number: &str, building: &str) -> RoomSchedule {
    RoomSchedule {
        building_code: building.into(),
        building_name: "West Village H".into(),
        room: number.into(),
        bookings,
    }
}

fn room_status(bookings: Vec<RoomBooking>, minutes: i32, weekday: i32) -> Availability {
    rooms::status_of(&room(bookings, "210", "WVH"), Moment::new(weekday, minutes))
}

fn banner_subjects() -> SubjectLookup {
    SubjectLookup::new(&[
        Subject { code: "CS".into(), name: "Computer Science".into() },
        Subject { code: "DS".into(), name: "Data Science".into() },
        Subject { code: "MATH".into(), name: "Mathematics".into() },
        Subject { code: "EECE".into(), name: "Electrical and Comp Engineerng".into() },
    ])
}

fn preq_row(and_or: &str, open: bool, test: &str, score: &str, subject: &str, number: &str, close: bool) -> String {
    format!(
        "<tr><td>{and_or}</td><td>{}</td><td>{test}</td><td>{score}</td><td>{subject}</td><td>{number}</td><td>Undergraduate</td><td>D-</td><td>{}</td></tr>",
        if open { "(" } else { "" },
        if close { ")" } else { "" }
    )
}

fn preq_table(rows: &[String]) -> String {
    format!("<section><table class=\"basePreqTable\"><thead><tr><th>And/Or</th></tr></thead><tbody>{}</tbody></table></section>", rows.join(""))
}

// MARK: Cloud sync

#[test]
fn payloads_round_trip() {
    let mut preferences = AppPreferences::default();
    preferences.preferred_campus = Some("Boston".into());
    preferences.calendar_start_hour = 9;
    let schedule = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let degree = degree_plan(Uuid::new_v4(), DATE);
    let watch = watch_item("12345", DATE);
    let course = saved_course(cs3000(), DATE);
    let mut changed_at = BTreeMap::new();
    changed_at.insert("preferredCampus".into(), DATE);

    let schedule_payload = sync::payload_for(&Entity::SchedulePlan(schedule.clone()), &empty_times()).unwrap();
    assert_eq!(sync::decode(&schedule_payload, RecordType::SchedulePlan).unwrap(), RemoteValue::SchedulePlan(schedule.clone()));
    let degree_payload = sync::payload_for(&Entity::DegreePlan(degree.clone()), &empty_times()).unwrap();
    assert_eq!(sync::decode(&degree_payload, RecordType::DegreePlan).unwrap(), RemoteValue::DegreePlan(sync::synced_form(&degree)));
    let watch_payload = sync::payload_for(&Entity::WatchItem(watch.clone()), &empty_times()).unwrap();
    assert_eq!(sync::decode(&watch_payload, RecordType::WatchItem).unwrap(), RemoteValue::WatchItem(watch));
    let saved_payload = sync::payload_for(&Entity::SavedCourse(course.clone()), &empty_times()).unwrap();
    assert_eq!(sync::decode(&saved_payload, RecordType::SavedCourse).unwrap(), RemoteValue::SavedCourse(course));
    let preferences_payload = sync::payload_for(&Entity::Preferences(preferences.clone()), &changed_at).unwrap();
    assert_eq!(
        sync::decode(&preferences_payload, RecordType::Preferences).unwrap(),
        RemoteValue::Preferences { values: SyncedPreferences::from_app(&preferences), changed_at: changed_at.clone() }
    );

    let mut upper = serde_json::to_value(&schedule).unwrap();
    upper["id"] = serde_json::json!(schedule.id.to_string().to_ascii_uppercase());
    let decoded: SchedulePlan = serde_json::from_value(upper).unwrap();
    assert_eq!(decoded.id, schedule.id);
    assert_eq!(CloudSyncKey::new(RecordType::SchedulePlan, schedule.id.to_string().to_ascii_uppercase()), Entity::SchedulePlan(schedule).key());
}

#[test]
fn payloads_from_newer_versions_with_extra_fields_still_decode() {
    let watch = watch_item("12345", DATE);
    let payload = sync::payload_for(&Entity::WatchItem(watch.clone()), &empty_times()).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    json["fieldFromTheFuture"] = serde_json::json!({"nested": true});
    let encoded = serde_json::to_vec(&json).unwrap();
    assert_eq!(sync::decode(&encoded, RecordType::WatchItem).unwrap(), RemoteValue::WatchItem(watch));
}

#[test]
fn record_keys_use_stable_ids() {
    let schedule = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    assert_eq!(Entity::SchedulePlan(schedule.clone()).key(), CloudSyncKey::new(RecordType::SchedulePlan, schedule.id.to_string()));
    assert_eq!(Entity::SavedCourse(saved_course(cs3000(), DATE)).key().record_name, "202710|CS 3000");
    assert_eq!(Entity::WatchItem(watch_item("12345", DATE)).key().record_name, "202710|12345");
    assert_eq!(Entity::Preferences(AppPreferences::default()).key(), CloudSyncKey::preferences());

    for unusable in ["", "_private", "Café", &"x".repeat(256), "tab\there"] {
        let name = CloudSyncKey::record_name_for_id(unusable);
        assert!(name.starts_with("sha256-"), "{unusable}");
        assert!(name.len() <= 255);
        assert_eq!(name, CloudSyncKey::record_name_for_id(unusable));
    }
    assert_ne!(CloudSyncKey::record_name_for_id("a"), CloudSyncKey::record_name_for_id("_a"));
}

/// Linux stand-in for the CloudKit entitlement check.
#[test]
fn container_identifier_comes_from_info_plist() {
    assert!(sync::container_identifier().is_none());
    assert!(!sync::consults_container_identifier());
    assert!(!sync::consults_code_signature());
    let payload = sync::payload_for(&Entity::SchedulePlan(schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE)), &empty_times()).unwrap();
    let text = String::from_utf8(payload).unwrap();
    assert!(!text.contains("iCloud."));
    assert_eq!(renamed_without_entitlement(), "Renamed elsewhere");
}

#[test]
fn device_preferences_are_not_synced() {
    let mut preferences = AppPreferences::default();
    preferences.selected_term_code = Some("202730".into());
    preferences.has_completed_onboarding = true;
    preferences.auto_refresh_seats = false;
    preferences.background_refresh_enabled = false;
    preferences.preferred_campus = Some("Boston".into());
    let payload = sync::payload_for(&Entity::Preferences(preferences.clone()), &empty_times()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    let values = json["values"].as_object().unwrap();
    let keys: BTreeSet<_> = values.keys().cloned().collect();
    assert_eq!(keys, ["preferredCampus", "showWeekends", "calendarStartHour", "calendarEndHour"].into_iter().map(str::to_string).collect());
    let text = String::from_utf8(payload).unwrap();
    for device_only in ["selectedTermCode", "hasCompletedOnboarding", "autoRefreshSeats", "backgroundRefreshEnabled", "202730"] {
        assert!(!text.contains(device_only), "{device_only}");
    }
    let mut other = preferences.clone();
    other.selected_term_code = Some("202710".into());
    other.has_completed_onboarding = false;
    other.auto_refresh_seats = true;
    other.background_refresh_enabled = true;
    assert_eq!(sync::fingerprint_of(&Entity::Preferences(other)), sync::fingerprint_of(&Entity::Preferences(preferences.clone())));
    let mut remote = SyncedPreferences::from_app(&AppPreferences::default());
    remote.preferred_campus = Some("Oakland".into());
    remote.show_weekends = true;
    let applied = remote.applied(&preferences);
    assert_eq!(applied.preferred_campus.as_deref(), Some("Oakland"));
    assert!(applied.show_weekends);
    assert_eq!(applied.selected_term_code.as_deref(), Some("202730"));
    assert!(applied.has_completed_onboarding);
    assert!(!applied.auto_refresh_seats);
    assert!(!applied.background_refresh_enabled);
}

#[test]
fn remote_preferences_merge_keeps_device_only_fields() {
    let mut data = UserData::default();
    data.preferences.selected_term_code = Some("202730".into());
    data.preferences.has_completed_onboarding = true;
    data.preferences.background_refresh_enabled = false;
    let mut metadata = synced_metadata(&data);
    let mut remote = AppPreferences::default();
    remote.preferred_campus = Some("Oakland".into());
    let mut changed = BTreeMap::new();
    changed.insert("preferredCampus".into(), DATE + 60.0);
    let record = incoming(&Entity::Preferences(remote), DATE + 60.0, sync::SCHEMA_VERSION, &changed);
    let result = sync::merge(&[record], &[], &Snapshot::from_data(&data), &mut metadata, DATE);
    let Entity::Preferences(merged) = &result.saves[0] else { panic!("expected preferences") };
    assert_eq!(merged.preferred_campus.as_deref(), Some("Oakland"));
    assert_eq!(merged.selected_term_code.as_deref(), Some("202730"));
    assert!(merged.has_completed_onboarding);
    assert!(!merged.background_refresh_enabled);
}

#[test]
fn degree_plan_student_details_stay_on_device() {
    let plan = degree_plan(Uuid::new_v4(), DATE);
    let text = String::from_utf8(sync::payload_for(&Entity::DegreePlan(plan.clone()), &empty_times()).unwrap()).unwrap();
    assert!(!text.contains("SAMPLE STUDENT"));
    assert!(!text.contains("000000000"));
    assert!(!text.contains("studentName"));
    let mut without_student = plan.clone();
    without_student.student_name = None;
    without_student.student_id = None;
    assert_eq!(sync::fingerprint_of(&Entity::DegreePlan(without_student)), sync::fingerprint_of(&Entity::DegreePlan(plan.clone())));
    let mut remote = sync::synced_form(&plan);
    remote.name = "BSCS (updated)".into();
    let Entity::DegreePlan(result) = sync::local_value(&RemoteValue::DegreePlan(remote), Some(&Entity::DegreePlan(plan))) else {
        panic!("expected a degree plan");
    };
    assert_eq!(result.name, "BSCS (updated)");
    assert_eq!(result.student_name.as_deref(), Some("SAMPLE STUDENT"));
    assert_eq!(result.student_id.as_deref(), Some("000000000"));
}

#[test]
fn watch_item_seat_checks_are_device_only() {
    let item = watch_item("12345", DATE);
    let mut checked = item.clone();
    checked.last_seats_remaining = 3;
    checked.last_waitlist_remaining = 0;
    checked.seat_capacity = 45;
    checked.last_checked_at = Some(DATE + 600.0);
    checked.last_notified_at = Some(DATE + 600.0);
    assert_eq!(sync::fingerprint_of(&Entity::WatchItem(checked.clone())), sync::fingerprint_of(&Entity::WatchItem(item.clone())));
    let mut intent = item.clone();
    intent.notify_on_waitlist = true;
    assert_ne!(sync::fingerprint_of(&Entity::WatchItem(intent)), sync::fingerprint_of(&Entity::WatchItem(item.clone())));
    let mut remote = item;
    remote.notify_on_waitlist = true;
    remote.last_seats_remaining = 0;
    remote.last_checked_at = Some(DATE - 600.0);
    let Entity::WatchItem(merged) = sync::local_value(&RemoteValue::WatchItem(remote), Some(&Entity::WatchItem(checked))) else {
        panic!("expected a watch item");
    };
    assert!(merged.notify_on_waitlist);
    assert_eq!(merged.last_seats_remaining, 3);
    assert_eq!(merged.seat_capacity, 45);
    assert_eq!(merged.last_checked_at, Some(DATE + 600.0));
    assert_eq!(merged.last_notified_at, Some(DATE + 600.0));
}

#[test]
fn fingerprints_ignore_set_order() {
    let mut first = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    first.courses[0].excluded_crns.clear();
    let mut second = first.clone();
    for n in 1..=40 {
        first.courses[0].excluded_crns.insert(format!("{}", 20_000 + n));
    }
    for n in (1..=40).rev() {
        second.courses[0].excluded_crns.insert(format!("{}", 20_000 + n));
    }
    assert_eq!(first, second);
    let left = sync::payload_for(&Entity::SchedulePlan(first.clone()), &empty_times()).unwrap();
    let right = sync::payload_for(&Entity::SchedulePlan(second), &empty_times()).unwrap();
    assert_eq!(left, right);
    assert_eq!(sync::fingerprint_of(&Entity::SchedulePlan(first.clone())), sync::fingerprint_of(&Entity::SchedulePlan(first.clone())));
    let json: serde_json::Value = serde_json::from_slice(&left).unwrap();
    let excluded = json["courses"][0]["excludedCRNs"].as_array().unwrap();
    let values: Vec<&str> = excluded.iter().map(|item| item.as_str().unwrap()).collect();
    assert_eq!(values.len(), 40);
    let mut sorted = values.clone();
    sorted.sort();
    assert_eq!(values, sorted);
}

#[test]
fn fingerprints_survive_the_local_file() {
    let mut data = UserData::default();
    data.plans = vec![schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE + 0.75)];
    data.saved = vec![saved_course(cs3000(), DATE + 0.25)];
    data.watchlist = vec![watch_item("12345", DATE + 0.5)];
    let reloaded = store::round_trip_user_file(&data).unwrap();
    let before = Snapshot::from_data(&data);
    let after = Snapshot::from_data(&reloaded);
    assert_eq!(before.items.keys().collect::<Vec<_>>(), after.items.keys().collect::<Vec<_>>());
    for (key, item) in &before.items {
        assert_eq!(after.get(key).map(|found| found.fingerprint.as_str()), Some(item.fingerprint.as_str()), "{key:?}");
    }
}

#[test]
fn diff_finds_adds_changes_and_deletes() {
    let schedule = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let watch = watch_item("12345", DATE);
    let course = saved_course(cs3000(), DATE);
    let mut data = UserData::default();
    data.plans = vec![schedule];
    data.watchlist = vec![watch.clone()];
    data.saved = vec![course.clone()];
    let mut metadata = synced_metadata(&data);
    assert!(metadata.pending_changes(&Snapshot::from_data(&data)).is_empty());
    let added = schedule_plan("Plan B", Uuid::new_v4(), "202710", false, DATE);
    data.plans.push(added.clone());
    data.watchlist[0].notify_on_waitlist = true;
    data.watchlist[0].last_seats_remaining = 7;
    data.saved.clear();
    let snapshot = Snapshot::from_data(&data);
    metadata.observe_local_data(&snapshot, DATE + 10.0);
    let changes = metadata.pending_changes(&snapshot);
    let saves: HashSet<_> = changes.saves.iter().cloned().collect();
    let expected: HashSet<_> = [Entity::SchedulePlan(added).key(), Entity::WatchItem(watch).key()].into_iter().collect();
    assert_eq!(saves, expected);
    assert_eq!(changes.deletes, vec![Entity::SavedCourse(course.clone()).key()]);
    for key in &changes.saves {
        metadata.record_saved(key, snapshot.get(key).map(|item| item.fingerprint.as_str()), Some(&[3]));
    }
    for key in changes.deletes.clone() {
        metadata.record_deleted_from_icloud(&key);
    }
    assert!(metadata.pending_changes(&snapshot).is_empty());
    assert!(metadata.records.get(&Entity::SavedCourse(course).key()).is_none());
}

#[test]
fn fresh_device_uploads_everything() {
    let mut data = UserData::default();
    data.plans = vec![schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE)];
    data.saved = vec![saved_course(cs3000(), DATE)];
    let mut metadata = Metadata::default();
    let snapshot = Snapshot::from_data(&data);
    metadata.observe_local_data(&snapshot, DATE);
    let changes = metadata.pending_changes(&snapshot);
    assert_eq!(changes.saves.iter().cloned().collect::<HashSet<_>>(), snapshot.items.keys().cloned().collect());
    assert!(changes.deletes.is_empty());
    assert!(changes.saves.contains(&CloudSyncKey::preferences()));
}

#[test]
fn journal_dates_local_changes() {
    let mut data = UserData::default();
    data.plans = vec![schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE)];
    let plan_key = Entity::SchedulePlan(data.plans[0].clone()).key();
    let mut metadata = Metadata::default();
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE);
    assert!(metadata.records.get(&plan_key).unwrap().local_changed_at.is_none());
    assert!(metadata.preference_changed_at.is_empty());
    data.plans[0].is_primary = true;
    data.preferences.show_weekends = true;
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 300.0);
    assert_eq!(metadata.records.get(&plan_key).unwrap().local_changed_at, Some(DATE + 300.0));
    assert_eq!(metadata.preference_changed_at.get("showWeekends").copied(), Some(DATE + 300.0));
    assert_eq!(sync::modified_at(&Entity::SchedulePlan(data.plans[0].clone()), Some(DATE + 300.0), &empty_times()), DATE + 300.0);
    assert_eq!(sync::modified_at(&Entity::Preferences(data.preferences.clone()), None, &metadata.preference_changed_at), DATE + 300.0);
    data.plans.clear();
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 600.0);
    assert!(metadata.records.get(&plan_key).is_none());
}

#[test]
fn modified_at_rules() {
    let plan = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE + 100.0);
    assert_eq!(sync::modified_at(&Entity::SchedulePlan(plan.clone()), None, &empty_times()), DATE + 100.0);
    assert_eq!(sync::modified_at(&Entity::SchedulePlan(plan), Some(DATE + 50.0), &empty_times()), DATE + 100.0);
    assert_eq!(sync::modified_at(&Entity::WatchItem(watch_item("12345", DATE)), None, &empty_times()), DATE);
    assert_eq!(sync::modified_at(&Entity::SavedCourse(saved_course(cs3000(), DATE)), Some(DATE + 9.0), &empty_times()), DATE + 9.0);
    assert_eq!(sync::modified_at(&Entity::Preferences(AppPreferences::default()), None, &empty_times()), sync::DISTANT_PAST);
}

#[test]
fn turning_sync_off_forgets_icloud_but_not_local_history() {
    let mut data = UserData::default();
    data.plans = vec![schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE)];
    data.saved = vec![saved_course(cs3000(), DATE)];
    let mut metadata = synced_metadata(&data);
    data.saved.clear();
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 5.0);
    metadata.is_zone_saved = true;
    metadata.last_sync_date = Some(DATE);
    metadata.forget_icloud();
    assert!(!metadata.has_data_in_icloud());
    assert!(!metadata.is_zone_saved);
    assert!(metadata.last_sync_date.is_none());
    let plan_state = metadata.records.get(&Entity::SchedulePlan(data.plans[0].clone()).key()).unwrap();
    assert!(plan_state.local_fingerprint.is_some());
    assert!(plan_state.system_fields.is_none());
    assert!(metadata.pending_changes(&Snapshot::from_data(&data)).deletes.is_empty());
}

#[test]
fn metadata_round_trips() {
    let mut data = UserData::default();
    data.plans = vec![schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE)];
    data.preferences.preferred_campus = Some("Boston".into());
    let mut metadata = synced_metadata(&data);
    metadata.account_record_name = Some("_user".into());
    metadata.preference_changed_at.insert("preferredCampus".into(), DATE);
    metadata.records.insert(CloudSyncKey::new(RecordType::WatchItem, "202710|1"), RecordState { is_server_copy_unreadable: true, ..RecordState::default() });
    let encoded = serde_json::to_vec(&metadata).unwrap();
    let decoded: Metadata = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, metadata);
}

#[test]
fn remote_changes_apply_without_echo() {
    let original = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![original.clone()];
    let mut metadata = synced_metadata(&data);
    let mut remote = original;
    remote.name = "Renamed elsewhere".into();
    remote.updated_at = DATE + 60.0;
    let result = sync::merge(
        &[incoming(&Entity::SchedulePlan(remote.clone()), DATE + 60.0, sync::SCHEMA_VERSION, &empty_times())],
        &[],
        &Snapshot::from_data(&data),
        &mut metadata,
        DATE + 120.0,
    );
    assert_eq!(result.saves, vec![Entity::SchedulePlan(remote.clone())]);
    assert!(result.kept_local.is_empty());
    let key = Entity::SchedulePlan(remote).key();
    assert_eq!(metadata.records.get(&key).unwrap().system_fields.as_deref(), Some(&[2][..]));
    assert_eq!(metadata.records.get(&key).unwrap().local_changed_at, Some(DATE + 60.0));
    data = sync::applying_cloud_changes(&data, &result.saves, &result.deletes);
    assert_eq!(data.plans.iter().map(|plan| plan.name.as_str()).collect::<Vec<_>>(), vec!["Renamed elsewhere"]);
    let after = Snapshot::from_data(&data);
    metadata.observe_local_data(&after, DATE + 180.0);
    assert!(metadata.pending_changes(&after).is_empty());
    assert_eq!(metadata.records.get(&key).unwrap().local_changed_at, Some(DATE + 60.0));
}

#[test]
fn identical_remote_copy_changes_nothing() {
    let plan = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![plan.clone()];
    let mut metadata = Metadata::default();
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE);
    let result = sync::merge(
        &[incoming(&Entity::SchedulePlan(plan.clone()), DATE, sync::SCHEMA_VERSION, &empty_times())],
        &[],
        &Snapshot::from_data(&data),
        &mut metadata,
        DATE,
    );
    assert!(!result.changes_local_data());
    assert!(!metadata.pending_changes(&Snapshot::from_data(&data)).saves.contains(&Entity::SchedulePlan(plan).key()));
}

#[test]
fn newer_local_change_wins_conflict() {
    let original = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![original.clone()];
    let mut metadata = synced_metadata(&data);
    data.plans[0].name = "Edited here".into();
    data.plans[0].updated_at = DATE + 100.0;
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 100.0);
    let mut remote = original.clone();
    remote.name = "Edited elsewhere".into();
    remote.updated_at = DATE + 50.0;
    let key = Entity::SchedulePlan(original).key();
    let result = sync::merge(
        &[incoming(&Entity::SchedulePlan(remote), DATE + 50.0, sync::SCHEMA_VERSION, &empty_times())],
        &[],
        &Snapshot::from_data(&data),
        &mut metadata,
        DATE,
    );
    assert!(result.saves.is_empty());
    assert_eq!(result.kept_local, vec![key.clone()]);
    assert_eq!(metadata.records.get(&key).unwrap().system_fields.as_deref(), Some(&[2][..]));
    assert_eq!(metadata.pending_changes(&Snapshot::from_data(&data)).saves, vec![key]);
}

#[test]
fn newer_remote_change_wins_conflict() {
    let original = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![original.clone()];
    let mut metadata = synced_metadata(&data);
    data.plans[0].name = "Edited here".into();
    data.plans[0].updated_at = DATE + 50.0;
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 50.0);
    let mut remote = original;
    remote.name = "Edited elsewhere".into();
    remote.updated_at = DATE + 100.0;
    let result = sync::merge(
        &[incoming(&Entity::SchedulePlan(remote.clone()), DATE + 100.0, sync::SCHEMA_VERSION, &empty_times())],
        &[],
        &Snapshot::from_data(&data),
        &mut metadata,
        DATE,
    );
    assert_eq!(result.saves, vec![Entity::SchedulePlan(remote)]);
    assert!(result.kept_local.is_empty());
}

#[test]
fn ties_go_to_icloud() {
    let original = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![original.clone()];
    let mut metadata = synced_metadata(&data);
    data.plans[0].name = "Edited here".into();
    data.plans[0].updated_at = DATE + 50.0;
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 50.0);
    let mut remote = original;
    remote.name = "Edited elsewhere".into();
    remote.updated_at = DATE + 50.0;
    let result = sync::merge(
        &[incoming(&Entity::SchedulePlan(remote.clone()), DATE + 50.0, sync::SCHEMA_VERSION, &empty_times())],
        &[],
        &Snapshot::from_data(&data),
        &mut metadata,
        DATE,
    );
    assert_eq!(result.saves, vec![Entity::SchedulePlan(remote)]);
}

#[test]
fn remote_deletion_removes_unchanged_values() {
    let course = saved_course(cs3000(), DATE);
    let mut data = UserData::default();
    data.saved = vec![course.clone()];
    let mut metadata = synced_metadata(&data);
    let key = Entity::SavedCourse(course).key();
    let result = sync::merge(&[], &[key.clone()], &Snapshot::from_data(&data), &mut metadata, DATE);
    assert_eq!(result.deletes, vec![key.clone()]);
    assert!(metadata.records.get(&key).is_none());
    data = sync::applying_cloud_changes(&data, &result.saves, &result.deletes);
    assert!(data.saved.is_empty());
    assert!(metadata.pending_changes(&Snapshot::from_data(&data)).is_empty());
}

#[test]
fn remote_deletion_keeps_locally_edited_values() {
    let item = watch_item("12345", DATE);
    let mut data = UserData::default();
    data.watchlist = vec![item.clone()];
    let mut metadata = synced_metadata(&data);
    data.watchlist[0].notify_on_waitlist = true;
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 30.0);
    let key = Entity::WatchItem(item).key();
    let result = sync::merge(&[], &[key.clone()], &Snapshot::from_data(&data), &mut metadata, DATE);
    assert!(result.deletes.is_empty());
    assert_eq!(result.kept_local, vec![key.clone()]);
    assert!(metadata.records.get(&key).unwrap().system_fields.is_none());
    assert_eq!(metadata.pending_changes(&Snapshot::from_data(&data)).saves, vec![key]);
}

#[test]
fn local_deletion_newer_than_remote_edit_stands() {
    let original = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![original.clone()];
    let mut metadata = synced_metadata(&data);
    data.plans.clear();
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 100.0);
    let key = Entity::SchedulePlan(original.clone()).key();
    let mut older = original.clone();
    older.name = "Edited elsewhere, earlier".into();
    let mut newer = original;
    newer.name = "Edited elsewhere, later".into();
    newer.updated_at = DATE + 200.0;
    let mut keep_deleted = metadata.clone();
    let stands = sync::merge(
        &[incoming(&Entity::SchedulePlan(older), DATE + 50.0, sync::SCHEMA_VERSION, &empty_times())],
        &[],
        &Snapshot::from_data(&data),
        &mut keep_deleted,
        DATE,
    );
    assert!(stands.saves.is_empty());
    assert_eq!(keep_deleted.pending_changes(&Snapshot::from_data(&data)).deletes, vec![key]);
    let resurrected = sync::merge(
        &[incoming(&Entity::SchedulePlan(newer.clone()), DATE + 200.0, sync::SCHEMA_VERSION, &empty_times())],
        &[],
        &Snapshot::from_data(&data),
        &mut metadata,
        DATE,
    );
    assert_eq!(resurrected.saves, vec![Entity::SchedulePlan(newer)]);
}

#[test]
fn records_from_newer_versions_are_left_alone() {
    let plan = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![plan.clone()];
    let mut metadata = synced_metadata(&data);
    let key = Entity::SchedulePlan(plan).key();
    let mut newer = data.plans[0].clone();
    newer.name = "Written by version 2".into();
    let mut record = incoming(&Entity::SchedulePlan(newer), DATE + 60.0, sync::SCHEMA_VERSION + 1, &empty_times());
    record.system_fields = Some(vec![9]);
    let result = sync::merge(&[record], &[], &Snapshot::from_data(&data), &mut metadata, DATE);
    assert_eq!(result.skipped, vec![key.clone()]);
    assert!(!result.changes_local_data());
    assert!(metadata.records.get(&key).unwrap().is_server_copy_unreadable);
    data.plans[0].name = "Edited here".into();
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 120.0);
    assert!(metadata.pending_changes(&Snapshot::from_data(&data)).is_empty());
    data.plans.clear();
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 180.0);
    assert!(metadata.pending_changes(&Snapshot::from_data(&data)).is_empty());
}

#[test]
fn unreadable_payloads_are_skipped_not_deleted() {
    let plan = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![plan.clone()];
    let mut metadata = synced_metadata(&data);
    let key = Entity::SchedulePlan(plan.clone()).key();
    let garbage = IncomingRecord {
        key: key.clone(),
        schema_version: 1,
        modified_at: DATE + 60.0,
        payload: Some(b"{ not json".to_vec()),
        system_fields: None,
    };
    let empty = IncomingRecord {
        key: CloudSyncKey::new(RecordType::SavedCourse, "202710|CS 9999"),
        schema_version: 1,
        modified_at: DATE,
        payload: None,
        system_fields: None,
    };
    let mut mismatched = incoming(&Entity::SchedulePlan(schedule_plan("Other", Uuid::new_v4(), "202710", false, DATE)), DATE + 60.0, 1, &empty_times());
    mismatched.key = CloudSyncKey::new(RecordType::SchedulePlan, Uuid::new_v4().to_string());
    let result = sync::merge(&[garbage, empty.clone(), mismatched.clone()], &[], &Snapshot::from_data(&data), &mut metadata, DATE);
    let skipped: HashSet<_> = result.skipped.iter().cloned().collect();
    assert_eq!(skipped, [key, empty.key, mismatched.key].into_iter().collect());
    assert!(!result.changes_local_data());
    assert_eq!(sync::applying_cloud_changes(&data, &result.saves, &result.deletes).plans, vec![plan]);
}

#[test]
fn primary_plan_conflict_keeps_one_primary() {
    let first = schedule_plan("First", Uuid::new_v4(), "202710", true, DATE);
    let second = schedule_plan("Second", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.plans = vec![first.clone(), second.clone()];
    let mut metadata = synced_metadata(&data);
    let mut remote = second;
    remote.is_primary = true;
    let result = sync::merge(
        &[incoming(&Entity::SchedulePlan(remote), DATE + 60.0, sync::SCHEMA_VERSION, &empty_times())],
        &[],
        &Snapshot::from_data(&data),
        &mut metadata,
        DATE + 90.0,
    );
    data = sync::applying_cloud_changes(&data, &result.saves, &result.deletes);
    assert_eq!(data.plans.iter().filter(|plan| plan.is_primary).map(|plan| plan.name.as_str()).collect::<Vec<_>>(), vec!["Second"]);
    let first_key = Entity::SchedulePlan(first).key();
    assert_eq!(metadata.records.get(&first_key).unwrap().local_changed_at, Some(DATE + 90.0));
    assert_eq!(metadata.pending_changes(&Snapshot::from_data(&data)).saves, vec![first_key]);
}

#[test]
fn degree_plan_primary_conflict_keeps_one_primary() {
    let mut mine = degree_plan(Uuid::new_v4(), DATE);
    mine.name = "Mine".into();
    let mut data = UserData::default();
    data.degree_plans = vec![mine];
    let mut metadata = synced_metadata(&data);
    let mut theirs = sync::synced_form(&degree_plan(Uuid::new_v4(), DATE + 60.0));
    theirs.name = "Theirs".into();
    let result = sync::merge(
        &[incoming(&Entity::DegreePlan(theirs), DATE + 60.0, sync::SCHEMA_VERSION, &empty_times())],
        &[],
        &Snapshot::from_data(&data),
        &mut metadata,
        DATE + 90.0,
    );
    data = sync::applying_cloud_changes(&data, &result.saves, &result.deletes);
    assert_eq!(data.degree_plans.len(), 2);
    assert_eq!(data.degree_plans.iter().filter(|plan| plan.is_primary).map(|plan| plan.name.as_str()).collect::<Vec<_>>(), vec!["Theirs"]);
    assert_eq!(data.degree_plans.iter().find(|plan| plan.name == "Mine").and_then(|plan| plan.student_name.as_deref()), Some("SAMPLE STUDENT"));
}

#[test]
fn preferences_merge_field_by_field() {
    let mut data = UserData::default();
    let mut metadata = synced_metadata(&data);
    data.preferences.preferred_campus = Some("Boston".into());
    metadata.observe_local_data(&Snapshot::from_data(&data), DATE + 200.0);
    let mut remote = AppPreferences::default();
    remote.calendar_start_hour = 7;
    let mut changed = BTreeMap::new();
    changed.insert("calendarStartHour".into(), DATE + 100.0);
    let record = incoming(&Entity::Preferences(remote), DATE + 100.0, sync::SCHEMA_VERSION, &changed);
    let result = sync::merge(&[record], &[], &Snapshot::from_data(&data), &mut metadata, DATE);
    let Entity::Preferences(merged) = &result.saves[0] else { panic!("expected preferences") };
    assert_eq!(merged.preferred_campus.as_deref(), Some("Boston"));
    assert_eq!(merged.calendar_start_hour, 7);
    assert_eq!(result.kept_local, vec![CloudSyncKey::preferences()]);
    data = sync::applying_cloud_changes(&data, &result.saves, &result.deletes);
    let snapshot = Snapshot::from_data(&data);
    metadata.observe_local_data(&snapshot, DATE + 300.0);
    assert_eq!(metadata.pending_changes(&snapshot).saves, vec![CloudSyncKey::preferences()]);
    assert_eq!(metadata.preference_changed_at.get("calendarStartHour").copied(), Some(DATE + 100.0));
    assert_eq!(metadata.preference_changed_at.get("preferredCampus").copied(), Some(DATE + 200.0));
}

#[test]
fn customized_preferences_beat_defaults_of_unknown_age() {
    let defaults = SyncedPreferences::from_app(&AppPreferences::default());
    let mut custom = defaults.clone();
    custom.calendar_start_hour = 10;
    custom.show_weekends = true;
    let keep_custom = SyncedPreferences::merge(&custom, &BTreeMap::new(), &defaults, &BTreeMap::new());
    assert_eq!(keep_custom.values, custom);
    let adopt_custom = SyncedPreferences::merge(&defaults, &BTreeMap::new(), &custom, &BTreeMap::new());
    assert_eq!(adopt_custom.values, custom);
    let mut dated_at = BTreeMap::new();
    dated_at.insert("showWeekends".into(), DATE);
    let dated = SyncedPreferences::merge(&defaults, &dated_at, &custom, &BTreeMap::new());
    assert!(!dated.values.show_weekends);
    assert_eq!(dated.values.calendar_start_hour, 10);
}

#[test]
fn invalid_remote_calendar_hours_fall_back_to_defaults() {
    let json = r#"{"values":{"calendarStartHour":30,"calendarEndHour":2,"showWeekends":true},"changedAt":{}}"#;
    let RemoteValue::Preferences { values, .. } = sync::decode(json.as_bytes(), RecordType::Preferences).unwrap() else {
        panic!("expected preferences");
    };
    let defaults = AppPreferences::default();
    assert_eq!(values.calendar_start_hour, defaults.calendar_start_hour);
    assert_eq!(values.calendar_end_hour, defaults.calendar_end_hour);
    assert!(values.show_weekends);
    assert!(values.preferred_campus.is_none());
}

#[test]
fn applying_cloud_changes_replaces_inserts_and_deletes() {
    let newest = watch_item("3", DATE + 300.0);
    let oldest = watch_item("1", DATE + 100.0);
    let plan = schedule_plan("My Schedule", Uuid::new_v4(), "202710", false, DATE);
    let mut data = UserData::default();
    data.watchlist = vec![newest, oldest];
    data.plans = vec![plan.clone()];
    data.saved = vec![saved_course(cs3000(), DATE)];
    let mut renamed = plan;
    renamed.name = "Renamed".into();
    let middle = watch_item("2", DATE + 200.0);
    let removed = Entity::SavedCourse(saved_course(cs3000(), DATE)).key();
    let result = sync::applying_cloud_changes(
        &data,
        &[Entity::SchedulePlan(renamed), Entity::WatchItem(middle), Entity::SavedCourse(saved_course(cs3500(), DATE + 50.0))],
        &[removed],
    );
    assert_eq!(result.plans.iter().map(|plan| plan.name.as_str()).collect::<Vec<_>>(), vec!["Renamed"]);
    assert_eq!(result.watchlist.iter().map(|item| item.crn.as_str()).collect::<Vec<_>>(), vec!["3", "2", "1"]);
    assert_eq!(result.saved.iter().map(|course| course.key.clone()).collect::<Vec<_>>(), vec![cs3500()]);
    assert_eq!(result.preferences, data.preferences);
}

#[test]
fn status_lines() {
    let now = DATE;
    assert_eq!(SyncPhase::UpToDate { last_sync: Some(now - 20.0) }.summary(now), "Up to date · just now");
    assert_eq!(SyncPhase::UpToDate { last_sync: Some(now + 20.0) }.summary(now), "Up to date · just now");
    assert!(SyncPhase::UpToDate { last_sync: Some(now - 120.0) }.summary(now).starts_with("Up to date · "));
    assert_eq!(SyncPhase::UpToDate { last_sync: None }.summary(now), "Up to date");
    assert_eq!(SyncPhase::Syncing.summary(now), "Syncing…");
    assert_eq!(SyncPhase::Unavailable(UnavailableReason::NoAccount).summary(now), "Sign in to iCloud in Settings to sync");
    let unconfigured = SyncPhase::Unavailable(UnavailableReason::NotConfigured).summary(now);
    assert_eq!(unconfigured, "iCloud sync isn't set up in this build");
    assert!(unconfigured.as_bytes().contains(&b'\''));
    assert_eq!(SyncPhase::Error("Your iCloud storage is full.".into()).summary(now), "Your iCloud storage is full.");
}

/// Linux stand-in for the CloudKit entitlement check.
#[test]
fn entitlement_check() {
    assert!(!sync::consults_container_identifier());
    assert!(!sync::consults_code_signature());
    assert!(sync::code_signature_entitlements(b"entitlements").is_none());
    assert_eq!(renamed_without_entitlement(), "Renamed elsewhere");
}

/// Linux stand-in for the CloudKit entitlement check.
#[test]
fn reads_entitlements_from_code_signature() {
    assert!(!sync::consults_code_signature());
    assert!(sync::code_signature_entitlements(b"not a binary").is_none());
    assert!(sync::code_signature_entitlements(&[]).is_none());
    assert!(sync::code_signature_entitlements(&[0xcf, 0xfa, 0xed, 0xfe]).is_none());
    assert_eq!(renamed_without_entitlement(), "Renamed elsewhere");
}

// MARK: Watchlist

#[test]
fn a_section_that_gains_seats_raises_one_alert() {
    let item = shell_watch("12345", 0, 0, 40, false, "202710");
    let mut live = HashMap::new();
    live.insert("202710".into(), vec![shell_section("12345", 3, 0, 40)]);
    let outcome = watchlist::compare(&[item], &live, DATE);
    assert_eq!(outcome.alerts.len(), 1);
    let alert = &outcome.alerts[0];
    assert_eq!(alert.kind, AlertKind::Seats);
    assert_eq!(alert.title(), "Seats open in CS 3000");
    assert_eq!(alert.body(), "3 of 40 seats in Lecture 02 (CRN 12345). Register soon!");
    assert_eq!(alert.url(), "termorganize://section/202710/12345");
    assert_eq!(outcome.items[0].last_seats_remaining, 3);
    assert_eq!(outcome.items[0].last_notified_at, Some(DATE));
    assert_eq!(outcome.items[0].last_checked_at, Some(DATE));
}

#[test]
fn a_section_that_already_had_seats_raises_no_alert() {
    let mut live = HashMap::new();
    live.insert("202710".into(), vec![shell_section("12345", 5, 0, 40)]);
    let outcome = watchlist::compare(&[shell_watch("12345", 2, 0, 40, false, "202710")], &live, DATE);
    assert!(outcome.alerts.is_empty());
    assert_eq!(outcome.items[0].last_seats_remaining, 5);
}

#[test]
fn waitlist_alerts_need_the_opt_in() {
    let mut live = HashMap::new();
    live.insert("202710".into(), vec![shell_section("12345", 0, 2, 40)]);
    let opted_out = watchlist::compare(&[shell_watch("12345", 0, 0, 40, false, "202710")], &live, DATE);
    assert!(opted_out.alerts.is_empty());
    let opted_in = watchlist::compare(&[shell_watch("12345", 0, 0, 40, true, "202710")], &live, DATE);
    assert_eq!(opted_in.alerts.iter().map(|alert| alert.kind.clone()).collect::<Vec<_>>(), vec![AlertKind::Waitlist]);
    assert_eq!(opted_in.alerts[0].body(), "2 waitlist spots open in Lecture 02 (CRN 12345). Join soon!");
}

#[test]
fn a_section_the_registrar_no_longer_lists_is_reported_missing() {
    let item = shell_watch("99999", 1, 0, 40, false, "202710");
    let mut live = HashMap::new();
    live.insert("202710".into(), vec![shell_section("12345", 3, 0, 40)]);
    let outcome = watchlist::compare(&[item.clone()], &live, DATE);
    assert_eq!(outcome.missing_ids, vec![item.id()]);
    assert!(outcome.alerts.is_empty());
    assert_eq!(outcome.items[0].last_seats_remaining, 1);
}

#[test]
fn terms_that_were_not_checked_stay_untouched() {
    let other = shell_watch("12345", 0, 0, 40, false, "202730");
    let mut live = HashMap::new();
    live.insert("202710".into(), vec![shell_section("12345", 3, 0, 40)]);
    let outcome = watchlist::compare(&[other.clone()], &live, DATE);
    assert_eq!(outcome.items, vec![other]);
    assert_eq!(outcome.checked_term_codes, vec!["202710".to_string()]);
}

#[test]
fn statuses_describe_the_last_check() {
    let open = shell_watch("12345", 3, 0, 40, false, "202710");
    assert_eq!(WatchStatus::of(&open, false), WatchStatus::Open);
    assert_eq!(WatchStatus::Open.detail(&open), "3 of 40 open");
    assert_eq!(WatchStatus::Open.spoken_detail(&open), "3 of 40 seats open");
    let no_limit = shell_watch("12345", 1, 0, 0, false, "202710");
    assert_eq!(WatchStatus::Open.detail(&no_limit), "1 open");
    assert_eq!(WatchStatus::Open.spoken_detail(&no_limit), "1 seat open");
    let waitlist = shell_watch("12345", 0, 1, 40, false, "202710");
    assert_eq!(WatchStatus::of(&waitlist, false), WatchStatus::Waitlist);
    assert_eq!(WatchStatus::Waitlist.detail(&waitlist), "Full · 1 waitlist spot open");
    let full = shell_watch("12345", 0, 0, 40, false, "202710");
    assert_eq!(WatchStatus::of(&full, false), WatchStatus::Full);
    assert_eq!(WatchStatus::Full.detail(&full), "Full");
    assert_eq!(WatchStatus::of(&open, true), WatchStatus::Missing);
    assert_eq!(WatchStatus::Missing.detail(&open), "No longer listed");
}

#[test]
fn ago_wording_grows_with_the_time() {
    assert_eq!(watchlist::ago(DATE - 30.0, DATE), "just now");
    assert_eq!(watchlist::ago(DATE - 5.0 * 60.0, DATE), "5 min ago");
    assert_eq!(watchlist::ago(DATE - 3.0 * 3_600.0, DATE), "3 hr ago");
    assert_eq!(watchlist::ago(DATE - 30.0 * 3_600.0, DATE), "yesterday");
    assert_eq!(watchlist::ago(DATE - 4.0 * 86_400.0, DATE), "4 days ago");
    assert_eq!(watchlist::ago(DATE + 60.0, DATE), "just now");
}

// MARK: Rooms

#[test]
fn a_class_occupies_its_start_up_to_but_not_including_its_end() {
    let classes = vec![booking("CS 3000", 600, 665, &[2], None, None)];
    assert_eq!(room_status(classes.clone(), 599, 2), Availability::Free { until: Some(600) });
    assert!(!room_status(classes.clone(), 600, 2).is_free());
    assert!(!room_status(classes.clone(), 664, 2).is_free());
    assert_eq!(room_status(classes.clone(), 665, 2), Availability::Free { until: None });
    assert_eq!(room_status(classes, 620, 3), Availability::Free { until: None });
}

#[test]
fn back_to_back_classes_are_one_stretch_of_use() {
    let classes = vec![booking("CS 3000", 600, 665, &[2], None, None), booking("MATH 1341", 665, 730, &[2], None, None)];
    let Availability::Busy { until, free_until, .. } = room_status(classes, 620, 2) else { panic!("expected the room to be in use") };
    assert_eq!(until, 730);
    assert_eq!(free_until, None);
}

#[test]
fn a_gap_between_classes_is_reported() {
    let classes = vec![booking("CS 3000", 600, 665, &[2], None, None), booking("MATH 1341", 720, 780, &[2], None, None)];
    let Availability::Busy { until, free_until, .. } = room_status(classes.clone(), 620, 2) else { panic!("expected the room to be in use") };
    assert_eq!(until, 665);
    assert_eq!(free_until, Some(720));
    assert_eq!(room_status(classes, 670, 2), Availability::Free { until: Some(720) });
}

#[test]
fn statuses_read_as_sentences() {
    assert_eq!(Availability::Free { until: Some(600) }.summary(), "Free until 10:00 AM");
    assert_eq!(Availability::Free { until: None }.summary(), "Free for the rest of the day");
    let classes = vec![booking("CS 3000", 600, 665, &[2], None, None)];
    assert_eq!(room_status(classes.clone(), 620, 2).summary(), "In use until 11:05 AM");
    assert_eq!(room_status(classes, 620, 2).detailed_summary(), "In use: CS 3000 until 11:05 AM");
}

#[test]
fn a_room_free_for_under_half_an_hour_is_only_briefly_free() {
    let classes = vec![booking("CS 3000", 600, 665, &[2], None, None)];
    let soon = Moment::new(2, 580);
    let early = Moment::new(2, 500);
    assert!(room_status(classes.clone(), 580, 2).is_free_only_briefly(soon));
    assert!(!room_status(classes, 500, 2).is_free_only_briefly(early));
}

#[test]
fn short_sessions_only_count_on_their_own_dates() {
    let workshop = booking("CS 2500", 600, 665, &[2], Some("2026-09-15"), Some("2026-09-16"));
    let classroom = room(vec![workshop], "210", "WVH");
    assert!(rooms::bookings_of(&classroom, 2, None).is_empty());
    assert_eq!(rooms::bookings_of(&classroom, 2, Some("2026-09-15")).len(), 1);
    assert!(rooms::bookings_of(&classroom, 2, Some("2026-10-06")).is_empty());
}

#[test]
fn free_stretches_are_the_gaps_between_classes() {
    let day = vec![
        booking("CS 3000", 600, 665, &[2], None, None),
        booking("MATH 1341", 665, 730, &[2], None, None),
        booking("PHYS 1151", 780, 840, &[2], None, None),
    ];
    assert_eq!(rooms::free_stretches(&day, 420, 1_320), vec![420..600, 730..780, 840..1_320]);
    assert_eq!(rooms::free_stretches(&day, 620, 800), vec![730..780]);
    assert!(rooms::free_stretches(&day, 600, 730).is_empty());
    assert_eq!(rooms::free_stretches(&[], 420, 1_320), vec![420..1_320]);
    assert!(rooms::free_stretches(&[], 600, 600).is_empty());
    let overlapping = vec![booking("CS 3000", 600, 700, &[2], None, None), booking("CS 3001", 650, 680, &[2], None, None)];
    assert_eq!(rooms::free_stretches(&overlapping, 600, 800), vec![700..800]);
}

#[test]
fn free_room_counts_are_the_rooms_with_no_class_during_the_whole_hour() {
    let busy_morning = room(vec![booking("CS 3000", 9 * 60 + 50, 11 * 60 + 30, &[2], None, None)], "210", "RY");
    let busy_afternoon = room(vec![booking("MATH 1341", 13 * 60, 14 * 60, &[2], None, None)], "300", "RY");
    let empty = room(vec![], "400", "RY");
    let counts = rooms::free_room_counts(&[busy_morning.clone(), busy_afternoon, empty], 2);
    assert_eq!(counts.len(), 15);
    assert_eq!(counts[0], 3);
    assert_eq!(counts[2], 2);
    assert_eq!(counts[3], 2);
    assert_eq!(counts[4], 2);
    assert_eq!(counts[5], 3);
    assert_eq!(counts[6], 2);
    assert_eq!(counts[7], 3);
    assert_eq!(rooms::free_room_counts(&[], 2), vec![0; 15]);
    assert_eq!(rooms::free_room_counts(&[busy_morning], 0), vec![1; 15]);
}

#[test]
fn search_matches_buildings_and_rooms_by_their_words() {
    let ryder = BuildingSummary {
        code: "RY".into(),
        name: "Ryder Hall".into(),
        campus_code: Some("BOS".into()),
        rooms: vec![room(vec![], "210", "RY"), room(vec![], "300", "RY")],
    };
    let snell = BuildingSummary {
        code: "SL".into(),
        name: "Snell Library".into(),
        campus_code: Some("BOS".into()),
        rooms: vec![room(vec![], "210", "SL")],
    };
    let all = rooms::availability_of(&[ryder, snell], Moment::new(2, 600));
    assert_eq!(rooms::filter_availability(&all, "ry").iter().map(|item| item.building.code.as_str()).collect::<Vec<_>>(), vec!["RY"]);
    assert_eq!(rooms::filter_availability(&all, "snell").iter().map(|item| item.building.code.as_str()).collect::<Vec<_>>(), vec!["SL"]);
    assert_eq!(rooms::filter_availability(&all, "ryder").first().map(|item| item.free.len()), Some(2));
    let by_room = rooms::filter_availability(&all, "210");
    assert_eq!(by_room.iter().map(|item| item.building.code.as_str()).collect::<Vec<_>>(), vec!["RY", "SL"]);
    assert_eq!(by_room.first().map(|item| item.free.iter().map(|entry| entry.room.room.as_str()).collect::<Vec<_>>()), Some(vec!["210"]));
    assert_eq!(rooms::filter_availability(&all, "  ").len(), 2);
}

#[test]
fn moments_stay_within_the_week() {
    let wrapped = Moment::new(8, 99_999);
    assert_eq!(wrapped.weekday, 1);
    assert_eq!(wrapped.minutes, 24 * 60 - 1);
    assert!(Moment::new(2, 600).is_within_class_hours());
    assert!(!Moment::new(2, 6 * 60).is_within_class_hours());
    assert_eq!(Moment::new(2, 14 * 60 + 30).label(), "Tuesday at 2:30 PM");
}

#[test]
fn follow_up_notes_say_what_comes_next() {
    let today = vec![booking("CS 3000", 600, 665, &[2], None, None), booking("MATH 1341", 720, 780, &[2], None, None)];
    assert_eq!(rooms::follow_up_note(&Availability::Free { until: Some(600) }, &today), "Next: CS 3000, 10:00 – 11:05 AM");
    assert_eq!(rooms::follow_up_note(&Availability::Free { until: None }, &today), "No more classes today.");
    assert_eq!(rooms::follow_up_note(&Availability::Free { until: None }, &[]), "No classes are scheduled here today.");
    let Availability::Busy { until, free_until, course } = rooms::status_of_bookings(&today, 620) else { panic!("expected the room to be in use") };
    assert_eq!(rooms::follow_up_note(&Availability::Busy { until, free_until, course }, &today), "Then free until 12:00 PM");
    assert_eq!(rooms::free_summary(12, 40), "12 of 40 rooms free");
    assert_eq!(rooms::free_summary(1, 1), "1 of 1 room free");
}

#[test]
fn hour_labels_use_the_twelve_hour_clock() {
    assert_eq!(hour_label(0), "12 AM");
    assert_eq!(hour_label(8), "8 AM");
    assert_eq!(hour_label(12), "12 PM");
    assert_eq!(hour_label(21), "9 PM");
    assert_eq!(hour_label(24), "12 AM");
}

#[test]
fn legal_text_names_the_license_and_where_to_read_it() {
    assert_eq!(legal::LICENSE_URL, "https://www.gnu.org/licenses/agpl-3.0.html");
    assert!(legal::SEARCHNEU_SUMMARY.contains(&format!("{} adaptation of SearchNEU", legal::PLATFORM)));
}

// MARK: Catalog support

#[test]
fn day_presets_describe_their_days() {
    assert_eq!(SearchDayPreset::selection(None), SearchDayPreset::Any);
    assert_eq!(SearchDayPreset::selection(Some(DaySet::from_indices([1, 2, 3, 4]))), SearchDayPreset::NoFridays);
    assert_eq!(SearchDayPreset::selection(Some(DaySet::from_indices([2, 5]))), SearchDayPreset::Tf);
    assert_eq!(SearchDayPreset::selection(Some(DaySet::from_indices([1, 5]))), SearchDayPreset::Custom);
    assert!(SearchDayPreset::matching(Some(DaySet::from_indices([1, 5]))).is_none());
    for preset in SearchDayPreset::ALL {
        if preset == SearchDayPreset::Custom {
            continue;
        }
        assert_eq!(SearchDayPreset::selection(preset.days()), preset);
    }
}

#[test]
fn filter_chip_titles() {
    assert_eq!(search::filter_days_title(None), "Days");
    assert_eq!(search::filter_days_title(Some(DaySet::from_indices([1, 2, 3, 4]))), "No Fridays");
    assert_eq!(search::filter_days_title(Some(DaySet::from_indices([1, 5]))), "MF only");
    assert_eq!(search::filter_days_title(Some(DaySet(0))), "No set days");
    assert_eq!(search::filter_subjects_title(&HashSet::new()), "Subject");
    assert_eq!(search::filter_subjects_title(&HashSet::from(["CS".into()])), "CS");
    assert_eq!(search::filter_subjects_title(&HashSet::from(["CS".into(), "DS".into(), "MATH".into()])), "3 subjects");
    assert_eq!(search::filter_nupaths_title(&HashSet::from([NuPath::Fq, NuPath::Wi])), "FQ, WI");
    assert_eq!(search::filter_nupaths_title(&HashSet::from([NuPath::Fq, NuPath::Wi, NuPath::Ce])), "NUpath · 3");
    assert_eq!(search::filter_level_title(0), "Under 1000");
    assert_eq!(search::filter_level_title(3), "3000");
}

#[test]
fn time_of_day_conversion_round_trips() {
    for minutes in [0, 9 * 60, 12 * 60 + 35, 17 * 60, 23 * 60 + 59] {
        assert_eq!(search::minutes_from_date(search::date_from_minutes(minutes)), minutes);
    }
    assert_eq!(search::minutes_from_date(search::date_from_minutes(-30)), 0);
    assert_eq!(search::minutes_from_date(search::date_from_minutes(5_000)), 24 * 60 - 1);
}

#[test]
fn quick_searches_apply_exactly_one_filter() {
    for preset in HomeQuickSearch::ALL {
        assert_eq!(preset.filters().active_count(), 1, "{}", preset.title());
        assert!(!preset.symbol().is_empty());
    }
    assert!(HomeQuickSearch::OpenSeats.filters().open_seats_only);
    assert!(HomeQuickSearch::Honors.filters().honors_only);
    assert_eq!(HomeQuickSearch::Online.filters().instruction, InstructionFilter::Online);
    assert_eq!(HomeQuickSearch::NoFridays.filters().allowed_days, Some(DaySet::from_indices([1, 2, 3, 4])));
    assert_eq!(HomeQuickSearch::FirstYear.filters().levels, HashSet::from([1]));
}

#[test]
fn quick_searches_find_courses() {
    let index = bundled_index();
    for preset in HomeQuickSearch::ALL {
        let response = search::search(&SearchQuery { text: String::new(), filters: preset.filters(), sort: search::SearchSort::Relevance }, index);
        assert!(!response.results.is_empty(), "{} found nothing in the bundled catalog", preset.title());
    }
}

#[test]
fn browse_summary_lines() {
    assert_eq!(search::browse_summary_text(42, 42, 0, false, "course"), "42 courses");
    assert_eq!(search::browse_summary_text(1, 1, 0, false, "course"), "1 course");
    assert_eq!(search::browse_summary_text(12, 42, 9, false, "course"), "12 of 42 courses · 9 with open seats");
    assert_eq!(search::browse_summary_text(9, 42, 9, true, "course"), "9 of 42 courses");
    assert_eq!(search::browse_summary_text(2, 2, 0, false, "professor"), "2 professors");
    assert_eq!(search::browse_level_title(1), "1000-level");
    assert_eq!(search::browse_level_title(0), "Under 1000");
    assert_eq!(search::browse_level_noun(3), "3000-level courses");
    assert_eq!(search::browse_level_noun(0), "courses under 1000");
}

#[test]
fn popular_subjects_are_the_busiest() {
    let index = bundled_index();
    let popular = search::popular_subjects(index, 12);
    assert_eq!(popular.len(), 12);
    let counts_map = search::course_count_by_subject(index);
    let counts: Vec<i32> = popular.iter().map(|subject| counts_map.get(&subject.code).copied().unwrap_or(0)).collect();
    let mut sorted = counts.clone();
    sorted.sort_by(|left, right| right.cmp(left));
    assert_eq!(counts, sorted);
    let busiest_outside = index
        .active_subjects
        .iter()
        .filter(|subject| !popular.iter().any(|item| item.code == subject.code))
        .map(|subject| counts_map.get(&subject.code).copied().unwrap_or(0))
        .max()
        .unwrap_or(0);
    assert!(counts.last().copied().unwrap_or(0) >= busiest_outside);
    assert_eq!(
        search::popular_subjects(index, 3).iter().map(|subject| subject.code.as_str()).collect::<Vec<_>>(),
        popular.iter().take(3).map(|subject| subject.code.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn search_examples_all_find_something() {
    let index = bundled_index();
    let examples = search::search_examples(index);
    assert!(examples.len() >= 3);
    for example in examples {
        let response = search::search(&SearchQuery { text: example.title.clone(), filters: SearchFilters::default(), sort: search::SearchSort::Relevance }, index);
        let found = !response.results.is_empty() || response.crn_match.is_some() || !response.instructors.is_empty();
        assert!(found, "“{}” ({}) found nothing", example.title, example.caption);
    }
}

#[test]
fn professor_page_orders_sections_and_gives_each_course_its_own_color() {
    let index = bundled_index();
    let summary = index.instructors.iter().find(|summary| summary.course_keys.len() >= 3).expect("instructor");
    let sections = professors::sections_of(summary, index);
    assert_eq!(sections.len(), summary.crns.len());
    for (earlier, later) in sections.iter().zip(sections.iter().skip(1)) {
        assert!(earlier.course_key() <= later.course_key());
    }
    let keys: Vec<_> = summary.course_keys.iter().take(10).cloned().collect();
    let colors = professors::course_colors(&keys);
    assert_eq!(colors.len(), keys.len());
    assert_eq!(colors.values().collect::<HashSet<_>>().len(), keys.len());
    assert_eq!(professors::course_colors(&keys), colors);
}

#[test]
fn professor_page_texts() {
    let a = key("CS", "3000");
    let b = key("CS", "3001");
    let section = |course: &CourseKey, number: &str| CourseSection {
        crn: format!("{}{number}", course.number),
        subject: course.subject.clone(),
        number: course.number.clone(),
        title: "T".into(),
        section_number: number.into(),
        part_of_term: "1".into(),
        seat_capacity: 10,
        seats_remaining: 5,
        enrollment: 5,
        waitlist_capacity: 0,
        waitlist_count: 0,
        waitlist_remaining: 0,
        class_type: "Lecture".into(),
        honors: false,
        campus: "Boston".into(),
        instruction_method: None,
        meetings: vec![],
        instructors: None,
        cross_list_crns: vec![],
        attributes: vec![],
        description: None,
        prereqs: None,
        coreqs: None,
    };
    let sections = vec![section(&a, "01"), section(&a, "04"), section(&b, "02")];
    assert_eq!(professors::summary_text(&sections), "Teaches 3 sections across 2 courses");
    assert_eq!(professors::summary_text(&sections[2..3]), "Teaches 1 section across 1 course");
    assert_eq!(professors::section_note(&a, &sections), "Sections 01, 04");
    assert_eq!(professors::section_note(&b, &sections), "Section 02");
    assert_eq!(professors::rate_my_professors_url("Jane Doe"), "https://www.ratemyprofessors.com/search/professors/696?q=Jane%20Doe");
    assert!(professors::rate_my_professors_url("José O'Neil-Smith").contains("q=Jos%C3%A9%20O%27Neil-Smith"));
}

#[test]
fn degree_plan_standing_texts() {
    let plan = Uuid::new_v4();
    let completed = DegreePlanStanding { plan_id: plan, kind: DegreeStandingKind::Completed { via: None } };
    assert_eq!(completed.text(), "Completed");
    assert_eq!(completed.symbol(), "checkmark.circle.fill");
    assert_eq!(completed.spoken_text(), "Completed, in your degree plan");
    let via = DegreePlanStanding { plan_id: plan, kind: DegreeStandingKind::Completed { via: Some(key("CS", "2510")) } };
    assert_eq!(via.text(), "Completed via CS 2510");
    let planned = DegreePlanStanding { plan_id: plan, kind: DegreeStandingKind::Planned { semester: "Spring 2027".into(), via: None } };
    assert_eq!(planned.text(), "Planned · Spring 2027");
    assert_eq!(planned.symbol(), "calendar");
    let planned_via = DegreePlanStanding { plan_id: plan, kind: DegreeStandingKind::Planned { semester: "Spring 2027".into(), via: Some(key("CS", "2510")) } };
    assert_eq!(planned_via.text(), "Planned via CS 2510 · Spring 2027");
    assert_eq!(planned_via.spoken_text(), "Planned via CS 2510 for Spring 2027 in your degree plan");
}

/// `UserDataStore.courseIsInSchedule` from CatalogSupportTests.courseIsInScheduleCountsOnlyThePlansOfTheTerm.
#[test]
fn course_is_in_schedule_counts_only_the_plans_of_the_term() {
    let course = key("CS", "3000");
    let mut data = UserData::default();
    assert!(!data.course_is_in_schedule("202710", &course, None));
    let plan = data.create_plan("Plan A", "202710");
    assert!(plan.is_primary);
    data.add_course(course.clone(), plan.id);
    assert!(data.course_is_in_schedule("202710", &course, None));
    assert!(!data.course_is_in_schedule("202630", &course, None));
    assert!(!data.course_is_in_schedule("202710", &course, Some("12726")));
    data.update_plan(plan.id, |plan| plan.selected_crns.push("12726".into()));
    assert!(data.course_is_in_schedule("202710", &course, Some("12726")));
}

// MARK: Banner

#[test]
fn parses_nested_prerequisites() {
    let subjects = banner_subjects();
    let html = preq_table(&[
        preq_row("", true, "", "", "", "", false),
        preq_row("", true, "", "", "Computer Science", "2100", false),
        preq_row("Or", false, "", "", "Computer Science - MMC", "201M", false),
        preq_row("Or", false, "", "", "Computer Science", "2510", false),
        preq_row("Or", false, "", "", "Data Science", "2500", true),
        preq_row("And", true, "", "", "Computer Science", "1800", false),
        preq_row("Or", false, "", "", "Mathematics", "209M", false),
        preq_row("Or", false, "", "", "Electrical and Comp Engineerng", "2160", true),
        preq_row("", false, "", "", "", "", true),
        preq_row("Or", false, "", "", "Computer Science", "2700", false),
    ]);
    let requisite = banner::prerequisites(&html, &subjects).expect("prerequisites");
    let Requisite::Or(top) = &requisite else { panic!("expected an OR at the top, got {requisite:?}") };
    assert_eq!(top.len(), 2);
    assert_eq!(top.last(), Some(&Requisite::Course(key("CS", "2700"))));
    let Requisite::And(groups) = &top[0] else { panic!("expected an AND group") };
    assert_eq!(groups.len(), 2);
    assert!(requisite.referenced_courses().contains(&key("EECE", "2160")));
    assert!(requisite.referenced_courses().contains(&key("CS", "201M")));
}

#[test]
fn parses_single_course_and_tests() {
    let subjects = banner_subjects();
    let single = preq_table(&[preq_row("", false, "", "", "Computer Science", "2100", false)]);
    assert_eq!(banner::prerequisites(&single, &subjects), Some(Requisite::Course(key("CS", "2100"))));
    let with_test = preq_table(&[
        preq_row("", false, "", "", "Mathematics", "1341", false),
        preq_row("Or", false, "Graduate Admission", "", "", "", false),
    ]);
    let parsed = banner::prerequisites(&with_test, &subjects).unwrap();
    assert_eq!(parsed, Requisite::Or(vec![Requisite::Course(key("MATH", "1341")), Requisite::Test { name: "Graduate Admission".into(), score: None }]));
}

#[test]
fn empty_prerequisites_are_nil() {
    let html = "<section aria-labelledby=\"preReqs\"><h3>Catalog Prerequisites</h3> No prerequisite information available.</section>";
    assert!(banner::prerequisites(html, &banner_subjects()).is_none());
}

#[test]
fn parses_corequisites() {
    let html = r#"
        <section><table class="basePreqTable"><thead><tr><th>Subject</th></tr></thead><tbody>
        <tr><td>Computer Science</td><td>3001</td><td>Recitation for CS 3000</td></tr>
        </tbody></table></section>
        "#;
    assert_eq!(banner::corequisites(html, &banner_subjects()), Some(Requisite::Course(key("CS", "3001"))));
}

#[test]
fn parses_description_and_catalog_details() {
    let description = r#"
        <section aria-labelledby="courseDescription">
            <!--display course description-->
            Covers the derivative &amp; integration.<br/>
        </section>
        "#;
    assert_eq!(banner::description(description), "Covers the derivative & integration.");
    let details = r#"
        <section aria-labelledby="catalog">
         <span class="status-bold">Title:</span>Algorithms and Data<br/>
         <span class="status-bold">College:</span>
         <span>Khoury Coll of Comp Sciences CS</span><br/>
         <span class="status-bold">Department:</span>
         <span>Computer Science CS</span><br/>
        </section>
        "#;
    let parsed = banner::catalog_details(details);
    assert_eq!(parsed.title.as_deref(), Some("Algorithms and Data"));
    assert_eq!(parsed.college.as_deref(), Some("Khoury College of Computer Sciences"));
    assert_eq!(parsed.department.as_deref(), Some("Computer Science"));
    let subjects = banner_subjects();
    assert!(banner::mutual_exclusions("No Mutual Exclusion information available.", &subjects).is_empty());
    assert!(banner::mutual_exclusions("<p>Not open to students who have taken CS 2510.</p>", &subjects).contains(&key("CS", "2510")));
}

#[test]
fn decodes_entities() {
    assert_eq!(html::decode_entities("Fin Accounting &amp; Reporting"), "Fin Accounting & Reporting");
    assert_eq!(html::decode_entities("O&#39;Brien &#x2014; &quot;Hi&quot;"), "O'Brien — \"Hi\"");
    assert_eq!(html::decode_entities("AT&T"), "AT&T");
}

#[test]
fn converts_banner_meeting_times() {
    let meeting = banner::meeting_from_banner("MWR", 800, 905, "DG", "070", "09/09/2026", "12/13/2026");
    assert_eq!(meeting.start, Some(480));
    assert_eq!(meeting.end, Some(545));
    assert_eq!(meeting.days.compact_label(), "MWR");
    assert_eq!(meeting.location_label(), "DG 070");
    assert_eq!(meeting.start_date.as_deref(), Some("2026-09-09"));
    assert!(!meeting.is_final_exam());
}

// MARK: Core model

#[test]
fn term_names() {
    assert_eq!(Term { code: "202710".into(), banner_description: "Fall 2026 Semester".into() }.name(), "Fall 2026");
    assert_eq!(Term { code: "202630".into(), banner_description: "Spring 2026 Semester (View Only)".into() }.name(), "Spring 2026");
    let cps = Term { code: "202635".into(), banner_description: "Spring 2026 CPS Quarter (View Only)".into() };
    assert_eq!(cps.name(), "Spring 2026 · CPS Quarter");
    assert_eq!(cps.short_name(), "Spring 2026");
    assert_eq!(cps.college(), College::Cps);
    assert!(cps.is_view_only());
    let blank = Term { code: "202710".into(), banner_description: String::new() };
    assert!(matches!(blank.season(), Season::Fall));
    assert_eq!(blank.calendar_year(), 2026);
    assert_eq!(Term { code: "202630".into(), banner_description: String::new() }.calendar_year(), 2026);
}

#[test]
fn course_key_parsing() {
    assert_eq!(CourseKey::parse("CS 3000"), Some(key("CS", "3000")));
    assert_eq!(CourseKey::parse("cs3000"), Some(key("CS", "3000")));
    assert_eq!(CourseKey::parse("math-209m"), Some(key("MATH", "209M")));
    assert_eq!(CourseKey::parse("3000"), None);
    assert_eq!(key("CS", "3000").level(), Some(3));
    assert!(key("CS", "2100") < key("CS", "3000"));
    assert!(key("CS", "900") < key("CS", "1800"));
}

#[test]
fn day_sets() {
    let mwr = DaySet::from_indices([1, 3, 4]);
    assert_eq!(mwr.compact_label(), "MWR");
    assert!(mwr.contains_day(3));
    assert!(!mwr.contains_day(2));
    assert_eq!(mwr.count(), 3);
    assert!(mwr.is_subset(DaySet::WEEKDAYS));
    assert!(!DaySet::SATURDAY.is_subset(DaySet::WEEKDAYS));
}

#[test]
fn time_formatting() {
    assert_eq!(minutes_from_hhmm(1435), 875);
    assert_eq!(label_minutes(875, true), "2:35 PM");
    assert_eq!(label_minutes(0, true), "12:00 AM");
    assert_eq!(range_label(630, 695), "10:30 – 11:35 AM");
    assert_eq!(range_label(705, 805), "11:45 AM – 1:25 PM");
    assert_eq!(credit_label(4.0, 4.0), "4 credits");
    assert_eq!(credit_label(1.0, 1.0), "1 credit");
    assert_eq!(credit_label(1.0, 4.0), "1–4 credits");
}

#[test]
fn requisite_helpers() {
    let a = key("CS", "2100");
    let b = key("CS", "2510");
    let c = key("CS", "1800");
    let nested = Requisite::And(vec![
        Requisite::Or(vec![Requisite::Course(a.clone()), Requisite::Or(vec![Requisite::Course(b.clone())])]),
        Requisite::And(vec![Requisite::Course(c.clone())]),
    ]);
    let simplified = nested.simplified().unwrap();
    assert_eq!(simplified, Requisite::And(vec![Requisite::Or(vec![Requisite::Course(a.clone()), Requisite::Course(b.clone())]), Requisite::Course(c.clone())]));
    assert_eq!(nested.referenced_courses(), vec![a.clone(), b, c.clone()]);
    assert_eq!(simplified.summary(), "(CS 2100 or CS 2510) and CS 1800");
    let done: HashSet<_> = [a, c].into_iter().collect();
    assert!(simplified.is_satisfied(&done, false));
    assert!(!simplified.is_satisfied(&HashSet::from([key("CS", "2100")]), false));
}

#[test]
fn instructor_names() {
    assert_eq!(Instructor { name: "Ptitsyna, Natalia".into(), primary: true, email: None }.display_name(), "Natalia Ptitsyna");
    assert_eq!(Instructor { name: "Staff".into(), primary: false, email: None }.display_name(), "Staff");
}

#[test]
fn catalog_archive_round_trip() {
    let term = Term { code: "202710".into(), banner_description: "Fall 2026 Semester".into() };
    let course = Course {
        subject: "CS".into(),
        number: "3000".into(),
        title: "Algorithms and Data".into(),
        description: "Introduces…".into(),
        min_credits: 4.0,
        max_credits: 4.0,
        attributes: vec!["NCFQ".into()],
        prereqs: Some(Requisite::Course(key("CS", "2100"))),
        coreqs: None,
        postreqs: vec![],
        special_topics: false,
        college: None,
        department: None,
        details_loaded: true,
    };
    let mut catalog = TermCatalog {
        version: 1,
        term,
        generated_at: 0.0,
        sections_updated_at: 0.0,
        courses: vec![course],
        sections: vec![],
        subjects: vec![],
        attributes: vec![],
        campuses: vec![],
    };
    catalog.recompute_postreqs();
    let data = archive::encode(&catalog).unwrap();
    let decoded = archive::decode(&data).unwrap();
    assert_eq!(decoded.courses.first().map(|course| course.title.as_str()), Some("Algorithms and Data"));
    assert_eq!(decoded.courses.first().map(|course| course.nupaths()), Some(vec![NuPath::Fq]));
    assert_eq!(decoded.courses.first().and_then(|course| course.prereqs.clone()), Some(Requisite::Course(key("CS", "2100"))));
    assert_eq!(archive::term_code_from_file_name("term-202710.catalog").as_deref(), Some("202710"));
}

#[test]
fn widget_occurrences_skip_finished_classes() {
    let now = upnext::eastern_to_unix(2026, 9, 16, 10, 0);
    let meetings = vec![
        WidgetMeeting {
            id: "a".into(),
            crn: "1".into(),
            course_code: "CS 3000".into(),
            title: "Algo".into(),
            location: "WVH 210".into(),
            class_type: "Lecture".into(),
            weekday: 3,
            start: 9 * 60,
            end: 9 * 60 + 65,
            color_index: 0,
            start_date: None,
            end_date: None,
        },
        WidgetMeeting {
            id: "b".into(),
            crn: "2".into(),
            course_code: "MATH 2331".into(),
            title: "LinAlg".into(),
            location: "RI 300".into(),
            class_type: "Lecture".into(),
            weekday: 3,
            start: 13 * 60,
            end: 14 * 60 + 5,
            color_index: 1,
            start_date: None,
            end_date: None,
        },
    ];
    let data = WidgetScheduleData {
        generated_at: now,
        term_code: "202710".into(),
        term_name: "Fall 2026".into(),
        plan_name: "Plan".into(),
        classes_start: None,
        classes_end: None,
        meetings,
    };
    let during = data.occurrences(now, 3);
    assert_eq!(during.first().map(|item| item.meeting.course_code.as_str()), Some("CS 3000"));
    assert_eq!(during.len(), 3);
    let after = data.occurrences(now + 30.0 * 60.0, 1);
    assert_eq!(after.first().map(|item| item.meeting.course_code.as_str()), Some("MATH 2331"));
    assert_eq!(data.classes_on(now).len(), 2);
}

// MARK: Search

#[test]
fn bundled_catalog_is_complete() {
    let index = bundled_index();
    assert!(index.courses.len() > 2_500);
    assert!(index.catalog.sections.len() > 7_000);
    assert!(index.courses.iter().all(|course| course.details_loaded));
    assert!(!index.instructors.is_empty());
    assert!(!index.buildings.is_empty());
}

#[test]
fn exact_course_codes() {
    assert_eq!(top_codes("CS 3000", SearchFilters::default()).first().map(String::as_str), Some("CS 3000"));
    assert_eq!(top_codes("cs3000", SearchFilters::default()).first().map(String::as_str), Some("CS 3000"));
    assert_eq!(top_codes("cs 30", SearchFilters::default()).first().map(String::as_str), Some("CS 3000"));
}

#[test]
fn titles_and_shorthand() {
    assert_eq!(top_codes("algorithms and data", SearchFilters::default()).first().map(String::as_str), Some("CS 3000"));
    assert_eq!(top_codes("linear algebra", SearchFilters::default()).first().map(String::as_str), Some("MATH 2331"));
    assert!(top_codes("orgo", SearchFilters::default()).iter().all(|code| code.starts_with("CHM") || code.starts_with("CHEM")));
    assert_eq!(top_codes("linalg", SearchFilters::default()).first().map(String::as_str), Some("MATH 2331"));
    assert!(top_codes("algoritms", SearchFilters::default()).contains(&"CS 3000".to_string()));
}

#[test]
fn crn_lookup() {
    let index = bundled_index();
    let section = index.catalog.sections.first().expect("section");
    let response = search::search(&SearchQuery { text: section.crn.clone(), filters: SearchFilters::default(), sort: search::SearchSort::Relevance }, index);
    assert_eq!(response.crn_match.as_ref().map(|found| found.crn.as_str()), Some(section.crn.as_str()));
}

#[test]
fn subject_and_professor_matches() {
    let index = bundled_index();
    assert_eq!(
        search::search(&SearchQuery { text: "cs".into(), filters: SearchFilters::default(), sort: search::SearchSort::Relevance }, index)
            .subjects
            .iter()
            .map(|subject| subject.code.as_str())
            .collect::<Vec<_>>(),
        vec!["CS"]
    );
    let instructor = index.instructors.iter().find(|summary| summary.display_name.split_whitespace().count() == 2).expect("instructor");
    let response = search::search(
        &SearchQuery { text: instructor.display_name.clone(), filters: SearchFilters::default(), sort: search::SearchSort::Relevance },
        index,
    );
    assert!(response.instructors.iter().any(|found| found.name == instructor.name), "expected {}", instructor.display_name);
}

#[test]
fn filters_require_a_matching_section() {
    let index = bundled_index();
    let mut filters = SearchFilters::default();
    filters.subjects = HashSet::from(["CS".into()]);
    filters.open_seats_only = true;
    filters.allowed_days = Some(DaySet::from_indices([1, 3, 4]));
    let results = search::search(&SearchQuery { text: String::new(), filters: filters.clone(), sort: search::SearchSort::Relevance }, index).results;
    assert!(!results.is_empty());
    for result in results {
        assert_eq!(result.course.subject, "CS");
        assert!(index.sections_for(&result.course.key()).iter().any(|section| filters.matches_section(section)));
    }
}
