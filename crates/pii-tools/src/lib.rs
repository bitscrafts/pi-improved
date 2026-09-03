//! `pii-tools`: the built-in tool set (F3.5–F3.9).
//!
//! Every tool receives an attenuated [`Ctx`]; all filesystem access goes
//! through [`Ctx::fs()`] and all process execution through [`ProcCap::run`]
//! (`pii-core::ctx::ProcOutput`) so capability denial is enforced in one
//! place. [`std::process::Command`] appears nowhere in this crate.

pub mod bash;
pub mod fs;
pub mod sandbox;

use std::collections::BTreeMap;
use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::Ctx;
use serde_json::Value;

use crate::bash::BashTool;
use crate::fs::{EditTool, FsReadTool, FsWriteTool, MultiEditTool};

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
}

/// A named set of tools.
#[derive(Clone, Default)]
pub struct ToolSet {
    tools: BTreeMap<&'static str, Arc<dyn Tool>>,
}

impl ToolSet {
    /// The five built-ins (F6.1 allowlist), bound to `workspace`.
    #[must_use]
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
            Arc::new(BashTool { workspace }) as Arc<dyn Tool>,
        ] {
            tools.insert(t.name(), t);
        }
        Self { tools }
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
        }
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
