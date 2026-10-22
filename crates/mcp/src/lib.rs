// SPDX-License-Identifier: AGPL-3.0-only
//! Local Model Context Protocol server for Term Organize.
//!
//! Unofficial. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.

pub mod catalogs;
pub mod present;
pub mod server;
pub mod tools;

pub use server::{resolve_mcp_data_dir, TermOrganize, INSTRUCTIONS};
