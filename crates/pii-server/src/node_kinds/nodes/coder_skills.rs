//! Per-run skill materialisation for `coder` nodes (spec 225 R3, R4).
//!
//! A node pins each skill it uses by name and by the sha256 of the document text it was reviewed at. Before anything
//! spawns, the stored text is checked against the pin; a mismatch is `skill.drift` and refuses the node. The checked
//! text is written into a directory for that run only, which the container binds read-only over `.pi/skills` and
//! `.claude/skills`. Nothing is written into the project.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pii_store::MemStore;

/// The lowercase sha256 of `bytes` as hex: the one pin hash, shared with `pii skill add` (spec 233 R1).
pub use pii_memory::skill::sha256_hex;

/// The skill text the store holds, by name. The daemon supplies the memory store behind this; [`NoSkills`] holds
/// nothing, so a pinned skill refuses as missing.
pub trait SkillSource: Send + Sync {
    /// The stored document text of skill `name`, or `None` when the store does not hold it.
    fn skill_text(&self, name: &str) -> Option<String>;
}

/// The source with no skills in it: every pinned skill refuses as missing.
pub struct NoSkills;

impl SkillSource for NoSkills {
    fn skill_text(&self, _name: &str) -> Option<String> {
        None
    }
}

/// The daemon's skill source: the memory store's `DocKind::Skill` documents, named as `pii skill show` names them.
///
/// A name the store does not hold, or one that names more than one skill, is `None` (spec 227 R1).
pub struct StoreSkills {
    store: Arc<MemStore>,
}

impl StoreSkills {
    /// A source over `store`, which the daemon's registry also hands to the memory nodes.
    #[must_use]
    pub fn new(store: Arc<MemStore>) -> Self {
        Self { store }
    }
}

impl SkillSource for StoreSkills {
    fn skill_text(&self, name: &str) -> Option<String> {
        let docs = pii_memory::skill::live(&self.store).ok()?;
        let picked = pii_memory::skill::pick(&docs, name, |d| (d.0.as_str(), d.1.as_str()));
        match picked.as_slice() {
            [one] => Some(one.1.clone()),
            _ => None,
        }
    }
}

/// A pinned skill whose stored text does not hash to its pin. `found` is `missing` when the store has no such skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    /// The skill's name.
    pub name: String,
    /// The sha256 the node pinned.
    pub pinned: String,
    /// The sha256 of the stored text, or `missing`.
    pub found: String,
}

/// Why a node's skills are refused before spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillRefusal {
    /// The stored text no longer matches the pin (spec 225 R4).
    Drift(Drift),
    /// The node's `skills` config is malformed, or a name cannot be a directory name.
    Invalid(String),
}

/// A skill checked against its pin and ready to write for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The skill's name, which is its directory name under `skills/`.
    pub name: String,
    /// The sha256 of `text`, which the pin matched.
    pub sha256: String,
    /// The stored document text, exactly as the store holds it.
    pub text: String,
}

/// Reads the node's `skills` config (`[{ name, sha256 }, ...]`), checks each pin against `source`, and returns the
/// skills to materialise. No `skills` config means no skills.
///
/// # Errors
/// [`SkillRefusal::Invalid`] for a malformed entry or an unsafe name; [`SkillRefusal::Drift`] for the first pin the
/// store no longer matches.
pub fn check(
    source: &dyn SkillSource,
    cfg: &serde_json::Value,
) -> Result<Vec<Rendered>, SkillRefusal> {
    let Some(entries) = cfg.get("skills") else {
        return Ok(Vec::new());
    };
    let entries = entries.as_array().ok_or_else(|| {
        SkillRefusal::Invalid("skills must be a list of { name, sha256 }".to_owned())
    })?;
    entries
        .iter()
        .map(|entry| {
            let name = entry.get("name").and_then(serde_json::Value::as_str);
            let pinned = entry.get("sha256").and_then(serde_json::Value::as_str);
            let (Some(name), Some(pinned)) = (name, pinned) else {
                return Err(SkillRefusal::Invalid(
                    "each skill needs a name and a sha256".to_owned(),
                ));
            };
            if !safe_name(name) {
                return Err(SkillRefusal::Invalid(format!(
                    "skill name is not safe: {name}"
                )));
            }
            let Some(text) = source.skill_text(name) else {
                return Err(SkillRefusal::Drift(Drift {
                    name: name.to_owned(),
                    pinned: pinned.to_owned(),
                    found: "missing".to_owned(),
                }));
            };
            let found = sha256_hex(text.as_bytes());
            if found != pinned {
                return Err(SkillRefusal::Drift(Drift {
                    name: name.to_owned(),
                    pinned: pinned.to_owned(),
                    found,
                }));
            }
            Ok(Rendered {
                name: name.to_owned(),
                sha256: found,
                text,
            })
        })
        .collect()
}

/// A name that is safe as one path component: ASCII letters, digits, `-` and `_`.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Writes `rendered` under `run_dir/skills`, one `<name>/SKILL.md` each, with the sha256 as the first line, then
/// removes every write bit. Fails if `run_dir` already exists: a run materialises once.
///
/// # Errors
/// Any I/O error, including `AlreadyExists` for a `run_dir` that is already there.
pub fn materialise(run_dir: &Path, rendered: &[Rendered]) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;

    if let Some(parent) = run_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(run_dir)?;
    let skills = run_dir.join("skills");
    std::fs::create_dir(&skills)?;
    for skill in rendered {
        let dir = skills.join(&skill.name);
        std::fs::create_dir(&dir)?;
        let file = dir.join("SKILL.md");
        std::fs::write(&file, format!("sha256={}\n{}", skill.sha256, skill.text))?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))?;
    }
    std::fs::set_permissions(&skills, std::fs::Permissions::from_mode(0o555))?;
    Ok(skills)
}
