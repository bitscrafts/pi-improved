//! The binary's build label against the HEAD a real lane run is about to use (spec 320). Pure: the label and HEAD are
//! parameters, so the rule is testable without a repository, a clock or a transport.

const UNKNOWN: &str = "unknown";
/// The commit part of a label, as `git rev-parse --short=12` prints it.
const SHA_LEN: usize = 12;

/// What the guard says before a real run: nothing, the escape-hatch note, or the refusal.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Guard {
    Run,
    Note(String),
    Abort(String),
}

/// `pii lane rev`'s one line: the label, byte for byte.
pub(crate) fn one_line(label: &str) -> String {
    format!("rev {label}")
}

/// The commit part of a label: the label with one trailing `-dirty` removed.
fn commit(label: &str) -> &str {
    label.strip_suffix("-dirty").unwrap_or(label)
}

/// `head` is HEAD's full sha, None when git could not say. Only HEAD is compared, never the working tree (spec 320 R6).
pub(crate) fn guard(label: &str, head: Option<&str>, stale_ok: bool) -> Guard {
    let sha = head.and_then(|h| h.get(..SHA_LEN)).unwrap_or(UNKNOWN);
    if label != UNKNOWN && sha != UNKNOWN && commit(label) == sha {
        return Guard::Run;
    }
    if stale_ok {
        Guard::Note(format!(
            "NOTE: running a binary built at {label} while HEAD is {sha} (PII_STALE_BIN_OK=1)"
        ))
    } else {
        Guard::Abort(format!(
            "ABORT: this binary was built at {label}; HEAD is {sha} — rebuild: cargo build -p pii-cli --bin pii"
        ))
    }
}
