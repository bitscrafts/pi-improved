//! `state.md` management (R2).
//!
//! The boss's durable "where am I" source of truth. Written at every
//! milestone; never inferred from transcript history. Durable facts
//! (lessons, resolutions, model-routing learnings) belong in external
//! memory, not here — `state.md` only ever tracks goal/phase/step and
//! small routing properties.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use pii_core::{Ctx, PiiError};

/// The filename written under a boss state directory.
const FILE_NAME: &str = "state.md";

/// The boss's persistent progress record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StateDoc {
    /// The overarching goal driving the current cycle.
    pub goal: String,
    /// The current named phase (e.g. "localize", "patch", "`pending_human`").
    pub phase: String,
    /// The last completed step, for resumption after a restart.
    pub last_step: String,
    /// The next planned step.
    pub next_step: String,
    /// Small key/value map for decisions and model-routing learnings.
    pub properties: BTreeMap<String, String>,
    /// Milliseconds since the epoch; strictly increases across every
    /// [`StateDoc::touch`].
    pub updated_at: u64,
}

impl StateDoc {
    /// Loads `state.md` from `dir`, or a fresh default doc when the file is
    /// absent (a first-run boss has no prior state).
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] if the file exists but cannot be
    /// read, or [`PiiError::Invalid`] if its contents are malformed.
    pub fn load(dir: &Path) -> Result<Self, PiiError> {
        let path = dir.join(FILE_NAME);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)
            .map_err(|e| PiiError::Internal(format!("read {}: {e}", path.display())))?;
        Self::parse(&text)
    }

    /// Serializes and writes `state.md` into `dir`, creating the directory
    /// if it does not already exist.
    ///
    /// # Errors
    /// Returns [`PiiError::Internal`] on any filesystem failure.
    pub fn save(&self, dir: &Path) -> Result<(), PiiError> {
        fs::create_dir_all(dir)
            .map_err(|e| PiiError::Internal(format!("create {}: {e}", dir.display())))?;
        let path = dir.join(FILE_NAME);
        fs::write(&path, self.render())
            .map_err(|e| PiiError::Internal(format!("write {}: {e}", path.display())))
    }

    /// Bumps `updated_at` to the current time via `ctx`'s injected clock,
    /// guaranteeing a strictly later value even under coarse clock
    /// resolution or repeated calls within the same millisecond.
    pub fn touch(&mut self, ctx: &Ctx) {
        self.updated_at = ctx.now_ms().max(self.updated_at + 1);
    }

    /// The compact system-prompt fragment for compaction: a goal line
    /// followed by phase/step context. Properties are deliberately omitted
    /// — durable learnings live in external memory, not the compaction
    /// summary.
    #[must_use]
    pub fn summarize(&self) -> String {
        format!(
            "Goal: {}\nPhase: {}\nLast step: {}\nNext step: {}\n",
            self.goal, self.phase, self.last_step, self.next_step
        )
    }

    /// Renders the on-disk `state.md` text.
    fn render(&self) -> String {
        let mut out = String::from("# pii-evolve state\n\n");
        let _ = writeln!(out, "goal: {}", self.goal);
        let _ = writeln!(out, "phase: {}", self.phase);
        let _ = writeln!(out, "last_step: {}", self.last_step);
        let _ = writeln!(out, "next_step: {}", self.next_step);
        let _ = writeln!(out, "updated_at: {}", self.updated_at);
        out.push_str("\n## properties\n");
        for (key, value) in &self.properties {
            let _ = writeln!(out, "{key}: {value}");
        }
        out
    }

    /// Parses the `state.md` text produced by [`StateDoc::render`].
    fn parse(text: &str) -> Result<Self, PiiError> {
        let mut doc = Self::default();
        let mut in_properties = false;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if line == "## properties" {
                in_properties = true;
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            if in_properties {
                doc.properties.insert(key.to_owned(), value.to_owned());
                continue;
            }
            match key {
                "goal" => value.clone_into(&mut doc.goal),
                "phase" => value.clone_into(&mut doc.phase),
                "last_step" => value.clone_into(&mut doc.last_step),
                "next_step" => value.clone_into(&mut doc.next_step),
                "updated_at" => {
                    doc.updated_at = value.parse().map_err(|_| {
                        PiiError::Invalid(format!("state.md: bad updated_at {value:?}"))
                    })?;
                }
                _ => {}
            }
        }
        Ok(doc)
    }
}
