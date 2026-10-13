//! Academic-catalog requirement types. JSON matches Swift `Codable`.

use super::section::CourseKey;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogYear {
    pub start_year: i32,
}

impl CatalogYear {
    pub fn new(start_year: i32) -> Self {
        Self { start_year }
    }
    pub fn parse(text: &str) -> Option<Self> {
        let re = regex::Regex::new(r"(\d{4})\s*[-–]\s*(\d{2,4})").ok()?;
        let cap = re.captures(text)?;
        let start: i32 = cap[1].parse().ok()?;
        let end: i32 = cap[2].parse().ok()?;
        if end == start + 1 || end == (start + 1) % 100 {
            Some(Self { start_year: start })
        } else {
            None
        }
    }
    pub fn slug(self) -> String {
        format!("{}-{}", self.start_year, self.start_year + 1)
    }
    pub fn label(self) -> String {
        format!("{}–{}", self.start_year, self.start_year + 1)
    }
    /// July is the catalog turnover.
    pub fn containing(year: i32, month: u32) -> Self {
        Self { start_year: if month >= 7 { year } else { year - 1 } }
    }
}

impl std::fmt::Display for CatalogYear {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEditions {
    pub current: CatalogYear,
    pub archived: Vec<CatalogYear>,
    #[serde(default)]
    pub fetched_at: f64,
    #[serde(default)]
    pub is_estimated: bool,
}

impl CatalogEditions {
    pub const SITE: &'static str = "https://catalog.northeastern.edu/";

