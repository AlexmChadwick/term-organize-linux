//! Shared models. JSON field names match the Swift `Codable` catalogs and user data.

mod course;
mod degree;
mod preferences;
mod program;
mod requisite;
mod section;
mod term;
mod user;

pub use course::*;
pub use degree::*;
pub use preferences::*;
pub use program::*;
pub use requisite::*;
pub use section::*;
pub use term::*;
pub use user::*;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermCatalog {
    pub version: i32,
    pub term: Term,
    #[serde(default)]
    pub generated_at: f64,
    #[serde(default)]
    pub sections_updated_at: f64,
    pub courses: Vec<Course>,
    pub sections: Vec<CourseSection>,
    pub subjects: Vec<Subject>,
    pub attributes: Vec<AttributeInfo>,
    pub campuses: Vec<Campus>,
}

impl TermCatalog {
    pub fn recompute_postreqs(&mut self) {
        use std::collections::{BTreeSet, HashMap};
        let mut map: HashMap<CourseKey, BTreeSet<CourseKey>> = HashMap::new();
        for course in &self.courses {
            if let Some(prereqs) = &course.prereqs {
                for key in prereqs.referenced_courses() {
                    map.entry(key).or_default().insert(course.key());
                }
            }
        }
        for course in &mut self.courses {
            course.postreqs = map.remove(&course.key()).unwrap_or_default().into_iter().collect();
        }
    }
}

/// Finder-like ordering: case-insensitive, digit runs compared as numbers.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let ac: Vec<char> = a.chars().flat_map(|c| c.to_lowercase()).collect();
    let bc: Vec<char> = b.chars().flat_map(|c| c.to_lowercase()).collect();
    let (mut i, mut j) = (0, 0);
    while i < ac.len() && j < bc.len() {
        if ac[i].is_ascii_digit() && bc[j].is_ascii_digit() {
            let mut an = 0u64;
            let mut bn = 0u64;
            while i < ac.len() && ac[i].is_ascii_digit() {
                an = an.saturating_mul(10).saturating_add((ac[i] as u8 - b'0') as u64);
                i += 1;
            }
            while j < bc.len() && bc[j].is_ascii_digit() {
                bn = bn.saturating_mul(10).saturating_add((bc[j] as u8 - b'0') as u64);
                j += 1;
            }
            match an.cmp(&bn) {
                std::cmp::Ordering::Equal => {}
                other => return other,
            }
        } else {
            match ac[i].cmp(&bc[j]) {
                std::cmp::Ordering::Equal => {
                    i += 1;
                    j += 1;
                }
                other => return other,
            }
        }
    }
    ac.len().cmp(&bc.len())
}

pub fn list_join(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => {
            let mut s = items[..items.len() - 1].join(", ");
            s.push_str(", and ");
            s.push_str(&items[items.len() - 1]);
            s
        }
    }
}
