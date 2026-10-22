//! Layers a project-local config over the global `~/.pii/pii.toml`.
//!
//! The CLAUDE.md model: a global file, a project file, and the project file wins for the sections
//! a project may set. Lives apart from `config.rs` to keep that file under the effective-line
//! limit; the method is still `Config::merge_project`.

use pii_abi::PiiError;

use super::config::Config;

/// The ONE resolution of "the workspace's project config": `<workspace>/.pii/pii.toml`. Start and
/// `sys.reload` both come here, so the two can never disagree about which file is read (spec 235
/// R3).
#[must_use]
pub fn project_path(workspace: &std::path::Path) -> std::path::PathBuf {
    workspace.join(".pii").join("pii.toml")
}

/// Reads the project file when it exists NOW, and `None` when it does not.
///
/// The existence check lives in this one function because the rule is the same at start and at
/// reload: an absent project file leaves the global config alone, and a file that appears (or
/// disappears) after start is honoured on the next read. A file that exists but does not parse is
/// an error, never a silent fall back to the global config.
///
/// # Errors
/// [`PiiError::Invalid`] when the project file exists and cannot be parsed.
pub fn load_project(path: Option<&std::path::Path>) -> Result<Option<Config>, PiiError> {
    match path {
        Some(p) if p.exists() => Config::load(p).map(Some),
        _ => Ok(None),
    }
}

impl Config {
    /// Layers a project-local config over the global one. A project value wins for the overridable sections; a
    /// global-only setting in the project file is REFUSED, not dropped, so a repository cannot widen the network
    /// grants, the capability ceiling, or where the daemon writes.
    ///
    /// The project's `[[model_tier]]` list replaces the global list whole (not merged), so the effective routing
    /// is always one file's list. An absent project config returns the global config unchanged.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] naming the setting when a project file sets a global-only section or key.
    pub fn merge_project(global: Self, project: Option<Self>) -> Result<Self, PiiError> {
        let Some(project) = project else {
            return Ok(global);
        };
        let mut refused = Vec::new();
        if !project.net.grants.is_empty() {
            refused.push("[net]");
        }
        if !project.grant_ceiling.is_empty() {
            refused.push("grant_ceiling");
        }
        if project.core.runtime_dir.is_some() {
            refused.push("[core].runtime_dir");
        }
        if project.core.log_file.is_some() {
            refused.push("[core].log_file");
        }
        if project.core.audit.is_some() {
            refused.push("[core].audit");
        }
        if project.core.observer.is_some() {
            refused.push("[core].observer");
        }
        if !refused.is_empty() {
            return Err(PiiError::Invalid(format!(
                "project config sets global-only settings: {} (only the global ~/.pii/pii.toml may set them)",
                refused.join(", ")
            )));
        }
        let mut effective = global;
        if project.core.model.is_some() {
            effective.core.model = project.core.model;
        }
        if project.llama.host.is_some() {
            effective.llama.host = project.llama.host;
        }
        if !project.model_tier.is_empty() {
            effective.model_tier = project.model_tier;
        }
        Ok(effective)
    }
}
