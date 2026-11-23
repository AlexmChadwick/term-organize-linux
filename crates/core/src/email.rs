//! Instructor addresses stored in a catalog snapshot.
//!
//! Banner sometimes puts several addresses in one field, and some of those are
//! personal. A kept address has the domain northeastern.edu or neu.edu exactly.
//! Subdomains such as husky.neu.edu, ccs.neu.edu, and mail.neu.edu are dropped.

use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressRedaction {
    pub kept: usize,
    pub removed_by_domain: BTreeMap<String, usize>,
}

/// True only for a complete `local@domain` string whose domain is exactly
/// northeastern.edu or neu.edu, compared after ASCII case folding. Nothing is
/// trimmed, so a trailing dot, a trailing letter, or non-ASCII text in the
/// domain makes the address disallowed.
pub fn is_northeastern_address(address: &str) -> bool {
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local.bytes().all(is_local_byte)
        && domain.is_ascii()
        && (domain.eq_ignore_ascii_case("northeastern.edu")
            || domain.eq_ignore_ascii_case("neu.edu"))
}

/// Keep Northeastern addresses from a Banner field. Fields often list more than
/// one address, separated by spaces, commas, or semicolons.
pub fn retain_northeastern_addresses(field: &str) -> Option<String> {
    let kept: Vec<&str> = addresses_in(field)
        .into_iter()
        .filter(|address| is_northeastern_address(address))
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join(" "))
    }
}

pub fn addresses_in(text: &str) -> Vec<&str> {
    spans_to_addresses(text, email_spans(text, false))
}

/// Addresses in free text. One period right after an address is sentence
/// punctuation, not part of the address, so it is left out of the candidate
/// before strict validation. A second period stays and fails validation.
pub fn prose_addresses_in(text: &str) -> Vec<&str> {
    spans_to_addresses(text, email_spans(text, true))
}

fn spans_to_addresses(text: &str, spans: Vec<(usize, usize)>) -> Vec<&str> {
    spans
        .into_iter()
        .filter_map(|(start, end)| text.get(start..end))
        .collect()
}

pub fn redact_json_emails(value: &mut Value) -> AddressRedaction {
    let mut scan = AddressRedaction::default();
    walk(value, &mut scan);
    scan
}

fn walk(value: &mut Value, scan: &mut AddressRedaction) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if is_email_key(key) {
                    redact_email_node(child, scan);
                } else {
                    walk(child, scan);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, scan);
            }
        }
        Value::String(text) => {
            let next = redact_text(text, scan);
            if next != *text {
                *text = next;
            }
        }
        _ => {}
    }
}

fn is_email_key(key: &str) -> bool {
    matches!(key, "email" | "emailAddress" | "email_address")
}

fn redact_email_node(value: &mut Value, scan: &mut AddressRedaction) {
    let Value::String(text) = value else {
        return;
    };
    for address in addresses_in(text) {
        note(scan, address);
    }
    match retain_northeastern_addresses(text) {
        Some(kept) => *value = Value::String(kept),
        None => *value = Value::Null,
    }
}

fn redact_text(text: &str, scan: &mut AddressRedaction) -> String {
    let spans = email_spans(text, true);
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut changed = false;
    for (start, end) in spans {
        let Some(address) = text.get(start..end) else {
            continue;
        };
        out.push_str(&text[last..start]);
        if is_northeastern_address(address) {
            scan.kept += 1;
            out.push_str(address);
        } else {
            changed = true;
            if let Some(domain) = domain_of(address) {
                *scan.removed_by_domain.entry(domain).or_default() += 1;
            }
        }
        last = end;
    }
    out.push_str(&text[last..]);
    if changed {
        tidy_gap(&out)
    } else {
        text.to_string()
    }
}

fn note(scan: &mut AddressRedaction, address: &str) {
    if is_northeastern_address(address) {
        scan.kept += 1;
    } else if let Some(domain) = domain_of(address) {
        *scan.removed_by_domain.entry(domain).or_default() += 1;
    }
}

fn domain_of(address: &str) -> Option<String> {
    let (_, domain) = address.split_once('@')?;
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_ascii_lowercase())
    }
}

fn tidy_gap(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch == ' ' {
            if !space {
                out.push(' ');
            }
            space = true;
            continue;
        }
        if matches!(ch, ',' | '.' | ';' | ':') && out.ends_with(' ') {
            out.pop();
        }
        space = false;
        out.push(ch);
    }
    out
}

