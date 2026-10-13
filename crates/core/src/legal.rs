//! Appropriate Legal Notices (AGPL-3.0 §§0, 5d). Linux wording replaces the Apple-only privacy path.

pub const APP_NAME: &str = "Term Organize";
pub const APP_COPYRIGHT: &str = "Copyright (C) 2026 Term Organize contributors.";
pub const UPSTREAM_COPYRIGHT: &str = "Copyright (C) Sandbox at Northeastern University and the SearchNEU contributors";
pub const LICENSE_NAME: &str = "GNU Affero General Public License v3.0";
pub const LICENSE_GRANT: &str = "Term Organize is free software: you can redistribute it and modify it under the terms of the GNU Affero General Public License, version 3, as published by the Free Software Foundation.";
pub const NO_WARRANTY: &str = "This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more details.";
pub const LICENSE_NOTICE: &str = "You should have received a copy of the GNU Affero General Public License along with this program. If not, see https://www.gnu.org/licenses/.";
pub const SOURCE_CODE_URL: &str = "https://github.com/AlexmChadwick/Northeastern-Term-Organize";
pub const UNOFFICIAL_NOTICE: &str = "Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.";
pub const PLATFORM: &str = "Linux";
pub const SEARCHNEU_SUMMARY: &str = "This app is an independent Linux adaptation of SearchNEU, the course search built by Sandbox at Northeastern University (sandboxnu). Portions of this app — including the Banner scraping logic, requisite parsing and schedule generation — are derived from SearchNEU's source code.";
pub const PRIVACY_NOTICE: &str = "No accounts, no analytics, no tracking. Your schedules, degree plans and watchlist are stored on this device. If folder sync is on, they are also written to a folder you choose so you can copy them between computers. Degree audit PDFs are read on the device, and the student name and ID they contain are left out of exports unless you include them, and they are not written to the sync folder. Schedule descriptions you type are parsed on this device. The app only contacts nubanner.neu.edu and catalog.northeastern.edu to fetch public course and degree-requirement information.";
pub const DATA_NOTICE: &str = "Course information is © Northeastern University and is retrieved from its public class search. Northeastern University and its marks belong to their owners and are used only to say where the data comes from.";
pub const SEARCHNEU_REPOSITORY: &str = "https://github.com/sandboxnu/searchneu";
pub const SEARCHNEU_WEBSITE: &str = "https://searchneu.com";
pub const LICENSE_URL: &str = "https://www.gnu.org/licenses/agpl-3.0.html";

pub fn source_offer() -> String {
    format!("Term Organize is free software licensed under the GNU Affero General Public License v3.0. Its complete corresponding source code is available at {SOURCE_CODE_URL}.")
}

pub fn reflow(raw: &str) -> String {
    let mut paragraphs = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    let flush = |lines: &mut Vec<&str>, paragraphs: &mut Vec<String>| {
        if lines.is_empty() { return; }
        let indent = lines[0].chars().take_while(|c| *c == ' ').count();
        let joined = lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        let pad = " ".repeat(indent);
        paragraphs.push(format!("{pad}{joined}"));
        lines.clear();
    };
    for line in raw.lines() {
        if line.trim().is_empty() {
            flush(&mut lines, &mut paragraphs);
        } else {
            lines.push(line);
        }
    }
    flush(&mut lines, &mut paragraphs);
    paragraphs.join("\n\n")
}

pub fn load_license_text() -> Option<String> {
    let candidates = [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/legal/LICENSE.txt"),
        std::path::PathBuf::from("/app/share/termorganize/LICENSE.txt"),
        std::path::PathBuf::from("data/legal/LICENSE.txt"),
    ];
    for path in candidates {
        if let Ok(raw) = std::fs::read_to_string(path) {
            return Some(reflow(&raw));
        }
    }
    None
}
