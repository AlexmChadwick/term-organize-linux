//! Minimal HTML helpers for the fragments Banner and the academic catalog return.

use regex::Regex;
use std::sync::OnceLock;

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "ndash" => "–",
        "mdash" => "—",
        "rsquo" => "’",
        "lsquo" => "‘",
        "rdquo" => "”",
        "ldquo" => "“",
        "hellip" => "…",
        "eacute" => "é",
        "copy" => "©",
        "reg" => "®",
        "bull" => "•",
        "middot" => "·",
        _ => return None,
    })
}

pub fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '&' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let end = (i + 1..chars.len().min(i + 13)).find(|&j| chars[j] == ';');
        let Some(semi) = end else {
            out.push('&');
            i += 1;
            continue;
        };
        let entity: String = chars[i + 1..semi].iter().collect();
        let replacement = if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).map(|c| c.to_string())
        } else if let Some(dec) = entity.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32).map(|c| c.to_string())
        } else {
            named_entity(&entity.to_ascii_lowercase()).map(|s| s.to_string())
        };
        if let Some(rep) = replacement {
            out.push_str(&rep);
            i = semi + 1;
        } else {
            out.push('&');
            i += 1;
        }
    }
    out
}

pub fn plain_text(html: &str) -> String {
    let comment = regex_comment();
    let mut s = comment.replace_all(html, "").to_string();
    s = Regex::new(r"(?i)<br\s*/?>").unwrap().replace_all(&s, "\n").to_string();
    s = Regex::new(r"(?i)</p>").unwrap().replace_all(&s, "\n").to_string();
    s = Regex::new(r"<[^>]*>").unwrap().replace_all(&s, "").to_string();
    s = decode_entities(&decode_entities(&s));
    let space = Regex::new(r"[ \t\u{00A0}]+").unwrap();
    let mut result: Vec<String> = Vec::new();
    for line in s.lines() {
        let line = space.replace_all(line, " ").trim().to_string();
        if line.is_empty() && result.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        result.push(line);
    }
    while result.last().is_some_and(|l| l.is_empty()) {
        result.pop();
    }
    result.join("\n")
}

fn regex_comment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").unwrap())
}

/// Rows of the first `<tbody>` as plain-text cells.
pub fn table_rows(html: &str) -> Option<Vec<Vec<String>>> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<tbody")?;
    let after = start + "<tbody".len();
    let gt = html[after..].find('>')? + after + 1;
    let end_rel = lower[gt..].find("</tbody>")?;
    let body = &html[gt..gt + end_rel];
    let row_re = Regex::new(r"(?is)<tr[^>]*>([\s\S]*?)</tr>").unwrap();
    let cell_re = Regex::new(r"(?is)<td[^>]*>([\s\S]*?)</td>").unwrap();
    let rows = row_re
        .captures_iter(body)
        .map(|cap| {
            cell_re
                .captures_iter(&cap[1])
                .map(|c| plain_text(&c[1]))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Some(rows)
}

pub fn captures(pattern: &str, text: &str) -> Vec<String> {
    let Ok(re) = Regex::new(pattern) else { return vec![] };
    re.captures_iter(text)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}
