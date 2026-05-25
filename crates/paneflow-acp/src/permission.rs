//! Permission callback plumbing.
//!
//! The ACP agent calls `session/request_permission` whenever it needs the
//! user to allow or deny a tool call. paneflow-acp converts that request
//! into a [`PermissionDecision`] by invoking a caller-provided
//! [`PermissionCallback`]. The mapping back to ACP's
//! [`RequestPermissionOutcome`] lives in [`map_decision`]:
//!
//! - `AllowOnce` -> `Selected(SelectedPermissionOutcome { option_id })`
//!   using the first option advertised by the request whose kind is
//!   `AllowOnce`, falling back to the first option overall.
//! - `AllowAlways` -> `Selected(...)` picking the first `AllowAlways`
//!   option; falls back to `AllowOnce` semantics if none is offered.
//!   The persistence of the "always" pattern is the caller's job --
//!   this layer only ships the wire response.
//! - `Reject` -> `Cancelled` (also the fallback when the request has no
//!   advertised options).
//!
//! See US-002 of `tasks/prd-agents-view.md` and US-111 of
//! `tasks/prd-agent-ui-refactor-2026-Q3.md`.

use agent_client_protocol::schema::{
    PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Boxed future alias used in the permission callback trait so it stays
/// object-safe without depending on `async_trait`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Decision returned by a [`PermissionCallback`].
///
/// `AllowOnce` and `Reject` mirror the historical `Allow` / `Deny`
/// semantics one-to-one (US-018 of `prd-agents-view.md`). `AllowAlways`
/// is the new variant introduced by US-111 -- the consent persists for
/// matching future calls. Pattern persistence happens above this layer
/// in `paneflow-app::agents::runtime`, the wire mapping below only
/// communicates "the user picked the always option" to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Allow this specific call.
    AllowOnce,
    /// Allow this call AND a pattern of future calls (the pattern is
    /// recorded in `paneflow.json` by the caller before sending this
    /// decision to the runtime).
    AllowAlways,
    /// Refuse this call.
    Reject,
}

/// Caller-provided decision policy for inbound permission requests. The
/// concrete UI wiring (Allow / Deny buttons) is the consumer's job; this
/// trait is just the seam.
pub trait PermissionCallback: Send + Sync + 'static {
    fn decide(&self, request: &RequestPermissionRequest) -> BoxFuture<'_, PermissionDecision>;
}

/// Synchronous, `Fn`-based [`PermissionCallback`] for tests and trivial
/// "always allow" / "always deny" policies.
pub struct FnPermissionCallback<F>(F);

impl<F> FnPermissionCallback<F>
where
    F: Fn(&RequestPermissionRequest) -> PermissionDecision + Send + Sync + 'static,
{
    pub fn new(f: F) -> Arc<Self> {
        Arc::new(Self(f))
    }
}

impl<F> PermissionCallback for FnPermissionCallback<F>
where
    F: Fn(&RequestPermissionRequest) -> PermissionDecision + Send + Sync + 'static,
{
    fn decide(&self, request: &RequestPermissionRequest) -> BoxFuture<'_, PermissionDecision> {
        let decision = (self.0)(request);
        Box::pin(async move { decision })
    }
}

/// Convenience: an "always allow" callback (useful as a default in tests
/// and in the missing-agents empty state).
pub fn always_allow() -> Arc<dyn PermissionCallback> {
    FnPermissionCallback::new(|_| PermissionDecision::AllowOnce)
}

/// Convenience: an "always deny" callback.
pub fn always_deny() -> Arc<dyn PermissionCallback> {
    FnPermissionCallback::new(|_| PermissionDecision::Reject)
}

/// Translate a [`PermissionDecision`] into the ACP wire response
/// (US-111 of `tasks/prd-agent-ui-refactor-2026-Q3.md`).
///
/// - `AllowOnce` picks the first option whose `kind` is
///   [`PermissionOptionKind::AllowOnce`], falling back to the very
///   first advertised option if the agent did not classify any.
/// - `AllowAlways` prefers a `PermissionOptionKind::AllowAlways`
///   option; if the agent did not offer one (rare -- Claude Code and
///   Codex both do today), the response falls back to AllowOnce so
///   the agent does not stall. Persistence of the "always" pattern
///   happens above this layer.
/// - `Reject` always maps to `Cancelled`.
///
/// If the request arrives with zero options (degenerate agent), every
/// decision collapses to `Cancelled` so the agent never receives an
/// empty selection.
pub fn map_decision(
    request: &RequestPermissionRequest,
    decision: PermissionDecision,
) -> RequestPermissionResponse {
    let outcome = match decision {
        PermissionDecision::AllowOnce => select_option(request, &[PermissionOptionKind::AllowOnce]),
        PermissionDecision::AllowAlways => select_option(
            request,
            &[
                PermissionOptionKind::AllowAlways,
                PermissionOptionKind::AllowOnce,
            ],
        ),
        PermissionDecision::Reject => RequestPermissionOutcome::Cancelled,
    };
    RequestPermissionResponse::new(outcome)
}

