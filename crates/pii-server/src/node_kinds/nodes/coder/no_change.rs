//! The no-change refusal (spec 239).
//!
//! A node whose agent changed nothing does not pass when its criteria already passed before the agent ran: the
//! criteria could not tell that the agent did anything, so the run certifies nothing.

/// Why a passing run is refused, or `None` when it stands.
///
/// Refused only when the verdict passed, the tree is unchanged, and every criterion passed on the tree as the agent
/// found it. A criterion that failed before the agent ran and passes now means something changed, so `tree_changed`
/// cannot be `no`; an `unknown` tree is not evidence that nothing changed; and no pre-run means nothing was measured.
#[must_use]
pub fn no_change_refusal(
    pre_passed: Option<bool>,
    tree_changed: &str,
    verdict_passed: bool,
) -> Option<String> {
    if verdict_passed && pre_passed == Some(true) && tree_changed == "no" {
        return Some(String::from(
            "no change: the agent changed nothing and the criteria already passed before it ran",
        ));
    }
    None
}
