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
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, TextContent, Tool};
use rmcp::transport::stdio;
use rmcp::{ErrorData, ServerHandler, serve_server, tool_handler};

/// The dynamic tool-dispatch function type: given a tool-call context, return
/// a boxed future resolving to the MCP tool response.
type DispatchFn = for<'a> fn(
    ToolCallContext<'a, PiiMcpServer>,
) -> std::pin::Pin<
    Box<dyn Future<Output = Result<CallToolResponse, ErrorData>> + Send + 'a>,
>;

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

    /// Builds the `ToolRouter` rmcp 3.2 uses to dispatch `tools/list` and
    /// `tools/call`. One route per pii builtin tool.
    fn tool_router() -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        let mut router = rmcp::handler::server::router::tool::ToolRouter::new();
        // A dynamic dispatcher: any pii tool name -> its Tool::call.
        let dispatch: DispatchFn = |ctx: ToolCallContext<'_, Self>| {
            let name = ctx.name.to_string();
            let args = ctx
                .arguments
                .map_or_else(|| serde_json::json!({}), serde_json::Value::Object);
            // Clone the owned state out of the borrow so the returned future
            // is 'static (new_dyn requires it).
            let tools = ctx.service.tools.clone();
            let workspace = ctx.service.workspace.clone();
            let result = tools.get(&name).map_or_else(
                || Err(format!("unknown tool '{name}'")),
                |tool| {
                    let mut caps = pii_core::CapSet::empty();
                    caps.kinds = pii_core::CapKind::FS_READ
                        | pii_core::CapKind::FS_WRITE
                        | pii_core::CapKind::PROC
                        | pii_core::CapKind::MEMORY_READ
                        | pii_core::CapKind::MEMORY_WRITE;
                    caps.fs_read = pii_core::PathScope::new(vec![workspace.clone()]);
                    caps.fs_write = pii_core::PathScope::new(vec![workspace.clone()]);
                    caps.proc_argv0 = pii_core::ArgvScope::new(vec![String::from("/bin/sh")]);
                    let ctx = Ctx::root(caps, pii_core::Budget::unlimited());
                    tool.call(&ctx, &args).map_err(|e| e.to_string())
                },
            );
            let out = match result {
                Ok(v) => CallToolResult::success(vec![ContentBlock::Text(TextContent::new(
                    v.to_string(),
                ))]),
                Err(e) => CallToolResult::success(vec![ContentBlock::Text(TextContent::new(
                    format!("error: {e}"),
                ))]),
            };
            Box::pin(std::future::ready(Ok(CallToolResponse::from(out))))
        };
        // Register one route per builtin tool so `tools/list` reports them.
        for spec in ToolSet::builtin(std::path::PathBuf::from(".")).specs() {
            let schema: serde_json::Map<String, serde_json::Value> =
                spec.schema.as_object().cloned().unwrap_or_default();
            let tool = Tool::new(spec.name.clone(), spec.description.clone(), schema);
            router = router.with_route(rmcp::handler::server::router::tool::ToolRoute::new_dyn(
                tool, dispatch,
            ));
        }
        router
    }
}

#[tool_handler(router = Self::tool_router())]
impl ServerHandler for PiiMcpServer {}

/// `pii mcp` — start the MCP server over stdio, or run the client.
fn main() {
    let mut args = std::env::args_os();
    let _argv0 = args.next();
    let sub = args.next().and_then(|a| a.into_string().ok());
    if sub.as_deref() == Some("client") {
        // `pii mcp client <command> [args...]` — spawn an external MCP
        // server, list its tools, and call the first one (a smoke test).
        let cmd: Vec<String> = args.map(|a| a.into_string().unwrap_or_default()).collect();
        if cmd.is_empty() {
            eprintln!("usage: pii mcp client <command> [args...]");
            std::process::exit(2);
        }
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
            if let Err(e) = run_client(&cmd).await {
                eprintln!("error: {e}");
                std::process::exit(2);
            }
        });
    } else {
        // Default: the MCP server over stdio.
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
            // R6 (spec 22): a BOUNDED MCP server lifetime — `waiting()` resolves
            // when the stdio transport closes (e.g. a client sending EOF), so the
            // server exits cleanly instead of parking forever on `pending()`. This
            // is the fix for the "MCP client hang = server parks forever" caveat.
            let running = match serve_server(server, (r, w)).await {
                Ok(running) => running,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            };
            let _ = running.waiting().await;
        });
    }
}

