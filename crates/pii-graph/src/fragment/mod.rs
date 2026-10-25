//! Spec 212: graph fragments — a skill or hook graph written once as a file
//! under `graphs/fragments/`, attached by name to a host graph, and woven in
//! at load time.
//!
//! A host declares `[[attach]]` tables. Each names a fragment file, the
//! surface it attaches to, and the sha256 `pin` the fragment must still match.
//! [`weave`] inlines the fragment's nodes as `<attach>.<node>`, stamps each
//! with `woven-from: <fragment>#<node>` and `hook: <attach>`, and splices them
//! into the host at the named surface. Inlining happens once, at load time: a
//! fragment is never generated or resolved during a run.
//!
//! The module is split by role (450-line gate): [`check`] decides what a
//! fragment file may be; [`weave`] splices the checked fragments into a host.

mod call;
mod call_needs;
mod call_record;
mod call_rewrite;
mod check;
mod shared;
mod splice;
mod weave;

pub use call_record::CallRecord;
pub use check::{Fragment, check_fragment, declares_fragment, sha256_hex};
pub use weave::{Inlined, Woven, load_host, load_host_with_graphs, weave};
