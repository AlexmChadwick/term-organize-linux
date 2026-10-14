//! Northeastern "View Plan" text and a PDF in the same layout.

use crate::model::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SEPARATOR: &str = "-------------------------------------------------------------------------------------------------------------------";
pub const TITLE_LENGTH: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    Unreadable,
    NotADegreePlan,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable => write!(f, "That file couldn't be read. Export the plan again as a PDF and try once more."),
            Self::NotADegreePlan => write!(f, "This doesn't look like a degree audit plan. In the degree audit planner, open your plan and choose View Plan, then import the PDF it creates."),
        }
    }
}

pub struct ParseResult {
    pub plan: DegreePlan,
    pub skipped_lines: Vec<String>,
}

pub fn parse(text: &str, file_name: Option<&str>) -> Result<ParseResult, ImportError> {
    let mut plan = DegreePlan::default();
    plan.name.clear();
    plan.imported_from = file_name.map(|s| s.to_string());
    let mut skipped = vec![];
    let mut saw = false;
    #[derive(Clone, Copy)]
    enum Section {
        Header,
        Completed,
        Term,
        Between,
    }
    let mut section = Section::Header;
    for raw in text.lines() {
        let line = normalize_line(raw);
        if line.is_empty() || line == "-" || line == "•" {
            continue;
        }
        if let Some(value) = header_value(&line, "Printed:") {
            plan.printed_at = parse_printed_date(&value).map(|t| t.to_string());
            continue;
        }
        if let Some(value) =
            header_value(&line, "StudentID:").or_else(|| header_value(&line, "Student ID:"))
        {
            plan.student_id = blank_none(&value);
            continue;
        }
        if let Some(value) = header_value(&line, "Student:") {
            plan.student_name = blank_none(&value);
            continue;
        }
        if let Some(value) = header_value(&line, "Degree Audit Plan Name:")
            .or_else(|| header_value(&line, "Plan Name:"))
        {
            plan.name = value;
            saw = true;
            continue;
        }
        if let Some(value) = header_value(&line, "Program:") {
            plan.program = blank_none(&value);
            continue;
        }
        if line.to_ascii_lowercase().starts_with("completed courses") {
            section = Section::Completed;
            saw = true;
            continue;
        }
        if is_separator(&line) {
            section = Section::Between;
            continue;
        }
        if let Some(term) = parse_term_header(&line) {
            plan.terms.push(term);
            section = Section::Term;
            saw = true;
            continue;
        }
        if let Some(course) = parse_course_line(&line) {
            match section {
                Section::Term | Section::Between if !plan.terms.is_empty() => {
                    let n = plan.terms.len() - 1;
                    plan.terms[n].courses.push(course);
                }
                _ => plan.completed.push(course),
            }
            continue;
        }
        skipped.push(line);
    }
    if !saw || (plan.completed.is_empty() && plan.terms.is_empty() && plan.name.is_empty()) {
        return Err(ImportError::NotADegreePlan);
    }
    if plan.name.is_empty() {
        plan.name = "Imported Plan".into();
    }
    plan.sort_terms();
    Ok(ParseResult {
        plan,
        skipped_lines: skipped,
    })
}

pub fn parse_term_header(line: &str) -> Option<DegreePlanTerm> {
    let re =
        regex::Regex::new(r"(?i)^(.+?)\s+(\d{4})\s*\(\s*-?\d+(?:\.\d+)?\s*Hours?\s*\)$").unwrap();
    let cap = re.captures(line)?;
    let season = PlanSeason::parse_export(cap[1].trim())?;
    let year: i32 = cap[2].parse().ok()?;
    Some(DegreePlanTerm::new(season, year))
}

pub fn parse_course_line(line: &str) -> Option<DegreePlanCourse> {
    let re = regex::Regex::new(r"(?i)^(.+?)\s*\(\s*(\d+(?:\.\d+)?)\s*Hours?\s*\)\s*(.*)$").unwrap();
    let cap = re.captures(line)?;
    let credits: f64 = cap[2].parse().ok()?;
    let code = cap[1].trim();
    let title = cap[3].trim().to_string();
    if let Some(key) = parse_course_code(code) {
        let mut course = DegreePlanCourse::catalog(key, title, credits);
        course.label = key_label(code);
        return Some(course);
    }
    if code.len() <= 40 && !code.contains(':') {
        return Some(DegreePlanCourse::other(code, title, credits));
    }
    None
}