/// Pick the first option in `request.options` whose `kind` matches any
/// of `preferred_kinds`, in order. Falls back to the first advertised
/// option overall when no preferred kind is present. Returns
/// `Cancelled` if the agent supplied zero options.
fn select_option(
    request: &RequestPermissionRequest,
    preferred_kinds: &[PermissionOptionKind],
) -> RequestPermissionOutcome {
    if request.options.is_empty() {
        tracing::warn!(
            target: "paneflow_acp::permission",
            "Allow decision but the request carries zero options; replying Cancelled",
        );
        return RequestPermissionOutcome::Cancelled;
    }
    for kind in preferred_kinds {
        if let Some(opt) = request.options.iter().find(|o| &o.kind == kind) {
            return RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                opt.option_id.clone(),
            ));
        }
    }
    let fallback = &request.options[0];
    RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(fallback.option_id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{
        PermissionOption, PermissionOptionId, PermissionOptionKind, SessionId, ToolCallId,
        ToolCallUpdate, ToolCallUpdateFields,
    };

    fn make_request_with_kinds(
        options: &[(&str, PermissionOptionKind)],
    ) -> RequestPermissionRequest {
        let opts = options
            .iter()
            .map(|(id, kind)| {
                PermissionOption::new(
                    PermissionOptionId::from((*id).to_string()),
                    format!("{id}-name"),
                    *kind,
                )
            })
            .collect();
        RequestPermissionRequest::new(
            SessionId::from("sess".to_string()),
            ToolCallUpdate::new(
                ToolCallId::from("tool-1".to_string()),
                ToolCallUpdateFields::new(),
            ),
            opts,
        )
    }

    fn make_request(option_ids: &[&str]) -> RequestPermissionRequest {
        let options: Vec<(&str, PermissionOptionKind)> = option_ids
            .iter()
            .map(|id| (*id, PermissionOptionKind::AllowOnce))
            .collect();
        make_request_with_kinds(&options)
    }

    fn assert_selected(resp: RequestPermissionResponse, expected_id: &str) {
        match resp.outcome {
            RequestPermissionOutcome::Selected(sel) => {
                assert_eq!(&*sel.option_id.0, expected_id);
            }
            other => panic!("expected Selected, got {other:?}"),
        }
    }

    #[test]
    fn allow_once_picks_allow_once_kind() {
        let req = make_request_with_kinds(&[
            ("allow-once", PermissionOptionKind::AllowOnce),
            ("allow-always", PermissionOptionKind::AllowAlways),
            ("reject", PermissionOptionKind::RejectOnce),
        ]);
        assert_selected(
            map_decision(&req, PermissionDecision::AllowOnce),
            "allow-once",
        );
    }

    #[test]
    fn allow_always_prefers_allow_always_kind() {
        let req = make_request_with_kinds(&[
            ("allow-once", PermissionOptionKind::AllowOnce),
            ("allow-always", PermissionOptionKind::AllowAlways),
            ("reject", PermissionOptionKind::RejectOnce),
        ]);
        assert_selected(
            map_decision(&req, PermissionDecision::AllowAlways),
            "allow-always",
        );
    }

    #[test]
    fn allow_always_falls_back_to_allow_once_when_missing() {
        // US-111: if the agent did not offer an AllowAlways option,
        // the decision still resolves (to an AllowOnce-equivalent on
        // the wire) so the user is not blocked by a misclassifying
        // wrapper. The persistence pattern still lands on disk.
        let req = make_request_with_kinds(&[
            ("allow-once", PermissionOptionKind::AllowOnce),
            ("reject", PermissionOptionKind::RejectOnce),
        ]);
        assert_selected(
            map_decision(&req, PermissionDecision::AllowAlways),
            "allow-once",
        );
    }

    #[test]
    fn reject_maps_to_cancelled() {
        let req = make_request(&["allow", "deny"]);
        let resp = map_decision(&req, PermissionDecision::Reject);
        assert!(matches!(resp.outcome, RequestPermissionOutcome::Cancelled));
    }

    #[test]
    fn allow_with_zero_options_falls_back_to_cancelled() {
        let req = make_request(&[]);
        let resp = map_decision(&req, PermissionDecision::AllowOnce);
        assert!(matches!(resp.outcome, RequestPermissionOutcome::Cancelled));
        let req = make_request(&[]);
        let resp = map_decision(&req, PermissionDecision::AllowAlways);
        assert!(matches!(resp.outcome, RequestPermissionOutcome::Cancelled));
    }

    #[tokio::test]
    async fn fn_callback_round_trip() {
        let cb = FnPermissionCallback::new(|_| PermissionDecision::AllowOnce);
        let req = make_request(&["allow-once"]);
        assert_eq!(cb.decide(&req).await, PermissionDecision::AllowOnce);
    }
}
