//! Run attribution (spec 188): the origin every run is stamped with, and the
//! `since_deploy` block of `sys.status`, read from run headers in the store.

use std::sync::OnceLock;

use pii_abi::{PiiError, RunOrigin, RunStatus};
use pii_proto::sys::SinceDeploy;
use pii_store::Store;

use super::identity::REVISION;

/// The running process's origin: the build revision and the wall clock at
/// first call. Built once per process, so every run of one process shares it.
#[must_use]
pub fn process_origin() -> RunOrigin {
    static ORIGIN: OnceLock<RunOrigin> = OnceLock::new();
    ORIGIN
        .get_or_init(|| {
            use pii_core::Clock as _;
            RunOrigin {
                revision: REVISION.to_owned(),
                process_start_ms: pii_core::SystemClock.now_ms(),
            }
        })
        .clone()
}

/// Counts the runs in `store` whose header names `revision`, in one pass over
/// the run table. A header with no revision is `unattributed` and never counts
/// as any deploy's.
///
/// # Errors
/// Returns [`PiiError`] when the run table cannot be read.
pub fn since_deploy(store: &Store, revision: &str) -> Result<SinceDeploy, PiiError> {
    let mut out = SinceDeploy {
        revision: revision.to_owned(),
        ..SinceDeploy::default()
    };
    store.for_each_run_header(|h| {
        if h.revision.is_empty() {
            out.unattributed += 1;
        } else if h.revision == revision {
            out.runs += 1;
            let by = &mut out.runs_by_status;
            match h.status {
                RunStatus::Running => by.running += 1,
                RunStatus::Suspended => by.suspended += 1,
                RunStatus::Succeeded => by.succeeded += 1,
                RunStatus::Failed => by.failed += 1,
            }
        }
    })?;
    out.armed = out.runs_by_status.suspended;
    Ok(out)
}
