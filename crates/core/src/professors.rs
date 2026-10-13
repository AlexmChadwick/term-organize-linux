//! A professor's sections, one color per course.

use crate::index::{CatalogIndex, InstructorSummary};
use crate::model::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct ProfessorCourse {
    pub key: CourseKey,
    pub title: String,
    pub color_index: i32,
    pub sections: Vec<CourseSection>,
}

#[derive(Clone, Debug)]
pub struct ProfessorPage {
    pub instructor: InstructorSummary,
    pub courses: Vec<ProfessorCourse>,
}

pub fn page(index: &CatalogIndex, name: &str) -> Option<ProfessorPage> {
    let instructor = index.instructors.iter().find(|i| i.name.eq_ignore_ascii_case(name) || i.display_name.eq_ignore_ascii_case(name))?.clone();
    let mut courses = Vec::new();
    for key in &instructor.course_keys {
        let mut sections: Vec<CourseSection> = index.sections_for(key).into_iter().filter(|s| {
            s.instructors.as_ref().is_some_and(|list| list.iter().any(|i| i.name == instructor.name))
        }).collect();
        sections.sort_by(|a, b| a.section_number.cmp(&b.section_number).then(a.crn.cmp(&b.crn)));
        let title = index.course(key).map(|c| c.title.clone()).unwrap_or_default();
        courses.push(ProfessorCourse { key: key.clone(), title, color_index: courses.len() as i32 % 10, sections });
    }
    courses.sort_by(|a, b| a.key.cmp(&b.key));
    for (i, course) in courses.iter_mut().enumerate() { course.color_index = i as i32 % 10; }
    Some(ProfessorPage { instructor, courses })
}

/// Sections still in the index, ordered by course then section number (`ProfessorPage.sections`).
pub fn sections_of(summary: &InstructorSummary, index: &CatalogIndex) -> Vec<CourseSection> {
    let mut sections: Vec<CourseSection> = summary.crns.iter().filter_map(|crn| index.section_crn(crn).cloned()).collect();
    sections.sort_by(|left, right| {
        left.course_key().cmp(&right.course_key()).then_with(|| crate::natural_cmp(&left.section_number, &right.section_number))
    });
    sections
}

fn color_slot(code: &str) -> usize {
    let mut hash: u64 = 5381;
    for byte in code.as_bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u64::from(*byte));
    }
    (hash % 10) as usize
}

/// A stable palette slot per course. Collisions walk forward while a free color remains
/// (`ProfessorPage.courseColors` / `CourseColor.forCourse`).
pub fn course_colors(keys: &[CourseKey]) -> HashMap<CourseKey, usize> {
    let mut used = HashSet::new();
    let mut colors = HashMap::new();
    for key in keys {
        let mut slot = color_slot(&key.to_string());
        if used.len() < 10 {
            while used.contains(&slot) {
                slot = (slot + 1) % 10;
            }
        }
        used.insert(slot);
        colors.insert(key.clone(), slot);
    }
    colors
}

pub fn section_note(key: &CourseKey, sections: &[CourseSection]) -> String {
    let numbers: Vec<&str> = sections.iter().filter(|section| &section.course_key() == key).map(|section| section.section_number.as_str()).collect();
    let label = if numbers.len() == 1 { "Section" } else { "Sections" };
    format!("{label} {}", numbers.join(", "))
}

pub fn summary_text(sections: &[CourseSection]) -> String {
    let courses = sections.iter().map(|section| section.course_key()).collect::<HashSet<_>>().len();
    format!("Teaches {} across {}", count_phrase(sections.len(), "section"), count_phrase(courses, "course"))
}

fn count_phrase(count: usize, noun: &str) -> String {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}

pub fn rate_my_professors_url(display_name: &str) -> String {
    format!("https://www.ratemyprofessors.com/search/professors/696?q={}", percent_encode(display_name))
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        let plain = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if plain {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
