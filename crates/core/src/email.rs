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

    /// Problems found in an unsanitized catalog value. Email fields must hold
    /// only complete allowed tokens. Every string is scanned for literal and
    /// `[at]`/`[dot]` style addresses.
    fn raw_violations(value: &Value, path: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let here = format!("{path}.{key}");
                    if is_email_key(key) {
                        if let Value::String(text) = child {
                            for token in
                                text.split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                            {
                                if !token.is_empty() && !is_northeastern_address(token) {
                                    out.push(format!("{here}: email token {token:?}"));
                                }
                            }
                        }
                    }
                    raw_violations(child, &here, out);
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    raw_violations(item, &format!("{path}[{i}]"), out);
                }
            }
            Value::String(text) => {
                for address in prose_addresses_in(text) {
                    if !is_northeastern_address(address) {
                        out.push(format!("{path}: address {address:?}"));
                    }
                }
                let plain = deobfuscate(text);
                for address in prose_addresses_in(&plain) {
                    if !is_northeastern_address(address) {
                        out.push(format!("{path}: obfuscated address {address:?}"));
                    }
                }
            }
            _ => {}
        }
    }

    fn deobfuscate(text: &str) -> String {
        let mut out = text.to_ascii_lowercase();
        for (tag, plain) in [("at", "@"), ("dot", ".")] {
            for (open, close) in [("[", "]"), ("(", ")"), ("{", "}")] {
                let tag = format!("{open}{tag}{close}");
                out = out.replace(&format!(" {tag} "), plain).replace(&tag, plain);
            }
        }
        out
    }

    fn violations(value: &Value) -> Vec<String> {
        let mut out = Vec::new();
        raw_violations(value, "$", &mut out);
        out
    }

    #[test]
    fn raw_check_flags_mutations() {
        assert!(
            violations(&serde_json::json!({"email": "a@neu.edu b@northeastern.edu"})).is_empty()
        );
        for bad in [
            "a@husky.neu.edu",
            "a@gmail.com",
            "a@northeastern.edu..",
            "a@neu.eduX",
            "a@neu.edu\u{e9}",
            "a [at] gmail [dot] com",
            "a(at)gmail(dot)com",
            "a{AT}yahoo{DOT}com",
            "a[at]husky[dot]neu[dot]edu",
        ] {
            let in_email = serde_json::json!({"sections": [{"email": bad}]});
            assert!(!violations(&in_email).is_empty(), "email field: {bad}");
            let in_prose = serde_json::json!({"description": format!("Contact {bad} soon")});
            assert!(!violations(&in_prose).is_empty(), "prose: {bad}");
        }
        assert!(!violations(&serde_json::json!({"email": "plain text"})).is_empty());
        assert!(!violations(&serde_json::json!({"email": "a@northeastern.edu."})).is_empty());
        assert!(violations(&serde_json::json!({"note": "Contact a@northeastern.edu."})).is_empty());
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
            let literal: usize = {
                let text = std::str::from_utf8(&json).expect("catalog text");
                addresses_in(text).len()
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