/// Spawns an external MCP server, lists its tools, and calls the first one.
async fn run_client(cmd: &[String]) -> Result<(), String> {
    use rmcp::model::{CallToolRequestParams, PaginatedRequestParams};
    use rmcp::service::serve_client;
    use rmcp::transport::child_process::TokioChildProcess;

    let (prog, rest) = cmd
        .split_first()
        .ok_or_else(|| String::from("client needs a command"))?;
    let mut command = tokio::process::Command::new(prog);
    command.args(rest);
    // R6 (spec 22): silence the spawned server's stderr so a child that
    // writes diagnostics cannot wedge the stdio pipe the client reads from.
    command.stderr(std::process::Stdio::null());
    let child = TokioChildProcess::new(command).map_err(|e| e.to_string())?;
    let running = serve_client((), child).await.map_err(|e| e.to_string())?;
    let peer = running.peer();
    let tools = peer
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "tools: {:?}",
        tools
            .tools
            .iter()
            .map(|t| t.name.as_ref())
            .collect::<Vec<_>>()
    );
    if let Some(tool) = tools.tools.first() {
        let name = tool.name.to_string();
        let call_out = peer
            .call_tool_once(CallToolRequestParams::new(name.clone()))
            .await
            .map_err(|e| e.to_string())?;
        println!("call {name}: {call_out:?}");
    }
    // R6 (spec 22): cancel the running service so the client half of the
    // self-test is a CLEAN exit code (not a hang on a parked child). `peer`'s
    // borrow of `running` ends at its last use above (NLL); `cancel` consumes
    // the service and terminates the spawned child.
    running
        .cancel()
        .await
        .map_err(|e| format!("cancel client: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;
    use std::sync::Arc;

    use super::PiiMcpServer;

    /// R2 (spec 21): the MCP server exposes pii's builtin tools.
    #[test]
    fn server_lists_builtin_tools() {
        let server = PiiMcpServer::new(PathBuf::from("."));
        let names: Vec<String> = server.tools.specs().into_iter().map(|s| s.name).collect();
        for want in [
            "fs_read",
            "fs_write",
            "edit",
            "multi_edit",
            "grep",
            "glob",
            "bash",
            "memory_search",
            "memory_put",
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

    /// Spec 024 N1/N5: `memory_put` then `memory_search`, called exactly as
    /// the MCP dispatcher calls them (same `Tool::call`, same capability
    /// grant the router hands out), against a REAL daemon on a REAL UDS
    /// socket. This is the live evidence the 2026-09-11 caveats doc (C1/C8)
    /// could only produce manually, promoted to CI.
    // `call_daemon` (pii-tools) blocks its calling thread on
    // `std::thread::spawn(...).join()`; a single-worker runtime would starve
    // the background accept loop below of a thread to run on and deadlock.
    #[tokio::test(flavor = "multi_thread")]
    async fn memory_tools_round_trip_through_a_real_daemon() {
        use pii_core::{Budget, CapKind, CapSet, Ctx};
        use pii_server::conn::{PeerIdentity, serve_connection};
        use pii_server::uds::test_bind_socket;
        use pii_server::{Engine, EngineConfig};
        use pii_tools::Tool as _;
        use pii_tools::memory::{MemoryPutTool, MemorySearchTool};

        let dir = tempfile::Builder::new()
            .prefix("pii-mcp-memory-")
            .tempdir_in("/projects/pi-improved/_tmp")
            .unwrap();
        let sock = dir.path().join("control.sock");
        let fixtures_hello = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join("fixtures/phase2/hello.json");
        let engine = Engine::new(EngineConfig {
            workspace: dir.path().to_path_buf(),
            store_path: dir.path().join("store.redb"),
            model: format!("scripted:{}", fixtures_hello.display()),
            sandbox: pii_tools::sandbox::SandboxProfileName::Off,
            route_policy: std::collections::BTreeMap::new(),
            session_runs_cap: pii_server::DEFAULT_SESSION_RUNS_CAP,
            graph_dir: dir.path().join("graphs"),
        })
        .expect("engine");
        let listener = test_bind_socket(&sock).expect("bind");
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(serve_connection(
                    Arc::clone(&engine),
                    stream,
                    PeerIdentity::InProcess,
                ));
            }
        });

        // The same CapSet `tool_router`'s dispatch closure grants an MCP
        // caller (`FS_READ|FS_WRITE|PROC|MEMORY_READ|MEMORY_WRITE`); memory
        // only needs the memory bits.
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::MEMORY_READ | CapKind::MEMORY_WRITE;
        let ctx = Ctx::root(caps, Budget::unlimited());

        let put_out = MemoryPutTool {
            sock_override: Some(sock.clone()),
        }
        .call(&ctx, &serde_json::json!({"text": "mcp memory e2e fact"}))
        .expect("memory_put");
        let id = put_out
            .get("id")
            .and_then(|v| v.as_str())
            .expect("put must return an id")
            .to_owned();
        assert!(!id.is_empty());

        let search_out = MemorySearchTool {
            sock_override: Some(sock),
        }
        .call(&ctx, &serde_json::json!({"query": "mcp memory e2e fact"}))
        .expect("memory_search");
        let hits = search_out
            .get("hits")
            .and_then(|v| v.as_array())
            .expect("hits array");
        assert!(
            hits.iter()
                .any(|h| h.get("doc").and_then(|d| d.as_str()) == Some(id.as_str())),
            "put doc {id} not found in search hits: {hits:?}"
        );
    }
}
