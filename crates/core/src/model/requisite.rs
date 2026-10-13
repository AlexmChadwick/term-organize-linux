use super::section::CourseKey;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Requisite {
    Course(CourseKey),
    Test { name: String, score: Option<f64> },
    And(Vec<Requisite>),
    Or(Vec<Requisite>),
}

impl Requisite {
    pub fn referenced_courses(&self) -> Vec<CourseKey> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        fn visit(r: &Requisite, seen: &mut std::collections::HashSet<CourseKey>, out: &mut Vec<CourseKey>) {
            match r {
                Requisite::Course(k) => {
                    if seen.insert(k.clone()) {
                        out.push(k.clone());
                    }
                }
                Requisite::Test { .. } => {}
                Requisite::And(items) | Requisite::Or(items) => items.iter().for_each(|i| visit(i, seen, out)),
            }
        }
        visit(self, &mut seen, &mut out);
        out
    }

    pub fn is_satisfied(&self, completed: &std::collections::HashSet<CourseKey>, assume_tests: bool) -> bool {
        match self {
            Requisite::Course(k) => completed.contains(k),
            Requisite::Test { .. } => assume_tests,
            Requisite::And(items) => items.iter().all(|i| i.is_satisfied(completed, assume_tests)),
            Requisite::Or(items) => items.iter().any(|i| i.is_satisfied(completed, assume_tests)),
        }
    }

    pub fn map_courses<F: Fn(&CourseKey) -> CourseKey>(&self, f: &F) -> Requisite {
        match self {
            Requisite::Course(k) => Requisite::Course(f(k)),
            Requisite::Test { .. } => self.clone(),
            Requisite::And(items) => Requisite::And(items.iter().map(|i| i.map_courses(f)).collect()),
            Requisite::Or(items) => Requisite::Or(items.iter().map(|i| i.map_courses(f)).collect()),
        }
    }

    pub fn simplified(&self) -> Option<Requisite> {
        match self {
            Requisite::Course(_) | Requisite::Test { .. } => Some(self.clone()),
            Requisite::And(items) => flatten(items, true),
            Requisite::Or(items) => flatten(items, false),
        }
    }

    pub fn is_condition(&self) -> bool {
        matches!(self, Requisite::And(_) | Requisite::Or(_))
    }

    pub fn summary(&self) -> String {
        match self {
            Requisite::Course(k) => k.to_string(),
            Requisite::Test { name, score } => match score {
                Some(s) => {
                    let n = if (s - s.round()).abs() < 1e-9 { format!("{}", *s as i64) } else { format!("{s}") };
                    format!("{name} ({n})")
                }
                None => name.clone(),
            },
            Requisite::And(items) => items
                .iter()
                .map(|i| if i.is_condition() { format!("({})", i.summary()) } else { i.summary() })
                .collect::<Vec<_>>()
                .join(" and "),
            Requisite::Or(items) => items
                .iter()
                .map(|i| if i.is_condition() { format!("({})", i.summary()) } else { i.summary() })
                .collect::<Vec<_>>()
                .join(" or "),
        }
    }
}

fn flatten(items: &[Requisite], and: bool) -> Option<Requisite> {
    let mut flat = Vec::new();
    for item in items {
        if let Some(child) = item.simplified() {
            match (&child, and) {
                (Requisite::And(inner), true) | (Requisite::Or(inner), false) => flat.extend(inner.clone()),
                _ => flat.push(child),
            }
        }
    }
    match flat.len() {
        0 => None,
        1 => Some(flat.remove(0)),
        _ => Some(if and { Requisite::And(flat) } else { Requisite::Or(flat) }),
    }
}

impl Serialize for Requisite {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match self {
            Requisite::Course(k) => {
                map.serialize_entry("s", &k.subject)?;
                map.serialize_entry("n", &k.number)?;
            }
            Requisite::Test { name, score } => {
                map.serialize_entry("test", name)?;
                if let Some(score) = score {
                    map.serialize_entry("score", score)?;
                }
            }
            Requisite::And(items) => map.serialize_entry("and", items)?,
            Requisite::Or(items) => map.serialize_entry("or", items)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Requisite {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Requisite;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "requisite object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut subject = None;
                let mut number = None;
                let mut test = None;
                let mut score = None;
                let mut and = None;
                let mut or = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "s" => subject = Some(map.next_value()?),
                        "n" => number = Some(map.next_value()?),
                        "test" => test = Some(map.next_value()?),
                        "score" => score = Some(map.next_value()?),
                        "and" => and = Some(map.next_value()?),
                        "or" => or = Some(map.next_value()?),
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }
                if let Some(items) = and {
                    return Ok(Requisite::And(items));
                }
                if let Some(items) = or {
                    return Ok(Requisite::Or(items));
                }
                if let Some(name) = test {
                    return Ok(Requisite::Test { name, score });
                }
                Ok(Requisite::Course(CourseKey {
                    subject: subject.ok_or_else(|| de::Error::missing_field("s"))?,
                    number: number.ok_or_else(|| de::Error::missing_field("n"))?,
                }))
            }
        }
        deserializer.deserialize_map(V)
    }
}
