//! JSON user data under the XDG data directory, with one primary schedule and degree plan.

use crate::model::UserData;
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("TERMORGANIZE_DATA") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("termorganize");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/termorganize");
    }
    PathBuf::from("termorganize-data")
}

pub fn user_path() -> PathBuf {
    data_dir().join("user.json")
}

pub fn load() -> UserData {
    let path = user_path();
    let Ok(text) = std::fs::read_to_string(&path) else { return UserData::default() };
    let mut data = UserData::from_json_lossy(&text);
    keep_one_primary(&mut data);
    data
}

pub fn save(data: &UserData) -> Result<(), String> {
    let mut data = data.clone();
    keep_one_primary(&mut data);
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    let path = user_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

pub fn keep_one_primary(data: &mut UserData) {
    let mut seen = false;
    for plan in &mut data.plans {
        if plan.is_primary {
            if seen { plan.is_primary = false; } else { seen = true; }
        }
    }
    let mut seen = false;
    for plan in &mut data.degree_plans {
        if plan.is_primary {
            if seen { plan.is_primary = false; } else { seen = true; }
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
