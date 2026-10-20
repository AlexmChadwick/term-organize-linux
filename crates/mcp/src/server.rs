// SPDX-License-Identifier: AGPL-3.0-only
//! MCP stdio server. Protocol revision 2025-06-18.

use crate::tools::{self, ToolError};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Implementation, ProtocolVersion, ServerCapabilities, ServerConfig,
};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

pub const INSTRUCTIONS: &str = "Unofficial Northeastern course planner. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox. Reads the bundled Fall 2026 and Spring 2027 catalogs only. Can read and update degree plans stored in the local Term Organize user file. Does not use the network, does not log in, and does not return student name or student ID.";

const WRITE_TOOLS: &[&str] = &[
    "create_plan",
    "rename_plan",
    "add_course",
    "remove_course",
    "move_course",
    "set_section",
    "apply_reorganize",
    "delete_plan",
];

pub struct TermOrganize {
    tool_router: ToolRouter<Self>,
    data_dir: PathBuf,
    read_only: bool,
}

impl TermOrganize {
    pub fn new(data_dir: PathBuf, read_only: bool) -> Self {
        let mut tool_router = Self::tool_router();
        if read_only {
            for name in WRITE_TOOLS {
                tool_router.disable_route(*name);
            }
        }
        Self {
            tool_router,
            data_dir,
            read_only,
        }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn tool_definitions(&self) -> Vec<rmcp::model::Tool> {
        self.tool_router.list_all()
    }
}

/// `--data-dir`, then `TERM_ORGANIZE_MCP_DATA`, then the shared app data directory.
pub fn resolve_mcp_data_dir(flag: Option<&Path>, mcp_env: Option<&str>) -> PathBuf {
    if let Some(dir) = flag {
        return dir.to_path_buf();
    }
    if let Some(dir) = mcp_env.map(str::trim).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    termorganize_core::store::data_dir()
}

fn finish(result: Result<serde_json::Value, ToolError>) -> CallToolResult {
    match result {
        Ok(value) => CallToolResult::structured(value),
        Err(error) => CallToolResult::structured_error(error.to_value()),
    }
}

#[tool_router]
impl TermOrganize {
    #[tool(
        description = "List bundled terms (Fall 2026 and Spring 2027).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn list_terms(&self) -> CallToolResult {
        CallToolResult::structured(tools::list_terms())
    }

    #[tool(
        description = "Search courses in one bundled term.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn search_courses(&self, Parameters(args): Parameters<tools::SearchArgs>) -> CallToolResult {
        finish(tools::search_courses(args))
    }

    #[tool(
        description = "Course details and every section in one bundled term.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn get_course(&self, Parameters(args): Parameters<tools::CourseArgs>) -> CallToolResult {
        finish(tools::get_course(args))
    }

    #[tool(
        description = "Report meeting clashes and unknown CRNs.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn check_schedule(&self, Parameters(args): Parameters<tools::CheckArgs>) -> CallToolResult {
        finish(tools::check_schedule(args))
    }

    #[tool(
        description = "Conflict-free section combinations, best first.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn suggest_schedule(&self, Parameters(args): Parameters<tools::SuggestArgs>) -> CallToolResult {
        finish(tools::suggest_schedule(args))
    }

    #[tool(
        description = "Move courses across the terms already on a plan so sections exist and chosen CRNs do not overlap.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn reorganize_plan(
        &self,
        Parameters(args): Parameters<tools::ReorganizeArgs>,
    ) -> CallToolResult {
        finish(tools::reorganize_plan(args))
    }

    #[tool(
        description = "List saved degree plans: id, name, terms, updated time, and revision.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn list_plans(&self) -> CallToolResult {
        CallToolResult::structured(tools::list_plans(&self.data_dir))
    }

    #[tool(
        description = "One saved degree plan, its sections, and validation. Omits student name and student ID.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn get_plan(&self, Parameters(args): Parameters<tools::PlanIdArgs>) -> CallToolResult {
        finish(tools::get_plan(&self.data_dir, &args.plan_id))
    }

    #[tool(
        description = "Create a degree plan in the local Term Organize user file.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn create_plan(&self, Parameters(args): Parameters<tools::CreatePlanArgs>) -> CallToolResult {
        finish(tools::create_plan(&self.data_dir, args))
    }

    #[tool(
        description = "Rename a degree plan. Fails when expected_revision is stale.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn rename_plan(&self, Parameters(args): Parameters<tools::RenameArgs>) -> CallToolResult {
        finish(tools::rename_plan(&self.data_dir, args))
    }

    #[tool(
        description = "Add a catalog course to a term already on the plan.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn add_course(&self, Parameters(args): Parameters<tools::AddCourseArgs>) -> CallToolResult {
        finish(tools::add_course(&self.data_dir, args))
    }

    #[tool(
        description = "Remove a course from one term of a saved plan.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    fn remove_course(
        &self,
        Parameters(args): Parameters<tools::RemoveCourseArgs>,
    ) -> CallToolResult {
        finish(tools::remove_course(&self.data_dir, args))
    }

    #[tool(
        description = "Move a course from one term on the plan to another.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn move_course(&self, Parameters(args): Parameters<tools::MoveCourseArgs>) -> CallToolResult {
        finish(tools::move_course(&self.data_dir, args))
    }

    #[tool(
        description = "Choose a section CRN for a course already on the plan.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn set_section(&self, Parameters(args): Parameters<tools::SetSectionArgs>) -> CallToolResult {
        finish(tools::set_section(&self.data_dir, args))
    }

    #[tool(
        description = "Run reorganize_plan on a saved degree plan and store the result.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    fn apply_reorganize(
        &self,
        Parameters(args): Parameters<tools::RevisionArgs>,
    ) -> CallToolResult {
        finish(tools::apply_reorganize(&self.data_dir, args))
    }

    #[tool(
        description = "Delete a degree plan. Requires confirm true and the current revision.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    fn delete_plan(&self, Parameters(args): Parameters<tools::DeleteArgs>) -> CallToolResult {
        finish(tools::delete_plan(&self.data_dir, args))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TermOrganize {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("term-organize", env!("CARGO_PKG_VERSION"))
                    .with_title("Term Organize")
                    .with_description(INSTRUCTIONS),
            )
            .with_instructions(INSTRUCTIONS)
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2025_06_18])
    }
}
