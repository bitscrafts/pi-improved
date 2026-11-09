//! `pii-tools`: the built-in tool set (F3.5–F3.9).
//!
//! Every tool receives an attenuated [`Ctx`]; all filesystem access goes
//! through [`Ctx::fs()`] and all process execution through [`ProcCap::run`]
//! (`pii-core::ctx::ProcOutput`) so capability denial is enforced in one
//! place. [`std::process::Command`] appears nowhere in this crate.

pub mod bash;
pub mod fs;
pub mod memory;
pub mod mqtt;
mod observe;
pub mod run;
pub mod sandbox;
pub mod search;
pub mod worktree;

use std::collections::BTreeMap;
use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::Ctx;
use serde_json::Value;

use crate::bash::BashTool;
use crate::fs::{EditTool, FsReadTool, FsWriteTool, MultiEditTool};
use crate::memory::{GraphFindTool, MemoryPutTool, MemorySearchTool};
use crate::run::RunTool;
use crate::search::{GlobTool, GrepTool};
use crate::worktree::WorktreeTool;

/// A callable tool.
pub trait Tool: Send + Sync {
    /// Tool name (also the spec key).
    fn name(&self) -> &'static str;
    /// The `ToolSpec` exposed to the model.
    fn spec(&self) -> pii_model::ToolSpec;
    /// Executes the tool.
    ///
    /// # Errors
    /// [`PiiError::Denied`] on capability/scope denial, [`PiiError::Invalid`]
    /// for bad arguments.
    fn call(&self, ctx: &Ctx, args: &Value) -> Result<Value, PiiError>;

    /// Wall-clock bound the agent loop enforces on one [`Tool::call`]
    /// (spec 12, R3), in milliseconds. `None` means the operation is fast
    /// and local enough that no bound applies. Defaults to `None`; tools
    /// that spawn processes or walk unbounded trees override it.
    fn timeout(&self) -> Option<u64> {
        None
    }
}

/// A named set of tools.
#[derive(Clone, Default)]
pub struct ToolSet {
    tools: BTreeMap<&'static str, Arc<dyn Tool>>,
    /// Receives one event per reported call (see [`ToolSet::observe_call`]).
    observer: Option<Arc<dyn Observer>>,
}

impl ToolSet {
    /// The five built-ins (F6.1 allowlist), bound to `workspace`.
    #[must_use]
    #[allow(clippy::needless_pass_by_value)] // reason: callers pass owned PathBufs
    pub fn builtin(workspace: std::path::PathBuf) -> Self {
        let mut tools: BTreeMap<&'static str, Arc<dyn Tool>> = BTreeMap::new();
        for t in [
            Arc::new(FsReadTool {
                workspace: workspace.clone(),
            }) as Arc<dyn Tool>,
            Arc::new(FsWriteTool {
                workspace: workspace.clone(),
            }),
            Arc::new(EditTool {
                workspace: workspace.clone(),
            }),
            Arc::new(MultiEditTool {
                workspace: workspace.clone(),
            }),
            Arc::new(GrepTool {
                workspace: workspace.clone(),
            }),
            Arc::new(GlobTool {
                workspace: workspace.clone(),
            }),
            Arc::new(BashTool {
                workspace: workspace.clone(),
            }) as Arc<dyn Tool>,
            // run: an argv executed without a shell (spec 399 R1); the graph `tool` node's exec path.
            Arc::new(RunTool {
                workspace: workspace.clone(),
            }) as Arc<dyn Tool>,
            Arc::new(WorktreeTool { workspace }),
            Arc::new(MemorySearchTool::default()) as Arc<dyn Tool>,
            Arc::new(MemoryPutTool::default()) as Arc<dyn Tool>,
            // graph_find: the graph catalogue, read beside memory_search (spec 388 R4).
            Arc::new(GraphFindTool::default()) as Arc<dyn Tool>,
        ] {
            tools.insert(t.name(), t);
        }
        Self {
            tools,
            observer: None,
        }
    }

    /// Adds the `mqtt` tool, backed by the broker at `url`: the model then has 11 tools instead of 10, and the
    /// eleventh is the only one that can touch the house's devices.
    ///
    /// # Errors
    /// Whatever the channel refuses when it cannot parse the url.
    pub fn with_mqtt(self, url: &str, observer: Arc<dyn Observer>) -> Result<Self, PiiError> {
        let ops: Arc<dyn MqttOps> = Arc::new(LiveOps::new(url, observer));
        Ok(self.with_tool(Arc::new(MqttTool::new(ops))))
    }

    /// A subset by name; unknown names are ignored (allowlist semantics).
    #[must_use]
    pub fn subset(&self, names: &[String]) -> Self {
        Self {
            tools: self
                .tools
                .iter()
                .filter(|(k, _)| names.iter().any(|n| n == *k))
                .map(|(k, v)| (*k, Arc::clone(v)))
                .collect(),
            observer: self.observer.clone(),
        }
    }

    /// Adds (or replaces) a single tool by name. Used by tests to inject a
    /// slow/fake tool; not part of the normal allowlist construction.
    #[must_use]
    pub fn with_tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.insert(tool.name(), tool);
        self
    }

    /// Looks a tool up by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Specs for every tool in the set.
    #[must_use]
    pub fn specs(&self) -> Vec<pii_model::ToolSpec> {
        self.tools.values().map(|t| t.spec()).collect()
    }
}

use mqtt::{LiveOps, MqttOps, MqttTool};
use pii_observe::Observer;

#[cfg(test)]
mod tests {

    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{BashTool, FsReadTool, GlobTool, GrepTool, Tool, ToolSet};

    /// Spec 12, R3: every tool declares its own bound via `Tool::timeout`
    /// — bash 120s (builds/tests are legitimately long), grep/glob 30s
    /// (unbounded tree walks), fs reads unbounded (fast local ops).
    #[test]
    fn tool_timeout_metadata() {
        let ws = std::path::PathBuf::from(".");
        assert_eq!(
            BashTool {
                workspace: ws.clone()
            }
            .timeout(),
            Some(120_000)
        );
        assert_eq!(
            GrepTool {
                workspace: ws.clone()
            }
            .timeout(),
            Some(30_000)
        );
        assert_eq!(
            GlobTool {
                workspace: ws.clone()
            }
            .timeout(),
            Some(30_000)
        );
        assert_eq!(
            FsReadTool {
                workspace: ws.clone()
            }
            .timeout(),
            None
        );
        // The set's dynamic lookup agrees with the static declarations.
        let set = ToolSet::builtin(ws);
        assert_eq!(set.get("bash").map(|t| t.timeout()), Some(Some(120_000)));
        assert_eq!(set.get("grep").map(|t| t.timeout()), Some(Some(30_000)));
        assert_eq!(set.get("fs_read").map(|t| t.timeout()), Some(None));
    }
}
