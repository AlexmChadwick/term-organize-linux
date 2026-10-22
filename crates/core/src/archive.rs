//! LZFSE-compressed JSON catalogs (`.catalog`), the same files the Mac and iOS apps ship.

use crate::model::TermCatalog;
use std::path::{Path, PathBuf};

extern "C" {
    fn lzfse_decode_buffer(
        dst: *mut u8,
        dst_size: usize,
        src: *const u8,
        src_size: usize,
        scratch: *mut u8,
    ) -> usize;
    fn lzfse_encode_buffer(
        dst: *mut u8,
        dst_size: usize,
        src: *const u8,
        src_size: usize,
        scratch: *mut u8,
    ) -> usize;
}

pub fn file_name(term_code: &str) -> String {
    format!("term-{term_code}.catalog")
}

pub fn term_code_from_file_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("term-")?.strip_suffix(".catalog")?;
    if rest.len() == 6 && rest.chars().all(|c| c.is_ascii_digit()) {
        Some(rest.to_string())
    } else {
        None
    }
}

pub fn encode(catalog: &TermCatalog) -> Result<Vec<u8>, String> {
    serde_json::to_vec(catalog).map_err(|e| format!("catalog JSON: {e}"))
}

pub fn decode(data: &[u8]) -> Result<TermCatalog, String> {
    let json = json_bytes(data)?;
    serde_json::from_slice(&json).map_err(|e| format!("catalog JSON: {e}"))
}

/// JSON payload of a `.catalog` file. Plain JSON is accepted; otherwise the
/// bytes are LZFSE, which is how the bundled snapshot is stored.
pub fn json_bytes(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.first() == Some(&b'{') {
        Ok(data.to_vec())
    } else {
        decompress_lzfse(data)
    }
}

pub fn compress_lzfse(src: &[u8]) -> Result<Vec<u8>, String> {
    let mut cap = src.len() / 2 + 64 * 1024;
    if cap < 1024 {
        cap = 1024;
    }
    let limit = src.len().saturating_add(1024 * 1024).max(cap);
    while cap <= limit {
        let mut dst = vec![0u8; cap];
        let written = unsafe {
            lzfse_encode_buffer(
                dst.as_mut_ptr(),
                dst.len(),
                src.as_ptr(),
                src.len(),
                std::ptr::null_mut(),
            )
        };
        if written > 0 && written < cap {
            dst.truncate(written);
            return Ok(dst);
        }
        cap = cap.saturating_mul(2);
    }
    Err("could not compress the catalog (LZFSE)".into())
}

pub fn read(path: &Path) -> Result<TermCatalog, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    decode(&data)
}

fn decompress_lzfse(src: &[u8]) -> Result<Vec<u8>, String> {
    let mut cap = 8 * 1024 * 1024;
    while cap <= 64 * 1024 * 1024 {
        let mut dst = vec![0u8; cap];
        let n = unsafe {
            lzfse_decode_buffer(
                dst.as_mut_ptr(),
                dst.len(),
                src.as_ptr(),
                src.len(),
                std::ptr::null_mut(),
            )
        };
        if n > 0 && n < cap {
            dst.truncate(n);
            return Ok(dst);
        }
        cap *= 2;
    }
    Err("could not decompress the catalog (LZFSE)".into())
}

/// Directories searched for shipped `term-*.catalog` files.
///
/// `TERMORGANIZE_DATA/catalog` replaces the shipped set when it already holds
/// at least one snapshot. Otherwise the search includes the install prefix
/// next to the executable (`bin/../share/termorganize/catalog`, which is where
/// the Flatpak and the AppImage put the snapshots), the repo
/// data, the Flatpak share path, and `data/catalog` next to the working directory.
pub fn bundled_catalog_directories() -> Vec<PathBuf> {
    let exe = std::env::current_exe().ok();
    bundled_catalog_directories_from(
        std::env::var_os("TERMORGANIZE_DATA").map(PathBuf::from),
        exe.as_deref(),
    )
}

fn bundled_catalog_directories_from(
    data_override: Option<PathBuf>,
    exe: Option<&Path>,
) -> Vec<PathBuf> {
    if let Some(dir) = data_override {
        let catalog = dir.join("catalog");
        if !catalogs_from_dirs(std::slice::from_ref(&catalog)).is_empty() {
            return vec![catalog];
        }
    }
    let mut dirs = Vec::new();
    if let Some(exe) = exe {
        if let Some(bin_dir) = exe.parent() {
            dirs.push(bin_dir.join("../share/termorganize/catalog"));
        }
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/catalog"));
    dirs.push(PathBuf::from("/app/share/termorganize/catalog"));
    dirs.push(PathBuf::from("data/catalog"));
    dirs
}

/// Shipped catalogs, newest term code first. The same term is kept once, from
/// the first directory that has it.
pub fn bundled_catalog_paths() -> Vec<PathBuf> {
    catalogs_from_dirs(&bundled_catalog_directories())
}

pub fn bundled_term_codes() -> Vec<String> {
    bundled_catalog_paths()
        .iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(term_code_from_file_name)
        })
        .collect()
}

