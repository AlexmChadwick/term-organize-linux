//! Lookup tables built from a term catalog.

use crate::model::*;
use crate::search::{make_stats, SearchDocument};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Debug, Default)]
pub struct CourseStats {
    pub section_count: i32,
    pub open_section_count: i32,
    pub seats_remaining: i32,
    pub seat_capacity: i32,
    pub waitlist_open_count: i32,
    pub campuses: BTreeSet<String>,
    pub class_types: BTreeSet<String>,
    pub has_honors: bool,
    pub has_online: bool,
    pub has_in_person: bool,
    pub instructor_names: Vec<String>,
    pub meeting_days: DaySet,
}

impl CourseStats {
    pub fn is_full(&self) -> bool {
        self.section_count > 0 && self.open_section_count == 0
    }
    pub fn availability_label(&self) -> String {
        if self.section_count == 0 { return "No sections".into(); }
        if self.section_count == 1 {
            return if self.open_section_count == 1 { "1 section · open".into() } else { "1 section · full".into() };
        }
        format!("{} of {} sections open", self.open_section_count, self.section_count)
    }
}

#[derive(Clone, Debug)]
pub struct InstructorSummary {
    pub name: String,
    pub display_name: String,
    pub email: Option<String>,
    pub crns: Vec<String>,
    pub course_keys: Vec<CourseKey>,
}

