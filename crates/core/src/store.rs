//! JSON user data under the XDG data directory, with one primary schedule and degree plan.
//!
//! The Flatpak app writes under `~/.var/app/org.termorganize.TermOrganize/data/termorganize`
//! because that is `$XDG_DATA_HOME/termorganize` inside the sandbox. A native or AppImage
//! build uses `$XDG_DATA_HOME/termorganize` or `~/.local/share/termorganize`. When
//! `TERMORGANIZE_DATA` is unset, [`data_dir`] uses whichever of those already has
//! `user.json` (the newer file if both do).

use crate::merge;
use crate::model::{DegreePlan, UserData};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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

/// The file the app loaded, plus the SHA-256 and modification time of those bytes.
///
/// A later save treats the file as unchanged when the hash still matches. A newer
/// modification time with the same hash is the same file. A different `planRevisions`
/// map is an outside edit even before the hash is consulted.
#[derive(Clone, Debug)]
pub struct FileStamp {
    pub baseline: UserData,
    pub hash: String,
    pub mtime: Option<SystemTime>,
}

impl FileStamp {
    pub fn empty() -> Self {
        Self {
            baseline: UserData::default(),
            hash: String::new(),
            mtime: None,
        }
    }
}

pub fn load_stamped(dir: &Path) -> (UserData, FileStamp) {
    let stamp = read_stamp(dir);
    (stamp.baseline.clone(), stamp)
}

pub fn read_stamp(dir: &Path) -> FileStamp {
    match inspect(dir) {
        Ok(disk) if disk.unreadable => FileStamp {
            baseline: UserData::default(),
            hash: disk.hash,
            mtime: disk.mtime,
        },
        Ok(disk) => FileStamp {
            baseline: disk.data.unwrap_or_default(),
            hash: disk.hash,
            mtime: disk.mtime,
        },
        Err(_) => FileStamp::empty(),
    }
}

/// True when `user.json` changed since `stamp` was taken.
///
/// `planRevisions` differing from the loaded file is a change. So is a different
/// SHA-256. A newer modification time with the same hash is not: the bytes are
/// the file the app already loaded.
pub(crate) fn file_changed(
    stamp: &FileStamp,
    hash: &str,
    mtime: Option<SystemTime>,
    revisions: &BTreeMap<String, u64>,
) -> bool {
    if revisions != &stamp.baseline.plan_revisions {
        return true;
    }
    if hash == stamp.hash {
        let _ = mtime;
        return false;
    }
    let _ = mtime != stamp.mtime;
    true
}

pub fn load_user(dir: &Path) -> UserData {
    load_user_checked(dir).unwrap_or_else(|_| UserData::default())
}

/// Read `user.json` for a plan tool.
///
/// A missing file is an empty user. A file that exists but cannot be read,
/// is not JSON, or has a newer `version` than this build understands is an
/// error. Callers must not replace that file.
pub fn load_user_checked(dir: &Path) -> Result<UserData, String> {
    let path = dir.join("user.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UserData::default());
        }
        Err(error) => {
            return Err(format!("{} could not be read ({error})", path.display()));
        }
    };
    parse_user_text(&path, &text)
}

fn parse_user_text(path: &Path, text: &str) -> Result<UserData, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("{} is malformed JSON ({error})", path.display()))?;
    let Some(object) = value.as_object() else {
        return Err(format!(
            "{} is malformed JSON (expected an object)",
            path.display()
        ));
    };
    if let Some(version) = object.get("version") {
        let Some(version) = version.as_i64() else {
            return Err(format!(
                "{} is malformed JSON (version is not an integer)",
                path.display()
            ));
        };
        if version > i64::from(crate::model::USER_FILE_VERSION) {
            return Err(format!(
                "{} has unknown newer version {version}",
                path.display()
            ));
        }
    }
    let mut data = UserData::from_json_lossy(text);
    keep_one_primary(&mut data);
    Ok(data)
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
    let previous = load_user_checked(dir)?;
    let mut data = data.clone();
    keep_one_primary(&mut data);
    bump_plan_revisions(&mut data, &previous);
    commit_user(dir, &data).map(|_| ())
}

/// Save `local` into `dir`. When the file changed since `stamp`, plans edited on
/// only one side are combined. A plan edited on both sides keeps the copy on disk,
/// and the app's copy is stored as "<name> (edited in app)".
pub fn save_merging(
    dir: &Path,
    local: &UserData,
    stamp: &FileStamp,
) -> Result<(UserData, FileStamp), String> {
    let _lock = UserLock::acquire(dir)?;
    save_merging_locked(dir, local, stamp)
}

