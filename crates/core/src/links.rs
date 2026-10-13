//! termorganize:// and neuclasssearch:// links.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppLink {
    Course { term: String, subject: String, number: String },
    Section { term: String, crn: String },
    Schedule { id: Option<String> },
    Watchlist,
    Plan { id: String },
    Search { query: String },
    DegreePlans,
    File { path: String },
}

pub fn parse(url: &str) -> Option<AppLink> {
    let rest = url
        .strip_prefix("termorganize://")
        .or_else(|| url.strip_prefix("neuclasssearch://"))?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<&str> = path.trim_matches('/').split('/').filter(|p| !p.is_empty()).collect();
    match parts.first().copied() {
        Some("course") if parts.len() >= 4 => Some(AppLink::Course { term: parts[1].into(), subject: parts[2].into(), number: parts[3].into() }),
        Some("section") if parts.len() >= 3 => Some(AppLink::Section { term: parts[1].into(), crn: parts[2].into() }),
        Some("schedule") => Some(AppLink::Schedule { id: parts.get(1).map(|s| (*s).to_string()) }),
        Some("watchlist") => Some(AppLink::Watchlist),
        Some("plan") if parts.len() >= 2 => Some(AppLink::Plan { id: parts[1].into() }),
        Some("search") => {
            let q = query.split('&').find_map(|p| p.strip_prefix("q=")).unwrap_or("").replace('+', " ");
            Some(AppLink::Search { query: percent_decode(&q) })
        }
        Some("plans") | Some("degree") => Some(AppLink::DegreePlans),
        Some("file") => Some(AppLink::File { path: parts[1..].join("/") }),
        _ => None,
    }
}

pub fn section_url(term: &str, crn: &str) -> String {
    format!("termorganize://section/{term}/{crn}")
}

pub fn course_url(term: &str, subject: &str, number: &str) -> String {
    format!("termorganize://course/{term}/{subject}/{number}")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
