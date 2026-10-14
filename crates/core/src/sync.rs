//! Local folder sync standing in for iCloud. Last write wins. A remote delete is kept
//! unless the value was edited locally since. One primary plan. Device-only fields stay put.

use crate::model::*;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Record {
    pub key: String,
    pub kind: String,
    pub updated_at: f64,
    pub deleted: bool,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Default)]
pub struct SyncState {
    pub records: HashMap<String, Record>,
    pub local_modified: HashMap<String, f64>,
}

const DEVICE_PREF_FIELDS: &[&str] = &[
    "selectedTermCode",
    "hasCompletedOnboarding",
    "searchHistory",
    "folderSyncEnabled",
    "folderSyncPath",
    "backgroundRefreshEnabled",
    "seatCheckMinutes",
    "autoRefreshSeats",
    "colorScheme",
];

pub fn record_key_plan(id: &str) -> String {
    format!("schedule:{id}")
}
pub fn record_key_degree(id: &str) -> String {
    format!("degree:{id}")
}
pub fn record_key_watch(id: &str) -> String {
    format!("watch:{id}")
}
pub fn record_key_saved(term: &str, key: &CourseKey) -> String {
    format!("saved:{term}:{}", key)
}

pub fn diff(local: &[Record], remote: &[Record]) -> (Vec<Record>, Vec<String>) {
    let remote_keys: HashSet<_> = remote.iter().map(|r| r.key.clone()).collect();
    let local_keys: HashSet<_> = local.iter().map(|r| r.key.clone()).collect();
    let mut changes = vec![];
    for rec in local {
        match remote.iter().find(|r| r.key == rec.key) {
            Some(other) if other.payload != rec.payload || other.deleted != rec.deleted => {
                changes.push(rec.clone())
            }
            None => changes.push(rec.clone()),
            _ => {}
        }
    }
    let deletes = remote_keys.difference(&local_keys).cloned().collect();
    (changes, deletes)
}

/// Newer timestamp wins. A tie goes to the remote copy. A remote deletion removes a value
/// that was not edited locally after the deletion. A newer local edit keeps it.
pub fn merge_record(
    local: Option<&Record>,
    remote: Option<&Record>,
    local_edit: Option<f64>,
) -> Option<Record> {
    match (local, remote) {
        (None, None) => None,
        (Some(l), None) => Some(l.clone()),
        (None, Some(r)) => {
            if r.deleted {
                None
            } else {
                Some(r.clone())
            }
        }
        (Some(l), Some(r)) => {
            if r.deleted {
                if local_edit.is_some_and(|t| t > r.updated_at) {
                    Some(l.clone())
                } else {
                    None
                }
            } else if l.updated_at > r.updated_at {
                Some(l.clone())
            } else {
                Some(r.clone())
            }
        }
    }
}

pub fn merge_preferences(
    local: &AppPreferences,
    remote: &serde_json::Value,
    remote_newer: bool,
) -> AppPreferences {
    if !remote_newer {
        return local.clone();
    }
    let mut value = serde_json::to_value(local).unwrap_or(serde_json::json!({}));
    if let (Some(dst), Some(src)) = (value.as_object_mut(), remote.as_object()) {
        for (k, v) in src {
            if DEVICE_PREF_FIELDS.contains(&k.as_str()) {
                continue;
            }
            dst.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(value).unwrap_or_else(|_| local.clone())
}

pub fn strip_device_fields(plan: &mut DegreePlan) {
    plan.student_name = None;
    plan.student_id = None;
}

pub fn strip_watch_checks(item: &mut WatchItem) {
    item.last_checked_at = None;
    item.last_notified_at = None;
    item.last_seats_remaining = 0;
    item.last_waitlist_remaining = 0;
}

pub fn ensure_one_primary_plans(plans: &mut [SchedulePlan]) {
    let mut seen = false;
    for plan in plans.iter_mut() {
        if plan.is_primary {
            if seen {
                plan.is_primary = false;
            } else {
                seen = true;
            }
        }
    }
}

pub fn export_snapshot(data: &UserData) -> Vec<Record> {
    let mut records = vec![];
    let prefs = serde_json::to_value(&data.preferences).unwrap_or(serde_json::json!({}));
    records.push(Record {
        key: "preferences".into(),
        kind: "preferences".into(),
        updated_at: 0.0,
        deleted: false,
        payload: prefs,
    });
    for plan in &data.plans {
        records.push(Record {
            key: record_key_plan(&plan.id.to_string()),
            kind: "schedule".into(),
            updated_at: plan.updated_at,
            deleted: false,
            payload: serde_json::to_value(plan).unwrap_or(serde_json::json!({})),
        });
    }
    for plan in &data.degree_plans {
        let mut copy = plan.clone();
        strip_device_fields(&mut copy);
        records.push(Record {
            key: record_key_degree(&plan.id.to_string()),
            kind: "degree".into(),
            updated_at: 0.0,
            deleted: false,
            payload: serde_json::to_value(&copy).unwrap_or(serde_json::json!({})),
        });
    }
    records
}

// --- Cloud sync (the pure half of CloudSyncModel / CloudSyncState / CloudSyncStatus) ---

pub const SCHEMA_VERSION: i32 = 1;
/// January 1 of year 1, the same sentinel Swift uses for `Date.distantPast`.
pub const DISTANT_PAST: f64 = -62_135_596_800.0;

/// Linux stand-in for the CloudKit entitlement check: this build never reads an iCloud container id.
pub fn container_identifier() -> Option<&'static str> {
    None
}

pub fn consults_container_identifier() -> bool {
    false
}

pub fn consults_code_signature() -> bool {
    false
}

/// Linux stand-in for the CloudKit entitlement check: sync does not read a code signature.
pub fn code_signature_entitlements(_bytes: &[u8]) -> Option<Vec<u8>> {
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecordType {
    #[serde(rename = "SchedulePlan")]
    SchedulePlan,
    #[serde(rename = "DegreePlan")]
    DegreePlan,
    #[serde(rename = "WatchItem")]
    WatchItem,
    #[serde(rename = "SavedCourse")]
    SavedCourse,
    #[serde(rename = "Preferences")]
    Preferences,
}

impl RecordType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SchedulePlan => "SchedulePlan",
            Self::DegreePlan => "DegreePlan",
            Self::WatchItem => "WatchItem",
            Self::SavedCourse => "SavedCourse",
            Self::Preferences => "Preferences",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CloudSyncKey {
    pub record_type: RecordType,
    pub record_name: String,
}

impl CloudSyncKey {
    pub fn new(record_type: RecordType, record_name: impl Into<String>) -> Self {
        Self {
            record_type,
            record_name: normalize_record_name(record_name.into()),
        }
    }

    pub fn preferences() -> Self {
        Self::new(RecordType::Preferences, "preferences")
    }

    /// CloudKit record names are 1–255 printable ASCII characters that do not start with `_`.
    pub fn record_name_for_id(id: &str) -> String {
        let usable = !id.is_empty()
            && id.len() <= 255
            && !id.starts_with('_')
            && id.chars().all(|ch| {
                let value = ch as u32;
                (0x20..0x7f).contains(&value)
            });
        if usable {
            normalize_record_name(id.to_string())
        } else {
            format!("sha256-{}", hex_digest(id.as_bytes()))
        }
    }
}

fn normalize_record_name(name: String) -> String {
    match Uuid::parse_str(&name) {
        Ok(id) => id.to_string(),
        Err(_) => name,
    }
}

impl PartialOrd for CloudSyncKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CloudSyncKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.record_type
            .as_str()
            .cmp(other.record_type.as_str())
            .then(self.record_name.cmp(&other.record_name))
    }
}

