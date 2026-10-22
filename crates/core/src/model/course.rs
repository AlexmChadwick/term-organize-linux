use super::requisite::Requisite;
use super::section::CourseKey;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Course {
    pub subject: String,
    pub number: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub min_credits: f64,
    #[serde(default)]
    pub max_credits: f64,
    #[serde(default)]
    pub attributes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prereqs: Option<Requisite>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coreqs: Option<Requisite>,
    #[serde(default)]
    pub postreqs: Vec<CourseKey>,
    #[serde(default)]
    pub special_topics: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub college: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
    #[serde(default = "default_true")]
    pub details_loaded: bool,
}

fn default_true() -> bool {
    true
}

impl Course {
    pub fn key(&self) -> CourseKey {
        CourseKey {
            subject: self.subject.clone(),
            number: self.number.clone(),
        }
    }
    pub fn code(&self) -> String {
        self.key().to_string()
    }
    pub fn credits_label(&self) -> String {
        credit_label(self.min_credits, self.max_credits)
    }
    pub fn nupaths(&self) -> Vec<NuPath> {
        NuPath::from_codes(&self.attributes)
    }

    /// True when the course details were loaded and they list no prerequisites.
    /// A loaded course omits `prereqs` when there are none. A course whose
    /// details were not loaded leaves the field empty too, and that is unknown, not none.
    pub fn has_no_prerequisites(&self) -> bool {
        self.details_loaded
            && self
                .prereqs
                .as_ref()
                .is_none_or(|req| req.simplified().is_none())
    }

    /// True when the course details were loaded and they list no corequisites.
    /// Same split as [`Self::has_no_prerequisites`]: omitted on a loaded course means none,
    /// and empty before the details load means unknown.
    pub fn has_no_corequisites(&self) -> bool {
        self.details_loaded
            && self
                .coreqs
                .as_ref()
                .is_none_or(|req| req.simplified().is_none())
    }
}

pub fn credit_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

pub fn credit_label(min: f64, max: f64) -> String {
    let text = if (min - max).abs() < 1e-9 {
        credit_number(min)
    } else {
        format!("{}–{}", credit_number(min), credit_number(max))
    };
    let word = if (max - 1.0).abs() < 1e-9 && (min - 1.0).abs() < 1e-9 {
        "credit"
    } else {
        "credits"
    };
    format!("{text} {word}")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    pub code: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeInfo {
    pub code: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Campus {
    pub code: String,
    pub name: String,
}

impl Campus {
    pub fn is_online(&self) -> bool {
        self.name.to_ascii_lowercase().contains("online")
    }
}

pub const HONORS_ATTRIBUTE: &str = "GNHN";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NuPath {
    #[serde(rename = "NCND")]
    Nd,
    #[serde(rename = "NCEI")]
    Ei,
    #[serde(rename = "NCIC")]
    Ic,
    #[serde(rename = "NCFQ")]
    Fq,
    #[serde(rename = "NCSI")]
    Si,
    #[serde(rename = "NCAD")]
    Ad,
    #[serde(rename = "NCDD")]
    Dd,
    #[serde(rename = "NCER")]
    Er,
    #[serde(rename = "NCW1")]
    Wf,
    #[serde(rename = "NCWI")]
    Wi,
    #[serde(rename = "NCW3")]
    Wd,
    #[serde(rename = "NCEX")]
    Ex,
    #[serde(rename = "NCCE")]
    Ce,
}

impl NuPath {
    pub const ALL: [NuPath; 13] = [
        Self::Nd,
        Self::Ei,
        Self::Ic,
        Self::Fq,
        Self::Si,
        Self::Ad,
        Self::Dd,
        Self::Er,
        Self::Wf,
        Self::Wi,
        Self::Wd,
        Self::Ex,
        Self::Ce,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::Nd => "NCND",
            Self::Ei => "NCEI",
            Self::Ic => "NCIC",
            Self::Fq => "NCFQ",
            Self::Si => "NCSI",
            Self::Ad => "NCAD",
            Self::Dd => "NCDD",
            Self::Er => "NCER",
            Self::Wf => "NCW1",
            Self::Wi => "NCWI",
            Self::Wd => "NCW3",
            Self::Ex => "NCEX",
            Self::Ce => "NCCE",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Nd => "ND",
            Self::Ei => "EI",
            Self::Ic => "IC",
            Self::Fq => "FQ",
            Self::Si => "SI",
            Self::Ad => "AD",
            Self::Dd => "DD",
            Self::Er => "ER",
            Self::Wf => "WF",
            Self::Wi => "WI",
            Self::Wd => "WD",
            Self::Ex => "EX",
            Self::Ce => "CE",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Nd => "Natural & Designed World",
            Self::Ei => "Creative Expression & Innovation",
            Self::Ic => "Interpreting Culture",
            Self::Fq => "Formal & Quantitative Reasoning",
            Self::Si => "Societies & Institutions",
            Self::Ad => "Analyzing & Using Data",
            Self::Dd => "Difference & Diversity",
            Self::Er => "Ethical Reasoning",
            Self::Wf => "First-Year Writing",
            Self::Wi => "Writing Intensive",
            Self::Wd => "Advanced Writing in the Disciplines",
            Self::Ex => "Integration Experience",
            Self::Ce => "Capstone Experience",
        }
    }

    pub fn from_codes(codes: &[String]) -> Vec<NuPath> {
        Self::ALL
            .into_iter()
            .filter(|p| codes.iter().any(|c| c == p.code()))
            .collect()
    }

    pub fn from_short(short: &str) -> Option<NuPath> {
        Self::ALL
            .into_iter()
            .find(|p| p.short().eq_ignore_ascii_case(short))
    }
}

/// What the primary degree plan does with a course (`CourseDegreePlanStanding`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DegreeStandingKind {
    Completed {
        via: Option<CourseKey>,
    },
    Planned {
        semester: String,
        via: Option<CourseKey>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DegreePlanStanding {
    pub plan_id: Uuid,
    pub kind: DegreeStandingKind,
}

impl DegreePlanStanding {
    pub fn text(&self) -> String {
        match &self.kind {
            DegreeStandingKind::Completed { via: None } => "Completed".into(),
            DegreeStandingKind::Completed { via: Some(key) } => format!("Completed via {key}"),
            DegreeStandingKind::Planned {
                semester,
                via: None,
            } => format!("Planned · {semester}"),
            DegreeStandingKind::Planned {
                semester,
                via: Some(key),
            } => format!("Planned via {key} · {semester}"),
        }
    }
    pub fn symbol(&self) -> &'static str {
        match self.kind {
            DegreeStandingKind::Completed { .. } => "checkmark.circle.fill",
            DegreeStandingKind::Planned { .. } => "calendar",
        }
    }
    pub fn spoken_text(&self) -> String {
        match &self.kind {
            DegreeStandingKind::Completed { via: None } => "Completed, in your degree plan".into(),
            DegreeStandingKind::Completed { via: Some(key) } => {
                format!("Completed via {key}, in your degree plan")
            }
            DegreeStandingKind::Planned {
                semester,
                via: None,
            } => format!("Planned for {semester} in your degree plan"),
            DegreeStandingKind::Planned {
                semester,
                via: Some(key),
            } => format!("Planned via {key} for {semester} in your degree plan"),
        }
    }
}
