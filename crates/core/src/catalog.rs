//! Turn Banner section JSON into a term catalog, and keep those catalogs on disk.
//!
//! Port of `CatalogBuilder.swift`.

use crate::archive::{self, file_name};
use crate::html::decode_entities;
use crate::index::CatalogIndex;
use crate::model::*;
use crate::store;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BannerSection {
    #[serde(default)]
    pub course_reference_number: String,
    #[serde(default)]
    pub part_of_term: Option<String>,
    #[serde(default)]
    pub course_number: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub sequence_number: Option<String>,
    #[serde(default)]
    pub campus_description: Option<String>,
    #[serde(default)]
    pub schedule_type_description: Option<String>,
    #[serde(default)]
    pub course_title: Option<String>,
    #[serde(default)]
    pub credit_hours: Option<f64>,
    #[serde(default)]
    pub maximum_enrollment: Option<i32>,
    #[serde(default)]
    pub enrollment: Option<i32>,
    #[serde(default)]
    pub seats_available: Option<i32>,
    #[serde(default)]
    pub wait_capacity: Option<i32>,
    #[serde(default)]
    pub wait_count: Option<i32>,
    #[serde(default)]
    pub wait_available: Option<i32>,
    #[serde(default)]
    pub cross_list: Option<String>,
    #[serde(default)]
    pub cross_list_capacity: Option<i32>,
    #[serde(default)]
    pub cross_list_count: Option<i32>,
    #[serde(default)]
    pub cross_list_available: Option<i32>,
    #[serde(default)]
    pub credit_hour_high: Option<f64>,
    #[serde(default)]
    pub credit_hour_low: Option<f64>,
    #[serde(default)]
    pub meetings_faculty: Option<Vec<BannerMeetingFaculty>>,
    #[serde(default)]
    pub section_attributes: Option<Vec<BannerAttribute>>,
    #[serde(default)]
    pub instructional_method_description: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BannerAttribute {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BannerMeetingFaculty {
    #[serde(default)]
    pub faculty: Option<Vec<BannerFaculty>>,
    #[serde(default)]
    pub meeting_time: Option<BannerMeetingTime>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BannerFaculty {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email_address: Option<String>,
    #[serde(default)]
    pub primary_indicator: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BannerMeetingTime {
    #[serde(default)]
    pub begin_time: Option<String>,
    #[serde(default)]
    pub end_time: Option<String>,
    #[serde(default)]
    pub building: Option<String>,
    #[serde(default)]
    pub building_description: Option<String>,
    #[serde(default)]
    pub room: Option<String>,
    #[serde(default)]
    pub campus: Option<String>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub meeting_type: Option<String>,
    #[serde(default)]
    pub meeting_type_description: Option<String>,
    #[serde(default)]
    pub monday: Option<bool>,
    #[serde(default)]
    pub tuesday: Option<bool>,
    #[serde(default)]
    pub wednesday: Option<bool>,
    #[serde(default)]
    pub thursday: Option<bool>,
    #[serde(default)]
    pub friday: Option<bool>,
    #[serde(default)]
    pub saturday: Option<bool>,
    #[serde(default)]
    pub sunday: Option<bool>,
}

fn blank(value: Option<&str>) -> Option<String> {
    let text = value.unwrap_or("").trim();
    if text.is_empty() { None } else { Some(text.to_string()) }
}

fn entities(text: &str) -> String {
    decode_entities(&decode_entities(text))
}

impl BannerMeetingTime {
    pub fn to_meeting(&self) -> Meeting {
        let mut days = DaySet(0);
        if self.sunday == Some(true) { days.0 |= 1 << 0; }
        if self.monday == Some(true) { days.0 |= 1 << 1; }
        if self.tuesday == Some(true) { days.0 |= 1 << 2; }
        if self.wednesday == Some(true) { days.0 |= 1 << 3; }
        if self.thursday == Some(true) { days.0 |= 1 << 4; }
        if self.friday == Some(true) { days.0 |= 1 << 5; }
        if self.saturday == Some(true) { days.0 |= 1 << 6; }
        let start = self.begin_time.as_deref().and_then(|s| s.parse::<i32>().ok()).map(minutes_from_hhmm);
        let end = self.end_time.as_deref().and_then(|s| s.parse::<i32>().ok()).map(minutes_from_hhmm);
        let room = self.room.as_deref().map(str::trim).filter(|r| !r.is_empty() && !r.eq_ignore_ascii_case("ROOM")).map(str::to_string);
        let kind = if self.meeting_type.as_deref() == Some("FNEX") {
            "Final Exam".into()
        } else {
            entities(self.meeting_type_description.as_deref().unwrap_or("Class"))
        };
        Meeting {
            days,
            start,
            end,
            building_code: blank(self.building.as_deref()),
            building: self.building_description.as_deref().map(entities).and_then(|s| blank(Some(&s))),
            room,
            campus_code: blank(self.campus.as_deref()),
            kind,
            start_date: self.start_date.as_deref().and_then(banner_date),
            end_date: self.end_date.as_deref().and_then(banner_date),
        }
    }
}

fn banner_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.len() == 10 && raw.as_bytes().get(4) == Some(&b'-') {
        return Some(raw.to_string());
    }
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.len() == 3 {
        let month: u32 = parts[0].parse().ok()?;
        let day: u32 = parts[1].parse().ok()?;
        let year: i32 = parts[2].parse().ok()?;
        return Some(format!("{year:04}-{month:02}-{day:02}"));
    }
    None
}

impl BannerFaculty {
    fn to_instructor(&self) -> Option<Instructor> {
        let name = blank(self.display_name.as_deref())?;
        Some(Instructor { name, primary: self.primary_indicator.unwrap_or(false), email: blank(self.email_address.as_deref()) })
    }
}

pub fn make_sections(banner: &[BannerSection], previous: &HashMap<String, CourseSection>) -> Vec<CourseSection> {
    let mut cross_lists: HashMap<String, Vec<String>> = HashMap::new();
    for section in banner {
        if let Some(group) = blank(section.cross_list.as_deref()) {
            cross_lists.entry(group).or_default().push(section.course_reference_number.clone());
        }
    }
    banner
        .iter()
        .map(|section| {
            let attributes = section.section_attributes.clone().unwrap_or_default();
            let honors = attributes.iter().any(|a| a.code == HONORS_ATTRIBUTE || a.description.as_deref().unwrap_or("").trim() == "Honors");
            let meetings = section.meetings_faculty.clone().unwrap_or_default().into_iter().filter_map(|m| m.meeting_time.map(|t| t.to_meeting())).collect();
            let instructors: Vec<Instructor> = section
                .meetings_faculty
                .clone()
                .unwrap_or_default()
                .into_iter()
                .flat_map(|m| m.faculty.unwrap_or_default())
                .filter_map(|f| f.to_instructor())
                .collect();
            let prior = previous.get(&section.course_reference_number);
            let cross = blank(section.cross_list.as_deref())
                .and_then(|group| cross_lists.get(&group).cloned())
                .unwrap_or_default()
                .into_iter()
                .filter(|crn| crn != &section.course_reference_number)
                .collect();
            CourseSection {
                crn: section.course_reference_number.clone(),
                subject: section.subject.clone(),
                number: section.course_number.clone(),
                title: entities(section.course_title.as_deref().unwrap_or("")),
                section_number: section.sequence_number.clone().unwrap_or_default(),
                part_of_term: section.part_of_term.clone().unwrap_or_else(|| "1".into()),
                seat_capacity: section.cross_list_capacity.or(section.maximum_enrollment).unwrap_or(0),
                seats_remaining: section.cross_list_available.or(section.seats_available).unwrap_or(0),
                enrollment: section.cross_list_count.or(section.enrollment).unwrap_or(0),
                waitlist_capacity: section.wait_capacity.unwrap_or(0),
                waitlist_count: section.wait_count.unwrap_or(0),
                waitlist_remaining: section.wait_available.unwrap_or(0),
                class_type: entities(section.schedule_type_description.as_deref().unwrap_or("Lecture")),
                honors,
                campus: entities(section.campus_description.as_deref().unwrap_or("")),
                instruction_method: section.instructional_method_description.as_deref().map(entities),
                meetings,
                instructors: if instructors.is_empty() { prior.and_then(|p| p.instructors.clone()) } else { Some(instructors) },
                cross_list_crns: cross,
                attributes: attributes.iter().map(|a| a.code.clone()).collect(),
                description: prior.and_then(|p| p.description.clone()),
                prereqs: prior.and_then(|p| p.prereqs.clone()),
                coreqs: prior.and_then(|p| p.coreqs.clone()),
            }
        })
        .collect()
}

pub fn stub_courses(banner: &[BannerSection]) -> (Vec<Course>, Vec<AttributeInfo>) {
    let mut courses: BTreeMap<CourseKey, Course> = BTreeMap::new();
    let mut order = Vec::new();
    let mut attributes: BTreeMap<String, String> = BTreeMap::new();
    for section in banner {
        let key = CourseKey { subject: section.subject.clone(), number: section.course_number.clone() };
        let title = entities(section.course_title.as_deref().unwrap_or(""));
        let section_attributes = section.section_attributes.clone().unwrap_or_default();
        for attribute in &section_attributes {
            attributes.insert(attribute.code.clone(), entities(attribute.description.as_deref().unwrap_or(&attribute.code)).trim().to_string());
        }
        let is_topics = section_attributes.iter().any(|a| a.code == "TOPC");
        if let Some(existing) = courses.get_mut(&key) {
            let mut codes: Vec<String> = existing.attributes.iter().cloned().chain(section_attributes.iter().map(|a| a.code.clone()).filter(|c| c != HONORS_ATTRIBUTE)).collect();
            codes.sort();
            codes.dedup();
            existing.attributes = codes;
            if !existing.special_topics && is_topics {
                existing.special_topics = true;
                existing.title = "Special Topics".into();
            }
        } else {
            let special = title.to_ascii_lowercase().contains("special topics") || is_topics;
            let low = section.credit_hour_low.or(section.credit_hours).unwrap_or(0.0);
            courses.insert(
                key.clone(),
                Course {
                    subject: section.subject.clone(),
                    number: section.course_number.clone(),
                    title: if special { "Special Topics".into() } else { title },
                    description: String::new(),
                    min_credits: low,
                    max_credits: section.credit_hour_high.unwrap_or(low),
                    attributes: {
                        let mut codes: Vec<String> = section_attributes.iter().map(|a| a.code.clone()).filter(|c| c != HONORS_ATTRIBUTE).collect();
                        codes.sort();
                        codes
                    },
                    prereqs: None,
                    coreqs: None,
                    postreqs: vec![],
                    special_topics: special,
                    college: None,
                    department: None,
                    details_loaded: false,
                },
            );
            order.push(key);
        }
    }
    let attribute_list = attributes.into_iter().map(|(code, name)| AttributeInfo { code, name }).collect();
    (order.into_iter().filter_map(|key| courses.remove(&key)).collect(), attribute_list)
}

pub fn merge(
    existing: Option<&TermCatalog>,
    fresh: &[BannerSection],
    term: Term,
    subjects: Option<Vec<Subject>>,
    campuses: Option<Vec<Campus>>,
    attributes: Option<Vec<AttributeInfo>>,
    knowledge: &HashMap<CourseKey, Course>,
    now: f64,
) -> TermCatalog {
    let previous_sections: HashMap<String, CourseSection> = existing
        .map(|catalog| catalog.sections.iter().map(|s| (s.crn.clone(), s.clone())).collect())
        .unwrap_or_default();
    let previous_courses: HashMap<CourseKey, Course> = existing
        .map(|catalog| catalog.courses.iter().map(|c| (c.key(), c.clone())).collect())
        .unwrap_or_default();
    let sections = make_sections(fresh, &previous_sections);
    let (stubs, fresh_attributes) = stub_courses(fresh);
    let mut courses: Vec<Course> = stubs
        .into_iter()
        .map(|stub| {
            if let Some(mut known) = previous_courses.get(&stub.key()).cloned() {
                known.min_credits = stub.min_credits;
                known.max_credits = stub.max_credits;
                known.attributes = stub.attributes;
                known.special_topics = known.special_topics || stub.special_topics;
                return known;
            }
            if let Some(mut borrowed) = knowledge.get(&stub.key()).cloned() {
                borrowed.min_credits = stub.min_credits;
                borrowed.max_credits = stub.max_credits;
                borrowed.attributes = stub.attributes;
                borrowed.special_topics = borrowed.special_topics || stub.special_topics;
                borrowed.postreqs = vec![];
                return borrowed;
            }
            stub
        })
        .collect();
    courses.sort_by_key(|course| course.key());
    let mut merged_attributes: BTreeMap<String, AttributeInfo> = existing
        .map(|catalog| catalog.attributes.iter().map(|a| (a.code.clone(), a.clone())).collect())
        .unwrap_or_default();
    for attribute in attributes.unwrap_or_default().into_iter().chain(fresh_attributes) {
        merged_attributes.insert(attribute.code.clone(), attribute);
    }
    let mut section_list = sections;
    section_list.sort_by(|a, b| a.course_key().cmp(&b.course_key()).then(a.section_number.cmp(&b.section_number)));
    let mut catalog = TermCatalog {
        version: existing.map(|c| c.version).unwrap_or(1),
        term,
        generated_at: existing.map(|c| c.generated_at).unwrap_or(now),
        sections_updated_at: now,
        courses,
        sections: section_list,
        subjects: subjects.or_else(|| existing.map(|c| c.subjects.clone())).unwrap_or_default(),
        attributes: merged_attributes.into_values().collect(),
        campuses: campuses.or_else(|| existing.map(|c| c.campuses.clone())).unwrap_or_default(),
    };
    catalog.attributes.sort_by(|a, b| a.code.cmp(&b.code));
    catalog.recompute_postreqs();
    catalog
}

pub fn apply_course_refresh(catalog: &mut TermCatalog, course: &CourseKey, banner: &[BannerSection], now: f64) {
    let previous: HashMap<String, CourseSection> = catalog.sections.iter().filter(|s| &s.course_key() == course).map(|s| (s.crn.clone(), s.clone())).collect();
    let fresh = make_sections(banner, &previous);
    catalog.sections.retain(|s| &s.course_key() != course);
    catalog.sections.extend(fresh);
    catalog.sections.sort_by(|a, b| a.course_key().cmp(&b.course_key()).then(a.section_number.cmp(&b.section_number)));
    if !catalog.courses.iter().any(|c| &c.key() == course) && !banner.is_empty() {
        let (stubs, _) = stub_courses(banner);
        catalog.courses.extend(stubs);
        catalog.courses.sort_by_key(|course| course.key());
    }
    catalog.sections_updated_at = now;
}

pub fn cache_dir() -> PathBuf {
    store::data_dir().join("catalog")
}

pub fn cache_path(term_code: &str) -> PathBuf {
    cache_dir().join(file_name(term_code))
}

pub fn save_cached(catalog: &TermCatalog) -> Result<(), String> {
    let path = cache_path(&catalog.term.code);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string(catalog).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

pub fn load_cached(term_code: &str) -> Option<TermCatalog> {
    archive::read(&cache_path(term_code)).ok()
}

pub fn cached_catalogs() -> Vec<TermCatalog> {
    let dir = cache_dir();
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut catalogs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(code) = archive::term_code_from_file_name(name) else { continue };
        if let Some(catalog) = load_cached(&code) {
            catalogs.push(catalog);
        }
    }
    catalogs.sort_by(|a, b| b.term.code.cmp(&a.term.code));
    catalogs
}

pub fn indexes_on_disk() -> Vec<CatalogIndex> {
    cached_catalogs().into_iter().map(CatalogIndex::build).collect()
}

/// A row of Banner's cross-listed sections table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossListedSection {
    pub crn: String,
    pub course: CourseKey,
    pub title: String,
    pub section: String,
}

/// Sections named in a cross-list HTML table. An empty or header-only page is an empty list.
pub fn cross_listed_sections(html: &str, subjects: &crate::banner::SubjectLookup) -> Vec<CrossListedSection> {
    let Some(rows) = crate::html::table_rows(html) else { return Vec::new() };
    rows.into_iter()
        .filter_map(|cells| {
            if cells.len() < 4 {
                return None;
            }
            let crn = cells[0].trim().to_string();
            let subject = cells[1].trim().to_string();
            let number = cells[2].trim().to_string();
            if crn.is_empty() || !crn.chars().all(|c| c.is_ascii_digit()) || subject.is_empty() || number.is_empty() {
                return None;
            }
            Some(CrossListedSection {
                crn,
                course: CourseKey { subject: subjects.code(&subject), number },
                title: cells[3].trim().to_string(),
                section: cells.get(4).map(|text| text.trim().to_string()).unwrap_or_default(),
            })
        })
        .collect()
}
