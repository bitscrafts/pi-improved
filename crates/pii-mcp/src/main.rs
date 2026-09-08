//! `pii-mcp` — MCP server + client (P4.7, spec 21).
//!
//! Server: exposes pii's native tools (fs/edit/bash/grep/glob) as an MCP
//! server over stdio, so Hermes (or any MCP client) can call them and
//! exercise pii's engine. Client: consumes external MCP tool servers.
//! Feature-gated (default off) and quarantines `rmcp`.

#![forbid(unsafe_code)]
// reason: the MCP server binary must print startup/error messages to stderr.
#![allow(clippy::print_stdout, clippy::print_stderr)]
// reason: the CLI binary uses process::exit for fatal startup errors.
#![allow(clippy::exit)]

use std::future::Future;

use pii_core::Ctx;
use pii_tools::ToolSet;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    TextContent, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::stdio;
use rmcp::{ErrorData, ServerHandler, serve_server};

/// The MCP server: maps pii's `ToolSet` to MCP tools.
struct PiiMcpServer {
    tools: ToolSet,
    workspace: std::path::PathBuf,
}

impl PiiMcpServer {
    fn new(workspace: std::path::PathBuf) -> Self {
        Self {
            tools: ToolSet::builtin(workspace.clone()),
            workspace,
        }
    }

    /// A root ctx granting the capabilities the builtin tools need, scoped to
    /// the workspace (fs read/write + proc for bash). This is the MCP server's
    /// own grant — it exercises pii's tools as a trusted local client would.
    fn tool_ctx(&self) -> Ctx {
        let mut caps = pii_core::CapSet::empty();
        caps.kinds =
            pii_core::CapKind::FS_READ | pii_core::CapKind::FS_WRITE | pii_core::CapKind::PROC;
        caps.fs_read = pii_core::PathScope::new(vec![self.workspace.clone()]);
        caps.fs_write = pii_core::PathScope::new(vec![self.workspace.clone()]);
        caps.proc_argv0 = pii_core::StringScope::new(vec![String::from("/bin/sh")]);
        Ctx::root(caps, pii_core::Budget::unlimited())
    }
}

impl ServerHandler for PiiMcpServer {
    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let tools = self
            .tools
            .specs()
            .into_iter()
            .map(|s| {
                let schema: serde_json::Map<String, serde_json::Value> =
                    s.schema.as_object().cloned().unwrap_or_default();
                Tool::new(s.name, s.description, schema)
            })
            .collect();
        std::future::ready(Ok(ListToolsResult {
            tools,
            ..Default::default()
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + Send + '_ {
        let name = request.name.to_string();
        let args = request
            .arguments
            .map_or_else(|| serde_json::json!({}), serde_json::Value::Object);
        let ctx = self.tool_ctx();
        let result = self.tools.get(&name).map_or_else(
            || Err(format!("unknown tool '{name}'")),
            |tool| tool.call(&ctx, &args).map_err(|e| e.to_string()),
        );
        let out = match result {
            Ok(v) => {
                CallToolResult::success(vec![ContentBlock::Text(TextContent::new(v.to_string()))])
            }
            Err(e) => CallToolResult::success(vec![ContentBlock::Text(TextContent::new(format!(
                "error: {e}"
            )))]),
        };
        std::future::ready(Ok(CallToolResponse::from(out)))
    }
}

/// `pii mcp` — start the MCP server over stdio.
fn main() {
    let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let server = PiiMcpServer::new(workspace);
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    rt.block_on(async move {
        let (r, w) = stdio();
        let _ = serve_server(server, (r, w)).await;
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::PiiMcpServer;

    /// R2 (spec 21): the MCP server exposes pii's builtin tools.
    #[test]
    fn server_lists_builtin_tools() {
        let server = PiiMcpServer::new(std::path::PathBuf::from("."));
        let names: Vec<String> = server.tools.specs().into_iter().map(|s| s.name).collect();
        for want in [
            "fs_read",
            "fs_write",
            "edit",
            "multi_edit",
            "grep",
            "glob",
            "bash",
        ] {
            assert!(
                names.iter().any(|n| n == want),
                "missing tool {want}: {names:?}"
            );
        }
    }

    /// R2 (spec 21): the server can call the `fs_read` tool on a temp file.
    #[test]
    fn server_calls_fs_read_tool() {
        use pii_core::{Budget, CapKind, CapSet, Ctx};
        let dir = tempfile::Builder::new()
            .prefix("pii-mcp-")
            .tempdir_in("/projects/pi-improved/_tmp")
            .unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "hello mcp").unwrap();
        let server = PiiMcpServer::new(dir.path().to_path_buf());
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::FS_READ;
        caps.fs_read = pii_core::PathScope::new(vec![dir.path().to_path_buf()]);
        let ctx = Ctx::root(caps, Budget::unlimited());
        let out = server
            .tools
            .get("fs_read")
            .unwrap()
            .call(&ctx, &serde_json::json!({ "path": "a.txt" }))
            .unwrap();
        assert!(out.to_string().contains("hello mcp"), "got {out}");
    }
}