fn key_label(code: &str) -> String {
    parse_course_code(code)
        .map(|k| k.to_string())
        .unwrap_or_else(|| code.to_string())
}

pub fn parse_course_code(code: &str) -> Option<CourseKey> {
    let re = regex::Regex::new(r"^([A-Z]{2,5})\s*(\d{4}[A-Z]?)$").unwrap();
    let upper = code.to_ascii_uppercase();
    let cap = re.captures(&upper)?;
    Some(CourseKey {
        subject: cap[1].to_string(),
        number: cap[2].to_string(),
    })
}

pub fn normalize_line(line: &str) -> String {
    let mut text = String::new();
    let mut last_space = false;
    for ch in line.chars() {
        if ch.is_control() || ch.is_whitespace() {
            if !last_space {
                text.push(' ');
                last_space = true;
            }
        } else if unicode_format(ch) {
            continue;
        } else {
            text.push(ch);
            last_space = false;
        }
    }
    let text = text.trim().to_string();
    let bullets = ['-', '•', '–', '—'];
    if text.chars().next().is_some_and(|c| bullets.contains(&c)) && !is_separator(&text) {
        return text.trim_start_matches(bullets).trim().to_string();
    }
    text
}

fn unicode_format(ch: char) -> bool {
    matches!(ch, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}')
}

fn is_separator(line: &str) -> bool {
    line.chars().count() >= 10 && line.chars().all(|c| c == '-' || c == '_' || c == '=')
}

fn header_value(line: &str, label: &str) -> Option<String> {
    if line
        .to_ascii_lowercase()
        .starts_with(&label.to_ascii_lowercase())
    {
        Some(line[label.len()..].trim().to_string())
    } else {
        None
    }
}