impl Serialize for CloudSyncKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("CloudSyncKey", 2)?;
        state.serialize_field("type", &self.record_type)?;
        state.serialize_field("recordName", &self.record_name)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for CloudSyncKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = CloudSyncKey;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "cloud sync key")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut record_type = None;
                let mut record_name = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "type" => record_type = Some(map.next_value()?),
                        "recordName" => record_name = Some(map.next_value::<String>()?),
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }
                let record_type = record_type.ok_or_else(|| de::Error::missing_field("type"))?;
                let record_name =
                    record_name.ok_or_else(|| de::Error::missing_field("recordName"))?;
                Ok(CloudSyncKey::new(record_type, record_name))
            }
        }
        deserializer.deserialize_map(V)
    }
}

fn same_record(left: &CloudSyncKey, right: &CloudSyncKey) -> bool {
    left.record_type == right.record_type
        && left.record_name.eq_ignore_ascii_case(&right.record_name)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entity {
    SchedulePlan(SchedulePlan),
    DegreePlan(DegreePlan),
    WatchItem(WatchItem),
    SavedCourse(SavedCourse),
    Preferences(AppPreferences),
}

impl Entity {
    pub fn key(&self) -> CloudSyncKey {
        match self {
            Self::SchedulePlan(plan) => {
                CloudSyncKey::new(RecordType::SchedulePlan, plan.id.to_string())
            }
            Self::DegreePlan(plan) => {
                CloudSyncKey::new(RecordType::DegreePlan, plan.id.to_string())
            }
            Self::WatchItem(item) => CloudSyncKey::new(RecordType::WatchItem, item.id()),
            Self::SavedCourse(course) => CloudSyncKey::new(
                RecordType::SavedCourse,
                format!("{}|{}", course.term_code, course.key),
            ),
            Self::Preferences(_) => CloudSyncKey::preferences(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RemoteValue {
    SchedulePlan(SchedulePlan),
    DegreePlan(DegreePlan),
    WatchItem(WatchItem),
    SavedCourse(SavedCourse),
    Preferences {
        values: SyncedPreferences,
        changed_at: BTreeMap<String, f64>,
    },
}

impl RemoteValue {
    pub fn key(&self) -> CloudSyncKey {
        match self {
            Self::SchedulePlan(plan) => Entity::SchedulePlan(plan.clone()).key(),
            Self::DegreePlan(plan) => Entity::DegreePlan(plan.clone()).key(),
            Self::WatchItem(item) => Entity::WatchItem(item.clone()).key(),
            Self::SavedCourse(course) => Entity::SavedCourse(course.clone()).key(),
            Self::Preferences { .. } => CloudSyncKey::preferences(),
        }
    }
}

const PREF_FIELDS: [&str; 4] = [
    "preferredCampus",
    "showWeekends",
    "calendarStartHour",
    "calendarEndHour",
];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncedPreferences {
    pub preferred_campus: Option<String>,
    pub show_weekends: bool,
    pub calendar_start_hour: i32,
    pub calendar_end_hour: i32,
}

impl SyncedPreferences {
    pub fn from_app(preferences: &AppPreferences) -> Self {
        Self {
            preferred_campus: preferences.preferred_campus.clone(),
            show_weekends: preferences.show_weekends,
            calendar_start_hour: preferences.calendar_start_hour,
            calendar_end_hour: preferences.calendar_end_hour,
        }
    }

    pub fn applied(&self, preferences: &AppPreferences) -> AppPreferences {
        let mut result = preferences.clone();
        result.preferred_campus = self.preferred_campus.clone();
        result.show_weekends = self.show_weekends;
        result.calendar_start_hour = self.calendar_start_hour;
        result.calendar_end_hour = self.calendar_end_hour;
        result
    }

    pub fn fields_differing<'a>(&self, other: &'a Self) -> Vec<&'a str> {
        PREF_FIELDS
            .into_iter()
            .filter(|field| !pref_field_eq(field, self, other))
            .collect()
    }

    pub fn merge(
        local: &Self,
        local_changed_at: &BTreeMap<String, f64>,
        remote: &Self,
        remote_changed_at: &BTreeMap<String, f64>,
    ) -> PrefMerge {
        let defaults = Self::from_app(&AppPreferences::default());
        let mut values = remote.clone();
        let mut changed_at = remote_changed_at.clone();
        for field in PREF_FIELDS {
            let local_wins = match (local_changed_at.get(field), remote_changed_at.get(field)) {
                (Some(local_time), Some(remote_time)) => local_time > remote_time,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => {
                    let local_default = pref_field_eq(field, local, &defaults);
                    let remote_default = pref_field_eq(field, remote, &defaults);
                    remote_default && !local_default
                }
            };
            if !local_wins {
                continue;
            }
            copy_pref_field(field, &mut values, local);
            if let Some(local_time) = local_changed_at.get(field) {
                changed_at.insert(field.to_string(), *local_time);
            }
        }
        PrefMerge { values, changed_at }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrefMerge {
    pub values: SyncedPreferences,
    pub changed_at: BTreeMap<String, f64>,
}

fn pref_field_eq(field: &str, left: &SyncedPreferences, right: &SyncedPreferences) -> bool {
    match field {
        "preferredCampus" => left.preferred_campus == right.preferred_campus,
        "showWeekends" => left.show_weekends == right.show_weekends,
        "calendarStartHour" => left.calendar_start_hour == right.calendar_start_hour,
        "calendarEndHour" => left.calendar_end_hour == right.calendar_end_hour,
        _ => true,
    }
}

fn copy_pref_field(field: &str, dest: &mut SyncedPreferences, source: &SyncedPreferences) {
    match field {
        "preferredCampus" => dest.preferred_campus = source.preferred_campus.clone(),
        "showWeekends" => dest.show_weekends = source.show_weekends,
        "calendarStartHour" => dest.calendar_start_hour = source.calendar_start_hour,
        "calendarEndHour" => dest.calendar_end_hour = source.calendar_end_hour,
        _ => {}
    }
}

impl<'de> Deserialize<'de> for SyncedPreferences {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(Self::from_value(&value))
    }
}

impl SyncedPreferences {
    fn from_value(value: &serde_json::Value) -> Self {
        let defaults = Self::from_app(&AppPreferences::default());
        let obj = value.as_object();
        let preferred_campus = obj
            .and_then(|map| map.get("preferredCampus"))
            .and_then(|item| item.as_str())
            .map(|text| text.to_string());
        let show_weekends = obj
            .and_then(|map| map.get("showWeekends"))
            .and_then(|item| item.as_bool())
            .unwrap_or(defaults.show_weekends);
        let start = obj
            .and_then(|map| map.get("calendarStartHour"))
            .and_then(|item| item.as_i64())
            .map(|n| n as i32);
        let end = obj
            .and_then(|map| map.get("calendarEndHour"))
            .and_then(|item| item.as_i64())
            .map(|n| n as i32);
        let start = start.unwrap_or(defaults.calendar_start_hour);
        let end = end.unwrap_or(defaults.calendar_end_hour);
        let (calendar_start_hour, calendar_end_hour) =
            if (0..=23).contains(&start) && (1..=24).contains(&end) && start < end {
                (start, end)
            } else {
                (defaults.calendar_start_hour, defaults.calendar_end_hour)
            };
        Self {
            preferred_campus,
            show_weekends,
            calendar_start_hour,
            calendar_end_hour,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesPayload {
    values: SyncedPreferences,
    #[serde(default)]
    changed_at: BTreeMap<String, f64>,
}

pub fn payload_for(
    entity: &Entity,
    preference_changed_at: &BTreeMap<String, f64>,
) -> Result<Vec<u8>, String> {
    let value = match entity {
        Entity::SchedulePlan(plan) => serde_json::to_value(plan).map_err(|e| e.to_string())?,
        Entity::DegreePlan(plan) => {
            serde_json::to_value(synced_form(plan)).map_err(|e| e.to_string())?
        }
        Entity::WatchItem(item) => serde_json::to_value(item).map_err(|e| e.to_string())?,
        Entity::SavedCourse(course) => serde_json::to_value(course).map_err(|e| e.to_string())?,
        Entity::Preferences(preferences) => serde_json::to_value(&PreferencesPayload {
            values: SyncedPreferences::from_app(preferences),
            changed_at: preference_changed_at.clone(),
        })
        .map_err(|e| e.to_string())?,
    };
    Ok(canonical_bytes(value))
}

pub fn decode(payload: &[u8], record_type: RecordType) -> Result<RemoteValue, String> {
    match record_type {
        RecordType::SchedulePlan => Ok(RemoteValue::SchedulePlan(
            serde_json::from_slice(payload).map_err(|e| e.to_string())?,
        )),
        RecordType::DegreePlan => {
            let plan: DegreePlan = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
            Ok(RemoteValue::DegreePlan(synced_form(&plan)))
        }
        RecordType::WatchItem => Ok(RemoteValue::WatchItem(
            serde_json::from_slice(payload).map_err(|e| e.to_string())?,
        )),
        RecordType::SavedCourse => Ok(RemoteValue::SavedCourse(
            serde_json::from_slice(payload).map_err(|e| e.to_string())?,
        )),
        RecordType::Preferences => {
            let decoded: PreferencesPayload =
                serde_json::from_slice(payload).map_err(|e| e.to_string())?;
            Ok(RemoteValue::Preferences {
                values: decoded.values,
                changed_at: decoded.changed_at,
            })
        }
    }
}

pub fn fingerprint_of(entity: &Entity) -> String {
    let value = match entity {
        Entity::SchedulePlan(plan) => serde_json::to_value(plan).unwrap_or(serde_json::json!({})),
        Entity::DegreePlan(plan) => {
            serde_json::to_value(synced_form(plan)).unwrap_or(serde_json::json!({}))
        }
        Entity::WatchItem(item) => {
            serde_json::to_value(tracked_form(item)).unwrap_or(serde_json::json!({}))
        }
        Entity::SavedCourse(course) => {
            serde_json::to_value(course).unwrap_or(serde_json::json!({}))
        }
        Entity::Preferences(preferences) => {
            serde_json::to_value(SyncedPreferences::from_app(preferences))
                .unwrap_or(serde_json::json!({}))
        }
    };
    hex_digest(&canonical_bytes(value))
}

pub fn fingerprint_of_remote(value: &RemoteValue) -> String {
    match value {
        RemoteValue::SchedulePlan(plan) => fingerprint_of(&Entity::SchedulePlan(plan.clone())),
        RemoteValue::DegreePlan(plan) => fingerprint_of(&Entity::DegreePlan(plan.clone())),
        RemoteValue::WatchItem(item) => fingerprint_of(&Entity::WatchItem(item.clone())),
        RemoteValue::SavedCourse(course) => fingerprint_of(&Entity::SavedCourse(course.clone())),
        RemoteValue::Preferences { values, .. } => {
            let json = serde_json::to_value(values).unwrap_or(serde_json::json!({}));
            hex_digest(&canonical_bytes(json))
        }
    }
}

pub fn modified_at(
    entity: &Entity,
    local_changed_at: Option<f64>,
    preference_changed_at: &BTreeMap<String, f64>,
) -> f64 {
    match entity {
        Entity::SchedulePlan(plan) => plan
            .updated_at
            .max(local_changed_at.unwrap_or(DISTANT_PAST)),
        Entity::DegreePlan(plan) => {
            let updated = plan
                .updated_at
                .as_deref()
                .and_then(iso8601_to_unix)
                .unwrap_or(DISTANT_PAST);
            updated.max(local_changed_at.unwrap_or(DISTANT_PAST))
        }
        Entity::WatchItem(item) => local_changed_at.unwrap_or(item.added_at),
        Entity::SavedCourse(course) => local_changed_at.unwrap_or(course.saved_at),
        Entity::Preferences(_) => preference_changed_at
            .values()
            .copied()
            .max_by(f64::total_cmp)
            .unwrap_or(local_changed_at.unwrap_or(DISTANT_PAST)),
    }
}

pub fn synced_form(plan: &DegreePlan) -> DegreePlan {
    let mut plan = plan.clone();
    plan.student_name = None;
    plan.student_id = None;
    plan
}

fn tracked_form(item: &WatchItem) -> WatchItem {
    let mut item = item.clone();
    item.last_seats_remaining = 0;
    item.last_waitlist_remaining = 0;
    item.seat_capacity = 0;
    item.last_checked_at = None;
    item.last_notified_at = None;
    item
}

fn canonical_bytes(mut value: serde_json::Value) -> Vec<u8> {
    crate::store::floor_user_dates(&mut value);
    sort_excluded(&mut value);
    serde_json::to_vec(&value).unwrap_or_default()
}

fn sort_excluded(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for name in ["excludedCRNs", "excludedCrns"] {
                if let Some(serde_json::Value::Array(items)) = map.get_mut(name) {
                    items.sort_by(|left, right| {
                        left.as_str()
                            .unwrap_or("")
                            .cmp(right.as_str().unwrap_or(""))
                    });
                }
            }
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(child) = map.get_mut(&key) {
                    sort_excluded(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                sort_excluded(item);
            }
        }
        _ => {}
    }
}

#[derive(Clone, Debug)]
pub struct SnapshotItem {
    pub entity: Entity,
    pub fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub items: BTreeMap<CloudSyncKey, SnapshotItem>,
    pub preferences: AppPreferences,
}

impl Snapshot {
    pub fn from_data(data: &UserData) -> Self {
        let mut items = BTreeMap::new();
        let mut add = |entity: Entity| {
            let key = entity.key();
            if items.contains_key(&key) {
                return;
            }
            let fingerprint = fingerprint_of(&entity);
            items.insert(
                key,
                SnapshotItem {
                    entity,
                    fingerprint,
                },
            );
        };
        add(Entity::Preferences(data.preferences.clone()));
        for plan in &data.plans {
            add(Entity::SchedulePlan(plan.clone()));
        }
        for plan in &data.degree_plans {
            add(Entity::DegreePlan(plan.clone()));
        }
        for item in &data.watchlist {
            add(Entity::WatchItem(item.clone()));
        }
        for course in &data.saved {
            add(Entity::SavedCourse(course.clone()));
        }
        Self {
            items,
            preferences: data.preferences.clone(),
        }
    }

    pub fn get(&self, key: &CloudSyncKey) -> Option<&SnapshotItem> {
        self.items.get(key)
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_changed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_fields: Option<Vec<u8>>,
    #[serde(default)]
    pub is_server_copy_unreadable: bool,
}

impl RecordState {
    pub fn is_known_in_icloud(&self) -> bool {
        self.server_fingerprint.is_some()
            || self.system_fields.is_some()
            || self.is_server_copy_unreadable
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Metadata {
    pub records: BTreeMap<CloudSyncKey, RecordState>,
    pub has_observed_local_data: bool,
    pub observed_preferences: Option<SyncedPreferences>,
    pub preference_changed_at: BTreeMap<String, f64>,
    pub account_record_name: Option<String>,
    pub is_zone_saved: bool,
    pub last_sync_date: Option<f64>,
}

impl Metadata {
    pub fn has_data_in_icloud(&self) -> bool {
        self.records
            .values()
            .any(|state| state.is_known_in_icloud())
    }

    pub fn observe_local_data(&mut self, snapshot: &Snapshot, now: f64) {
        let first = !self.has_observed_local_data;
        for (key, item) in &snapshot.items {
            let mut state = self.records.get(key).cloned().unwrap_or_default();
            if state.local_fingerprint.as_deref() == Some(item.fingerprint.as_str()) {
                continue;
            }
            state.local_fingerprint = Some(item.fingerprint.clone());
            state.local_changed_at = if first { None } else { Some(now) };
            self.records.insert(key.clone(), state);
        }
        let missing: Vec<CloudSyncKey> = self
            .records
            .keys()
            .filter(|key| !snapshot.items.contains_key(key))
            .cloned()
            .collect();
        for key in missing {
            let Some(state) = self.records.get(&key).cloned() else {
                continue;
            };
            if state.local_fingerprint.is_none() {
                continue;
            }
            let mut deleted = state;
            deleted.local_fingerprint = None;
            deleted.local_changed_at = if first { None } else { Some(now) };
            if deleted.is_known_in_icloud() {
                self.records.insert(key, deleted);
            } else {
                self.records.remove(&key);
            }
        }
        let preferences = SyncedPreferences::from_app(&snapshot.preferences);
        if let Some(observed) = &self.observed_preferences.clone() {
            if !first {
                for field in preferences.fields_differing(observed) {
                    self.preference_changed_at.insert(field.to_string(), now);
                }
            }
        }
        self.observed_preferences = Some(preferences);
        self.has_observed_local_data = true;
    }

    pub fn pending_changes(&self, snapshot: &Snapshot) -> PendingChanges {
        let mut saves = Vec::new();
        let mut deletes = Vec::new();
        for (key, item) in &snapshot.items {
            let state = self.records.get(key);
            if state.is_some_and(|state| state.is_server_copy_unreadable) {
                continue;
            }
            if state.and_then(|state| state.server_fingerprint.as_deref())
                != Some(item.fingerprint.as_str())
            {
                saves.push(key.clone());
            }
        }
        for (key, state) in &self.records {
            if snapshot.items.contains_key(key) {
                continue;
            }
            if state.is_known_in_icloud() && !state.is_server_copy_unreadable {
                deletes.push(key.clone());
            }
        }
        saves.sort();
        deletes.sort();
        PendingChanges { saves, deletes }
    }

    pub fn forget_icloud(&mut self) {
        let keys: Vec<CloudSyncKey> = self.records.keys().cloned().collect();
        for key in keys {
            let Some(state) = self.records.get(&key).cloned() else {
                continue;
            };
            let mut local = state;
            local.server_fingerprint = None;
            local.system_fields = None;
            local.is_server_copy_unreadable = false;
            if local.local_fingerprint.is_none() {
                self.records.remove(&key);
            } else {
                self.records.insert(key, local);
            }
        }
        self.is_zone_saved = false;
        self.last_sync_date = None;
    }

    pub fn record_saved(
        &mut self,
        key: &CloudSyncKey,
        fingerprint: Option<&str>,
        system_fields: Option<&[u8]>,
    ) {
        let mut state = self.records.get(key).cloned().unwrap_or_default();
        state.server_fingerprint = fingerprint.map(|text| text.to_string());
        state.system_fields = system_fields.map(|bytes| bytes.to_vec());
        state.is_server_copy_unreadable = false;
        self.records.insert(key.clone(), state);
    }

    pub fn record_deleted_from_icloud(&mut self, key: &CloudSyncKey) {
        let Some(state) = self.records.get(key).cloned() else {
            return;
        };
        let mut state = state;
        state.server_fingerprint = None;
        state.system_fields = None;
        state.is_server_copy_unreadable = false;
        if state.local_fingerprint.is_none() {
            self.records.remove(key);
        } else {
            self.records.insert(key.clone(), state);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataDto<'a> {
    records: Vec<MetadataEntry>,
    has_observed_local_data: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_preferences: Option<&'a SyncedPreferences>,
    preference_changed_at: &'a BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_record_name: Option<&'a String>,
    is_zone_saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_sync_date: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct MetadataEntry {
    key: CloudSyncKey,
    state: RecordState,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetadataDtoOwned {
    #[serde(default)]
    records: Vec<MetadataEntry>,
    #[serde(default)]
    has_observed_local_data: bool,
    #[serde(default)]
    observed_preferences: Option<SyncedPreferences>,
    #[serde(default)]
    preference_changed_at: BTreeMap<String, f64>,
    #[serde(default)]
    account_record_name: Option<String>,
    #[serde(default)]
    is_zone_saved: bool,
    #[serde(default)]
    last_sync_date: Option<f64>,
}

impl Serialize for Metadata {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let records = self
            .records
            .iter()
            .map(|(key, state)| MetadataEntry {
                key: key.clone(),
                state: state.clone(),
            })
            .collect();
        MetadataDto {
            records,
            has_observed_local_data: self.has_observed_local_data,
            observed_preferences: self.observed_preferences.as_ref(),
            preference_changed_at: &self.preference_changed_at,
            account_record_name: self.account_record_name.as_ref(),
            is_zone_saved: self.is_zone_saved,
            last_sync_date: self.last_sync_date,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Metadata {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let dto = MetadataDtoOwned::deserialize(deserializer)?;
        let mut records = BTreeMap::new();
        for entry in dto.records {
            records.entry(entry.key).or_insert(entry.state);
        }
        Ok(Self {
            records,
            has_observed_local_data: dto.has_observed_local_data,
            observed_preferences: dto.observed_preferences,
            preference_changed_at: dto.preference_changed_at,
            account_record_name: dto.account_record_name,
            is_zone_saved: dto.is_zone_saved,
            last_sync_date: dto.last_sync_date,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PendingChanges {
    pub saves: Vec<CloudSyncKey>,
    pub deletes: Vec<CloudSyncKey>,
}

impl PendingChanges {
    pub fn is_empty(&self) -> bool {
        self.saves.is_empty() && self.deletes.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct IncomingRecord {
    pub key: CloudSyncKey,
    pub schema_version: i32,
    pub modified_at: f64,
    pub payload: Option<Vec<u8>>,
    pub system_fields: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergeResult {
    pub saves: Vec<Entity>,
    pub deletes: Vec<CloudSyncKey>,
    pub kept_local: Vec<CloudSyncKey>,
    pub skipped: Vec<CloudSyncKey>,
}

impl MergeResult {
    pub fn changes_local_data(&self) -> bool {
        !self.saves.is_empty() || !self.deletes.is_empty()
    }
}

pub fn merge(
    modifications: &[IncomingRecord],
    deletions: &[CloudSyncKey],
    snapshot: &Snapshot,
    metadata: &mut Metadata,
    now: f64,
) -> MergeResult {
    let mut working = Working::new(snapshot);
    let mut result = MergeResult::default();
    let mut applied = Vec::new();

    for record in modifications {
        let key = record.key.clone();
        let mut state = metadata.records.get(&key).cloned().unwrap_or_default();
        let was_known = state.is_known_in_icloud();
        let previous_server = state.server_fingerprint.clone();
        let remote = record
            .payload
            .as_deref()
            .and_then(|payload| decode(payload, key.record_type).ok());
        let readable = record.schema_version <= SCHEMA_VERSION
            && remote
                .as_ref()
                .is_some_and(|value| same_record(&value.key(), &key));
        if !readable {
            state.is_server_copy_unreadable = true;
            state.system_fields = record.system_fields.clone();
            metadata.records.insert(key.clone(), state);
            result.skipped.push(key);
            continue;
        }
        let remote = remote.expect("readable remote");
        let remote_fingerprint = fingerprint_of_remote(&remote);
        state.is_server_copy_unreadable = false;
        state.system_fields = record.system_fields.clone();
        state.server_fingerprint = Some(remote_fingerprint.clone());

        if let RemoteValue::Preferences { values, changed_at } = &remote {
            let local = SyncedPreferences::from_app(&working.preferences);
            let merged = SyncedPreferences::merge(
                &local,
                &metadata.preference_changed_at,
                values,
                changed_at,
            );
            if merged.values != local {
                working.write(Entity::Preferences(
                    merged.values.applied(&working.preferences),
                ));
            }
            metadata.preference_changed_at = merged.changed_at.clone();
            metadata.observed_preferences = Some(merged.values.clone());
            state.local_fingerprint = Some(fingerprint_of(&Entity::Preferences(
                working.preferences.clone(),
            )));
            state.local_changed_at = merged.changed_at.values().copied().max_by(f64::total_cmp);
            metadata.records.insert(key.clone(), state);
            if merged.values != *values {
                result.kept_local.push(key);
            }
            continue;
        }

        let Some(current) = working.local.get(&key).cloned() else {
            if was_known
                && state.local_fingerprint.is_none()
                && state
                    .local_changed_at
                    .is_some_and(|deleted_at| deleted_at > record.modified_at)
            {
                metadata.records.insert(key.clone(), state);
                result.kept_local.push(key);
                continue;
            }
            let value = local_value(&remote, None);
            let fingerprint = fingerprint_of(&value);
            working.write(value);
            state.local_fingerprint = Some(fingerprint);
            state.local_changed_at = Some(record.modified_at);
            metadata.records.insert(key.clone(), state);
            applied.push(key);
            continue;
        };

        let local_fingerprint = fingerprint_of(&current);
        if local_fingerprint == remote_fingerprint {
            state.local_fingerprint = Some(local_fingerprint);
            metadata.records.insert(key, state);
            continue;
        }
        if previous_server.as_deref() != Some(local_fingerprint.as_str()) {
            let local_time = modified_at(
                &current,
                state.local_changed_at,
                &metadata.preference_changed_at,
            );
            if local_time > record.modified_at {
                metadata.records.insert(key.clone(), state);
                result.kept_local.push(key);
                continue;
            }
        }
        let value = local_value(&remote, Some(&current));
        let fingerprint = fingerprint_of(&value);
        working.write(value);
        state.local_fingerprint = Some(fingerprint);
        state.local_changed_at = Some(record.modified_at);
        metadata.records.insert(key.clone(), state);
        applied.push(key);
    }

    for key in deletions {
        let state = metadata.records.get(key).cloned();
        if key.record_type == RecordType::Preferences {
            let mut kept = state.unwrap_or_default();
            kept.server_fingerprint = None;
            kept.system_fields = None;
            kept.is_server_copy_unreadable = false;
            metadata.records.insert(key.clone(), kept);
            result.kept_local.push(key.clone());
            continue;
        }
        let Some(current) = working.local.get(key).cloned() else {
            metadata.records.remove(key);
            continue;
        };
        let local_fingerprint = fingerprint_of(&current);
        if state
            .as_ref()
            .and_then(|item| item.server_fingerprint.as_deref())
            == Some(local_fingerprint.as_str())
        {
            working.delete(key);
            metadata.records.remove(key);
        } else {
            let mut kept = state.unwrap_or_default();
            kept.server_fingerprint = None;
            kept.system_fields = None;
            kept.is_server_copy_unreadable = false;
            kept.local_fingerprint = Some(local_fingerprint);
            metadata.records.insert(key.clone(), kept);
            result.kept_local.push(key.clone());
        }
    }

    normalize_primaries(&applied, &mut working, metadata, now);
    let mut saves: Vec<Entity> = working.written.into_values().collect();
    saves.sort_by_key(|left| left.key());
    let mut deleted: Vec<CloudSyncKey> = working.deleted.into_iter().collect();
    deleted.sort();
    result.saves = saves;
    result.deletes = deleted;
    result
}

pub fn local_value(remote: &RemoteValue, current: Option<&Entity>) -> Entity {
    match remote {
        RemoteValue::SchedulePlan(plan) => Entity::SchedulePlan(plan.clone()),
        RemoteValue::DegreePlan(plan) => {
            let mut plan = plan.clone();
            if let Some(Entity::DegreePlan(local)) = current {
                plan.student_name = local.student_name.clone();
                plan.student_id = local.student_id.clone();
            }
            Entity::DegreePlan(plan)
        }
        RemoteValue::WatchItem(item) => {
            let mut item = item.clone();
            if let Some(Entity::WatchItem(local)) = current {
                item.last_seats_remaining = local.last_seats_remaining;
                item.last_waitlist_remaining = local.last_waitlist_remaining;
                item.seat_capacity = local.seat_capacity;
                item.last_checked_at = local.last_checked_at;
                item.last_notified_at = local.last_notified_at;
            }
            Entity::WatchItem(item)
        }
        RemoteValue::SavedCourse(course) => Entity::SavedCourse(course.clone()),
        RemoteValue::Preferences { values, .. } => {
            let base = match current {
                Some(Entity::Preferences(local)) => local.clone(),
                _ => AppPreferences::default(),
            };
            Entity::Preferences(values.applied(&base))
        }
    }
}

fn normalize_primaries(
    applied: &[CloudSyncKey],
    working: &mut Working,
    metadata: &mut Metadata,
    now: f64,
) {
    let mut terms = BTreeMap::<String, ()>::new();
    let mut touches_degrees = false;
    for key in applied {
        match working.local.get(key) {
            Some(Entity::SchedulePlan(plan)) => {
                terms.insert(plan.term_code.clone(), ());
            }
            Some(Entity::DegreePlan(_)) => touches_degrees = true,
            _ => {}
        }
    }
    for term in terms.keys() {
        let primaries: Vec<CloudSyncKey> = working
            .local
            .iter()
            .filter_map(|(key, entity)| match entity {
                Entity::SchedulePlan(plan) if plan.term_code == *term && plan.is_primary => {
                    Some(key.clone())
                }
                _ => None,
            })
            .collect();
        demote_all_but_latest(&primaries, working, metadata, now);
    }
    if touches_degrees {
        let primaries: Vec<CloudSyncKey> = working
            .local
            .iter()
            .filter_map(|(key, entity)| match entity {
                Entity::DegreePlan(plan) if plan.is_primary => Some(key.clone()),
                _ => None,
            })
            .collect();
        demote_all_but_latest(&primaries, working, metadata, now);
    }
}

fn demote_all_but_latest(
    primaries: &[CloudSyncKey],
    working: &mut Working,
    metadata: &mut Metadata,
    now: f64,
) {
    if primaries.len() < 2 {
        return;
    }
    let mut keep = primaries[0].clone();
    for key in primaries.iter().skip(1) {
        if rank_after(metadata, working, key, &keep) {
            keep = key.clone();
        }
    }
    for key in primaries {
        if key == &keep {
            continue;
        }
        let Some(entity) = working.local.get(key).cloned() else {
            continue;
        };
        let demoted = match entity {
            Entity::SchedulePlan(mut plan) => {
                plan.is_primary = false;
                Entity::SchedulePlan(plan)
            }
            Entity::DegreePlan(mut plan) => {
                plan.is_primary = false;
                Entity::DegreePlan(plan)
            }
            other => other,
        };
        working.write(demoted.clone());
        let mut state = metadata.records.get(key).cloned().unwrap_or_default();
        state.local_fingerprint = Some(fingerprint_of(&demoted));
        state.local_changed_at = Some(now);
        metadata.records.insert(key.clone(), state);
    }
}

fn rank_after(
    metadata: &Metadata,
    working: &Working,
    candidate: &CloudSyncKey,
    current: &CloudSyncKey,
) -> bool {
    let Some(candidate_entity) = working.local.get(candidate) else {
        return false;
    };
    let Some(current_entity) = working.local.get(current) else {
        return true;
    };
    let candidate_time = modified_at(
        candidate_entity,
        metadata
            .records
            .get(candidate)
            .and_then(|state| state.local_changed_at),
        &metadata.preference_changed_at,
    );
    let current_time = modified_at(
        current_entity,
        metadata
            .records
            .get(current)
            .and_then(|state| state.local_changed_at),
        &metadata.preference_changed_at,
    );
    if candidate_time != current_time {
        return candidate_time > current_time;
    }
    candidate.record_name > current.record_name
}

struct Working {
    local: HashMap<CloudSyncKey, Entity>,
    preferences: AppPreferences,
    written: HashMap<CloudSyncKey, Entity>,
    deleted: HashSet<CloudSyncKey>,
}

impl Working {
    fn new(snapshot: &Snapshot) -> Self {
        let local = snapshot
            .items
            .iter()
            .map(|(key, item)| (key.clone(), item.entity.clone()))
            .collect();
        Self {
            local,
            preferences: snapshot.preferences.clone(),
            written: HashMap::new(),
            deleted: HashSet::new(),
        }
    }

    fn write(&mut self, entity: Entity) {
        let key = entity.key();
        if let Entity::Preferences(preferences) = &entity {
            self.preferences = preferences.clone();
        }
        self.local.insert(key.clone(), entity.clone());
        self.written.insert(key.clone(), entity);
        self.deleted.remove(&key);
    }

    fn delete(&mut self, key: &CloudSyncKey) {
        self.local.remove(key);
        self.written.remove(key);
        self.deleted.insert(key.clone());
    }
}

pub fn applying_cloud_changes(
    data: &UserData,
    saves: &[Entity],
    deletes: &[CloudSyncKey],
) -> UserData {
    let mut data = data.clone();
    if !deletes.is_empty() {
        let removed: HashSet<&CloudSyncKey> = deletes.iter().collect();
        data.plans
            .retain(|plan| !removed.contains(&Entity::SchedulePlan(plan.clone()).key()));
        data.degree_plans
            .retain(|plan| !removed.contains(&Entity::DegreePlan(plan.clone()).key()));
        data.watchlist
            .retain(|item| !removed.contains(&Entity::WatchItem(item.clone()).key()));
        data.saved
            .retain(|course| !removed.contains(&Entity::SavedCourse(course.clone()).key()));
    }
    for entity in saves {
        match entity {
            Entity::SchedulePlan(plan) => {
                if let Some(index) = data.plans.iter().position(|item| item.id == plan.id) {
                    data.plans[index] = plan.clone();
                } else {
                    data.plans.push(plan.clone());
                }
            }
            Entity::DegreePlan(plan) => {
                if let Some(index) = data.degree_plans.iter().position(|item| item.id == plan.id) {
                    data.degree_plans[index] = plan.clone();
                } else {
                    data.degree_plans.push(plan.clone());
                }
            }
            Entity::WatchItem(item) => {
                if let Some(index) = data
                    .watchlist
                    .iter()
                    .position(|existing| existing.id() == item.id())
                {
                    data.watchlist[index] = item.clone();
                } else {
                    let index = data
                        .watchlist
                        .iter()
                        .position(|existing| existing.added_at < item.added_at)
                        .unwrap_or(data.watchlist.len());
                    data.watchlist.insert(index, item.clone());
                }
            }
            Entity::SavedCourse(course) => {
                let id = format!("{}|{}", course.term_code, course.key);
                if let Some(index) = data
                    .saved
                    .iter()
                    .position(|existing| format!("{}|{}", existing.term_code, existing.key) == id)
                {
                    data.saved[index] = course.clone();
                } else {
                    let index = data
                        .saved
                        .iter()
                        .position(|existing| existing.saved_at < course.saved_at)
                        .unwrap_or(data.saved.len());
                    data.saved.insert(index, course.clone());
                }
            }
            Entity::Preferences(preferences) => data.preferences = preferences.clone(),
        }
    }
    data
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    NoAccount,
    Restricted,
    NotConfigured,
    TemporarilyUnavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncPhase {
    Off,
    Unavailable(UnavailableReason),
    Syncing,
    UpToDate { last_sync: Option<f64> },
    Error(String),
}

impl SyncPhase {
    pub fn summary(&self, now: f64) -> String {
        match self {
            Self::Off => "Off".into(),
            Self::Unavailable(UnavailableReason::NoAccount) => {
                "Sign in to iCloud in Settings to sync".into()
            }
            Self::Unavailable(UnavailableReason::Restricted) => {
                "iCloud is restricted on this device".into()
            }
            Self::Unavailable(UnavailableReason::NotConfigured) => {
                "iCloud sync isn't set up in this build".into()
            }
            Self::Unavailable(UnavailableReason::TemporarilyUnavailable) => {
                "Check your Apple Account in Settings to sync".into()
            }
            Self::Syncing => "Syncing…".into(),
            Self::UpToDate { last_sync: None } => "Up to date".into(),
            Self::UpToDate {
                last_sync: Some(last),
            } => format!("Up to date · {}", relative_label(*last, now)),
            Self::Error(message) => message.clone(),
        }
    }
}

fn relative_label(date: f64, now: f64) -> String {
    let date = date.min(now);
    let seconds = now - date;
    if seconds < 60.0 {
        "just now".into()
    } else if seconds < 3_600.0 {
        format!("{} min ago", (seconds / 60.0) as i64)
    } else if seconds < 86_400.0 {
        format!("{} hr ago", (seconds / 3_600.0) as i64)
    } else {
        let days = (seconds / 86_400.0) as i64;
        if days == 1 {
            "yesterday".into()
        } else {
            format!("{days} days ago")
        }
    }
}

pub fn unix_to_iso8601(seconds: f64) -> String {
    let whole = seconds.floor() as i64;
    let day = whole.div_euclid(86_400);
    let tod = whole.rem_euclid(86_400);
    let (year, month, date) = civil_from_days(day);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    format!("{year:04}-{month:02}-{date:02}T{hour:02}:{minute:02}:{second:02}Z")
}

pub fn iso8601_to_unix(text: &str) -> Option<f64> {
    let text = text.trim().trim_end_matches('Z');
    let (date, time) = text.split_once('T').unwrap_or((text, "00:00:00"));
    let mut parts = date.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: i32 = parts.next()?.parse().ok()?;
    let day: i32 = parts.next()?.parse().ok()?;
    let mut clock = time.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock
        .next()
        .unwrap_or("0")
        .trim_end_matches("Z")
        .parse()
        .ok()?;
    Some((days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second) as f64)
}

fn days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let mut year_i = i64::from(year);
    if month <= 2 {
        year_i -= 1;
    }
    let month = month as u32;
    let day = day as u32;
    let era = if year_i >= 0 { year_i } else { year_i - 399 } / 400;
    let yoe = (year_i - era * 400) as u64;
    let doy = (153 * u64::from(if month > 2 { month - 3 } else { month + 9 }) + 2) / 5
        + u64::from(day)
        - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let mut year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    (year as i32, month as u32, day as u32)
}

fn hex_digest(data: &[u8]) -> String {
    sha256(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut hash = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut words = [0_u32; 64];
        for i in 0..16 {
            words[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = words[i - 15].rotate_right(7)
                ^ words[i - 15].rotate_right(18)
                ^ (words[i - 15] >> 3);
            let s1 = words[i - 2].rotate_right(17)
                ^ words[i - 2].rotate_right(19)
                ^ (words[i - 2] >> 10);
            words[i] = words[i - 16]
                .wrapping_add(s0)
                .wrapping_add(words[i - 7])
                .wrapping_add(s1);
        }
        let mut a = hash[0];
        let mut b = hash[1];
        let mut c = hash[2];
        let mut d = hash[3];
        let mut e = hash[4];
        let mut f = hash[5];
        let mut g = hash[6];
        let mut h = hash[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(words[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }
    let mut out = [0_u8; 32];
    for (i, word) in hash.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}