/// Reload an outside write, or merge it when the open window also has edits.
///
/// `None` means the bytes still match the stamp, so memory stays as it is.
/// `Some` is the data and stamp to adopt. A dirty window is merged into the file
/// before that pair is returned.
pub fn fold_external(
    dir: &Path,
    memory: &UserData,
    stamp: &FileStamp,
) -> Result<Option<(UserData, FileStamp)>, String> {
    let _lock = UserLock::acquire(dir)?;
    let disk = inspect(dir)?;
    if disk.unreadable {
        return Ok(None);
    }
    let changed = match &disk.data {
        None => !stamp.hash.is_empty(),
        Some(data) => file_changed(stamp, &disk.hash, disk.mtime, &data.plan_revisions),
    };
    if !changed {
        return Ok(None);
    }
    let Some(disk_data) = disk.data else {
        return Ok(Some((UserData::default(), FileStamp::empty())));
    };
    if memory == &stamp.baseline {
        return Ok(Some((
            disk_data.clone(),
            FileStamp {
                baseline: disk_data,
                hash: disk.hash,
                mtime: disk.mtime,
            },
        )));
    }
    save_merging_locked(dir, memory, stamp).map(Some)
}

fn save_merging_locked(
    dir: &Path,
    local: &UserData,
    stamp: &FileStamp,
) -> Result<(UserData, FileStamp), String> {
    let mut last_error = String::from("could not read user.json");
    for _ in 0..3 {
        let disk = match inspect(dir) {
            Ok(disk) => disk,
            Err(error) => {
                last_error = error;
                continue;
            }
        };
        if disk.unreadable {
            return Err(
                "user.json on disk is not valid JSON, so it was left unchanged".to_string(),
            );
        }
        let changed = match &disk.data {
            None => !stamp.hash.is_empty(),
            Some(data) => file_changed(stamp, &disk.hash, disk.mtime, &data.plan_revisions),
        };
        let mut data = local.clone();
        if changed {
            if let Some(disk_data) = &disk.data {
                data = merge::merge(&stamp.baseline, local, disk_data);
            }
        }
        keep_one_primary(&mut data);
        let previous = disk.data.clone().unwrap_or_default();
        bump_plan_revisions(&mut data, &previous);
        let bytes = match commit_user(dir, &data) {
            Ok(bytes) => bytes,
            Err(error) => {
                last_error = error;
                continue;
            }
        };
        return Ok((
            data.clone(),
            FileStamp {
                baseline: data,
                hash: sha256_hex(&bytes),
                mtime: file_mtime(&dir.join("user.json")),
            },
        ));
    }
    Err(last_error)
}

struct DiskFile {
    hash: String,
    mtime: Option<SystemTime>,
    data: Option<UserData>,
    unreadable: bool,
}

fn inspect(dir: &Path) -> Result<DiskFile, String> {
    let path = dir.join("user.json");
    if !path.is_file() {
        return Ok(DiskFile {
            hash: String::new(),
            mtime: None,
            data: None,
            unreadable: false,
        });
    }
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    let hash = sha256_hex(&bytes);
    let mtime = file_mtime(&path);
    match parsed_user(&bytes) {
        Ok(data) => Ok(DiskFile {
            hash,
            mtime,
            data: Some(data),
            unreadable: false,
        }),
        Err(()) => Ok(DiskFile {
            hash,
            mtime,
            data: None,
            unreadable: true,
        }),
    }
}

fn parsed_user(bytes: &[u8]) -> Result<UserData, ()> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    if !value.is_object() {
        return Err(());
    }
    if let Some(version) = value.get("version") {
        let Some(version) = version.as_i64() else {
            return Err(());
        };
        if version > i64::from(crate::model::USER_FILE_VERSION) {
            return Err(());
        }
    }
    let text = String::from_utf8_lossy(bytes);
    let mut data = UserData::from_json_lossy(&text);
    keep_one_primary(&mut data);
    Ok(data)
}

