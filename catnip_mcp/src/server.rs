// FILE: catnip_mcp/src/server.rs
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicU32;

use catnip_core::constants::{FORMAT_INDENT_SIZE_DEFAULT, FORMAT_LINE_LENGTH_DEFAULT};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, schemars, tool, tool_handler, tool_router};

use catnip_vm::pipeline::PurePipeline;

use crate::debug_session::McpDebugSession;
use crate::resources;
use crate::tools;

// -- TOML definitions (loaded at startup from mcp.toml) --

#[derive(serde::Deserialize)]
struct McpDefs {
    tools: HashMap<String, ToolDef>,
    resource_templates: HashMap<String, ResourceTemplateDef>,
}

#[derive(serde::Deserialize)]
struct ToolDef {
    description: String,
}

#[derive(serde::Deserialize)]
struct ResourceTemplateDef {
    uri_template: String,
    description: String,
    mime_type: String,
}

// PurePipeline contains Rc (not Send), but we only access it through a
// std::sync::Mutex with short critical sections on a single-threaded
// tokio runtime. This is safe.
pub(crate) struct PipelineCell(PurePipeline);
unsafe impl Send for PipelineCell {}

/// Pure Rust MCP server for Catnip.
#[allow(dead_code)]
pub struct CatnipMcpServer {
    pub(crate) base_path: PathBuf,
    pub(crate) pipeline: Mutex<PipelineCell>,
    pub(crate) debug_counter: AtomicU32,
    pub(crate) debug_sessions: Mutex<HashMap<String, McpDebugSession>>,
    resource_templates: Vec<ResourceTemplate>,
    tool_router: ToolRouter<Self>,
}

impl std::fmt::Debug for CatnipMcpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatnipMcpServer")
            .field("base_path", &self.base_path)
            .finish_non_exhaustive()
    }
}

impl CatnipMcpServer {
    pub fn new() -> Result<Self, String> {
        let base_path = Self::find_base_path();
        let (tool_router, resource_templates) = Self::load_mcp_defs(&base_path)?;
        let pipeline = PurePipeline::new()?;
        Ok(Self {
            base_path,
            pipeline: Mutex::new(PipelineCell(pipeline)),
            debug_counter: AtomicU32::new(0),
            debug_sessions: Mutex::new(HashMap::new()),
            resource_templates,
            tool_router,
        })
    }

    pub(crate) fn with_pipeline<R>(&self, f: impl FnOnce(&mut PurePipeline) -> R) -> R {
        let mut guard = self.pipeline.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard.0)
    }

    fn find_base_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            for ancestor in exe.ancestors().skip(1) {
                if ancestor.join("docs").is_dir() {
                    return ancestor.to_path_buf();
                }
            }
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    /// Load tool descriptions and resource templates from mcp.toml.
    fn load_mcp_defs(base_path: &Path) -> Result<(ToolRouter<Self>, Vec<ResourceTemplate>), String> {
        let toml_path = base_path.join("mcp.toml");
        let toml_str =
            std::fs::read_to_string(&toml_path).map_err(|e| format!("failed to read {}: {e}", toml_path.display()))?;
        let defs: McpDefs = toml::from_str(&toml_str).map_err(|e| format!("failed to parse mcp.toml: {e}"))?;

        // Patch tool descriptions from TOML (schemas stay from schemars)
        let mut router = Self::tool_router();
        for (name, def) in &defs.tools {
            if let Some(route) = router.map.get_mut(name.as_str()) {
                route.attr.description = Some(Cow::Owned(def.description.clone()));
            }
        }

        // Build resource templates from TOML
        let templates = defs
            .resource_templates
            .iter()
            .map(|(name, def)| {
                Annotated::new(
                    RawResourceTemplate::new(&def.uri_template, name.as_str())
                        .with_description(&def.description)
                        .with_mime_type(&def.mime_type),
                    None,
                )
            })
            .collect();

        Ok((router, templates))
    }
}

