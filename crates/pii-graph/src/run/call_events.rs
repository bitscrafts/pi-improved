//! Spec 270 R2: a run journals each call it composed. The loader marks a call's head node with `call-from` (spec 270
//! R1), so every runner, and a resume, reads the same mark from the loaded graph.

use pii_abi::{PiiError, RunEvent};

use crate::journal::Journaler;
use crate::spec::GraphSpec;

/// Journals one `graph.call` observation per node whose cfg carries a `call-from` mark, in spec node order, so an outer
/// call precedes the call it holds. The caller flushes.
pub(super) fn journal(jr: &Journaler<'_>, spec: &GraphSpec) -> Result<(), PiiError> {
    for node in &spec.nodes {
        if let Some(fields) = node.cfg.get("call-from") {
            jr.append(&RunEvent::NodeObserved {
                node: node.id.clone(),
                kind: "graph.call".to_owned(),
                fields: fields.clone(),
            })?;
        }
    }
    Ok(())
}