/// Newest shipped catalog. A fresh launch opens this term.
pub fn bundled_catalog_path() -> Option<PathBuf> {
    bundled_catalog_paths().into_iter().next()
}

pub fn bundled_catalog_path_for(code: &str) -> Option<PathBuf> {
    bundled_catalog_paths().into_iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(term_code_from_file_name)
            .as_deref()
            == Some(code)
    })
}

/// Term opened at launch. A saved code is kept when that snapshot is available.
/// Otherwise the first code in `available` is used, and that list is newest first.
pub fn launch_term_code<'a>(saved: Option<&str>, available: &'a [String]) -> Option<&'a str> {
    if let Some(saved) = saved {
        if let Some(found) = available.iter().find(|code| code.as_str() == saved) {
            return Some(found.as_str());
        }
    }
    available.first().map(String::as_str)
}

fn catalogs_from_dirs(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut by_code = std::collections::BTreeMap::<String, PathBuf>::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(code) = term_code_from_file_name(name) else {
                continue;
            };
            by_code.entry(code).or_insert_with(|| entry.path());
        }
    }
    let mut pairs: Vec<_> = by_code.into_iter().collect();
    pairs.sort_by(|left, right| right.0.cmp(&left.0));
    pairs.into_iter().map(|(_, path)| path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lzfse_roundtrip_preserves_json() {
        let src = br#"{"version":1,"ok":true}"#;
        let compressed = compress_lzfse(src).expect("compress");
        assert_ne!(compressed.first(), Some(&b'{'));
        let back = json_bytes(&compressed).expect("decompress");
        assert_eq!(back, src);
    }

    #[test]
    fn catalogs_beside_the_executable_are_searched_first() {
        let root = std::env::temp_dir().join(format!(
            "termorganize-prefix-{}-catalog",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("bin");
        let catalogs = root.join("share/termorganize/catalog");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&catalogs).unwrap();
        let exe = bin.join("termorganize");
        std::fs::write(&exe, b"bin").unwrap();
        std::fs::write(catalogs.join("term-202730.catalog"), b"spring").unwrap();
        let dirs = bundled_catalog_directories_from(None, Some(&exe));
        assert_eq!(
            dirs.first(),
            Some(&bin.join("../share/termorganize/catalog"))
        );
        let found = catalogs_from_dirs(&dirs[..1]);
        assert_eq!(std::fs::read(&found[0]).unwrap(), b"spring");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn newer_term_code_sorts_first_and_the_first_directory_wins() {
        let root = std::env::temp_dir().join(format!(
            "termorganize-catalogs-{}-{}",
            std::process::id(),
            "order"
        ));
        let _ = std::fs::remove_dir_all(&root);
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("term-202710.catalog"), b"fall").unwrap();
        std::fs::write(first.join("term-202730.catalog"), b"from-first").unwrap();
        std::fs::write(second.join("term-202730.catalog"), b"from-second").unwrap();
        std::fs::write(second.join("notes.txt"), b"skip").unwrap();
        let paths = catalogs_from_dirs(&[first, second]);
        let names: Vec<_> = paths
            .iter()
            .map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "term-202730.catalog".to_string(),
                "term-202710.catalog".to_string()
            ]
        );
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"from-first");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn launch_term_defaults_to_newest_and_keeps_a_saved_term() {
        let available = vec!["202730".to_string(), "202710".to_string()];
        assert_eq!(launch_term_code(None, &available), Some("202730"));
        assert_eq!(launch_term_code(Some("202710"), &available), Some("202710"));
        assert_eq!(launch_term_code(Some("199910"), &available), Some("202730"));
        assert_eq!(launch_term_code(None, &[]), None);
    }

    #[test]
    fn bundled_catalogs_default_to_spring_2027() {
        assert_eq!(
            bundled_term_codes(),
            vec!["202730".to_string(), "202710".to_string()]
        );
        let path = bundled_catalog_path().expect("newest catalog");
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("term-202730.catalog")
        );
        let fall = bundled_catalog_path_for("202710").expect("fall catalog");
        assert!(fall.exists());
        let bytes = std::fs::read(&path).expect("read spring catalog");
        assert_ne!(bytes.first(), Some(&b'{'), "the snapshot is LZFSE");
        let catalog = decode(&bytes).expect("reference LZFSE decoder");
        assert_eq!(catalog.term.code, "202730");
        assert_eq!(catalog.courses.len(), 3_076);
        assert_eq!(catalog.sections.len(), 8_326);
        let mut omitted = 0;
        for section in &catalog.sections {
            let Some(instructors) = &section.instructors else {
                continue;
            };
            for instructor in instructors {
                match &instructor.email {
                    Some(email) => {
                        assert!(!email.is_empty(), "nil emails are omitted, not blank");
                        for address in crate::email::addresses_in(email) {
                            assert!(crate::email::is_northeastern_address(address), "{address}");
                        }
                    }
                    None => omitted += 1,
                }
            }
        }
        assert!(
            omitted > 0,
            "nil instructor emails are omitted from the snapshot"
        );
        let json = json_bytes(&bytes).expect("json");
        let text = String::from_utf8(json).expect("utf-8");
        assert!(!text.contains("\"email\":null"));
    }
}