    pub fn all(&self) -> Vec<CatalogYear> {
        let mut years = self.archived.clone();
        if !years.contains(&self.current) {
            years.push(self.current);
        }
        years.sort_by_key(|year| std::cmp::Reverse(year.start_year));
        years.dedup();
        years
    }
    pub fn contains(&self, year: CatalogYear) -> bool {
        year == self.current || self.archived.contains(&year)
    }
    pub fn base_url(&self, year: CatalogYear) -> String {
        if year == self.current {
            Self::SITE.to_string()
        } else {
            format!("{}archive/{}/", Self::SITE, year.slug())
        }
    }
    pub fn url(&self, year: CatalogYear, path: &str) -> String {
        let trimmed = path.trim_start_matches('/');
        format!("{}{trimmed}", self.base_url(year))
    }
    pub fn estimated(year: i32, month: u32) -> Self {
        let current = CatalogYear::containing(year, month);
        let archived = (2016..current.start_year).rev().map(CatalogYear::new).collect();
        Self { current, archived, fetched_at: 0.0, is_estimated: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgramLevel {
    Undergraduate,
    Graduate,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogProgram {
    pub title: String,
    pub path: String,
}

impl CatalogProgram {
    pub fn level(&self) -> ProgramLevel {
        if self.path.starts_with("undergraduate/") {
            ProgramLevel::Undergraduate
        } else if self.path.starts_with("graduate/") {
            ProgramLevel::Graduate
        } else {
            ProgramLevel::Other
        }
    }
    pub fn parts(&self) -> (String, Option<String>, Option<String>) {
        let re = regex::Regex::new(r"^(.+?),\s*([A-Za-z][A-Za-z.]{1,9})(?:\s*\(([^)]+)\))?\s*$").unwrap();
        if let Some(c) = re.captures(&self.title) {
            (c[1].to_string(), Some(c[2].to_string()), c.get(3).map(|m| m.as_str().to_string()))
        } else {
            (self.title.clone(), None, None)
        }
    }
    pub fn name(&self) -> String {
        self.parts().0
    }
    pub fn degree(&self) -> Option<String> {
        self.parts().1
    }
    pub fn campus(&self) -> Option<String> {
        self.parts().2
    }
    pub fn title_without_campus(&self) -> String {
        match self.degree() {
            Some(d) => format!("{}, {d}", self.name()),
            None => self.title.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversitywideRequirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpa: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub residency_hours: Option<f64>,
    #[serde(default)]
    pub requires_experiential_learning: bool,
    #[serde(default)]
    pub fetched_at: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramRequirements {
    pub year: CatalogYear,
    pub program: CatalogProgram,
    pub sections: Vec<RequirementSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_hours: Option<f64>,
    #[serde(default)]
    pub fetched_at: f64,
    #[serde(default)]
    pub course_titles: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub universitywide: Option<UniversitywideRequirements>,
}

impl ProgramRequirements {
    pub fn concentrations(&self) -> Vec<&RequirementSection> {
        self.sections.iter().filter(|s| s.is_concentration).collect()
    }
    pub fn applicable_sections(&self, concentration_id: Option<&str>) -> Vec<&RequirementSection> {
        self.sections.iter().filter(|s| !s.is_concentration || Some(s.id.as_str()) == concentration_id).collect()
    }
    pub fn all_rules(&self) -> Vec<&RequirementRule> {
        self.sections.iter().flat_map(|s| s.groups.iter().flat_map(|g| g.rules.iter())).collect()
    }
    pub fn rule(&self, id: &str) -> Option<&RequirementRule> {
        self.all_rules().into_iter().find(|r| r.id == id)
    }
    pub fn degree_short_name(&self, id: &str) -> Option<String> {
        for section in &self.sections {
            for group in &section.groups {
                if let Some(rule) = group.rules.iter().find(|r| r.id == id) {
                    if group.rules.len() == 1 {
                        if let Some(title) = self.title_of_rule(id) {
                            if section.is_concentration && group.title.is_none() && !rule.degree_names_courses() {
                                return Some(format!("{} concentration", section.concentration_name()));
                            }
                            return Some(title);
                        }
                    }
                    if rule.degree_names_courses() {
                        return Some(rule.display_label());
                    }
                    if let Some(title) = &group.title {
                        return Some(title.clone());
                    }
                    if section.is_concentration {
                        return Some(format!("{} concentration", section.concentration_name()));
                    }
                    return Some(section.title.clone());
                }
            }
        }
        None
    }
    pub fn title_of_rule(&self, id: &str) -> Option<String> {
        for section in &self.sections {
            for group in &section.groups {
                if let Some(rule) = group.rules.iter().find(|r| r.id == id) {
                    if group.rules.len() == 1 {
                        if let Some(title) = group.title.as_ref().or(if section.is_concentration { Some(&section.title) } else { None }) {
                            return Some(title.clone());
                        }
                    }
                    return Some(rule.display_label());
                }
            }
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSection {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub is_concentration: bool,
    #[serde(default)]
    pub groups: Vec<RequirementGroup>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl RequirementSection {
    pub fn concentration_name(&self) -> String {
        let re1 = regex::Regex::new(r"(?i)^(Concentration|Track|Option|Pathway)(\s+in\s+|\s*:\s*)").unwrap();
        let re2 = regex::Regex::new(r"(?i)\s+(Concentration|Track|Option|Pathway)$").unwrap();
        let t = re2.replace(&re1.replace(&self.title, ""), "").trim_matches(|c: char| c == '*' || c.is_whitespace()).to_string();
        t
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequirementGroup {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub rules: Vec<RequirementRule>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleKind {
    Required(CourseOption),
    Choose {
        count: Option<i32>,
        hours: Option<f64>,
        options: Vec<CourseOption>,
        ranges: Vec<CourseRange>,
    },
    GeneralElectives {
        hours: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementRule {
    pub id: String,
    pub kind: RuleKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<f64>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_categories: Option<Vec<Option<String>>>,
}

impl RequirementRule {
    pub fn display_label(&self) -> String {
        self.label.strip_suffix(':').unwrap_or(&self.label).to_string()
    }
    pub fn degree_names_courses(&self) -> bool {
        regex::Regex::new(r"^[A-Z]{2,5}\s?\d{4}").unwrap().is_match(&self.display_label())
    }
    pub fn degree_is_general_electives(&self) -> bool {
        matches!(self.kind, RuleKind::GeneralElectives { .. })
    }
    pub fn listed_courses(&self) -> Vec<CourseKey> {
        match &self.kind {
            RuleKind::Required(o) => o.all_courses(),
            RuleKind::Choose { options, .. } => options.iter().flat_map(|o| o.all_courses()).collect(),
            RuleKind::GeneralElectives { .. } => vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseOption {
    pub alternatives: Vec<Vec<CourseKey>>,
}

impl CourseOption {
    pub fn all_courses(&self) -> Vec<CourseKey> {
        let mut seen = std::collections::BTreeSet::new();
        let mut out = vec![];
        for alt in &self.alternatives {
            for key in alt {
                if seen.insert(key.clone()) {
                    out.push(key.clone());
                }
            }
        }
        out
    }
    pub fn label(&self) -> String {
        self.alternatives
            .iter()
            .map(|set| set.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" and "))
            .collect::<Vec<_>>()
            .join(" or ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseRange {
    pub subject: String,
    pub lowest: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highest: Option<i32>,
    #[serde(default)]
    pub except: Vec<CourseKey>,
    pub label: String,
}

impl CourseRange {
    pub fn contains(&self, key: &CourseKey) -> bool {
        if key.subject != self.subject || self.except.iter().any(|e| e == key) {
            return false;
        }
        let digits: String = key.number.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() != 4 {
            return false;
        }
        let number: i32 = digits.parse().unwrap_or(0);
        number >= self.lowest && self.highest.map(|h| number <= h).unwrap_or(true)
    }
    pub fn described_label(&self) -> String {
        let mut text = match (self.lowest > 0, self.highest) {
            (true, Some(h)) => format!("{} {} to {} {}", self.subject, self.lowest, self.subject, h),
            (true, None) => format!("{} {} or higher", self.subject, self.lowest),
            (false, Some(h)) => format!("{} courses up to {} {}", self.subject, self.subject, h),
            (false, None) => format!("Any {} course", self.subject),
        };
        if !self.except.is_empty() {
            text.push_str(", except ");
            text.push_str(&self.except.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(", "));
        }
        text
    }
}