// -- Tool parameter structs --

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ParseParams {
    #[schemars(description = "Catnip source code to parse")]
    pub code: String,
    #[schemars(description = "Parse level (0=tree, 1=IR, 2=analyzed IR)")]
    pub level: Option<i32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EvalParams {
    #[schemars(description = "Catnip source code to evaluate")]
    pub code: String,
    #[schemars(
        description = "Initial variables to set before evaluation (JSON object, supports nested arrays and objects)"
    )]
    pub context: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CheckParams {
    #[schemars(description = "Catnip source code to validate")]
    pub code: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FormatParams {
    #[schemars(description = "Catnip source code to format")]
    pub code: String,
    #[schemars(description = "Indentation size (default: 4)")]
    pub indent_size: Option<usize>,
    #[schemars(description = "Maximum line length (default: 120)")]
    pub line_length: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DebugStartParams {
    #[schemars(description = "Catnip source code to debug")]
    pub code: String,
    #[schemars(description = "Line numbers to break at (1-indexed)")]
    pub breakpoints: Option<Vec<i32>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DebugSessionParams {
    #[schemars(description = "Debug session ID")]
    pub session_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DebugStepParams {
    #[schemars(description = "Debug session ID")]
    pub session_id: String,
    #[schemars(description = "Step mode: into, over, or out (default: into)")]
    pub mode: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DebugEvalParams {
    #[schemars(description = "Debug session ID")]
    pub session_id: String,
    #[schemars(description = "Expression to evaluate")]
    pub expr: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DebugBreakpointParams {
    #[schemars(description = "Debug session ID")]
    pub session_id: String,
    #[schemars(description = "Line number (1-indexed)")]
    pub line: i32,
    #[schemars(description = "Action: add or remove (default: add)")]
    pub action: Option<String>,
}

// -- Tool routing --

#[tool_router]
impl CatnipMcpServer {
    #[tool(
        description = "Parse Catnip code and return structured IR as JSON. Levels: 0=parse tree (text), 1=IR (default), 2=executable IR after semantic analysis."
    )]
    fn parse_catnip(&self, Parameters(p): Parameters<ParseParams>) -> Result<CallToolResult, ErrorData> {
        tools::parse::handle(self, &p.code, p.level.unwrap_or(1))
    }

    #[tool(description = "Evaluate Catnip code and return result. Optionally pass initial variables via context.")]
    fn eval_catnip(&self, Parameters(p): Parameters<EvalParams>) -> Result<CallToolResult, ErrorData> {
        tools::eval::handle(self, &p.code, p.context.as_ref())
    }

    #[tool(description = "Validate Catnip syntax without execution.")]
    fn check_syntax(&self, Parameters(p): Parameters<CheckParams>) -> Result<CallToolResult, ErrorData> {
        tools::check::handle(self, &p.code)
    }

    #[tool(description = "Format Catnip code with configurable style.")]
    fn format_code(&self, Parameters(p): Parameters<FormatParams>) -> Result<CallToolResult, ErrorData> {
        tools::format::handle(
            &p.code,
            p.indent_size.unwrap_or(FORMAT_INDENT_SIZE_DEFAULT),
            p.line_length.unwrap_or(FORMAT_LINE_LENGTH_DEFAULT),
        )
    }

    #[tool(description = "Start a debug session. Returns state at first breakpoint or end of execution.")]
    fn debug_start(&self, Parameters(p): Parameters<DebugStartParams>) -> Result<CallToolResult, ErrorData> {
        tools::debug::handle_start(self, &p.code, p.breakpoints.as_deref())
    }

    #[tool(description = "Continue execution until next breakpoint.")]
    fn debug_continue(&self, Parameters(p): Parameters<DebugSessionParams>) -> Result<CallToolResult, ErrorData> {
        tools::debug::handle_continue(self, &p.session_id)
    }

    #[tool(description = "Step execution. Mode: 'into' (default), 'over', or 'out'.")]
    fn debug_step(&self, Parameters(p): Parameters<DebugStepParams>) -> Result<CallToolResult, ErrorData> {
        tools::debug::handle_step(self, &p.session_id, p.mode.as_deref().unwrap_or("into"))
    }

    #[tool(description = "Inspect local variables at current pause point.")]
    fn debug_inspect(&self, Parameters(p): Parameters<DebugSessionParams>) -> Result<CallToolResult, ErrorData> {
        tools::debug::handle_inspect(self, &p.session_id)
    }

    #[tool(description = "Evaluate an expression in the current debug scope.")]
    fn debug_eval(&self, Parameters(p): Parameters<DebugEvalParams>) -> Result<CallToolResult, ErrorData> {
        tools::debug::handle_eval(self, &p.session_id, &p.expr)
    }

    #[tool(description = "Add or remove a breakpoint at a line.")]
    fn debug_breakpoint(&self, Parameters(p): Parameters<DebugBreakpointParams>) -> Result<CallToolResult, ErrorData> {
        tools::debug::handle_breakpoint(self, &p.session_id, p.line, p.action.as_deref().unwrap_or("add"))
    }
}

// -- ServerHandler impl --

#[tool_handler]
impl ServerHandler for CatnipMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().enable_resources().build())
            .with_server_info(Implementation::new("catnip", env!("CARGO_PKG_VERSION")))
            .with_instructions("Catnip language server - parse, evaluate, format, and debug Catnip code".to_string())
    }

    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult {
            resource_templates: self.resource_templates.clone(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        resources::read_resource(&self.base_path, &request.uri)
    }
}
