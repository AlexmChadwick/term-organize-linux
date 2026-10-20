//! JSON user data under the XDG data directory, with one primary schedule and degree plan.
//!
//! The Flatpak app writes under `~/.var/app/org.termorganize.TermOrganize/data/termorganize`
//! because that is `$XDG_DATA_HOME/termorganize` inside the sandbox. A native or AppImage
//! build uses `$XDG_DATA_HOME/termorganize` or `~/.local/share/termorganize`. When
//! `TERMORGANIZE_DATA` is unset, [`data_dir`] uses whichever of those already has
//! `user.json` (the newer file if both do).

use crate::model::{DegreePlan, UserData};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const FLATPAK_APP_ID: &str = "org.termorganize.TermOrganize";
const LOCK_WAIT: Duration = Duration::from_secs(5);
const STALE_LOCK: Duration = Duration::from_secs(30);

pub fn flatpak_data_dir(home: &Path) -> PathBuf {
    home.join(".var/app")
        .join(FLATPAK_APP_ID)
        .join("data")
        .join("termorganize")
}

pub fn native_data_dir(home: Option<&Path>, xdg_data_home: Option<&Path>) -> PathBuf {
    if let Some(xdg) = xdg_data_home {
        return xdg.join("termorganize");
    }
    if let Some(home) = home {
        return home.join(".local/share/termorganize");
    }
    PathBuf::from("termorganize-data")
}

/// Pick the user-data directory. A location counts when it already contains `user.json`.
pub fn resolve_data_dir(home: Option<&Path>, xdg_data_home: Option<&Path>) -> PathBuf {
    let native = native_data_dir(home, xdg_data_home);
    let Some(home) = home else {
        return native;
    };
    let flatpak = flatpak_data_dir(home);
    match (user_file_mtime(&native), user_file_mtime(&flatpak)) {
        (Some(native_time), Some(flatpak_time)) if flatpak_time > native_time => flatpak,
        (None, Some(_)) => flatpak,
        _ => native,
    }
}

fn user_file_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join("user.json"))
        .and_then(|meta| meta.modified())
        .ok()
}

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("TERMORGANIZE_DATA") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    resolve_data_dir(home.as_deref(), xdg.as_deref())
}

pub fn user_path() -> PathBuf {
    data_dir().join("user.json")
}

pub fn load() -> UserData {
    load_user(&data_dir())
}

pub fn load_user(dir: &Path) -> UserData {
    let Ok(text) = std::fs::read_to_string(dir.join("user.json")) else {
        return UserData::default();
    };
    let mut data = UserData::from_json_lossy(&text);
    keep_one_primary(&mut data);
    data
}

pub fn save(data: &UserData) -> Result<(), String> {
    save_user(&data_dir(), data)
}

/// Write `user.json` atomically, keep one rolling backup, and bump degree-plan revisions
/// whose contents changed since the file on disk.
pub fn save_user(dir: &Path, data: &UserData) -> Result<(), String> {
    let _lock = UserLock::acquire(dir)?;
    save_user_locked(dir, data)
}

pub(crate) fn save_user_locked(dir: &Path, data: &UserData) -> Result<(), String> {
    let previous = load_user(dir);
    let mut data = data.clone();
    keep_one_primary(&mut data);
    bump_plan_revisions(&mut data, &previous);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    let path = dir.join("user.json");
    let backup = dir.join("user.json.bak");
    if path.is_file() {
        std::fs::copy(&path, &backup).map_err(|e| e.to_string())?;
    }
    let tmp = dir.join("user.json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn plan_revision(data: &UserData, id: uuid::Uuid) -> u64 {
    data.plan_revisions
        .get(&id.to_string())
        .copied()
        .unwrap_or(0)
}

fn plan_body(plan: &DegreePlan) -> String {
    let mut copy = plan.clone();
    copy.updated_at = None;
    serde_json::to_string(&copy).unwrap_or_default()
}

fn bump_plan_revisions(data: &mut UserData, previous: &UserData) {
    let mut next = std::collections::BTreeMap::new();
    for plan in &data.degree_plans {
        let id = plan.id.to_string();
        let old_revision = previous.plan_revisions.get(&id).copied().unwrap_or(0);
        let changed = match previous.degree_plans.iter().find(|old| old.id == plan.id) {
            None => true,
            Some(old) => plan_body(old) != plan_body(plan),
        };
        let revision = if changed {
            old_revision.saturating_add(1)
        } else {
            old_revision
        };
        if revision != 0 {
            next.insert(id, revision);
        }
    }
    data.plan_revisions = next;
}

pub(crate) struct UserLock {
    path: PathBuf,
}

impl UserLock {
    pub(crate) fn acquire(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join("user.json.lock");
        let started = std::time::Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                        .is_some_and(|age| age > STALE_LOCK);
                    if stale || started.elapsed() > LOCK_WAIT {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}

impl Drop for UserLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn keep_one_primary(data: &mut UserData) {
    let mut seen = false;
    for plan in &mut data.plans {
        if plan.is_primary {
            if seen {
                plan.is_primary = false;
            } else {
                seen = true;
            }
        }
    }
    let mut seen = false;
    for plan in &mut data.degree_plans {
        if plan.is_primary {
            if seen {
                plan.is_primary = false;
            } else {
                seen = true;
            }
        }
    }
}

pub fn export_json(data: &UserData, include_student: bool) -> String {
    let mut copy = data.clone();
    if !include_student {
        for plan in &mut copy.degree_plans {
            plan.student_name = None;
            plan.student_id = None;
        }
    }
    serde_json::to_string_pretty(&copy).unwrap_or_else(|_| "{}".into())
}

pub fn import_json(text: &str) -> UserData {
    let mut data = UserData::from_json_lossy(text);
    keep_one_primary(&mut data);
    data
}

/// The local user file keeps whole seconds (`JSONEncoder.iso8601` drops fractions).
pub fn round_trip_user_file(data: &UserData) -> Result<UserData, String> {
    let mut value = serde_json::to_value(data).map_err(|e| e.to_string())?;
    floor_user_dates(&mut value);
    let text = serde_json::to_string(&value).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

const INSTANT_KEYS: &[&str] = &[
    "createdAt",
    "updatedAt",
    "savedAt",
    "addedAt",
    "viewedAt",
    "lastCheckedAt",
    "lastNotifiedAt",
    "printedAt",
];

pub fn floor_user_dates(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if key == "changedAt" {
                    if let Some(serde_json::Value::Object(times)) = map.get_mut(&key) {
                        let names: Vec<String> = times.keys().cloned().collect();
                        for name in names {
                            if let Some(seconds) = times.get(&name).and_then(|v| v.as_f64()) {
                                times.insert(name, serde_json::json!(seconds.floor()));
                            }
                        }
                    }
                }
                if INSTANT_KEYS.contains(&key.as_str()) {
                    if let Some(seconds) = map.get(&key).and_then(|v| v.as_f64()) {
                        map.insert(key.clone(), serde_json::json!(seconds.floor()));
                    }
                }
                if let Some(child) = map.get_mut(&key) {
                    floor_user_dates(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                floor_user_dates(item);
            }
        }
        _ => {}
    }
}
