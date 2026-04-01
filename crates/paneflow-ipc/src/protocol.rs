// Shared JSON-RPC protocol types

use serde::{Deserialize, Serialize};

/// A JSON-RPC-style request sent by a client over the socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// Optional request identifier (omitted for notifications).
    #[serde(default)]
    pub id: Option<String>,
    /// The method name to invoke (e.g. "pane.create", "session.list").
    pub method: String,
    /// Optional parameters for the method.
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// A JSON-RPC-style response returned by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// Echoed request identifier.
    #[serde(default)]
    pub id: Option<String>,
    /// Whether the call succeeded.
    pub ok: bool,
    /// The result payload on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// The error payload on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// Describes a JSON-RPC error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Machine-readable error code (e.g. "NOT_FOUND", "INVALID_PARAMS").
    pub code: String,
    /// Human-readable description.
    pub message: String,
}

impl JsonRpcResponse {
    /// Build a successful response.
    pub fn success(id: Option<String>, result: serde_json::Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Build an error response.
    pub fn error(id: Option<String>, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(JsonRpcError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_request() {
        let req = JsonRpcRequest {
            id: Some("1".into()),
            method: "pane.create".into(),
            params: Some(serde_json::json!({"name": "main"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"method\":\"pane.create\""));
    }

    #[test]
    fn deserialize_request_without_optional_fields() {
        let json = r#"{"method":"session.list"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "session.list");
        assert!(req.id.is_none());
        assert!(req.params.is_none());
    }

    #[test]
    fn success_response_omits_error() {
        let resp = JsonRpcResponse::success(Some("1".into()), serde_json::json!({"count": 3}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"ok\":true"));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn error_response_omits_result() {
        let resp = JsonRpcResponse::error(Some("2".into()), "NOT_FOUND", "no such pane");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"ok\":false"));
        assert!(json.contains("\"NOT_FOUND\""));
        assert!(!json.contains("\"result\""));
    }

    #[test]
    fn roundtrip_request() {
        let req = JsonRpcRequest {
            id: Some("42".into()),
            method: "tab.close".into(),
            params: Some(serde_json::json!({"tab_id": "abc"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: JsonRpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, req.id);
        assert_eq!(decoded.method, req.method);
    }

    #[test]
    fn roundtrip_response() {
        let resp = JsonRpcResponse::error(None, "INTERNAL", "boom");
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: JsonRpcResponse = serde_json::from_str(&json).unwrap();
        assert!(!decoded.ok);
        assert!(decoded.error.is_some());
        assert_eq!(decoded.error.unwrap().code, "INTERNAL");
    }
}
