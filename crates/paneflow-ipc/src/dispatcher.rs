// US-013: JSON-RPC protocol dispatcher

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

/// Type alias for an async handler function that processes a JSON-RPC method call.
///
/// Receives optional parameters and returns a `JsonRpcResponse`.
pub type HandlerFn =
    Arc<dyn Fn(Option<Value>) -> Pin<Box<dyn Future<Output = JsonRpcResponse> + Send>> + Send + Sync>;

/// Routes incoming JSON-RPC requests to registered handler functions.
///
/// Each method name maps to a single `HandlerFn`. The dispatcher handles
/// parsing, routing, error generation for unknown methods and malformed input,
/// and serialization of the response back to JSON.
pub struct Dispatcher {
    handlers: HashMap<String, HandlerFn>,
}

impl Dispatcher {
    /// Create a new dispatcher with the built-in `system.ping` handler registered.
    pub fn new() -> Self {
        let mut dispatcher = Self {
            handlers: HashMap::new(),
        };
        dispatcher.register(
            "system.ping",
            Arc::new(|_params| {
                Box::pin(async {
                    JsonRpcResponse::success(None, serde_json::json!({"pong": true}))
                })
            }),
        );
        dispatcher
    }

    /// Register a handler for the given method name.
    ///
    /// If a handler was previously registered for this method, it is replaced.
    pub fn register(&mut self, method: &str, handler: HandlerFn) {
        self.handlers.insert(method.to_string(), handler);
    }

