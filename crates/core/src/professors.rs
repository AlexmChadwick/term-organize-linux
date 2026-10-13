//! A professor's sections, one color per course.

use crate::index::{CatalogIndex, InstructorSummary};
use crate::model::*;

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