fn commit_user(dir: &Path, data: &UserData) -> Result<Vec<u8>, String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let text = serde_json::to_string_pretty(data).map_err(|error| error.to_string())?;
    let bytes = text.into_bytes();
    let path = dir.join("user.json");
    let backup = dir.join("user.json.bak");
    if path.is_file() {
        std::fs::copy(&path, &backup).map_err(|error| error.to_string())?;
    }
    let tmp = dir.join("user.json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|error| error.to_string())?;
    let written = std::fs::read(&path).map_err(|error| error.to_string())?;
    if written != bytes {
        return Err("user.json changed while it was being saved".to_string());
    }
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

pub fn plan_revision(data: &UserData, id: uuid::Uuid) -> u64 {
    recorded_revision(data.plan_revisions.get(&id.to_string()).copied())
}

/// A stored counter, or 1 when the plan has never been given one.
///
/// Plans created in the app before revisions were recorded have no `planRevisions`
/// entry (or a 0). Those are revision 1, on read and on an `expected_revision` check.
fn recorded_revision(stored: Option<u64>) -> u64 {
    stored.filter(|revision| *revision > 0).unwrap_or(1)
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
        let existed = previous.degree_plans.iter().any(|old| old.id == plan.id);
        // A new plan's first save is revision 1. A plan already on disk with no
        // counter is already revision 1, so the next edit becomes 2.
        let old_revision = if existed {
            recorded_revision(previous.plan_revisions.get(&id).copied())
        } else {
            0
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DegreePlan;
    use std::fs;
    use std::time::Duration;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "termorganize-merge-{}-{}-{label}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn degree(name: &str) -> DegreePlan {
        DegreePlan {
            name: name.to_string(),
            ..DegreePlan::default()
        }
    }

    fn set_mtime(path: &Path, time: SystemTime) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(time).unwrap();
    }

    fn names(data: &UserData) -> Vec<String> {
        data.degree_plans
            .iter()
            .map(|plan| plan.name.clone())
            .collect()
    }

    #[test]
    fn revision_or_hash_marks_an_outside_edit_and_mtime_alone_does_not() {
        let mut stamp = FileStamp::empty();
        stamp.hash = "abc".to_string();
        stamp.mtime = Some(SystemTime::UNIX_EPOCH);
        stamp.baseline.plan_revisions.insert("plan".to_string(), 1);
        let same_rev = stamp.baseline.plan_revisions.clone();
        let later = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(30));
        assert!(
            !file_changed(&stamp, "abc", later, &same_rev),
            "a newer mtime with the same hash is the file the app loaded"
        );
        assert!(
            file_changed(&stamp, "def", stamp.mtime, &same_rev),
            "a different hash is an outside edit even when the mtime matches"
        );
        let mut bumped = same_rev.clone();
        bumped.insert("plan".to_string(), 2);
        assert!(
            file_changed(&stamp, "abc", stamp.mtime, &bumped),
            "a new planRevisions entry is an outside edit"
        );
    }

    #[test]
    fn a_plan_with_no_recorded_revision_is_revision_one() {
        let dir = temp_dir("rev1");
        let mut data = UserData::default();
        let id = data.add_degree_plan(degree("From the app"));
        data.plan_revisions.clear();
        data.plan_revisions.insert(id.to_string(), 0);
        assert_eq!(plan_revision(&data, id), 1);
        data.plan_revisions.clear();
        fs::write(dir.join("user.json"), serde_json::to_string(&data).unwrap()).unwrap();
        let loaded = load_user(&dir);
        assert!(loaded.plan_revisions.get(&id.to_string()).is_none());
        assert_eq!(plan_revision(&loaded, id), 1);

        let mut edited = loaded;
        edited
            .degree_plans
            .iter_mut()
            .find(|plan| plan.id == id)
            .unwrap()
            .name = "Edited once".into();
        save_user(&dir, &edited).unwrap();
        assert_eq!(plan_revision(&load_user(&dir), id), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn different_plans_merge_and_the_same_plan_is_copied() {
        let dir = temp_dir("plans");
        let mut app = UserData::default();
        let mine = app.add_degree_plan(degree("Mine"));
        let shared = app.add_degree_plan(degree("Shared"));
        let schedule = app.create_plan("Week", "202710").id;
        let (app, stamp) = save_merging(&dir, &app, &FileStamp::empty()).unwrap();
        let kept_revision = plan_revision(&app, mine);

        let mut mcp = load_user(&dir);
        mcp.add_degree_plan(degree("From MCP"));
        mcp.degree_plans
            .iter_mut()
            .find(|plan| plan.id == shared)
            .unwrap()
            .name = "Shared from MCP".to_string();
        mcp.create_plan("Other week", "202730");
        save_user(&dir, &mcp).unwrap();
        let disk_shared_revision = plan_revision(&load_user(&dir), shared);

        let mut editing = app.clone();
        editing
            .degree_plans
            .iter_mut()
            .find(|plan| plan.id == mine)
            .unwrap()
            .name = "Mine renamed".to_string();
        editing
            .degree_plans
            .iter_mut()
            .find(|plan| plan.id == shared)
            .unwrap()
            .name = "Shared from app".to_string();
        editing
            .plans
            .iter_mut()
            .find(|plan| plan.id == schedule)
            .unwrap()
            .name = "Week from app".to_string();
        let (saved, _) = save_merging(&dir, &editing, &stamp).unwrap();

        assert_eq!(
            saved
                .degree_plans
                .iter()
                .find(|plan| plan.id == mine)
                .unwrap()
                .name,
            "Mine renamed"
        );
        assert!(names(&saved).iter().any(|name| name == "From MCP"));
        assert_eq!(
            saved
                .degree_plans
                .iter()
                .find(|plan| plan.id == shared)
                .unwrap()
                .name,
            "Shared from MCP"
        );
        assert_eq!(plan_revision(&saved, shared), disk_shared_revision);
        assert!(saved.degree_plans.iter().any(|plan| {
            plan.name == "Shared from app (edited in app)" && plan.id != shared && !plan.is_primary
        }));
        assert_eq!(
            saved
                .plans
                .iter()
                .find(|plan| plan.id == schedule)
                .unwrap()
                .name,
            "Week from app"
        );
        assert!(saved.plans.iter().any(|plan| plan.name == "Other week"));
        assert_eq!(plan_revision(&saved, mine), kept_revision + 1);

        let text = fs::read_to_string(dir.join("user.json")).unwrap();
        let loaded = UserData::from_json_lossy(&text);
        assert_eq!(loaded.degree_plans.len(), saved.degree_plans.len());
        assert!(loaded
            .degree_plans
            .iter()
            .any(|plan| plan.name == "Shared from app (edited in app)"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn schedule_edited_on_both_sides_keeps_the_disk_copy() {
        let dir = temp_dir("schedule");
        let mut app = UserData::default();
        let schedule = app.create_plan("Week", "202710").id;
        let (app, stamp) = save_merging(&dir, &app, &FileStamp::empty()).unwrap();

        let mut mcp = load_user(&dir);
        mcp.plans
            .iter_mut()
            .find(|plan| plan.id == schedule)
            .unwrap()
            .name = "Week from MCP".to_string();
        save_user(&dir, &mcp).unwrap();

        let mut editing = app;
        editing
            .plans
            .iter_mut()
            .find(|plan| plan.id == schedule)
            .unwrap()
            .name = "Week from app".to_string();
        let (saved, _) = save_merging(&dir, &editing, &stamp).unwrap();
        assert_eq!(
            saved
                .plans
                .iter()
                .find(|plan| plan.id == schedule)
                .unwrap()
                .name,
            "Week from MCP"
        );
        assert!(saved.plans.iter().any(|plan| {
            plan.name == "Week from app (edited in app)" && plan.id != schedule && !plan.is_primary
        }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn untouched_save_does_not_fork_and_an_outside_delete_is_kept() {
        let dir = temp_dir("delete");
        let mut app = UserData::default();
        let kept = app.add_degree_plan(degree("Kept"));
        let dropped = app.add_degree_plan(degree("Dropped"));
        let (mut app, stamp) = save_merging(&dir, &app, &FileStamp::empty()).unwrap();
        app.degree_plans
            .iter_mut()
            .find(|plan| plan.id == kept)
            .unwrap()
            .program = Some("CS".to_string());
        let (app, stamp) = save_merging(&dir, &app, &stamp).unwrap();
        assert_eq!(app.degree_plans.len(), 2);
        assert!(names(&app)
            .iter()
            .all(|name| !name.contains("edited in app")));

        let mut mcp = load_user(&dir);
        mcp.delete_degree_plan(dropped);
        save_user(&dir, &mcp).unwrap();

        let mut editing = app.clone();
        editing.preferences.preferred_campus = Some("Boston".to_string());
        let (saved, _) = save_merging(&dir, &editing, &stamp).unwrap();
        assert_eq!(
            saved.preferences.preferred_campus.as_deref(),
            Some("Boston")
        );
        assert!(saved.degree_plans.iter().all(|plan| plan.id != dropped));
        assert_eq!(
            saved
                .degree_plans
                .iter()
                .find(|plan| plan.id == kept)
                .unwrap()
                .name,
            "Kept"
        );
        assert!(names(&saved)
            .iter()
            .all(|name| !name.contains("edited in app")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn app_edit_of_a_deleted_plan_is_saved_as_a_copy() {
        let dir = temp_dir("deleted");
        let mut app = UserData::default();
        let plan = app.add_degree_plan(degree("Shared"));
        let (app, stamp) = save_merging(&dir, &app, &FileStamp::empty()).unwrap();

        let mut mcp = load_user(&dir);
        mcp.delete_degree_plan(plan);
        save_user(&dir, &mcp).unwrap();

        let mut editing = app;
        editing
            .degree_plans
            .iter_mut()
            .find(|item| item.id == plan)
            .unwrap()
            .name = "Still editing".to_string();
        let (saved, _) = save_merging(&dir, &editing, &stamp).unwrap();
        assert!(saved.degree_plans.iter().all(|item| item.id != plan));
        assert!(saved
            .degree_plans
            .iter()
            .any(|item| item.name == "Still editing (edited in app)"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_bytes_with_a_new_mtime_do_not_fork_a_copy() {
        let dir = temp_dir("mtime");
        let mut app = UserData::default();
        app.add_degree_plan(degree("Kept"));
        let (app, stamp) = save_merging(&dir, &app, &FileStamp::empty()).unwrap();
        let path = dir.join("user.json");
        set_mtime(
            &path,
            stamp.mtime.unwrap_or(SystemTime::UNIX_EPOCH) + Duration::from_secs(3600),
        );
        let (saved, _) = save_merging(&dir, &app, &stamp).unwrap();
        assert_eq!(saved.degree_plans.len(), 1);
        assert_eq!(saved.degree_plans[0].name, "Kept");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_change_at_the_same_mtime_still_merges() {
        let dir = temp_dir("hash");
        let mut app = UserData::default();
        let mine = app.add_degree_plan(degree("Mine"));
        let (app, stamp) = save_merging(&dir, &app, &FileStamp::empty()).unwrap();
        let mtime = stamp.mtime.expect("saved file has an mtime");

        let mut mcp = load_user(&dir);
        mcp.add_degree_plan(degree("From MCP"));
        save_user(&dir, &mcp).unwrap();
        set_mtime(&dir.join("user.json"), mtime);

        let mut editing = app;
        editing
            .degree_plans
            .iter_mut()
            .find(|plan| plan.id == mine)
            .unwrap()
            .name = "Mine renamed".to_string();
        let (saved, _) = save_merging(&dir, &editing, &stamp).unwrap();
        assert_eq!(
            saved
                .degree_plans
                .iter()
                .find(|plan| plan.id == mine)
                .unwrap()
                .name,
            "Mine renamed"
        );
        assert!(names(&saved).iter().any(|name| name == "From MCP"));
        assert!(names(&saved)
            .iter()
            .all(|name| !name.contains("edited in app")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_window_reloads_an_outside_write_and_merges_when_dirty() {
        let dir = temp_dir("reload");
        let mut app = UserData::default();
        let shared = app.add_degree_plan(degree("Shared"));
        let (app, stamp) = save_merging(&dir, &app, &FileStamp::empty()).unwrap();

        let mut mcp = load_user(&dir);
        mcp.add_degree_plan(degree("From MCP"));
        save_user(&dir, &mcp).unwrap();
        let (data, _) = fold_external(&dir, &app, &stamp)
            .unwrap()
            .expect("an outside plan should reload");
        assert!(names(&data).iter().any(|name| name == "From MCP"));

        let mut mcp = load_user(&dir);
        mcp.degree_plans
            .iter_mut()
            .find(|plan| plan.id == shared)
            .unwrap()
            .name = "Shared from MCP".to_string();
        save_user(&dir, &mcp).unwrap();
        let mut dirty = app;
        dirty
            .degree_plans
            .iter_mut()
            .find(|plan| plan.id == shared)
            .unwrap()
            .name = "Shared from app".to_string();
        let (data, _) = fold_external(&dir, &dirty, &stamp)
            .unwrap()
            .expect("a dirty window should merge the outside edit");
        assert_eq!(
            data.degree_plans
                .iter()
                .find(|plan| plan.id == shared)
                .unwrap()
                .name,
            "Shared from MCP"
        );
        assert!(data
            .degree_plans
            .iter()
            .any(|plan| plan.name == "Shared from app (edited in app)"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_file_is_left_in_place() {
        let dir = temp_dir("bad");
        fs::write(dir.join("user.json"), b"not json").unwrap();
        let stamp = read_stamp(&dir);
        let error = save_merging(&dir, &UserData::default(), &stamp).unwrap_err();
        assert!(error.contains("not valid JSON"));
        assert_eq!(fs::read(dir.join("user.json")).unwrap(), b"not json");
        let _ = fs::remove_dir_all(&dir);
    }
}
