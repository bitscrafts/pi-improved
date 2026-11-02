//! Non-blocking claim of the daemon's durable stores (spec 344).
//!
//! The durable stores are exclusive: a second process that opens one is refused by its lock. The
//! daemon probes them before it binds anything, so the refusal is one attempt, one named store, and
//! nothing is written or created on the way.

use std::path::{Path, PathBuf};

/// The durable stores a daemon holds for its lifetime, under its durable dir.
const STORES: [&str; 2] = ["channel.redb", "timer.redb"];

/// A durable store that another process holds: the path this process could not lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locked {
    /// The store file that is held.
    pub path: PathBuf,
}

/// Probes each durable store under `dir` for a lock another process holds.
///
/// One attempt per store, no retry, no create. The probe keeps no lock: the owner opens the store right after.
///
/// # Errors
///
/// [`Locked`] names the first store another process holds. A missing store, or any other I/O error, is
/// not a claim failure: the store's own open reports it.
pub fn claim(dir: &Path) -> Result<(), Locked> {
    for name in STORES {
        let path = dir.join(name);
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        else {
            continue;
        };
        if matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
            return Err(Locked { path });
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod claim {
    use super::*;
    use std::fs::OpenOptions;

    #[test]
    fn free_stores_are_claimed_and_left_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        for name in STORES {
            std::fs::write(dir.path().join(name), b"keep").unwrap();
        }
        assert_eq!(claim(dir.path()), Ok(()));
        for name in STORES {
            assert_eq!(std::fs::read(dir.path().join(name)).unwrap(), b"keep");
        }
    }

    #[test]
    fn a_missing_store_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(claim(dir.path()), Ok(()));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_store_held_by_another_handle_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel.redb");
        std::fs::write(&path, b"held").unwrap();
        let holder = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        holder.try_lock().unwrap();
        assert_eq!(claim(dir.path()), Err(Locked { path }));
    }
}