fn blank_none(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub fn hours_label(credits: f64) -> String {
    let credits = if credits == 0.0 { 0.0 } else { credits };
    let word = if (credits - 1.0).abs() < 1e-9 {
        "Hour"
    } else {
        "Hours"
    };
    format!("({credits:.1} {word})")
}

pub fn export_code(course: &DegreePlanCourse) -> String {
    let Some(key) = &course.key else {
        return course.label.clone();
    };
    let subject = if key.subject.len() < 4 {
        format!("{:<4}", key.subject)
    } else {
        key.subject.clone()
    };
    format!("{subject}{}", key.number)
}

pub fn export_title(title: &str) -> String {
    title.chars().take(TITLE_LENGTH).collect()
}

pub fn course_line(course: &DegreePlanCourse) -> String {
    format!(
        "{} {} {}",
        export_code(course),
        hours_label(course.credits),
        export_title(&course.title)
    )
}

pub fn term_header(term: &DegreePlanTerm) -> String {
    let gap = if term.season.export_name().contains(' ') {
        " "
    } else {
        "  "
    };
    format!(
        "{}{gap}{} {} ",
        term.season.export_name(),
        term.year,
        hours_label(term.credits())
    )
}

pub fn printed_line(unix: i64) -> String {
    format!("Printed:  {}", format_ny(unix))
}

pub fn header_lines(plan: &DegreePlan, unix: i64, include_student: bool) -> Vec<String> {
    let mut lines = vec![printed_line(unix)];
    if include_student {
        lines.push(format!(
            "Student:  {}",
            plan.student_name.as_deref().unwrap_or("")
        ));
        lines.push(format!(
            "StudentID:  {}",
            plan.student_id.as_deref().unwrap_or("")
        ));
    }
    lines.push(format!("Degree Audit Plan Name:\t{}", plan.name));
    lines.push(format!(
        "Program:\t\t{}",
        plan.program.as_deref().unwrap_or("")
    ));
    lines
}

pub fn body_lines(plan: &DegreePlan) -> Vec<(i32, String)> {
    let mut lines = vec![(1, "Completed Courses:  ".into())];
    for course in &plan.completed {
        lines.push((2, course_line(course)));
    }
    for term in &plan.terms {
        lines.push((1, SEPARATOR.into()));
        lines.push((1, term_header(term)));
        for course in &term.courses {
            lines.push((2, course_line(course)));
        }
    }
    lines
}

pub fn text(plan: &DegreePlan, unix: i64, include_student: bool) -> String {
    let mut lines = header_lines(plan, unix, include_student);
    for (indent, line) in body_lines(plan) {
        lines.push(format!("{}{line}", "  ".repeat(indent as usize)));
    }
    lines.join("\n")
}

/// Seconds since 1970 parsed from `MM/dd/yyyy HH:mm` in America/New_York.
pub fn parse_printed_date(value: &str) -> Option<i64> {
    let re = regex::Regex::new(
        r"(\d{1,2})/(\d{1,2})/(\d{4})(?:\s+(\d{1,2}):(\d{2})(?:\s*([AaPp][Mm]))?)?",
    )
    .unwrap();
    let c = re.captures(value)?;
    let month: i32 = c[1].parse().ok()?;
    let day: i32 = c[2].parse().ok()?;
    let year: i32 = c[3].parse().ok()?;
    let mut hour: i32 = c.get(4).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
    let minute: i32 = c.get(5).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
    if let Some(ampm) = c.get(6) {
        let pm = ampm.as_str().eq_ignore_ascii_case("pm");
        if pm && hour < 12 {
            hour += 12;
        }
        if !pm && hour == 12 {
            hour = 0;
        }
    }
    Some(ny_to_unix(year, month, day, hour, minute))
}

pub fn format_ny(unix: i64) -> String {
    let (y, m, d, h, min) = unix_to_ny(unix);
    format!("{m:02}/{d:02}/{y:04} {h:02}:{min:02}")
}

fn ny_to_unix(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> i64 {
    let utc = civil_to_unix(year, month, day, hour, minute);
    let offset = if is_dst(year, month, day, hour) {
        4 * 3600
    } else {
        5 * 3600
    };
    utc + offset
}

fn unix_to_ny(unix: i64) -> (i32, i32, i32, i32, i32) {
    // Approximate: try EDT then EST.
    for offset in [4 * 3600, 5 * 3600] {
        let (y, m, d, h, min) = unix_to_civil(unix - offset);
        let dst = is_dst(y, m, d, h);
        if (dst && offset == 4 * 3600) || (!dst && offset == 5 * 3600) {
            return (y, m, d, h, min);
        }
    }
    unix_to_civil(unix - 5 * 3600)
}

fn is_dst(year: i32, month: i32, day: i32, hour: i32) -> bool {
    if !(3..=11).contains(&month) {
        return false;
    }
    if (4..=10).contains(&month) {
        return true;
    }
    let march = nth_sunday(year, 3, 2);
    let november = nth_sunday(year, 11, 1);
    if month == 3 {
        day > march || (day == march && hour >= 2)
    } else {
        day < november || (day == november && hour < 2)
    }
}

fn nth_sunday(year: i32, month: i32, n: i32) -> i32 {
    let mut count = 0;
    for day in 1..=31 {
        if weekday(year, month, day) == 0 {
            count += 1;
            if count == n {
                return day;
            }
        }
    }
    1
}

fn weekday(year: i32, month: i32, day: i32) -> i32 {
    let unix = civil_to_unix(year, month, day, 12, 0);
    ((unix / 86400 + 4).rem_euclid(7)) as i32
}

fn civil_to_unix(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> i64 {
    let mut y = year as i64;
    let mut m = month as i64;
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    days * 86400 + hour as i64 * 3600 + minute as i64 * 60
}

fn unix_to_civil(unix: i64) -> (i32, i32, i32, i32, i32) {
    let days = unix.div_euclid(86400);
    let rem = unix.rem_euclid(86400);
    let hour = (rem / 3600) as i32;
    let minute = ((rem % 3600) / 60) as i32;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as i32, d as i32, hour, minute)
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn place_line(
    pages: &mut Vec<Vec<String>>,
    baseline: &mut f64,
    x: f64,
    dy: f64,
    text: &str,
    font: &str,
    advance: bool,
) {
    if *baseline < 54.0 {
        pages.push(vec![]);
        *baseline = 788.0;
    }
    if !text.is_empty() {
        pages
            .last_mut()
            .unwrap()
            .push(text_op(x, *baseline + dy, text, font));
    }
    if advance {
        *baseline -= 18.0;
    }
}

/// A simple PDF (Helvetica) matching the audit's line layout.
pub fn pdf(plan: &DegreePlan, unix: i64, include_student: bool) -> Vec<u8> {
    let mut pages: Vec<Vec<String>> = vec![vec![]];
    let mut baseline = 788.0_f64;
    for line in header_lines(plan, unix, include_student) {
        let drawn = line.replace('\t', " ");
        let font = if drawn.starts_with("Degree Audit") || drawn.starts_with("Program:") {
            "F2"
        } else {
            "F1"
        };
        place_line(&mut pages, &mut baseline, 36.0, 0.0, &drawn, font, true);
    }
    place_line(&mut pages, &mut baseline, 36.0, 0.0, "", "F1", true);
    place_line(&mut pages, &mut baseline, 36.0, 0.0, "", "F1", true);
    for (indent, line) in body_lines(plan) {
        let x = match indent {
            0 => 36.0,
            1 => 96.0,
            _ => 126.0,
        };
        if line.chars().all(|c| c == '-') && line.len() >= 10 {
            place_line(
                &mut pages,
                &mut baseline,
                x,
                0.0,
                &SEPARATOR[..80.min(SEPARATOR.len())],
                "F1",
                true,
            );
        } else {
            if indent >= 2 {
                place_line(&mut pages, &mut baseline, x - 30.0, 0.0, "-", "F1", false);
            }
            place_line(&mut pages, &mut baseline, x, 0.0, &line, "F1", true);
        }
    }
    // Don't leave a trailing blank page from the last decrement.
    render_pdf(&pages)
}

fn text_op(x: f64, y: f64, text: &str, font: &str) -> String {
    format!(
        "BT /{font} 12 Tf {x:.1} {y:.1} Td ({}) Tj ET",
        pdf_escape(text)
    )
}

fn pdf_escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '(' | ')' | '\\' => format!("\\{c}"),
            c if c.is_ascii() => c.to_string(),
            c => format!("\\{:03o}", c as u32),
        })
        .collect()
}