#[derive(Clone, Debug)]
pub struct RoomBooking {
    pub crn: String,
    pub course: CourseKey,
    pub days: DaySet,
    pub start: i32,
    pub end: i32,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

impl RoomBooking {
    pub fn occurs_on(&self, iso: &str, weekday: i32) -> bool {
        self.days.contains_day(weekday) && self.active_on(iso)
    }
    pub fn active_on(&self, iso: &str) -> bool {
        if let Some(start) = &self.start_date {
            if iso < start.as_str() { return false; }
        }
        if let Some(end) = &self.end_date {
            if iso > end.as_str() { return false; }
        }
        true
    }
    pub fn is_short_session(&self) -> bool {
        let (Some(a), Some(b)) = (&self.start_date, &self.end_date) else { return false };
        days_between(a, b).map(|d| d < 21).unwrap_or(false)
    }
}

fn days_between(a: &str, b: &str) -> Option<i64> {
    Some(iso_ord(b)? - iso_ord(a)?)
}

fn iso_ord(iso: &str) -> Option<i64> {
    let mut p = iso.split('-');
    let y: i64 = p.next()?.parse().ok()?;
    let m: i64 = p.next()?.parse().ok()?;
    let d: i64 = p.next()?.parse().ok()?;
    Some(y * 372 + m * 31 + d)
}

#[derive(Clone, Debug)]
pub struct RoomSchedule {
    pub building_code: String,
    pub building_name: String,
    pub room: String,
    pub bookings: Vec<RoomBooking>,
}

impl RoomSchedule {
    pub fn id(&self) -> String {
        format!("{}|{}", self.building_code, self.room)
    }
    pub fn label(&self) -> String {
        format!("{} {}", self.building_code, self.room)
    }
}

#[derive(Clone, Debug)]
pub struct BuildingSummary {
    pub code: String,
    pub name: String,
    pub campus_code: Option<String>,
    pub rooms: Vec<RoomSchedule>,
}

#[derive(Clone, Debug)]
pub struct CatalogIndex {
    pub catalog: TermCatalog,
    pub courses: Vec<Course>,
    pub courses_by_key: HashMap<CourseKey, Course>,
    pub sections_by_course: HashMap<CourseKey, Vec<CourseSection>>,
    pub sections_by_crn: HashMap<String, CourseSection>,
    pub stats: HashMap<CourseKey, CourseStats>,
    pub subjects_by_code: BTreeMap<String, Subject>,
    pub active_subjects: Vec<Subject>,
    pub instructors: Vec<InstructorSummary>,
    pub buildings: Vec<BuildingSummary>,
    pub documents: Vec<SearchDocument>,
    pub campus_names: Vec<String>,
    pub class_types: Vec<String>,
}

impl CatalogIndex {
    pub fn build(catalog: TermCatalog) -> Self {
        let mut courses_by_key = HashMap::new();
        for course in &catalog.courses {
            courses_by_key.insert(course.key(), course.clone());
        }
        let mut sections_by_course: HashMap<CourseKey, Vec<CourseSection>> = HashMap::new();
        let mut sections_by_crn = HashMap::new();
        for section in &catalog.sections {
            sections_by_course.entry(section.course_key()).or_default().push(section.clone());
            sections_by_crn.insert(section.crn.clone(), section.clone());
        }
        let mut stats = HashMap::new();
        for (key, sections) in &sections_by_course {
            stats.insert(key.clone(), make_stats(sections));
        }
        let subjects_by_code: BTreeMap<_, _> = catalog.subjects.iter().map(|s| (s.code.clone(), s.clone())).collect();
        let mut counts: HashMap<String, i32> = HashMap::new();
        for course in &catalog.courses {
            *counts.entry(course.subject.clone()).or_default() += 1;
        }
        let mut active_subjects: Vec<Subject> = subjects_by_code.values().filter(|s| counts.contains_key(&s.code)).cloned().collect();
        active_subjects.sort_by(|a, b| a.code.cmp(&b.code));
        let mut instructor_map: BTreeMap<String, InstructorSummary> = BTreeMap::new();
        for section in &catalog.sections {
            if let Some(list) = &section.instructors {
                for ins in list {
                    let entry = instructor_map.entry(ins.name.clone()).or_insert_with(|| InstructorSummary {
                        name: ins.name.clone(),
                        display_name: ins.display_name(),
                        email: ins.email.clone(),
                        crns: vec![],
                        course_keys: vec![],
                    });
                    entry.crns.push(section.crn.clone());
                    let key = section.course_key();
                    if !entry.course_keys.contains(&key) { entry.course_keys.push(key); }
                }
            }
        }
        let instructors: Vec<_> = instructor_map.into_values().collect();
        let mut campus_count: HashMap<String, i32> = HashMap::new();
        let mut type_count: HashMap<String, i32> = HashMap::new();
        for section in &catalog.sections {
            if !section.campus.is_empty() { *campus_count.entry(section.campus.clone()).or_default() += 1; }
            if !section.class_type.is_empty() { *type_count.entry(section.class_type.clone()).or_default() += 1; }
        }
        let mut campus_names: Vec<_> = campus_count.into_iter().collect();
        campus_names.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut class_types: Vec<_> = type_count.into_iter().collect();
        class_types.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let buildings = build_buildings(&catalog.sections);
        let mut documents = Vec::with_capacity(catalog.courses.len());
        for course in &catalog.courses {
            let subject_name = subjects_by_code.get(&course.subject).map(|s| s.name.clone()).unwrap_or_default();
            let sections = sections_by_course.get(&course.key());
            let names = stats.get(&course.key()).map(|s| s.instructor_names.clone()).unwrap_or_default();
            let titles: Vec<String> = sections.map(|ss| ss.iter().map(|s| s.title.clone()).collect()).unwrap_or_default();
            documents.push(SearchDocument::build(course, &subject_name, &names, &titles));
        }
        let courses = catalog.courses.clone();
        Self {
            catalog,
            courses,
            courses_by_key,
            sections_by_course,
            sections_by_crn,
            stats,
            subjects_by_code,
            active_subjects,
            instructors,
            buildings,
            documents,
            campus_names: campus_names.into_iter().map(|(n, _)| n).collect(),
            class_types: class_types.into_iter().map(|(n, _)| n).collect(),
        }
    }
    pub fn course(&self, key: &CourseKey) -> Option<&Course> {
        self.courses_by_key.get(key)
    }
    pub fn sections_for(&self, key: &CourseKey) -> Vec<CourseSection> {
        self.sections_by_course.get(key).cloned().unwrap_or_default()
    }
    pub fn section_crn(&self, crn: &str) -> Option<&CourseSection> {
        self.sections_by_crn.get(crn)
    }
    pub fn stats_for(&self, key: &CourseKey) -> CourseStats {
        self.stats.get(key).cloned().unwrap_or_default()
    }
}

fn build_buildings(sections: &[CourseSection]) -> Vec<BuildingSummary> {
    let mut rooms: BTreeMap<(String, String), RoomSchedule> = BTreeMap::new();
    for section in sections {
        for meeting in section.weekly_meetings() {
            let Some(code) = meeting.building_code.clone().or(meeting.building.clone()) else { continue };
            if code.is_empty() || meeting.is_online() { continue; }
            let room = meeting.room.clone().unwrap_or_default();
            if room.is_empty() { continue; }
            let entry = rooms.entry((code.clone(), room.clone())).or_insert_with(|| RoomSchedule {
                building_code: code.clone(),
                building_name: meeting.building.clone().unwrap_or_else(|| code.clone()),
                room: room.clone(),
                bookings: vec![],
            });
            entry.bookings.push(RoomBooking {
                crn: section.crn.clone(),
                course: section.course_key(),
                days: meeting.days,
                start: meeting.start.unwrap_or(0),
                end: meeting.end.unwrap_or(0),
                start_date: meeting.start_date.clone(),
                end_date: meeting.end_date.clone(),
            });
        }
    }
    let mut by_building: BTreeMap<String, BuildingSummary> = BTreeMap::new();
    for ((code, _), room) in rooms {
        let building = by_building.entry(code.clone()).or_insert_with(|| BuildingSummary {
            code: code.clone(),
            name: room.building_name.clone(),
            campus_code: None,
            rooms: vec![],
        });
        building.rooms.push(room);
    }
    by_building.into_values().collect()
}