/// Candidates are the full run of local bytes before an `@` and the full run of
/// non-delimiter bytes after it. The domain is never trimmed, so validation
/// sees exactly what follows the `@`. With `prose`, a single trailing period
/// is dropped from the candidate first.
fn email_spans(text: &str, prose: bool) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'@' {
            let mut start = index;
            while start > 0 && is_local_byte(bytes[start - 1]) {
                start -= 1;
            }
            let mut end = index + 1;
            while end < bytes.len() && !is_domain_delimiter(bytes[end]) {
                end += 1;
            }
            if prose && end > index + 1 && bytes[end - 1] == b'.' {
                end -= 1;
            }
            if start < index && end > index + 1 && text[index + 1..end].contains('.') {
                spans.push((start, end));
                index = end;
                continue;
            }
        }
        index += 1;
    }
    spans
}

/// RFC 5322 atext plus dot, and any non-ASCII byte so a local part is never cut
/// short at a UTF-8 sequence.
fn is_local_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || byte >= 0x80
        || matches!(
            byte,
            b'.' | b'!'
                | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'/'
                | b'='
                | b'?'
                | b'^'
                | b'_'
                | b'`'
                | b'{'
                | b'|'
                | b'}'
                | b'~'
        )
}

fn is_domain_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || byte.is_ascii_control()
        || matches!(
            byte,
            b',' | b';' | b'<' | b'>' | b'(' | b')' | b'[' | b']' | b'"'
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive;
    use regex::Regex;

    #[test]
    fn mixed_banner_field_keeps_northeastern_addresses() {
        assert_eq!(
            retain_northeastern_addresses("ada@northeastern.edu ada.example@gmail.com").as_deref(),
            Some("ada@northeastern.edu")
        );
        assert!(retain_northeastern_addresses("person@comcast.net").is_none());
        assert_eq!(
            retain_northeastern_addresses("Ada@neu.edu").as_deref(),
            Some("Ada@neu.edu")
        );
        assert!(retain_northeastern_addresses("person@husky.neu.edu, other@coe.neu.edu").is_none());
        assert!(!is_northeastern_address("x@ccs.neu.edu"));
        assert!(!is_northeastern_address("x@ece.neu.edu"));
        assert!(!is_northeastern_address("x@sub.northeastern.edu"));
        assert!(!is_northeastern_address("lab@mail.neu.edu"));
        assert!(!is_northeastern_address("x@gmail.com"));
        assert!(!is_northeastern_address("x@harvard.edu"));
        assert!(!is_northeastern_address("x@va.gov"));
        assert!(!is_northeastern_address("x@notnortheastern.edu"));
        assert!(!is_northeastern_address("not an email"));
    }

    #[test]
    fn prose_drops_a_personal_address_and_keeps_the_work_one() {
        let mut value = serde_json::json!({
            "blurb": "Write ada@northeastern.edu or ada@gmail.com today.",
            "email": "ada@northeastern.edu ada@yahoo.com"
        });
        let scan = redact_json_emails(&mut value);
        assert_eq!(value["blurb"], "Write ada@northeastern.edu or today.");
        assert_eq!(value["email"], "ada@northeastern.edu");
        assert_eq!(scan.kept, 2);
        assert_eq!(scan.removed_by_domain.get("gmail.com"), Some(&1));
        assert_eq!(scan.removed_by_domain.get("yahoo.com"), Some(&1));
    }

    #[test]
    fn domain_is_validated_whole_with_no_trimming() {
        assert!(is_northeastern_address("x@northeastern.edu"));
        assert!(is_northeastern_address("x@NorthEastern.EDU"));
        assert!(is_northeastern_address("x@NEU.edu"));
        assert!(!is_northeastern_address("x@northeastern.edu."));
        assert!(!is_northeastern_address("x@neu.edu-"));
        assert!(!is_northeastern_address("x@northeastern.eduX"));
        assert!(!is_northeastern_address("x@northeastern.edu\u{e9}"));
        assert!(!is_northeastern_address("x@northea\u{17f}tern.edu"));
        assert!(!is_northeastern_address("x@neu.edu\u{2024}"));
        assert!(!is_northeastern_address("x@husky.neu.edu"));
        assert!(!is_northeastern_address(" x@neu.edu"));
        assert!(!is_northeastern_address("x@neu.edu "));
        assert!(!is_northeastern_address("@neu.edu"));
        assert!(!is_northeastern_address("a@b@neu.edu"));
    }

    #[test]
    fn extractor_keeps_whole_candidates() {
        for bad in [
            "x@northeastern.edu.",
            "x@northeastern.edu-",
            "x@northeastern.eduX",
            "x@northeastern.edu\u{e9}",
            "x@neu.edu\u{4e2d}",
            "x@northeastern.edu.evil.com",
        ] {
            assert!(retain_northeastern_addresses(bad).is_none(), "{bad}");
            if bad == "x@northeastern.edu." {
                continue;
            }
            let mut value = serde_json::json!({ "email": bad, "note": format!("see {bad} now") });
            redact_json_emails(&mut value);
            assert!(value["email"].is_null(), "{bad}");
            assert_eq!(value["note"], "see now", "{bad}");
        }
        assert_eq!(
            addresses_in("x@northeastern.edu. y"),
            ["x@northeastern.edu."]
        );
        assert_eq!(
            prose_addresses_in("x@northeastern.edu. y"),
            ["x@northeastern.edu"]
        );
        assert_eq!(
            addresses_in("a@neu.edu,b@neu.edu;c@neu.edu <d@neu.edu>"),
            ["a@neu.edu", "b@neu.edu", "c@neu.edu", "d@neu.edu"]
        );
    }

    #[test]
    fn prose_treats_one_trailing_period_as_punctuation() {
        let mut value = serde_json::json!({ "note": "Email x@northeastern.edu." });
        redact_json_emails(&mut value);
        assert_eq!(value["note"], "Email x@northeastern.edu.");
        assert_eq!(
            prose_addresses_in("Email x@northeastern.edu."),
            ["x@northeastern.edu"]
        );
        assert!(is_northeastern_address("x@northeastern.edu"));
        // Email fields stay strict.
        assert!(retain_northeastern_addresses("x@northeastern.edu.").is_none());
        assert!(!is_northeastern_address("x@northeastern.edu."));
        for bad in ["x@northeastern.edu..", "x@northeastern.edu.evil"] {
            let mut value = serde_json::json!({ "note": format!("see {bad} now") });
            redact_json_emails(&mut value);
            let note = value["note"].as_str().unwrap();
            assert!(!note.contains('@'), "{bad}: {note}");
            assert!(retain_northeastern_addresses(bad).is_none(), "{bad}");
            for address in prose_addresses_in(bad) {
                assert!(!is_northeastern_address(address), "{bad}");
            }
        }
    }

    #[test]
    fn punctuation_local_parts_are_kept_whole() {
        for local in [
            "o'brien",
            "a+b",
            "a!b",
            "a#b$c",
            "a%b",
            "a&b",
            "a*b",
            "a/b",
            "a=b",
            "a?b",
            "a^b",
            "a`b",
            "a{b}",
            "a|b",
            "a~b",
            "a_b",
            "a-b",
            "a.b",
            "J\u{f6}rg",
        ] {
            let address = format!("{local}@Northeastern.edu");
            assert!(is_northeastern_address(&address), "{address}");
            assert_eq!(addresses_in(&address), [address.as_str()]);
            assert_eq!(
                retain_northeastern_addresses(&format!("{address} z@gmail.com")).as_deref(),
                Some(address.as_str())
            );
            let mut value = serde_json::json!({ "bio": format!("Write {address} today.") });
            redact_json_emails(&mut value);
            assert_eq!(value["bio"], format!("Write {address} today."));
        }
        assert!(retain_northeastern_addresses("a!b@gmail.com").is_none());
    }

    #[test]
    fn archive_scrub_drops_disallowed_lookalike_domains() {
        let mut value = serde_json::json!({
            "faculty": [{"email": "a@northeastern.edu. b@neu.eduX c@neu.edu"}],
            "text": "x@neu.edu.. y@northeastern.edu\u{e9} ok@neu.edu"
        });
        redact_json_emails(&mut value);
        assert_eq!(value["faculty"][0]["email"], "c@neu.edu");
        assert_eq!(value["text"], ". ok@neu.edu");
    }

    // Raw-file oracle. It shares no code with the runtime validator or
    // extractor: its own domain check, tokenizer, and normalization.

    fn oracle_allowed(candidate: &str) -> bool {
        let Some((local, domain)) = candidate.rsplit_once('@') else {
            return false;
        };
        if local.is_empty() || local.contains(char::is_whitespace) || local.contains('@') {
            return false;
        }
        if !domain.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return false;
        }
        let domain = domain.to_ascii_lowercase();
        domain == "northeastern.edu" || domain == "neu.edu"
    }

    fn unescape_html(text: &str) -> String {
        static PATTERN: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new(r"(?i)&(?:#(\d+)|#x([0-9a-f]+)|([a-z]+));").unwrap()
        });
        PATTERN
            .replace_all(text, |caps: &regex::Captures| {
                let whole = caps[0].to_string();
                if let Some(code) = caps
                    .get(1)
                    .and_then(|m| m.as_str().parse::<u32>().ok())
                    .or_else(|| {
                        caps.get(2)
                            .and_then(|m| u32::from_str_radix(m.as_str(), 16).ok())
                    })
                {
                    return char::from_u32(code).map_or(whole, String::from);
                }
                match caps.get(3).map(|m| m.as_str().to_ascii_lowercase()) {
                    Some(name) => match name.as_str() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        "commat" => "@".into(),
                        "period" => ".".into(),
                        "nbsp" => " ".into(),
                        _ => whole,
                    },
                    None => whole,
                }
            })
            .into_owned()
    }

    fn percent_decode(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut raw = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|pair| std::str::from_utf8(pair).ok())
                .and_then(|pair| u8::from_str_radix(pair, 16).ok());
            match (bytes[i], hex) {
                (b'%', Some(byte)) => {
                    raw.push(byte);
                    i += 3;
                }
                (byte, _) => {
                    raw.push(byte);
                    i += 1;
                }
            }
        }
        String::from_utf8(raw).unwrap_or_else(|_| text.to_string())
    }

    /// HTML-unescape and percent-decode until the text stops changing.
    /// NFKC is not applied: domain checks must see the original characters.
    fn decode_layers(text: &str) -> String {
        let mut current = text.to_string();
        for _ in 0..4 {
            let next = percent_decode(&unescape_html(&current));
            if next == current {
                break;
            }
            current = next;
        }
        current
    }

    /// Decoded and NFKC-folded text for obfuscation detection only.
    /// Never use this for a domain allow check. Folding can turn a confusable
    /// into `northeastern.edu` or `neu.edu`.
    fn canonical_text(text: &str) -> String {
        let nfkc = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc();
        let mut current = text.to_string();
        for _ in 0..4 {
            let decoded = percent_decode(&unescape_html(&current));
            let next = nfkc.normalize(&decoded).into_owned();
            if next == current {
                break;
            }
            current = next;
        }
        current
    }

    fn oracle_candidates(text: &str) -> Vec<String> {
        static PATTERN: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new(r#"[^\s@<>,;"'()\[\]]+@[^\s@<>,;"'()\[\]]+"#).unwrap()
        });
        PATTERN
            .find_iter(text)
            .map(|m| m.as_str().to_string())
            .collect()
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum TokenClass {
        Atom,
        At,
        Delim,
    }

    /// Delimiters of the raw candidate regex: Unicode whitespace and the
    /// ASCII punctuation that regex class excludes. `@` is a separator, not
    /// a delimiter, so a second `@` ends the domain.
    fn is_oracle_delim(ch: char) -> bool {
        ch.is_whitespace()
            || matches!(
                ch,
                '<' | '>' | ',' | ';' | '"' | '\'' | '(' | ')' | '[' | ']'
            )
    }

    /// Classify one original character. NFKC decides whether a confusable
    /// acts as `@` or as a delimiter, but the caller keeps the original slice.
    fn token_class(ch: char, nfkc: &icu_normalizer::ComposingNormalizerBorrowed<'_>) -> TokenClass {
        if ch == '@' {
            return TokenClass::At;
        }
        if is_oracle_delim(ch) {
            return TokenClass::Delim;
        }
        if ch.is_ascii() {
            return TokenClass::Atom;
        }
        let mut buf = [0u8; 4];
        let folded = nfkc.normalize(ch.encode_utf8(&mut buf));
        if folded.is_empty() {
            return TokenClass::Atom;
        }
        let at_like = folded.chars().any(|c| c == '@')
            && folded.chars().all(|c| c == '@' || is_oracle_delim(c));
        if at_like {
            return TokenClass::At;
        }
        if folded.chars().all(is_oracle_delim) {
            return TokenClass::Delim;
        }
        TokenClass::Atom
    }

    /// Email-shaped slices of `text`. Confusable at-signs and delimiters are
    /// recognized by their NFKC fold, and each slice is still the original
    /// pre-NFKC substring.
    fn fold_aware_candidates(text: &str) -> Vec<String> {
        if text.is_ascii() && !text.contains('@') {
            return Vec::new();
        }
        let nfkc = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc();
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let classes: Vec<TokenClass> = chars
            .iter()
            .map(|(_, ch)| token_class(*ch, &nfkc))
            .collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if classes[i] != TokenClass::At {
                i += 1;
                continue;
            }
            let at = i;
            if at == 0 || classes[at - 1] != TokenClass::Atom {
                i += 1;
                continue;
            }
            let mut start = at - 1;
            while start > 0 && classes[start - 1] == TokenClass::Atom {
                start -= 1;
            }
            let mut end = at + 1;
            while end < chars.len() && classes[end] == TokenClass::Atom {
                end += 1;
            }
            if end == at + 1 {
                i += 1;
                continue;
            }
            let byte_start = chars[start].0;
            let byte_end = if end < chars.len() {
                chars[end].0
            } else {
                text.len()
            };
            out.push(text[byte_start..byte_end].to_string());
            i = end;
        }
        out
    }

    fn looks_obfuscated(text: &str) -> bool {
        static BRACKETED: std::sync::LazyLock<Regex> =
            std::sync::LazyLock::new(|| Regex::new(r"(?i)[\[({]\s*(?:at|dot)\s*[\])}]").unwrap());
        static SPACED: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new(r"(?i)\bat\s+[\w-]+\s+dot\s+[a-z]{2,}").unwrap()
        });
        static AT_HOST: std::sync::LazyLock<Regex> =
            std::sync::LazyLock::new(|| Regex::new(r"(?i)\w\s+at\s+(\S*)").unwrap());
        static HOST: std::sync::LazyLock<Regex> =
            std::sync::LazyLock::new(|| Regex::new(r"(?i)^[\w-]+\.[a-z]{2,}\b").unwrap());
        static DOT_TLD: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new(r"(?i)\w\s+dot\s+(?:edu|com|org|net|gov|io)\b").unwrap()
        });
        BRACKETED.is_match(text)
            || SPACED.is_match(text)
            || DOT_TLD.is_match(text)
            || AT_HOST
                .captures_iter(text)
                .any(|c| !c[1].contains('@') && HOST.is_match(&c[1]))
    }

    /// Problems found in an unsanitized catalog value.
    fn raw_violations(value: &Value, key: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (name, child) in map {
                    raw_violations(child, name, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    raw_violations(item, key, out);
                }
            }
            Value::String(text) if key == "email" => {
                // Strict: no leading or trailing whitespace, exactly one space
                // between addresses.
                for part in text.split(' ') {
                    if !oracle_allowed(part) {
                        out.push(format!("email field: {part:?} in {text:?}"));
                    }
                }
            }
            Value::String(text) => {
                // Find candidates on the decoded pre-NFKC text. The fold-aware
                // scan uses NFKC only to recognize confusable separators. The
                // allow check reads the original domain characters, so a long
                // s or a fullwidth stop cannot become an allowed domain.
                let decoded = decode_layers(text);
                for part in fold_aware_candidates(&decoded) {
                    let bare = part.strip_prefix("mailto:").unwrap_or(&part);
                    let bare = bare.strip_suffix('.').unwrap_or(bare);
                    if !oracle_allowed(bare) {
                        out.push(format!("{key}: {part}"));
                    }
                }
                // NFKC is for obfuscation detection only, never for a domain allow.
                let folded = canonical_text(text);
                if looks_obfuscated(&folded) {
                    out.push(format!("{key}: obfuscated {folded:?}"));
                }
            }
            _ => {}
        }
    }

    fn violations(value: &Value) -> Vec<String> {
        let mut out = Vec::new();
        raw_violations(value, "", &mut out);
        out
    }

    fn record(email: &str, description: &str) -> Value {
        serde_json::json!({"sections": [{"instructors": [{"name": "X", "email": email}], "description": description}]})
    }

    #[test]
    fn raw_check_accepts_allowed_text() {
        assert!(violations(&record(
            "A.b+c!d@Northeastern.EDU neu@neu.edu",
            "Write info@northeastern.edu."
        ))
        .is_empty());
        assert!(violations(&record(
            "a@neu.edu",
            "Meets at the library. Ask at the desk."
        ))
        .is_empty());
        assert!(violations(&record(
            "a@neu.edu",
            "jo&#64;neu.edu and jo%40northeastern.edu."
        ))
        .is_empty());
    }

    #[test]
    fn raw_check_flags_mutations() {
        for bad in [
            "a@husky.neu.edu",
            "a@gmail.com",
            "a@northeastern.edu..",
            "a@neu.eduX",
            "a@neu.edu\u{e9}",
            "a@neu.edu\u{200b}",
            "a [at] gmail [dot] com",
            "a(at)gmail(dot)com",
            "a{AT}yahoo{DOT}com",
            "a[at]husky[dot]neu[dot]edu",
        ] {
            assert!(
                !violations(&record(bad, "")).is_empty(),
                "email field: {bad}"
            );
            assert!(
                !violations(&record("a@neu.edu", &format!("Contact {bad} soon"))).is_empty(),
                "prose: {bad}"
            );
        }
        assert!(!violations(&record("plain text", "")).is_empty());
        assert!(!violations(&record("a@northeastern.edu.", "")).is_empty());
    }

    #[test]
    fn raw_check_flags_obfuscated_and_encoded_prose() {
        for bad in [
            "mail jo at example dot org",
            "mail jo at example.org",
            "jo&#64;example.org",
            "jo&#x40;example.org",
            "jo%40example.org",
            "jo\u{ff20}example.org",
            "jo@example&period;org",
            "mail person@northeastern.edu.evil.com today",
            "mail x@neu.edu..",
            "mail x@neu.edu!",
            // Long s (U+017F) and fullwidth full stop (U+FF0E) fold to an
            // allowed domain under NFKC. The original domain is not that domain.
            "jo@northea\u{017f}tern.edu",
            "jo@neu\u{ff0e}edu",
        ] {
            assert!(!violations(&record("a@neu.edu", bad)).is_empty(), "{bad}");
        }
        assert!(violations(&record("a@neu.edu", "mail x@neu.edu.")).is_empty());
    }

    #[test]
    fn raw_check_accepts_plain_northeastern_and_neu_addresses() {
        assert!(violations(&record(
            "jo@northeastern.edu jo@NEU.EDU",
            "Email jo@northeastern.edu or jo@NEU.EDU today."
        ))
        .is_empty());
    }

    #[test]
    fn raw_check_rejects_whitespace_around_email_values() {
        for bad in [
            "x@neu.edu\n",
            "x@neu.edu ",
            " x@neu.edu",
            "\nx@neu.edu",
            "x@neu.edu\t",
            "x@neu.edu  y@neu.edu",
            "x@neu.edu\ny@neu.edu",
        ] {
            assert!(!violations(&record(bad, "")).is_empty(), "{bad:?}");
        }
        assert!(violations(&record("x@neu.edu y@neu.edu", "")).is_empty());
    }

    #[test]
    fn bundled_catalog_addresses_are_northeastern() {
        // Enumerate the checkout copy directly. The production lookup honors
        // TERMORGANIZE_DATA and installed catalogs, which could hide a regression.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/catalog");
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("{}: {err}", dir.display()))
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(archive::term_code_from_file_name)
                    .is_some()
            })
            .collect();
        paths.sort();
        let codes: Vec<String> = paths
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(archive::term_code_from_file_name)
            })
            .collect();
        assert!(codes.iter().any(|code| code == "202710"));
        assert!(codes.iter().any(|code| code == "202730"));
        for path in paths {
            let raw = std::fs::read(&path).expect("bundled catalog");
            let json = archive::json_bytes(&raw).expect("catalog json");
            let value: Value = serde_json::from_slice(&json).expect("catalog json");
            let code = value["term"]["code"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let courses = value["courses"].as_array().map_or(0, |rows| rows.len());
            let sections = value["sections"].as_array().map_or(0, |rows| rows.len());
            assert!(courses > 2_500, "{code} courses {courses}");
            assert!(sections > 7_000, "{code} sections {sections}");
            let literal = {
                let text = std::str::from_utf8(&json).expect("catalog text");
                oracle_candidates(text).len()
            };
            assert!(literal > 500, "{code}: expected institutional addresses");
            let found = violations(&value);
            assert!(
                found.is_empty(),
                "{code}: {} violations, first {:?}",
                found.len(),
                &found[..found.len().min(5)]
            );
        }
    }
}
