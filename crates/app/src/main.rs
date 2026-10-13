//! Term Organize for Linux. Unofficial, and not affiliated with Northeastern University or Sandbox.

mod search_provider;
mod ui;

use std::collections::HashMap;
use std::path::PathBuf;
use termorganize_core::archive::{self, bundled_catalog_path};
use termorganize_core::index::CatalogIndex;
use termorganize_core::links::{self, AppLink};
use termorganize_core::model::*;
use termorganize_core::search::{self, SearchQuery};
use termorganize_core::store;
use termorganize_core::watchlist;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Term Organize — unofficial Northeastern term planner\n\n  termorganize [file-or-link]\n  termorganize --search QUERY\n  termorganize --search-provider\n  termorganize --seat-watch\n  termorganize --smoke\n  termorganize --version");
        return;
    }
    if args.iter().any(|a| a == "--version") {
        println!("Term Organize {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--seat-watch") {
        std::process::exit(seat_watch());
    }
    if args.iter().any(|a| a == "--search-provider") {
        std::process::exit(search_provider::serve());
    }
    if let Some(query) = args.iter().position(|a| a == "--search").and_then(|i| args.get(i + 1)) {
        print_search(query);
        return;
    }
    let smoke = args.iter().any(|a| a == "--smoke");
    let open = args.into_iter().find(|a| !a.starts_with('-'));
    ui::run(open, smoke);
}

pub fn load_index() -> Option<CatalogIndex> {
    let preferred = store::load().preferences.selected_term_code;
    if let Some(code) = preferred.as_deref() {
        if let Some(index) = crate::ui::index_for_code(code) {
            return Some(index);
        }
    }
    let path = bundled_catalog_path()?;
    let mut catalog = archive::read(&path).ok()?;
    catalog.recompute_postreqs();
    Some(CatalogIndex::build(catalog))
}

fn print_search(text: &str) {
    let Some(index) = load_index() else {
        eprintln!("No catalog snapshot found. Set TERMORGANIZE_DATA or run from the repo.");
        std::process::exit(1);
    };
    let response = search::search(&SearchQuery { text: text.into(), ..SearchQuery::default() }, &index);
    if let Some(section) = &response.crn_match {
        println!("crn\t{}\t{}", section.crn, section.course_key());
    }
    for hit in response.results.iter().take(8) {
        println!("{}\t{}\t{}", hit.course.code(), hit.course.title, hit.course.credits_label());
    }
}

fn seat_watch() -> i32 {
    let Some(index) = load_index() else {
        eprintln!("termorganize: no catalog for seat watch");
        return 1;
    };
    let mut data = store::load();
    let term = data.preferences.selected_term_code.clone().unwrap_or_else(|| index.catalog.term.code.clone());
    let watched: Vec<_> = data.watchlist.iter().filter(|w| w.term_code == term).cloned().collect();
    let mut live: HashMap<String, Vec<CourseSection>> = HashMap::new();
    live.insert(term.clone(), index.catalog.sections.clone());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let report = watchlist::compare(&watched, &live, now);
    for updated in &report.items {
        if let Some(slot) = data.watchlist.iter_mut().find(|w| w.id() == updated.id()) {
            *slot = updated.clone();
        }
    }
    let _ = store::save(&data);
    for alert in &report.alerts {
        let _ = std::process::Command::new("notify-send")
            .args(["--app-name=Term Organize", &alert.title(), &alert.body()])
            .status();
        println!("alert\t{}\t{}", alert.crn, alert.body());
    }
    if report.alerts.is_empty() {
        println!("ok\t{} watched", watched.len());
    }
    0
}

pub fn open_target(raw: &str) -> Option<AppLink> {
    if raw.contains("://") {
        return links::parse(raw);
    }
    let path = PathBuf::from(raw);
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        return Some(AppLink::File { path: path.display().to_string() });
    }
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm")) {
        return Some(AppLink::File { path: path.display().to_string() });
    }
    None
}
