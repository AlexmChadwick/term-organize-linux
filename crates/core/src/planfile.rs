// SPDX-License-Identifier: AGPL-3.0-only
//! Degree-plan edits against the same `user.json` the app loads.
//!
//! Unofficial. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.

use crate::model::{DegreePlan, DegreePlanTerm, PlanSeason, UserData};
use crate::store::{load_user_checked, plan_revision, save_user_locked, UserLock};
use crate::sync;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanFileError {
    NotFound,
    RevisionConflict { actual: u64 },
    Rejected(String),
    Io(String),
}

pub fn create_plan(
    dir: &Path,
    name: &str,
    terms: Vec<DegreePlanTerm>,
) -> Result<(DegreePlan, u64), PlanFileError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(PlanFileError::Rejected("A plan needs a name.".into()));
    }
    let _lock = UserLock::acquire(dir).map_err(PlanFileError::Io)?;
    let mut data = read_user(dir)?;
    let now = now_iso();
    let mut plan = DegreePlan {
        name: name.to_string(),
        terms,
        is_primary: data.degree_plans.is_empty(),
        created_at: Some(now.clone()),
        updated_at: Some(now),
        ..DegreePlan::default()
    };
    plan.sort_terms();
    let id = data.add_degree_plan(plan);
    save_user_locked(dir, &data).map_err(PlanFileError::Io)?;
    let stored = read_user(dir)?;
    let saved = stored
        .degree_plans
        .iter()
        .find(|plan| plan.id == id)
        .cloned()
        .ok_or_else(|| PlanFileError::Io("the new plan was not stored".into()))?;
    Ok((saved, plan_revision(&stored, id)))
}

pub fn update_plan(
    dir: &Path,
    id: Uuid,
    expected_revision: u64,
    mutate: impl FnOnce(&mut DegreePlan) -> Result<(), String>,
) -> Result<(DegreePlan, u64), PlanFileError> {
    let _lock = UserLock::acquire(dir).map_err(PlanFileError::Io)?;
    let mut data = read_user(dir)?;
    if !data.degree_plans.iter().any(|plan| plan.id == id) {
        return Err(PlanFileError::NotFound);
    }
    let actual = plan_revision(&data, id);
    if actual != expected_revision {
        return Err(PlanFileError::RevisionConflict { actual });
    }
    let plan = data
        .degree_plans
        .iter_mut()
        .find(|plan| plan.id == id)
        .ok_or(PlanFileError::NotFound)?;
    mutate(plan).map_err(PlanFileError::Rejected)?;
    plan.updated_at = Some(now_iso());
    plan.sort_terms();
    save_user_locked(dir, &data).map_err(PlanFileError::Io)?;
    let stored = read_user(dir)?;
    let saved = stored
        .degree_plans
        .iter()
        .find(|plan| plan.id == id)
        .cloned()
        .ok_or(PlanFileError::NotFound)?;
    Ok((saved, plan_revision(&stored, id)))
}

pub fn delete_plan(dir: &Path, id: Uuid, expected_revision: u64) -> Result<(), PlanFileError> {
    let _lock = UserLock::acquire(dir).map_err(PlanFileError::Io)?;
    let mut data = read_user(dir)?;
    if !data.degree_plans.iter().any(|plan| plan.id == id) {
        return Err(PlanFileError::NotFound);
    }
    let actual = plan_revision(&data, id);
    if actual != expected_revision {
        return Err(PlanFileError::RevisionConflict { actual });
    }
    data.delete_degree_plan(id);
    save_user_locked(dir, &data).map_err(PlanFileError::Io)?;
    Ok(())
}

pub fn read_user(dir: &Path) -> Result<UserData, PlanFileError> {
    load_user_checked(dir).map_err(PlanFileError::Io)
}

pub fn term_name(season: PlanSeason, year: i32) -> String {
    format!("{} {year}", season.export_name())
}

fn now_iso() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    sync::unix_to_iso8601(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{self, resolve_data_dir};
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn set_mtime(path: &std::path::Path, time: SystemTime) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(time).unwrap();
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "termorganize-plans-{}-{}-{label}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_update_conflict_and_app_loader_round_trip() {
        let dir = temp_dir("round");
        let (plan, revision) = create_plan(&dir, "Fall load", vec![]).unwrap();
        assert_eq!(revision, 1);
        let loaded = store::load_user(&dir);
        assert_eq!(loaded.degree_plans[0].name, "Fall load");
        assert_eq!(store::plan_revision(&loaded, plan.id), 1);
        let text = fs::read_to_string(dir.join("user.json")).unwrap();
        let from_app = UserData::from_json_lossy(&text);
        assert_eq!(from_app.degree_plans[0].id, plan.id);

        let (renamed, next) = update_plan(&dir, plan.id, 1, |plan| {
            plan.name = "Renamed".into();
            Ok(())
        })
        .unwrap();
        assert_eq!(next, 2);
        assert_eq!(renamed.name, "Renamed");
        assert!(dir.join("user.json.bak").is_file());

        let conflict = update_plan(&dir, plan.id, 1, |plan| {
            plan.name = "Stale".into();
            Ok(())
        })
        .unwrap_err();
        assert_eq!(conflict, PlanFileError::RevisionConflict { actual: 2 });
        assert_eq!(store::load_user(&dir).degree_plans[0].name, "Renamed");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_dir_prefers_the_copy_that_already_has_a_user_file() {
        let home = temp_dir("home");
        let xdg = home.join("xdg");
        fs::create_dir_all(&xdg).unwrap();
        let native = store::native_data_dir(Some(&home), Some(&xdg));
        let flatpak = store::flatpak_data_dir(&home);
        assert_eq!(
            resolve_data_dir(Some(&home), Some(&xdg)),
            native,
            "neither file exists, so the native path is the default"
        );
        fs::create_dir_all(&flatpak).unwrap();
        fs::write(flatpak.join("user.json"), "{}").unwrap();
        set_mtime(
            &flatpak.join("user.json"),
            SystemTime::UNIX_EPOCH + Duration::from_secs(10),
        );
        assert_eq!(resolve_data_dir(Some(&home), Some(&xdg)), flatpak);
        fs::create_dir_all(&native).unwrap();
        fs::write(native.join("user.json"), "{}").unwrap();
        set_mtime(
            &native.join("user.json"),
            SystemTime::UNIX_EPOCH + Duration::from_secs(20),
        );
        assert_eq!(
            resolve_data_dir(Some(&home), Some(&xdg)),
            native,
            "the newer user.json wins when both exist"
        );
        let _ = fs::remove_dir_all(&home);
    }
}
