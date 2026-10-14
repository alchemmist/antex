//! Ensures launch observations survive setup failures.

use crate::AppServerTarget;

/// Emit once after connection, or as unconfirmed if startup returns early.
pub(super) struct Launch<F: FnOnce(&AppServerTarget, bool)>(pub(super) Option<F>);

impl<F: FnOnce(&AppServerTarget, bool)> Launch<F> {
    pub(super) fn record(mut self, target: &AppServerTarget, connected: bool) {
        if let Some(record) = self.0.take() {
            record(target, connected);
        }
    }
}

impl<F: FnOnce(&AppServerTarget, bool)> Drop for Launch<F> {
    fn drop(&mut self) {
        if let Some(record) = self.0.take() {
            record(&AppServerTarget::Embedded, /*connected*/ false);
        }
    }
}
