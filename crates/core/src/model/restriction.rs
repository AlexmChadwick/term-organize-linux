//! Enrollment restrictions from Banner's per-section HTML.
//!
//! A section is CPS-only when its Must Levels list is only CPS levels, or its
//! Must Colleges list is only the College of Professional Studies. Cannot lists
//! never make a section CPS-only. Missing HTML stays unknown.

use serde::{Deserialize, Serialize};

pub const CPS_ONLY_NOTE: &str = "Restricted to CPS (College of Professional Studies) students";
pub const CPS_SOME_NOTE: &str = "Some sections are CPS only";

const CPS_LEVEL_PREFIX: &str = "CPS -";
const CPS_COLLEGE: &str = "College of Professional Studies";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RestrictionKind {
    Must,
    Cannot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentRestriction {
    pub kind: RestrictionKind,
    /// Banner category, such as `Levels` or `Colleges`.
    pub category: String,
    pub values: Vec<String>,
}

impl EnrollmentRestriction {
    pub fn is_levels(&self) -> bool {
        self.category.eq_ignore_ascii_case("Levels")
    }

    pub fn is_colleges(&self) -> bool {
        self.category.eq_ignore_ascii_case("Colleges")
    }
}

/// True when the loaded restriction list limits the section to CPS students.
pub fn is_cps_only(restrictions: &[EnrollmentRestriction]) -> bool {
    let mut levels = Vec::new();
    let mut colleges = Vec::new();
    for restriction in restrictions {
        if restriction.kind != RestrictionKind::Must {
            continue;
        }
        if restriction.is_levels() {
            levels.extend(restriction.values.iter().cloned());
        } else if restriction.is_colleges() {
            colleges.extend(restriction.values.iter().cloned());
        }
    }
    let levels_only = !levels.is_empty() && levels.iter().all(|value| is_cps_level(value));
    let colleges_only = !colleges.is_empty() && colleges.iter().all(|value| is_cps_college(value));
    levels_only || colleges_only
}

pub fn is_cps_level(value: &str) -> bool {
    value.trim().starts_with(CPS_LEVEL_PREFIX)
}

pub fn is_cps_college(value: &str) -> bool {
    normalized_college(value).eq_ignore_ascii_case(CPS_COLLEGE)
}

fn normalized_college(value: &str) -> String {
    let bare = strip_parenthetical_code(value.trim());
    let mut out = format!(" {bare} ");
    for (from, to) in [
        (" Coll of ", " College of "),
        (" Col of ", " College of "),
        (" Coll ", " College "),
        (" Prof ", " Professional "),
    ] {
        out = out.replace(from, to);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_parenthetical_code(value: &str) -> &str {
    let Some(open) = value.rfind('(') else {
        return value;
    };
    let code = value[open + 1..].trim_end_matches(')').trim();
    if code.is_empty()
        || code.len() > 6
        || !code
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return value;
    }
    value[..open].trim()
}

/// `None` when nothing has been loaded. `Some(true)` when every known section is CPS-only.
/// Unknown sections stay out of the decision.
pub fn course_cps_only(known: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let mut flags = known.into_iter().flatten();
    let first = flags.next()?;
    Some(first && flags.all(|flag| flag))
}

/// Caption for a course in search and the plan.
/// All known sections CPS-only uses the section sentence. A mix uses the shorter note.
/// Nothing known, or no known CPS section, shows nothing.
pub fn cps_note(known: impl IntoIterator<Item = Option<bool>>) -> Option<&'static str> {
    let flags: Vec<bool> = known.into_iter().flatten().collect();
    let cps = flags.iter().filter(|flag| **flag).count();
    if flags.is_empty() || cps == 0 {
        None
    } else if cps == flags.len() {
        Some(CPS_ONLY_NOTE)
    } else {
        Some(CPS_SOME_NOTE)
    }
}