    /// Parse a raw JSON string, route to the appropriate handler, and return
    /// the serialized JSON response.
    ///
    /// - Malformed JSON produces a `parse_error` response.
    /// - Unknown methods produce a `method_not_found` response.
    /// - The response `id` always mirrors the request `id` (or `null` if absent).
    pub async fn dispatch(&self, raw_json: &str) -> String {
        let request = match serde_json::from_str::<JsonRpcRequest>(raw_json) {
            Ok(req) => req,
            Err(e) => {
                let resp = JsonRpcResponse::error(
                    None,
                    "parse_error",
                    format!("Failed to parse JSON-RPC request: {e}"),
                );
                return serde_json::to_string(&resp)
                    .expect("JsonRpcResponse serialization should never fail");
            }
        };

        let id = request.id.clone();

        let response = match self.handlers.get(&request.method) {
            Some(handler) => {
                let mut resp = handler(request.params).await;
                resp.id = id;
                resp
            }
            None => JsonRpcResponse::error(
                id,
                "method_not_found",
                format!("Unknown method: {}", request.method),
            ),
        };

        serde_json::to_string(&response)
            .expect("JsonRpcResponse serialization should never fail")
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Helper ─────────────────────────────────────────────────────────

    /// Parse a JSON string into a `JsonRpcResponse`, panicking on failure.
    fn parse_response(raw: &str) -> JsonRpcResponse {
        serde_json::from_str(raw).expect("response should be valid JSON")
    }

    // ── Constructor tests ──────────────────────────────────────────────

    #[test]
    fn new_dispatcher_has_system_ping() {
        let d = Dispatcher::new();
        assert!(d.handlers.contains_key("system.ping"));
    }

    #[test]
    fn default_trait_creates_same_as_new() {
        let d = Dispatcher::default();
        assert!(d.handlers.contains_key("system.ping"));
    }

    // ── system.ping ────────────────────────────────────────────────────

    #[tokio::test]
    async fn system_ping_returns_pong() {
        let d = Dispatcher::new();
        let raw = r#"{"id":"1","method":"system.ping"}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(resp.ok);
        assert_eq!(resp.id.as_deref(), Some("1"));
        assert_eq!(resp.result, Some(json!({"pong": true})));
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn system_ping_without_id() {
        let d = Dispatcher::new();
        let raw = r#"{"method":"system.ping"}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(resp.ok);
        assert!(resp.id.is_none());
        assert_eq!(resp.result, Some(json!({"pong": true})));
    }

    #[tokio::test]
    async fn system_ping_ignores_params() {
        let d = Dispatcher::new();
        let raw = r#"{"id":"p1","method":"system.ping","params":{"extra":"data"}}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(resp.ok);
        assert_eq!(resp.result, Some(json!({"pong": true})));
    }

    // ── Method not found ───────────────────────────────────────────────

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let d = Dispatcher::new();
        let raw = r#"{"id":"2","method":"nonexistent.method"}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(!resp.ok);
        assert_eq!(resp.id.as_deref(), Some("2"));
        assert!(resp.result.is_none());

        let err = resp.error.expect("should have error");
        assert_eq!(err.code, "method_not_found");
        assert!(
            err.message.contains("nonexistent.method"),
            "error message should contain the unknown method name"
        );
    }

    #[tokio::test]
    async fn unknown_method_without_id_returns_null_id() {
        let d = Dispatcher::new();
        let raw = r#"{"method":"no.such.method"}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(!resp.ok);
        assert!(resp.id.is_none());

        let err = resp.error.expect("should have error");
        assert_eq!(err.code, "method_not_found");
    }

    // ── Parse errors ───────────────────────────────────────────────────

    #[tokio::test]
    async fn malformed_json_returns_parse_error() {
        let d = Dispatcher::new();
        let raw = "this is not json at all";
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(!resp.ok);
        assert!(resp.id.is_none());
        assert!(resp.result.is_none());

        let err = resp.error.expect("should have error");
        assert_eq!(err.code, "parse_error");
        assert!(!err.message.is_empty());
    }

    #[tokio::test]
    async fn empty_string_returns_parse_error() {
        let d = Dispatcher::new();
        let resp_str = d.dispatch("").await;
        let resp = parse_response(&resp_str);

        assert!(!resp.ok);
        let err = resp.error.expect("should have error");
        assert_eq!(err.code, "parse_error");
    }

    #[tokio::test]
    async fn json_missing_method_field_returns_parse_error() {
        let d = Dispatcher::new();
        // Valid JSON but not a valid JsonRpcRequest (missing required `method`).
        let raw = r#"{"id":"3","params":{}}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(!resp.ok);
        assert!(resp.id.is_none());

        let err = resp.error.expect("should have error");
        assert_eq!(err.code, "parse_error");
    }

    #[tokio::test]
    async fn incomplete_json_returns_parse_error() {
        let d = Dispatcher::new();
        let raw = r#"{"id":"4","method":"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(!resp.ok);
        let err = resp.error.expect("should have error");
        assert_eq!(err.code, "parse_error");
    }

    // ── Custom handler registration ────────────────────────────────────

    #[tokio::test]
    async fn register_and_dispatch_custom_handler() {
        let mut d = Dispatcher::new();
        d.register(
            "test.echo",
            Arc::new(|params| {
                Box::pin(async move {
                    let value = params.unwrap_or(json!(null));
                    JsonRpcResponse::success(None, value)
                })
            }),
        );

        let raw = r#"{"id":"e1","method":"test.echo","params":{"msg":"hello"}}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(resp.ok);
        assert_eq!(resp.id.as_deref(), Some("e1"));
        assert_eq!(resp.result, Some(json!({"msg": "hello"})));
    }

    #[tokio::test]
    async fn handler_receives_none_when_params_omitted() {
        let mut d = Dispatcher::new();
        d.register(
            "test.check_params",
            Arc::new(|params| {
                Box::pin(async move {
                    let has_params = params.is_some();
                    JsonRpcResponse::success(None, json!({"has_params": has_params}))
                })
            }),
        );

        let raw = r#"{"id":"cp1","method":"test.check_params"}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(resp.ok);
        assert_eq!(resp.result, Some(json!({"has_params": false})));
    }

    #[tokio::test]
    async fn handler_can_return_error_response() {
        let mut d = Dispatcher::new();
        d.register(
            "test.fail",
            Arc::new(|_params| {
                Box::pin(async {
                    JsonRpcResponse::error(None, "test_error", "intentional failure")
                })
            }),
        );

        let raw = r#"{"id":"f1","method":"test.fail"}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(!resp.ok);
        assert_eq!(resp.id.as_deref(), Some("f1"));

        let err = resp.error.expect("should have error");
        assert_eq!(err.code, "test_error");
        assert_eq!(err.message, "intentional failure");
    }

    #[tokio::test]
    async fn replacing_handler_uses_new_one() {
        let mut d = Dispatcher::new();
        d.register(
            "test.replace",
            Arc::new(|_| {
                Box::pin(async { JsonRpcResponse::success(None, json!({"version": 1})) })
            }),
        );
        d.register(
            "test.replace",
            Arc::new(|_| {
                Box::pin(async { JsonRpcResponse::success(None, json!({"version": 2})) })
            }),
        );

        let raw = r#"{"id":"r1","method":"test.replace"}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert!(resp.ok);
        assert_eq!(resp.result, Some(json!({"version": 2})));
    }

    // ── ID propagation ─────────────────────────────────────────────────

    #[tokio::test]
    async fn response_id_matches_request_id() {
        let d = Dispatcher::new();
        let raw = r#"{"id":"unique-42","method":"system.ping"}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert_eq!(resp.id.as_deref(), Some("unique-42"));
    }

    #[tokio::test]
    async fn handler_id_is_overridden_by_request_id() {
        // Even if the handler sets its own id, dispatch should override it
        // with the request id.
        let mut d = Dispatcher::new();
        d.register(
            "test.own_id",
            Arc::new(|_| {
                Box::pin(async {
                    JsonRpcResponse::success(Some("handler-set-id".into()), json!(true))
                })
            }),
        );

        let raw = r#"{"id":"request-id","method":"test.own_id"}"#;
        let resp_str = d.dispatch(raw).await;
        let resp = parse_response(&resp_str);

        assert_eq!(
            resp.id.as_deref(),
            Some("request-id"),
            "dispatch should set response id from request, not from handler"
        );
    }

    // ── Dispatch produces valid JSON ───────────────────────────────────

    #[tokio::test]
    async fn dispatch_output_is_always_valid_json() {
        let d = Dispatcher::new();

        let cases = vec![
            r#"{"id":"1","method":"system.ping"}"#,
            r#"{"method":"unknown.method"}"#,
            "not json",
            "",
            r#"{"id":"2","method":"system.ping","params":null}"#,
        ];

        for raw in cases {
            let resp_str = d.dispatch(raw).await;
            assert!(
                serde_json::from_str::<Value>(&resp_str).is_ok(),
                "dispatch output should be valid JSON for input: {raw:?}"
            );
        }
    }

    // ── Multiple dispatches ────────────────────────────────────────────

    #[tokio::test]
    async fn multiple_sequential_dispatches() {
        let mut d = Dispatcher::new();
        d.register(
            "counter.get",
            Arc::new(|_| {
                Box::pin(async { JsonRpcResponse::success(None, json!(42)) })
            }),
        );

        for i in 0..5 {
            let id = format!("seq-{i}");
            let raw = format!(r#"{{"id":"{id}","method":"counter.get"}}"#);
            let resp_str = d.dispatch(&raw).await;
            let resp = parse_response(&resp_str);
            assert!(resp.ok);
            assert_eq!(resp.id.as_deref(), Some(id.as_str()));
            assert_eq!(resp.result, Some(json!(42)));
        }
    }
}
