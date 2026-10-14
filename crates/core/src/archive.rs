//! LZFSE-compressed JSON catalogs (`.catalog`), the same files the Mac and iOS apps ship.

use crate::model::TermCatalog;
use std::path::Path;

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

pub fn bundled_catalog_path() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("TERMORGANIZE_DATA") {
        let p = std::path::PathBuf::from(dir).join("catalog/term-202710.catalog");
        if p.exists() {
            return Some(p);
        }
    }
    let candidates = [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/catalog/term-202710.catalog"),
        std::path::PathBuf::from("/app/share/termorganize/catalog/term-202710.catalog"),
        std::path::PathBuf::from("data/catalog/term-202710.catalog"),
    ];
    candidates.into_iter().find(|p| p.exists())
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
}
