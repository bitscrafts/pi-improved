//! Filesystem capability gate.
//!
//! `Ctx` is the logical layer. It is **not** a security boundary against
//! native code in the same process; kernel confinement arrives in Phase 2.
//!
//! Every operation follows the same order, and the order IS the security
//! property: check → require cap → canonicalise → re-check scope on the
//! canonical path → only then perform I/O. A scope check on the pre-resolution
//! path is theatre; `fscap_symlink_escape_denied` exists to prove it.

use std::fs;
use std::io::Write;
use std::path::Path;

use pii_abi::PiiError;

use crate::cap::{CapKind, CapSet};
use crate::ctx::Ctx;
use crate::observe::ObsEvent;

/// A handle that can read/write/append within an enforced path scope.
pub struct FsCap<'a> {
    ctx: &'a Ctx,
}

impl<'a> FsCap<'a> {
    /// Wraps a context (not public; obtained via [`Ctx::fs`]).
    pub(crate) fn new(ctx: &'a Ctx) -> Self {
        Self { ctx }
    }

    fn check_access(&self, needed: CapKind) -> Result<(), PiiError> {
        self.ctx.check()?;
        let caps: &CapSet = self.ctx.caps();
        if !caps.kinds.contains(needed) {
            let name = if needed == CapKind::FS_READ {
                "fs_read"
            } else {
                "fs_write"
            };
            return Err(self.deny(name));
        }
        Ok(())
    }

    fn deny(&self, cap: &str) -> PiiError {
        self.ctx.observer().event(&ObsEvent::Denial {
            principal: self.ctx.principal_str(),
            cap: String::from(cap),
        });
        PiiError::Denied(cap.to_string())
    }

    /// Resolves a possibly-non-existent target to a canonical path: if the
    /// target does not exist, canonicalise its parent and re-join the name.
    fn canonical_for(p: &Path) -> Result<std::path::PathBuf, PiiError> {
        if let Ok(c) = dunce::canonicalize(p) {
            return Ok(c);
        }
        let parent = p
            .parent()
            .ok_or_else(|| PiiError::Invalid("no parent".into()))?;
        let name = p
            .file_name()
            .ok_or_else(|| PiiError::Invalid("no file name".into()))?;
        let c_parent = dunce::canonicalize(parent).map_err(|_| {
            PiiError::NotFound(format!("path parent not found: {}", parent.display()))
        })?;
        Ok(c_parent.join(name))
    }

    /// Reads a file, enforcing `FS_READ` and the read scope.
    ///
    /// # Errors
    /// Returns [`PiiError::Denied`] if the capability or scope is missing, or
    /// an I/O [`PiiError::Internal`] on failure.
    pub fn read(&self, p: &Path) -> Result<Vec<u8>, PiiError> {
        self.check_access(CapKind::FS_READ)?;
        let canonical = Self::canonical_for(p)?;
        let caps: &CapSet = self.ctx.caps();
        if !caps.fs_read.permits(&canonical) {
            return Err(self.deny("fs_read"));
        }
        fs::read(&canonical)
            .map_err(|e| PiiError::Internal(format!("read {}: {e}", canonical.display())))
    }

    /// Writes bytes to a file, enforcing `FS_WRITE` and the write scope.
    ///
    /// # Errors
    /// Returns [`PiiError::Denied`] if the capability or scope is missing, or
    /// an I/O [`PiiError::Internal`] on failure.
    pub fn write(&self, p: &Path, bytes: &[u8]) -> Result<(), PiiError> {
        self.check_access(CapKind::FS_WRITE)?;
        let canonical = Self::canonical_for(p)?;
        let caps: &CapSet = self.ctx.caps();
        if !caps.fs_write.permits(&canonical) {
            return Err(self.deny("fs_write"));
        }
        if let Some(dir) = canonical.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| PiiError::Internal(format!("mkdir {}: {e}", dir.display())))?;
        }
        fs::write(&canonical, bytes)
            .map_err(|e| PiiError::Internal(format!("write {}: {e}", canonical.display())))
    }

    /// Appends bytes to a file, enforcing `FS_WRITE` and the write scope.
    ///
    /// # Errors
    /// Returns [`PiiError::Denied`] if the capability or scope is missing, or
    /// an I/O [`PiiError::Internal`] on failure.
    pub fn append(&self, p: &Path, bytes: &[u8]) -> Result<(), PiiError> {
        self.check_access(CapKind::FS_WRITE)?;
        let canonical = Self::canonical_for(p)?;
        let caps: &CapSet = self.ctx.caps();
        if !caps.fs_write.permits(&canonical) {
            return Err(self.deny("fs_write"));
        }
        if let Some(dir) = canonical.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| PiiError::Internal(format!("mkdir {}: {e}", dir.display())))?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&canonical)
            .map_err(|e| PiiError::Internal(format!("append {}: {e}", canonical.display())))?;
        f.write_all(bytes)
            .map_err(|e| PiiError::Internal(format!("append {}: {e}", canonical.display())))
    }
}
