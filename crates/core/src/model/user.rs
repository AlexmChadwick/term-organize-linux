use super::degree::{CourseEquivalence, DegreePlan, RequirementSubstitution};
use super::preferences::SchedulePreferences;
use super::section::{CourseKey, DaySet};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCourse {
    pub term_code: String,
    pub key: CourseKey,
    #[serde(default)]
    pub saved_at: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentCourse {
    pub term_code: String,
    pub key: CourseKey,
    #[serde(default)]
    pub viewed_at: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earliest_start: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_end: Option<i32>,
    #[serde(default)]
    pub free_days: DaySet,
    #[serde(default = "yes")]
    pub include_honors: bool,
    #[serde(default = "yes")]
    pub include_online: bool,
    #[serde(default)]
    pub hide_full_sections: bool,
    #[serde(default)]
    pub min_seats: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub course_count: Option<i32>,
}

fn yes() -> bool {
    true
}

impl Default for ScheduleFilters {
    fn default() -> Self {
        Self {
            earliest_start: None,
            latest_end: None,
            free_days: DaySet(0),
            include_honors: true,
            include_online: true,
            hide_full_sections: false,
            min_seats: 0,
            campus: None,
            course_count: None,
        }
    }
}

impl ScheduleFilters {
    pub fn active_count(&self) -> i32 {
        let mut n = 0;
        if self.earliest_start.is_some() {
            n += 1;
        }
        if self.latest_end.is_some() {
            n += 1;
        }
        if !self.free_days.is_empty() {
            n += 1;
        }
        if !self.include_honors {
            n += 1;
        }
        if !self.include_online {
            n += 1;
        }
        if self.hide_full_sections || self.min_seats > 0 {
            n += 1;
        }
        if self.campus.is_some() {
            n += 1;
        }
        if self.course_count.is_some() {
            n += 1;
        }
        n
    }
}

pub const PALETTE_COUNT: usize = 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanCourse {
    pub key: CourseKey,
    #[serde(default = "yes")]
    pub is_required: bool,
    #[serde(default, alias = "excludedCrns", rename = "excludedCRNs")]
    pub excluded_crns: HashSet<String>,
    #[serde(default)]
    pub color_index: i32,
    /// Section the user picked. Empty means the schedule may choose one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_crn: Option<String>,
    /// Degree-plan row this schedule row was copied from, when sync created it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degree_row_id: Option<Uuid>,
    /// Credits chosen for this row. Absent on older files and on fixed-credit rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePlan {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub name: String,
    pub term_code: String,
    #[serde(default)]
    pub courses: Vec<PlanCourse>,
    #[serde(default)]
    pub selected_crns: Vec<String>,
    #[serde(default)]
    pub filters: ScheduleFilters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferences: Option<SchedulePreferences>,
    #[serde(default)]
    pub favorite_combinations: Vec<Vec<String>>,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub created_at: f64,
    #[serde(default)]
    pub updated_at: f64,
}

impl SchedulePlan {
    pub fn contains(&self, key: &CourseKey) -> bool {
        self.courses.iter().any(|c| &c.key == key)
    }
    pub fn next_color_index(&self) -> i32 {
        let used: HashSet<i32> = self.courses.iter().map(|c| c.color_index).collect();
        (0..PALETTE_COUNT as i32)
            .find(|i| !used.contains(i))
            .unwrap_or(self.courses.len() as i32 % PALETTE_COUNT as i32)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchItem {
    pub term_code: String,
    pub crn: String,
    pub course_key: CourseKey,
    #[serde(default)]
    pub course_title: String,
    #[serde(default)]
    pub section_label: String,
    #[serde(default)]
    pub added_at: f64,
    #[serde(default)]
    pub last_seats_remaining: i32,
    #[serde(default)]
    pub last_waitlist_remaining: i32,
    #[serde(default)]
    pub seat_capacity: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<f64>,
    #[serde(default)]
    pub notify_on_waitlist: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_notified_at: Option<f64>,
}

impl WatchItem {
    pub fn id(&self) -> String {
        format!("{}|{}", self.term_code, self.crn)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_term_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_campus: Option<String>,
    #[serde(default)]
    pub has_completed_onboarding: bool,
    #[serde(default)]
    pub show_weekends: bool,
    #[serde(default = "yes")]
    pub auto_refresh_seats: bool,
    #[serde(default = "yes")]
    pub background_refresh_enabled: bool,
    #[serde(default = "hour8")]
    pub calendar_start_hour: i32,
    #[serde(default = "hour21")]
    pub calendar_end_hour: i32,
    /// Minutes between seat checks while the app is open. Default 15.
    #[serde(default = "fifteen")]
    pub seat_check_minutes: i32,
    /// Linux stand-in for iCloud: a folder the merge engine syncs with. Off by default.
    #[serde(default)]
    pub folder_sync_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_sync_path: Option<String>,
    /// `default`, `light`, or `dark`. Missing means follow the system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_scheme: Option<String>,
    /// Student name and ID stay out of exports unless this is on.
    #[serde(default)]
    pub include_student_in_export: bool,
}

fn hour8() -> i32 {
    8
}
fn hour21() -> i32 {
    21
}
fn fifteen() -> i32 {
    15
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            selected_term_code: None,
            preferred_campus: None,
            has_completed_onboarding: false,
            show_weekends: false,
            auto_refresh_seats: true,
            background_refresh_enabled: true,
            calendar_start_hour: 8,
            calendar_end_hour: 21,
            seat_check_minutes: 15,
            folder_sync_enabled: false,
            folder_sync_path: None,
            color_scheme: None,
            include_student_in_export: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserData {
    #[serde(default = "one")]
    pub version: i32,
    #[serde(default)]
    pub preferences: AppPreferences,
    #[serde(default)]
    pub saved: Vec<SavedCourse>,
    #[serde(default)]
    pub recents: Vec<RecentCourse>,
    #[serde(default)]
    pub plans: Vec<SchedulePlan>,
    #[serde(default)]
    pub watchlist: Vec<WatchItem>,
    #[serde(default)]
    pub search_history: Vec<String>,
    #[serde(default)]
    pub degree_plans: Vec<DegreePlan>,
    /// Dismissed equivalence-suggestion pair ids.
    #[serde(default)]
    pub dismissed_equivalences: Vec<String>,
}

fn one() -> i32 {
    1
}

impl Default for UserData {
    fn default() -> Self {
        Self {
            version: 1,
            preferences: AppPreferences::default(),
            saved: vec![],
            recents: vec![],
            plans: vec![],
            watchlist: vec![],
            search_history: vec![],
            degree_plans: vec![],
            dismissed_equivalences: vec![],
        }
    }
}

impl UserData {
    /// Decode lists item by item so one unreadable entry does not drop the rest.
    pub fn from_json_lossy(text: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return Self::default();
        };
        let obj = value.as_object();
        let mut data = Self::default();
        if let Some(v) = obj.and_then(|o| o.get("version")).and_then(|v| v.as_i64()) {
            data.version = v as i32;
        }
        if let Some(p) = obj.and_then(|o| o.get("preferences")) {
            if let Ok(p) = serde_json::from_value(p.clone()) {
                data.preferences = p;
            }
        }
        data.saved = decode_each(obj.and_then(|o| o.get("saved")));
        data.recents = decode_each(obj.and_then(|o| o.get("recents")));
        data.plans = decode_each(obj.and_then(|o| o.get("plans")));
        data.watchlist = decode_each(obj.and_then(|o| o.get("watchlist")));
        data.search_history = decode_each(obj.and_then(|o| o.get("searchHistory")));
        data.degree_plans = decode_each(obj.and_then(|o| o.get("degreePlans")));
        data.dismissed_equivalences = decode_each(obj.and_then(|o| o.get("dismissedEquivalences")));
        data
    }
}

impl UserData {
    pub fn add_degree_plan(&mut self, mut plan: DegreePlan) -> Uuid {
        if !self.degree_plans.iter().any(|p| p.is_primary) {
            plan.is_primary = true;
        } else if plan.is_primary {
            for existing in &mut self.degree_plans {
                existing.is_primary = false;
            }
        }
        let id = plan.id;
        self.degree_plans.push(plan);
        id
    }

    pub fn set_primary_degree_plan(&mut self, id: Uuid) {
        if !self.degree_plans.iter().any(|p| p.id == id) {
            return;
        }
        for plan in &mut self.degree_plans {
            plan.is_primary = plan.id == id;
        }
    }

    pub fn delete_degree_plan(&mut self, id: Uuid) {
        self.degree_plans.retain(|p| p.id != id);
        if !self.degree_plans.iter().any(|p| p.is_primary) {
            if let Some(plan) = self.degree_plans.first_mut() {
                plan.is_primary = true;
            }
        }
    }

    pub fn primary_degree_plan(&self) -> Option<&DegreePlan> {
        self.degree_plans.iter().find(|p| p.is_primary)
    }

    /// Whether a plan for the term already holds the course, or the given section.
    /// `UserDataStore.courseIsInSchedule`: plans of that term only. A CRN counts when a
    /// plan's selected CRNs contain it; otherwise the plan must contain the course.
    pub fn course_is_in_schedule(
        &self,
        term_code: &str,
        key: &CourseKey,
        crn: Option<&str>,
    ) -> bool {
        let plans = self.plans.iter().filter(|plan| plan.term_code == term_code);
        if let Some(crn) = crn {
            plans
                .into_iter()
                .any(|plan| plan.selected_crns.iter().any(|item| item == crn))
        } else {
            plans.into_iter().any(|plan| plan.contains(key))
        }
    }

    /// The first plan of a term is the primary schedule (`UserDataStore.createPlan`).
    pub fn create_plan(&mut self, name: &str, term_code: &str) -> SchedulePlan {
        let plan = SchedulePlan {
            id: Uuid::new_v4(),
            name: name.to_string(),
            term_code: term_code.to_string(),
            courses: Vec::new(),
            selected_crns: Vec::new(),
            filters: ScheduleFilters::default(),
            preferences: None,
            favorite_combinations: Vec::new(),
            is_primary: !self.plans.iter().any(|plan| plan.term_code == term_code),
            created_at: 0.0,
            updated_at: 0.0,
        };
        self.plans.push(plan.clone());
        plan
    }

    /// Adds a course to a plan. Returns false if it was already there (`UserDataStore.addCourse`).
    pub fn add_course(&mut self, key: CourseKey, plan_id: Uuid) -> bool {
        let Some(plan) = self.plans.iter_mut().find(|plan| plan.id == plan_id) else {
            return false;
        };
        if plan.contains(&key) {
            return false;
        }
        let color_index = plan.next_color_index();
        plan.courses.push(PlanCourse {
            key,
            is_required: true,
            excluded_crns: HashSet::new(),
            color_index,
            pinned_crn: None,
            degree_row_id: None,
            credits: None,
        });
        true
    }

    pub fn update_plan(&mut self, id: Uuid, change: impl FnOnce(&mut SchedulePlan)) {
        if let Some(plan) = self.plans.iter_mut().find(|plan| plan.id == id) {
            change(plan);
        }
    }

    pub fn add_schedule(&mut self, mut plan: SchedulePlan) -> Uuid {
        if !self
            .plans
            .iter()
            .any(|p| p.is_primary && p.term_code == plan.term_code)
        {
            plan.is_primary = true;
        } else if plan.is_primary {
            for existing in &mut self.plans {
                if existing.term_code == plan.term_code {
                    existing.is_primary = false;
                }
            }
        }
        let id = plan.id;
        self.plans.push(plan);
        id
    }
}

fn decode_each<T: for<'de> Deserialize<'de>>(value: Option<&serde_json::Value>) -> Vec<T> {
    let Some(arr) = value.and_then(|v| v.as_array()) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|item| serde_json::from_value(item.clone()).ok())
        .collect()
}

/// Re-export so user data mentions these types exist for the compiler.
pub type _Eq = CourseEquivalence;
pub type _Sub = RequirementSubstitution;
