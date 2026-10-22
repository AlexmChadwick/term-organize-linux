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

pub fn is_northeastern_address(address: &str) -> bool {
    let address = address.trim().trim_end_matches('.');
    let Some((_, domain)) = address.rsplit_once('@') else {
        return false;
    };
    if address[..address.len() - domain.len() - 1].is_empty() {
        return false;
    }
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if domain.is_empty() || domain.contains('@') || domain.contains(' ') || !domain.contains('.') {
        return false;
    }
    domain == "northeastern.edu" || domain == "neu.edu"
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
    email_spans(text)
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
    let spans = email_spans(text);
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
    let (_, domain) = address.trim().trim_end_matches('.').rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if domain.is_empty() {
        None
    } else {
        Some(domain)
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

fn email_spans(text: &str) -> Vec<(usize, usize)> {
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
            while end < bytes.len() && is_domain_byte(bytes[end]) {
                end += 1;
            }
            while end > index + 1 && matches!(bytes[end - 1], b'.' | b'-') {
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

fn is_local_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'%' | b'+' | b'-' | b'\'')
}

fn is_domain_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')
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
        assert!(!is_northeastern_address("albanese.d@husky.neu.edu"));
        assert!(!is_northeastern_address("viola@ccs.neu.edu"));
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
    fn bundled_catalog_addresses_are_northeastern() {
        let paths = archive::bundled_catalog_paths();
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
            let mut value: Value = serde_json::from_slice(&json).expect("catalog json");
            let code = value["term"]["code"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let courses = value["courses"]
                .as_array()
                .map(|rows| rows.len())
                .unwrap_or(0);
            let sections = value["sections"]
                .as_array()
                .map(|rows| rows.len())
                .unwrap_or(0);
            assert!(courses > 2_500, "{code} courses {courses}");
            assert!(sections > 7_000, "{code} sections {sections}");
            let raw = serde_json::to_string(&value).expect("catalog text");
            let lower = raw.to_ascii_lowercase();
            if code == "202710" {
                assert!(
                    lower.contains("albanese.d@husky.neu.edu"),
                    "Fall 2026 should still store the student-domain address so the filter is what hides it"
                );
                assert!(
                    lower.contains("viola@ccs.neu.edu"),
                    "Fall 2026 should still store the ccs.neu.edu address so the filter is what hides it"
                );
            }
            let scan = redact_json_emails(&mut value);
            assert!(
                scan.kept > 500,
                "{code}: expected published northeastern addresses, kept {}",
                scan.kept
            );
            if code == "202710" {
                assert!(scan.kept > 1_000, "fall snapshot kept {}", scan.kept);
                assert!(
                    scan.removed_by_domain.contains_key("husky.neu.edu"),
                    "husky.neu.edu should be dropped: {:?}",
                    scan.removed_by_domain
                );
                assert!(
                    scan.removed_by_domain.contains_key("ccs.neu.edu"),
                    "ccs.neu.edu should be dropped: {:?}",
                    scan.removed_by_domain
                );
            }
            let redacted = serde_json::to_string(&value).expect("redacted catalog");
            let redacted_lower = redacted.to_ascii_lowercase();
            assert!(!redacted_lower.contains("albanese.d@husky.neu.edu"));
            assert!(!redacted_lower.contains("viola@ccs.neu.edu"));
            assert!(!redacted_lower.contains("@husky.neu.edu"));
            assert!(!redacted_lower.contains("@ccs.neu.edu"));
            assert!(!redacted_lower.contains("@mail.neu.edu"));
            for address in addresses_in(&redacted) {
                assert!(
                    is_northeastern_address(address),
                    "{code} kept a non-exact domain: {address}"
                );
            }
        }
    }
}