fn render_pdf(pages: &[Vec<String>]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let font1 = objects.len();
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    let font2 = objects.len();
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-BoldOblique >>".to_vec());
    let mut page_ids = Vec::new();
    let mut content_ids = Vec::new();
    for page in pages {
        let stream = page
            .iter()
            .filter(|l| !l.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let content = format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        );
        content_ids.push(objects.len());
        objects.push(content.into_bytes());
        page_ids.push(objects.len());
        let page_obj = format!(
            "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 595 842] /Contents {} 0 R /Resources << /Font << /F1 {} 0 R /F2 {} 0 R >> >> >>",
            0, 0, font1 + 1, font2 + 1
        );
        objects.push(page_obj.into_bytes());
    }
    let pages_index = objects.len();
    let kids = page_ids
        .iter()
        .map(|id| format!("{} 0 R", id + 1))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(
        format!(
            "<< /Type /Pages /Count {} /Kids [{kids}] >>",
            page_ids.len()
        )
        .into_bytes(),
    );
    // Fix parent references now that pages object id is known.
    for id in &page_ids {
        let text = String::from_utf8_lossy(&objects[*id]).replace(
            " /Parent 0 0 R",
            &format!(" /Parent {} 0 R", pages_index + 1),
        );
        let text = text.replace(" /Contents 0 0 R", &format!(" /Contents {} 0 R", id)); // wrong
        objects[*id] = text.into_bytes();
    }
    // Rewrite pages cleanly.
    for (i, id) in page_ids.iter().enumerate() {
        let content = content_ids[i] + 1;
        let page_obj = format!(
            "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 595 842] /Contents {content} 0 R /Resources << /Font << /F1 {} 0 R /F2 {} 0 R >> >> >>",
            pages_index + 1, font1 + 1, font2 + 1
        );
        objects[*id] = page_obj.into_bytes();
    }
    let catalog = objects.len();
    objects.push(format!("<< /Type /Catalog /Pages {} 0 R >>", pages_index + 1).into_bytes());
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
        out.extend(obj);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n", objects.len() + 1).into_bytes());
    out.extend(b"0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        out.extend(format!("{off:010} 00000 n \n").into_bytes());
    }
    out.extend(
        format!(
            "trailer << /Size {} /Root {} 0 R >>\nstartxref\n{xref}\n%%EOF",
            objects.len() + 1,
            catalog + 1
        )
        .into_bytes(),
    );
    out
}

/// Extract text operators from our PDF, one logical line each, for round-trip tests.
pub fn pdf_text(data: &[u8]) -> String {
    let raw = String::from_utf8_lossy(data);
    let re = regex::Regex::new(r"\((?:\\.|[^\\)])*\)").unwrap();
    let mut lines = Vec::new();
    for m in re.find_iter(&raw) {
        let inner = &m.as_str()[1..m.as_str().len() - 1];
        let text = unescape(inner);
        if text == "-" {
            continue;
        }
        lines.push(text);
    }
    lines.join("\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}
