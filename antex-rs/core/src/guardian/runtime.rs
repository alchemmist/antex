//! Binds the existing synchronous reviewer to one captured approval action.

use antex_extension_api::ExtensionFuture;
use antex_extension_api::SynchronousApprovalReviewer;
use antex_protocol::protocol::ReviewDecision;
use std::sync::Arc;

use super::ApprovalRequestReasons;
use super::GuardianApprovalRequest;
use super::GuardianReviewContext;
use super::GuardianReviewOptions;
use super::review::run_synchronous_review;
use crate::session::session::Session;

#[derive(Clone)]
pub(super) struct ReviewRuntime {
    pub(super) session: Arc<Session>,
    pub(super) context: GuardianReviewContext,
    pub(super) review_id: String,
    pub(super) request: GuardianApprovalRequest,
    pub(super) reasons: ApprovalRequestReasons,
    pub(super) options: GuardianReviewOptions,
}

impl SynchronousApprovalReviewer for ReviewRuntime {
    fn review(
        &self,
        reason: antex_protocol::approvals::GuardianReviewReason,
    ) -> ExtensionFuture<'_, ReviewDecision> {
        Box::pin(run_synchronous_review(self.clone(), reason))
    }
}
