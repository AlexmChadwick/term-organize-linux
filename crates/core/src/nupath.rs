//! NUpath matching: at most two limited areas per course, completed courses first.

use crate::model::NuPath;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct NuCourse {
    pub id: String,
    pub paths: HashSet<NuPath>,
    pub is_completed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct NuRequirement {
    pub path: NuPath,
    pub count: i32,
    pub limited: bool,
}

impl NuRequirement {
    pub const ALL: [NuRequirement; 13] = [
        NuRequirement { path: NuPath::Nd, count: 1, limited: true },
        NuRequirement { path: NuPath::Ei, count: 1, limited: true },
        NuRequirement { path: NuPath::Ic, count: 1, limited: true },
        NuRequirement { path: NuPath::Fq, count: 1, limited: true },
        NuRequirement { path: NuPath::Si, count: 1, limited: true },
        NuRequirement { path: NuPath::Ad, count: 1, limited: true },
        NuRequirement { path: NuPath::Dd, count: 1, limited: true },
        NuRequirement { path: NuPath::Er, count: 1, limited: true },
        NuRequirement { path: NuPath::Wf, count: 1, limited: false },
        NuRequirement { path: NuPath::Wi, count: 2, limited: false },
        NuRequirement { path: NuPath::Wd, count: 1, limited: false },
        NuRequirement { path: NuPath::Ex, count: 1, limited: false },
        NuRequirement { path: NuPath::Ce, count: 1, limited: false },
    ];
    pub fn title(self) -> String {
        format!("{} ({})", self.path.name(), self.path.short())
    }
}

pub const AREAS_PER_COURSE: usize = 2;

pub fn satisfied_through_coop(note: &str) -> Option<NuPath> {
    if !crate::degree_audit::is_coop_note(note) { return None; }
    let lower = note.to_ascii_lowercase();
    if lower.contains("integrating knowledge") || lower.contains("integration experience") || lower.contains("integration of experience") {
        return Some(NuPath::Ex);
    }
    if lower.contains("capstone") { return Some(NuPath::Ce); }
    NuPath::ALL.into_iter().find(|p| {
        let short = p.short().to_ascii_lowercase();
        lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == short)
    })
}

#[derive(Clone, Debug, Default)]
pub struct Matching {
    pub assigned: HashMap<NuPath, Vec<String>>,
}

impl Matching {
    pub fn new(courses: &[NuCourse], coop: &HashMap<NuPath, Vec<NuCourse>>) -> Self {
        let limited: Vec<NuPath> = NuRequirement::ALL.iter().filter(|r| r.limited).map(|r| r.path).collect();
        let mut assigned = match_limited(&limited, courses);
        for req in NuRequirement::ALL.iter().filter(|r| !r.limited) {
            let mut candidates: Vec<&NuCourse> = courses.iter().filter(|c| c.paths.contains(&req.path)).collect();
            if let Some(extra) = coop.get(&req.path) {
                candidates.extend(extra.iter());
            }
            let mut ordered: Vec<&NuCourse> = candidates.iter().copied().filter(|c| c.is_completed).collect();
            ordered.extend(candidates.iter().copied().filter(|c| !c.is_completed));
            let mut seen = HashSet::new();
            let ids: Vec<String> = ordered.into_iter().filter(|c| seen.insert(c.id.clone())).take(req.count as usize).map(|c| c.id.clone()).collect();
            assigned.insert(req.path, ids);
        }
        Self { assigned }
    }
    pub fn courses_for(&self, path: NuPath) -> Vec<String> {
        self.assigned.get(&path).cloned().unwrap_or_default()
    }
}

fn match_limited(areas: &[NuPath], courses: &[NuCourse]) -> HashMap<NuPath, Vec<String>> {
    let mut load: Vec<Vec<NuPath>> = vec![vec![]; courses.len()];
    let mut owner: HashMap<NuPath, usize> = HashMap::new();
    fn augment(area: NuPath, courses: &[NuCourse], load: &mut [Vec<NuPath>], owner: &mut HashMap<NuPath, usize>, allowed: &dyn Fn(usize) -> bool, visited: &mut HashSet<usize>) -> bool {
        for index in 0..courses.len() {
            if !allowed(index) || !courses[index].paths.contains(&area) || !visited.insert(index) { continue; }
            if load[index].len() < AREAS_PER_COURSE {
                load[index].push(area);
                owner.insert(area, index);
                return true;
            }
            let others = load[index].clone();
            for other in others {
                if augment(other, courses, load, owner, allowed, visited) {
                    load[index].retain(|p| *p != other);
                    load[index].push(area);
                    owner.insert(area, index);
                    return true;
                }
            }
        }
        false
    }
    for completed_only in [true, false] {
        for area in areas {
            if owner.contains_key(area) { continue; }
            let mut visited = HashSet::new();
            let allowed = |index: usize| if completed_only { courses[index].is_completed } else { true };
            let _ = augment(*area, courses, &mut load, &mut owner, &allowed, &mut visited);
        }
    }
    owner.into_iter().map(|(path, index)| (path, vec![courses[index].id.clone()])).collect()
}
