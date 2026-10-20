// SPDX-License-Identifier: AGPL-3.0-only
//! Bundled catalog snapshots. The server never downloads a catalog.

use std::sync::OnceLock;
use termorganize_core::archive;
use termorganize_core::index::CatalogIndex;

static CATALOGS: OnceLock<Vec<CatalogIndex>> = OnceLock::new();

pub fn catalogs() -> &'static [CatalogIndex] {
    CATALOGS
        .get_or_init(|| {
            archive::bundled_catalog_paths()
                .iter()
                .filter_map(|path| archive::read(path).ok().map(CatalogIndex::build))
                .collect()
        })
        .as_slice()
}
