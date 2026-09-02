//! Cancellation token that propagates to children but not to ancestors.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A shareable cancellation flag. Cloning shares the same flag; `child()`
/// creates a *new* flag linked to this one as an ancestor.
///
/// `is_cancelled()` is true if this token's own flag OR any ancestor flag is
/// set. Cancelling a child never affects its parent.
#[derive(Debug, Clone)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
    parent: Option<Arc<Self>>,
}

impl CancelToken {
    /// A fresh, uncancelled token with no ancestor.
    #[must_use]
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            parent: None,
        }
    }

    /// A child token: shares nothing with the parent but observes it.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            parent: Some(Arc::new(self.clone())),
        }
    }

    /// Sets this token's own flag.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// True if this token or any ancestor is cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        if self.flag.load(Ordering::SeqCst) {
            return true;
        }
        let mut cur = self.parent.as_ref();
        while let Some(p) = cur {
            if p.flag.load(Ordering::SeqCst) {
                return true;
            }
            cur = p.parent.as_ref();
        }
        false
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::CancelToken;

    #[test]
    fn cancel_token_propagates_to_child() {
        let parent = CancelToken::new();
        let child = parent.child();
        assert!(!child.is_cancelled());
        parent.cancel();
        assert!(child.is_cancelled());

        // Cancelling the child leaves the parent uncancelled.
        let parent2 = CancelToken::new();
        let child2 = parent2.child();
        child2.cancel();
        assert!(child2.is_cancelled());
        assert!(!parent2.is_cancelled());
    }
}
