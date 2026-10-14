use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    pub code: String,
    pub banner_description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum College {
    Neu,
    Cps,
    Law,
}

impl College {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Neu => "Northeastern",
            Self::Cps => "Professional Studies",
            Self::Law => "School of Law",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Season {
    Fall,
    Winter,
    Spring,
    Summer1,
    SummerFull,
    Summer2,
}

impl Season {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Fall => "Fall",
            Self::Winter => "Winter",
            Self::Spring => "Spring",
            Self::Summer1 => "Summer 1",
            Self::SummerFull => "Summer",
            Self::Summer2 => "Summer 2",
        }
    }
}

impl Term {
    pub fn college(&self) -> College {
        match self.code.chars().last() {
            Some('4') | Some('5') => College::Cps,
            Some('2') | Some('8') => College::Law,
            _ => College::Neu,
        }
    }

    pub fn season(&self) -> Season {
        let chars: Vec<char> = self.code.chars().collect();
        if chars.len() != 6 {
            return Season::Fall;
        }
        match chars[4] {
            '1' => Season::Fall,
            '2' => Season::Winter,
            '3' => Season::Spring,
            '4' => Season::Summer1,
            '6' => Season::Summer2,
            _ => Season::SummerFull,
        }
    }

    pub fn calendar_year(&self) -> i32 {
        let year: i32 = self
            .code
            .chars()
            .take(4)
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        if self.season() == Season::Fall {
            year - 1
        } else {
            year
        }
    }

    pub fn is_view_only(&self) -> bool {
        self.banner_description
            .to_ascii_lowercase()
            .contains("view only")
    }

    pub fn name(&self) -> String {
        let mut s = self.banner_description.replace("(View Only)", "");
        s = s.trim().to_string();
        match self.college() {
            College::Neu => s = s.replace(" Semester", ""),
            College::Cps | College::Law => {
                for marker in [" CPS ", " Law "] {
                    if let Some(pos) = s.find(marker) {
                        let head = &s[..pos];
                        let tail = s[pos..].trim();
                        s = format!("{head} · {tail}");
                        break;
                    }
                }
            }
        }
        s.trim().to_string()
    }

    pub fn short_name(&self) -> String {
        self.name().split(" · ").next().unwrap_or("").to_string()
    }
}

impl PartialOrd for Term {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Term {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.code.cmp(&other.code)
    }
}
