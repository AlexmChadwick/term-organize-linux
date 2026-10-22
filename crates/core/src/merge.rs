// SPDX-License-Identifier: AGPL-3.0-only
//! Merge an in-memory edit with a `user.json` that changed after the app loaded it.
//!
//! Unofficial. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.
//!
//! Plans the two sides did not both edit are kept as edited. A plan both sides edited keeps
//! the copy on disk, and the in-memory plan is saved beside it as "<name> (edited in app)".

use crate::model::{DegreePlan, SchedulePlan, UserData};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const EDITED_IN_APP: &str = " (edited in app)";

/// Combines `local` (the open app) with `disk` (the file now), using `baseline`
/// (the file when the app loaded it) to see which side edited each value.
pub fn merge(baseline: &UserData, local: &UserData, disk: &UserData) -> UserData {
    UserData {
        version: local.version.max(disk.version),
        preferences: pick(&baseline.preferences, &local.preferences, &disk.preferences),
        saved: merge_rows(&baseline.saved, &local.saved, &disk.saved, saved_id),
        recents: merge_rows(&baseline.recents, &local.recents, &disk.recents, recent_id),
        plans: merge_plans(&baseline.plans, &local.plans, &disk.plans, schedule_ops()),
        watchlist: merge_rows(
            &baseline.watchlist,
            &local.watchlist,
            &disk.watchlist,
            |item| item.id(),
        ),
        search_history: merge_history(
            &baseline.search_history,
            &local.search_history,
            &disk.search_history,
        ),
        degree_plans: merge_plans(
            &baseline.degree_plans,
            &local.degree_plans,
            &disk.degree_plans,
            degree_ops(),
        ),
        dismissed_equivalences: merge_rows(
            &baseline.dismissed_equivalences,
            &local.dismissed_equivalences,
            &disk.dismissed_equivalences,
            |item| item.clone(),
        ),
        plan_revisions: std::collections::BTreeMap::new(),
    }
}

fn saved_id(item: &crate::model::SavedCourse) -> String {
    format!(
        "{}|{}|{}",
        item.term_code, item.key.subject, item.key.number
    )
}

fn recent_id(item: &crate::model::RecentCourse) -> String {
    format!(
        "{}|{}|{}",
        item.term_code, item.key.subject, item.key.number
    )
}

fn pick<T: Clone + PartialEq>(baseline: &T, local: &T, disk: &T) -> T {
    // An in-memory edit wins, including when both sides changed this value.
    // Disk wins when only the file changed, and when neither side did.
    if local != baseline {
        local.clone()
    } else {
        disk.clone()
    }
}

fn merge_rows<T, K, F>(baseline: &[T], local: &[T], disk: &[T], id: F) -> Vec<T>
where
    T: Clone + PartialEq,
    K: Clone + Eq + std::hash::Hash,
    F: Fn(&T) -> K,
{
    let base_map = index_by(baseline, &id);
    let local_map = index_by(local, &id);
    let disk_map = index_by(disk, &id);
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for item in disk {
        let key = id(item);
        if !seen.insert(key.clone()) {
            continue;
        }
        let base_item = base_map.get(&key).copied();
        let local_item = local_map.get(&key).copied();
        let local_changed = local_item
            .is_some_and(|local_item| base_item.is_none_or(|base_item| base_item != local_item));
        let disk_changed = base_item.is_none_or(|base_item| base_item != item);
        if local_item.is_none() && base_item.is_some() && !disk_changed {
            continue;
        }
        let chosen = if local_changed && !disk_changed {
            local_item.unwrap_or(item)
        } else {
            item
        };
        result.push(chosen.clone());
    }
    for item in local {
        let key = id(item);
        if seen.contains(&key) {
            continue;
        }
        let base_item = base_map.get(&key).copied();
        let local_changed = base_item.is_none_or(|base_item| base_item != item);
        if base_item.is_some() && !disk_map.contains_key(&key) && !local_changed {
            continue;
        }
        result.push(item.clone());
    }
    result
}

