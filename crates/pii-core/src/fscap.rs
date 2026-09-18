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

use crate::CapKind;
use crate::cap::CapSet;
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
    /// target does not exist, canonicalise its *nearest existing ancestor*
    /// and re-join the non-existent tail. This keeps the scope check honest
    /// for targets whose parent directories do not exist yet — the check
    /// still happens on the canonical location the file *would* occupy.
    fn canonical_for(p: &Path) -> Result<std::path::PathBuf, PiiError> {
        if let Ok(c) = dunce::canonicalize(p) {
            return Ok(c);
        }
        let mut existing = p.to_path_buf();
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        loop {
            let parent = existing.parent().ok_or_else(|| {
                PiiError::NotFound(format!("path has no parent: {}", p.display()))
            })?;
            let name = existing
                .file_name()
                .ok_or_else(|| PiiError::Invalid(format!("no file name: {}", p.display())))?
                .to_os_string();
            tail.push(name);
            if parent.as_os_str().is_empty() {
                return Err(PiiError::NotFound(format!(
                    "path parent not found: {}",
                    p.display()
                )));
            }
            if let Ok(c) = dunce::canonicalize(parent) {
                let mut canon = c;
                for part in tail.iter().rev() {
                    canon.push(part);
                }
                return Ok(canon);
            }
            existing = parent.to_path_buf();
        }
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::path::{Path, PathBuf};

    use pii_abi::PiiError;

    use crate::CapKind;
    use crate::budget::Budget;
    use crate::cap::{CapSet, PathScope};
    use crate::ctx::Ctx;

    fn tmp_root() -> PathBuf {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/_tmp"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A root context granting `FS_WRITE` over exactly `allow`.
    fn write_ctx(allow: &[PathBuf]) -> Ctx {
        let mut caps = CapSet::empty();
        caps.kinds = CapKind::FS_WRITE;
        caps.fs_write = PathScope::new(allow.to_vec());
        Ctx::root(caps, Budget::unlimited())
    }

    #[test]
    fn fscap_write_outside_scope_denied() {
        let dir = tempfile::Builder::new()
            .prefix("fscap-out-")
            .tempdir_in(tmp_root())
            .unwrap();
        let ok_dir = dir.path().join("ok");
        std::fs::create_dir_all(&ok_dir).unwrap();
        let ok_canon = dunce::canonicalize(&ok_dir).unwrap();

        let ctx = write_ctx(&[ok_canon]);
        let outside = dir.path().join("nope").join("f");
        let err = ctx.fs().unwrap().write(&outside, b"x").unwrap_err();
        assert!(matches!(err, PiiError::Denied(_)), "got {err:?}");
        assert!(!outside.exists(), "file must not be created");
    }

    #[test]
    fn fscap_symlink_escape_denied() {
        let dir = tempfile::Builder::new()
            .prefix("fscap-link-")
            .tempdir_in(tmp_root())
            .unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("/etc", &link).unwrap();

        let scope = dunce::canonicalize(dir.path()).unwrap();
        let ctx = write_ctx(&[scope]);

        let target = link.join("pii-escape");
        let result = ctx.fs().unwrap().write(&target, b"escape");
        assert!(matches!(result, Err(PiiError::Denied(_))), "got {result:?}");
        assert!(!Path::new("/etc/pii-escape").exists());

        std::fs::remove_file(&link).unwrap();
    }
}
