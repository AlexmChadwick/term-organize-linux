// SPDX-License-Identifier: AGPL-3.0-only
//! stdio entry point for `term-organize-mcp`.
//!
//! Unofficial. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.

use rmcp::service::ServiceExt;
use std::path::PathBuf;
use term_organize_mcp::{resolve_mcp_data_dir, TermOrganize};

fn main() {
    let (flag, read_only, help) = parse_args();
    if help {
        println!(
            "term-organize-mcp\n\
             \n\
             Local MCP server for the bundled Term Organize catalogs and degree plans.\n\
             Unofficial. Not affiliated with Northeastern University or Sandbox.\n\
             \n\
             --data-dir PATH    degree-plan directory (overrides TERM_ORGANIZE_MCP_DATA)\n\
             --read-only        hide write tools (also TERM_ORGANIZE_MCP_READONLY=1)\n\
             --help             show this help"
        );
        return;
    }
    let mcp_env = std::env::var("TERM_ORGANIZE_MCP_DATA").ok();
    let dir = resolve_mcp_data_dir(flag.as_deref(), mcp_env.as_deref());
    if let Err(error) = serve(dir, read_only) {
        eprintln!("term-organize-mcp: {error}");
        std::process::exit(1);
    }
}

#[tokio::main]
async fn serve(dir: PathBuf, read_only: bool) -> Result<(), String> {
    let server = TermOrganize::new(dir, read_only);
    let running = server
        .serve(rmcp::transport::io::stdio())
        .await
        .map_err(|error| error.to_string())?;
    running.waiting().await.map_err(|error| error.to_string())?;
    Ok(())
}

fn parse_args() -> (Option<PathBuf>, bool, bool) {
    let mut data_dir = None;
    let mut read_only = std::env::var("TERM_ORGANIZE_MCP_READONLY").ok().as_deref() == Some("1");
    let mut help = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => help = true,
            "--read-only" => read_only = true,
            "--data-dir" => {
                let Some(path) = args.next() else {
                    eprintln!("term-organize-mcp: --data-dir needs a path");
                    std::process::exit(2);
                };
                data_dir = Some(PathBuf::from(path));
            }
            other if let Some(path) = other.strip_prefix("--data-dir=") => {
                data_dir = Some(PathBuf::from(path));
            }
            other => {
                eprintln!("term-organize-mcp: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    (data_dir, read_only, help)
}
