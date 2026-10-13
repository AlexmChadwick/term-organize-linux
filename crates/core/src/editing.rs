//! Edits a degree plan makes when a course is dropped onto a semester.
//! Port of `DegreePlanEditing.swift`.

use crate::model::*;
use uuid::Uuid;

pub fn add(course: DegreePlanCourse, term_id: Uuid, plan: &mut DegreePlan) {
    if let Some(term) = plan.terms.iter_mut().find(|term| term.id == term_id) {
        term.courses.push(course);
    }
}

pub fn move_course(course_id: Uuid, term_id: Uuid, plan: &mut DegreePlan) {
    let Some(moved) = take(course_id, plan) else { return };
    if let Some(term) = plan.terms.iter_mut().find(|term| term.id == term_id) {
        term.courses.push(moved);
    }
}

pub fn move_course_at(course_id: Uuid, term_id: Uuid, position: i32, plan: &mut DegreePlan) {
    let Some(destination) = plan.terms.iter().position(|term| term.id == term_id) else { return };
    let mut index = position;
    if let Some(current) = plan.terms[destination].courses.iter().position(|course| course.id == course_id) {
        if (current as i32) < position {
            index -= 1;
        }
    }
    let Some(moved) = take(course_id, plan) else { return };
    let destination = plan.terms.iter().position(|term| term.id == term_id).unwrap();
    let at = index.clamp(0, plan.terms[destination].courses.len() as i32) as usize;
    plan.terms[destination].courses.insert(at, moved);
}

fn take(course_id: Uuid, plan: &mut DegreePlan) -> Option<DegreePlanCourse> {
    for term in &mut plan.terms {
        if let Some(position) = term.courses.iter().position(|course| course.id == course_id) {
            return Some(term.courses.remove(position));
        }
    }
    None
}
