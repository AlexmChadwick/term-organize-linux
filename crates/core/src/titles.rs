//! Course-title comparison. Port of `CourseTitleMatcher.swift`.

use std::collections::BTreeSet;

pub const MINIMUM_SIMILARITY: f64 = 0.6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Title {
    pub words: BTreeSet<String>,
    pub parts: BTreeSet<String>,
    pub qualifiers: BTreeSet<String>,
    pub cut_off_word: Option<String>,
}

impl Title {
    pub fn parse(text: &str) -> Self {
        let mut words = BTreeSet::new();
        let mut parts = BTreeSet::new();
        let mut last = None;
        for raw in tokens(text) {
            if let Some(part) = part_of(&raw) {
                parts.insert(part);
                last = None;
                continue;
            }
            let singular = singular(&raw);
            let word = synonym(singular.as_str())
                .unwrap_or(singular.as_str())
                .to_string();
            if filler(&word) {
                last = None;
                continue;
            }
            words.insert(word.clone());
            last = Some(word);
        }
        let qualifiers = words.intersection(&qualifier_set()).cloned().collect();
        let trimmed = text.trim();
        let cut_off_word = if trimmed.chars().count() >= 29 {
            last.filter(|w| w.chars().count() >= 3)
        } else {
            None
        };
        Self {
            words,
            parts,
            qualifiers,
            cut_off_word,
        }
    }

    pub fn is_distinctive(&self) -> bool {
        self.words.len() >= 2 && !self.words.is_subset(&format_set())
    }
}

pub fn similarity(a: &str, b: &str) -> Option<f64> {
    similarity_min(a, b, MINIMUM_SIMILARITY)
}

pub fn similarity_min(a: &str, b: &str, minimum: f64) -> Option<f64> {
    similarity_titles(&Title::parse(a), &Title::parse(b), minimum)
}

pub fn similarity_titles(a: &Title, b: &Title, minimum: f64) -> Option<f64> {
    if !a.parts.is_empty() && !b.parts.is_empty() && a.parts != b.parts {
        return None;
    }
    if a.qualifiers != b.qualifiers {
        return None;
    }
    let a_words = completing(a, &b.words);
    let b_words = completing(b, &a.words);
    if a_words.len() < 2 || b_words.len() < 2 {
        return None;
    }
    let shared = a_words.intersection(&b_words).count();
    if shared == 0 {
        return None;
    }
    if a_words.iter().all(|w| format_set().contains(w))
        || b_words.iter().all(|w| format_set().contains(w))
    {
        return None;
    }
    let union = a_words.union(&b_words).count();
    let score = shared as f64 / union as f64;
    if score >= minimum {
        return Some(score);
    }
    let contained = shared == a_words.len() || shared == b_words.len();
    if contained {
        Some(minimum)
    } else {
        None
    }
}

fn completing(title: &Title, other: &BTreeSet<String>) -> BTreeSet<String> {
    let Some(cut) = &title.cut_off_word else {
        return title.words.clone();
    };
    if other.contains(cut) {
        return title.words.clone();
    }
    let Some(full) = other
        .iter()
        .find(|w| w.starts_with(cut.as_str()) && w.len() > cut.len())
    else {
        return title.words.clone();
    };
    let mut words = title.words.clone();
    words.remove(cut);
    words.insert(full.clone());
    words
}

fn tokens(text: &str) -> Vec<String> {
    let folded = text.replace('&', " and ").replace(['\'', '’'], "");
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in folded.chars() {
        let ch = strip_mark(ch);
        if ch.is_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn strip_mark(ch: char) -> char {
    match ch {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

fn part_of(token: &str) -> Option<String> {
    if token.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Some(token.to_string());
    }
    match token {
        "i" => Some("1".into()),
        "ii" => Some("2".into()),
        "iii" => Some("3".into()),
        "iv" => Some("4".into()),
        "v" => Some("5".into()),
        "vi" => Some("6".into()),
        "vii" => Some("7".into()),
        "viii" => Some("8".into()),
        "ix" => Some("9".into()),
        "x" => Some("10".into()),
        _ => None,
    }
}

fn singular(word: &str) -> String {
    if word.len() <= 3 {
        return word.to_string();
    }
    if word.ends_with("ies") && word.len() > 4 {
        return format!("{}y", &word[..word.len() - 3]);
    }
    if word.ends_with("sses") {
        return word[..word.len() - 2].to_string();
    }
    if word.ends_with('s')
        && !word.ends_with("ss")
        && !word.ends_with("us")
        && !word.ends_with("is")
    {
        return word[..word.len() - 1].to_string();
    }
    word.to_string()
}

fn synonym(word: &str) -> Option<&str> {
    match word {
        "laboratory" => Some("lab"),
        _ => None,
    }
}

fn filler(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "and"
            | "as"
            | "at"
            | "by"
            | "for"
            | "from"
            | "in"
            | "into"
            | "of"
            | "on"
            | "or"
            | "the"
            | "to"
            | "with"
            | "principle"
            | "introduction"
            | "introductory"
            | "intro"
            | "fundamental"
            | "topic"
            | "prncple"
            | "prncpl"
            | "intr"
            | "introd"
    )
}

fn qualifier_set() -> BTreeSet<String> {
    [
        "lab",
        "recitation",
        "advanced",
        "intermediate",
        "elementary",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn format_set() -> BTreeSet<String> {
    [
        "special",
        "directed",
        "independent",
        "study",
        "research",
        "thesis",
        "dissertation",
        "seminar",
        "project",
        "practicum",
        "capstone",
        "continuation",
        "experience",
        "internship",
        "co",
        "op",
        "work",
        "term",
        "workshop",
        "lab",
        "reading",
        "tutorial",
        "residency",
        "preparation",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}
