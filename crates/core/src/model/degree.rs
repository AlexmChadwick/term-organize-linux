//! Degree plans, substitutions, and student-declared equivalences.

use super::program::CourseRange;
use super::section::CourseKey;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn new_id() -> Uuid {
    Uuid::new_v4()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanSeason {
    Winter,
    Spring,
    SummerA,
    SummerFull,
    SummerB,
    Fall,
}

impl PlanSeason {
    pub fn export_name(self) -> &'static str {
        match self {
            Self::Winter => "Winter",
            Self::Spring => "Spring",
            Self::SummerA => "Summer Session A",
            Self::SummerFull => "Summer Full",
            Self::SummerB => "Summer Session B",
            Self::Fall => "Fall",
        }
    }
    pub fn short_name(self) -> &'static str {
        match self {
            Self::SummerA => "Summer A",
            Self::SummerB => "Summer B",
            Self::SummerFull => "Summer",
            other => other.export_name(),
        }
    }
    pub fn start_month(self) -> i32 {
        match self {
            Self::Winter | Self::Spring => 1,
            Self::SummerA | Self::SummerFull => 5,
            Self::SummerB => 7,
            Self::Fall => 9,
        }
    }
    pub fn order_within_month(self) -> i32 {
        match self {
            Self::Winter | Self::SummerA => 0,
            Self::Spring | Self::SummerFull => 1,
            _ => 0,
        }
    }
    pub fn parse_export(raw: &str) -> Option<Self> {
        let s = raw.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
        Some(match s.as_str() {
            "fall" | "autumn" => Self::Fall,
            "spring" => Self::Spring,
            "winter" => Self::Winter,
            "summer session a" | "summer a" | "summer 1" | "summer i" | "summer session 1" => Self::SummerA,
            "summer session b" | "summer b" | "summer 2" | "summer ii" | "summer session 2" => Self::SummerB,
            "summer full" | "summer" | "full summer" | "summer session full" => Self::SummerFull,
            _ => return None,
        })
    }
    pub fn matches_term_season(self, season: super::term::Season) -> bool {
        use super::term::Season::*;
        matches!(
            (self, season),
            (Self::Fall, Fall) | (Self::Spring, Spring) | (Self::Winter, Winter) | (Self::SummerA, Summer1 | SummerFull) | (Self::SummerB, Summer2 | SummerFull) | (Self::SummerFull, SummerFull | Summer1 | Summer2)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DegreePlanCourse {
    #[serde(default = "new_id")]
    pub id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<CourseKey>,
    pub label: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub credits: f64,
}

impl DegreePlanCourse {
    pub fn catalog(key: CourseKey, title: impl Into<String>, credits: f64) -> Self {
        let label = key.to_string();
        Self { id: Uuid::new_v4(), key: Some(key), label, title: title.into(), credits }
    }
    pub fn other(label: impl Into<String>, title: impl Into<String>, credits: f64) -> Self {
        Self { id: Uuid::new_v4(), key: None, label: label.into(), title: title.into(), credits }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DegreePlanTerm {
    #[serde(default = "new_id")]
    pub id: Uuid,
    pub season: PlanSeason,
    pub year: i32,
    #[serde(default)]
    pub courses: Vec<DegreePlanCourse>,
}

impl DegreePlanTerm {
    pub fn new(season: PlanSeason, year: i32) -> Self {
        Self { id: Uuid::new_v4(), season, year, courses: vec![] }
    }
    pub fn name(&self) -> String {
        format!("{} {}", self.season.export_name(), self.year)
    }
    pub fn short_name(&self) -> String {
        format!("{} {}", self.season.short_name(), self.year)
    }
    pub fn credits(&self) -> f64 {
        self.courses.iter().map(|c| c.credits).sum()
    }
    pub fn month_index(&self) -> i32 {
        self.year * 12 + self.season.start_month() - 1
    }
    pub fn sort_key(&self) -> i32 {
        self.month_index() * 4 + self.season.order_within_month()
    }
    pub fn matches_term(&self, term: &super::term::Term) -> bool {
        term.calendar_year() == self.year && self.season.matches_term_season(term.season())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DegreePlan {
    #[serde(default = "new_id")]
    pub id: Uuid,
    #[serde(default = "default_plan_name")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub student_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub student_id: Option<String>,
    #[serde(default)]
    pub completed: Vec<DegreePlanCourse>,
    #[serde(default)]
    pub terms: Vec<DegreePlanTerm>,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_program_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_program_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concentration_id: Option<String>,
    #[serde(default)]
    pub substitutions: Vec<RequirementSubstitution>,
    #[serde(default)]
    pub course_equivalences: Vec<CourseEquivalence>,
}

fn default_plan_name() -> String {
    "Degree Plan".into()
}

impl Default for DegreePlan {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: default_plan_name(),
            program: None,
            student_name: None,
            student_id: None,
            completed: vec![],
            terms: vec![],
            is_primary: false,
            imported_from: None,
            printed_at: None,
            created_at: None,
            updated_at: None,
            catalog_year: None,
            catalog_program_path: None,
            catalog_program_title: None,
            concentration_id: None,
            substitutions: vec![],
            course_equivalences: vec![],
        }
    }
}

impl DegreePlan {
    pub fn completed_credits(&self) -> f64 {
        self.completed.iter().map(|c| c.credits).sum()
    }
    pub fn planned_credits(&self) -> f64 {
        self.terms.iter().map(|t| t.credits()).sum()
    }
    pub fn completed_keys(&self) -> std::collections::BTreeSet<CourseKey> {
        self.completed.iter().filter_map(|c| c.key.clone()).collect()
    }
    pub fn sort_terms(&mut self) {
        self.terms.sort_by_key(|t| t.sort_key());
    }
    pub fn term_matching(&self, term: &super::term::Term) -> Option<&DegreePlanTerm> {
        self.terms.iter().find(|t| t.matches_term(term))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseEquivalence {
    #[serde(default = "new_id")]
    pub id: Uuid,
    pub course: CourseKey,
    pub counts_as: CourseKey,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstitutionChange {
    CountCourse(CourseKey),
    Replace(RequirementReplacement),
    Waive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSubstitution {
    #[serde(default = "new_id")]
    pub id: Uuid,
    #[serde(rename = "ruleID")]
    pub rule_id: String,
    pub rule_title: String,
    pub change: SubstitutionChange,
    #[serde(default)]
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementReplacement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "likeRuleID")]
    pub like_rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub like_rule_title: Option<String>,
    #[serde(default)]
    pub courses: Vec<CourseKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub course_numbers: Option<CourseNumberLimit>,
}

impl RequirementReplacement {
    pub fn summary(&self) -> String {
        let amount = if let Some(count) = self.count {
            if count == 1 { "1 course".into() } else { format!("{count} courses") }
        } else if let Some(hours) = self.hours {
            format!("{} semester hours", format_hours(hours))
        } else {
            "Courses".into()
        };
        let mut sources = vec![amount];
        if let Some(title) = &self.like_rule_title {
            sources.push(format!("like {title}"));
        }
        if !self.courses.is_empty() {
            sources.push(format!("from {}", self.courses.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", ")));
        }
        let mut text = sources.join(" ");
        if self.like_rule_title.is_some() {
            if let Some(summary) = self.course_numbers.as_ref().and_then(|n| n.summary()) {
                text.push_str(", ");
                text.push_str(&summary);
            }
        }
        text
    }
}

fn format_hours(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        let tenths = (value * 10.0).round() / 10.0;
        if (tenths - tenths.round()).abs() < 1e-9 {
            format!("{}", tenths.round() as i64)
        } else {
            format!("{tenths:.1}")
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CourseNumberLimit {
    SameAsRequirement,
    AnyNumber,
    AtLeast(i32),
}

impl CourseNumberLimit {
    pub fn summary(&self) -> Option<String> {
        match self {
            Self::SameAsRequirement => None,
            Self::AnyNumber => Some("any course number".into()),
            Self::AtLeast(n) => Some(format!("numbered {n} or higher")),
        }
    }
}

pub fn apply_number_limit(range: &CourseRange, limit: Option<&CourseNumberLimit>) -> CourseRange {
    let mut range = range.clone();
    match limit {
        None | Some(CourseNumberLimit::SameAsRequirement) => return range,
        Some(CourseNumberLimit::AnyNumber) => range.lowest = 0,
        Some(CourseNumberLimit::AtLeast(n)) => range.lowest = *n,
    }
    range.label = range.described_label();
    range
}
