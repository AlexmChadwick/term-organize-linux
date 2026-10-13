//! Local folder sync standing in for iCloud. Last write wins. A remote delete is kept
//! unless the value was edited locally since. One primary plan. Device-only fields stay put.

use crate::model::*;
use std::collections::{HashMap, HashSet};

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
    "selectedTermCode", "hasCompletedOnboarding", "searchHistory", "folderSyncEnabled", "folderSyncPath",
    "backgroundRefreshEnabled", "seatCheckMinutes", "autoRefreshSeats",
];

pub fn record_key_plan(id: &str) -> String { format!("schedule:{id}") }
pub fn record_key_degree(id: &str) -> String { format!("degree:{id}") }
pub fn record_key_watch(id: &str) -> String { format!("watch:{id}") }
pub fn record_key_saved(term: &str, key: &CourseKey) -> String { format!("saved:{term}:{}", key) }

pub fn diff(local: &[Record], remote: &[Record]) -> (Vec<Record>, Vec<String>) {
    let remote_keys: HashSet<_> = remote.iter().map(|r| r.key.clone()).collect();
    let local_keys: HashSet<_> = local.iter().map(|r| r.key.clone()).collect();
    let mut changes = vec![];
    for rec in local {
        match remote.iter().find(|r| r.key == rec.key) {
            Some(other) if other.payload != rec.payload || other.deleted != rec.deleted => changes.push(rec.clone()),
            None => changes.push(rec.clone()),
            _ => {}
        }
    }
    let deletes = remote_keys.difference(&local_keys).cloned().collect();
    (changes, deletes)
}

/// Newer timestamp wins. A tie goes to the remote copy. A remote deletion removes a value
/// that was not edited locally after the deletion. A newer local edit keeps it.
pub fn merge_record(local: Option<&Record>, remote: Option<&Record>, local_edit: Option<f64>) -> Option<Record> {
    match (local, remote) {
        (None, None) => None,
        (Some(l), None) => Some(l.clone()),
        (None, Some(r)) => if r.deleted { None } else { Some(r.clone()) },
        (Some(l), Some(r)) => {
            if r.deleted {
                if local_edit.is_some_and(|t| t > r.updated_at) { Some(l.clone()) } else { None }
            } else if l.updated_at > r.updated_at {
                Some(l.clone())
            } else {
                Some(r.clone())
            }
        }
    }
}

pub fn merge_preferences(local: &AppPreferences, remote: &serde_json::Value, remote_newer: bool) -> AppPreferences {
    if !remote_newer { return local.clone(); }
    let mut value = serde_json::to_value(local).unwrap_or(serde_json::json!({}));
    if let (Some(dst), Some(src)) = (value.as_object_mut(), remote.as_object()) {
        for (k, v) in src {
            if DEVICE_PREF_FIELDS.contains(&k.as_str()) { continue; }
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
            if seen { plan.is_primary = false; } else { seen = true; }
        }
    }
}

pub fn export_snapshot(data: &UserData) -> Vec<Record> {
    let mut records = vec![];
    let prefs = serde_json::to_value(&data.preferences).unwrap_or(serde_json::json!({}));
    records.push(Record { key: "preferences".into(), kind: "preferences".into(), updated_at: 0.0, deleted: false, payload: prefs });
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