struct PlanOps<T> {
    id: fn(&T) -> Uuid,
    name: fn(&T) -> String,
    set_name: fn(&mut T, String),
    set_id: fn(&mut T, Uuid),
    clear_primary: fn(&mut T),
}

fn schedule_ops() -> PlanOps<SchedulePlan> {
    PlanOps {
        id: |plan| plan.id,
        name: |plan| plan.name.clone(),
        set_name: |plan, name| plan.name = name,
        set_id: |plan, id| plan.id = id,
        clear_primary: |plan| plan.is_primary = false,
    }
}

fn degree_ops() -> PlanOps<DegreePlan> {
    PlanOps {
        id: |plan| plan.id,
        name: |plan| plan.name.clone(),
        set_name: |plan, name| plan.name = name,
        set_id: |plan, id| plan.id = id,
        clear_primary: |plan| plan.is_primary = false,
    }
}

fn merge_plans<T: Clone + PartialEq>(
    baseline: &[T],
    local: &[T],
    disk: &[T],
    ops: PlanOps<T>,
) -> Vec<T> {
    let base_map = index_by(baseline, ops.id);
    let local_map = index_by(local, ops.id);
    let mut result = Vec::new();
    let mut names = HashSet::new();
    let mut seen = HashSet::new();

    let mut add = |item: &T, fork: bool| {
        let mut copy = item.clone();
        if fork {
            (ops.set_id)(&mut copy, Uuid::new_v4());
            (ops.clear_primary)(&mut copy);
            let desired = format!("{}{EDITED_IN_APP}", display(&(ops.name)(&copy)));
            let unique = unique_name(&desired, &mut names);
            (ops.set_name)(&mut copy, unique);
        } else {
            names.insert((ops.name)(&copy).to_lowercase());
        }
        result.push(copy);
    };

    for item in disk {
        let key = (ops.id)(item);
        if !seen.insert(key) {
            continue;
        }
        let base_item = base_map.get(&key).copied();
        let local_item = local_map.get(&key).copied();
        let local_changed = local_item
            .is_some_and(|local_item| base_item.is_none_or(|base_item| base_item != local_item));
        let disk_changed = base_item.is_none_or(|base_item| base_item != item);
        if local_item.is_none() {
            if base_item.is_some() && !disk_changed {
                continue;
            }
            add(item, false);
            continue;
        }
        if local_changed && disk_changed && local_item.is_some_and(|local_item| local_item != item)
        {
            add(item, false);
            if let Some(local_item) = local_item {
                add(local_item, true);
            }
            continue;
        }
        if local_changed && !disk_changed {
            if let Some(local_item) = local_item {
                add(local_item, false);
            }
        } else {
            add(item, false);
        }
    }

    for item in local {
        let key = (ops.id)(item);
        if seen.contains(&key) {
            continue;
        }
        let base_item = base_map.get(&key).copied();
        let local_changed = base_item.is_none_or(|base_item| base_item != item);
        if base_item.is_some() && !local_changed {
            continue;
        }
        add(item, base_item.is_some());
    }
    result
}

fn merge_history(baseline: &[String], local: &[String], disk: &[String]) -> Vec<String> {
    if baseline == local {
        return disk.to_vec();
    }
    if baseline == disk {
        return local.to_vec();
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for item in local.iter().chain(disk.iter()) {
        if item.is_empty() || !seen.insert(item.to_lowercase()) {
            continue;
        }
        result.push(item.clone());
        if result.len() == 12 {
            break;
        }
    }
    result
}

fn index_by<T, K, F>(items: &[T], id: F) -> HashMap<K, &T>
where
    K: Eq + std::hash::Hash,
    F: Fn(&T) -> K,
{
    let mut map = HashMap::new();
    for item in items {
        map.insert(id(item), item);
    }
    map
}

fn display(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Plan".to_string()
    } else {
        trimmed.to_string()
    }
}

fn unique_name(desired: &str, used: &mut HashSet<String>) -> String {
    let mut name = desired.to_string();
    let mut n = 2;
    while !used.insert(name.to_lowercase()) {
        name = format!("{desired} {n}");
        n += 1;
    }
    name
}
