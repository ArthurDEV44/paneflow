// US-014: Core method handlers for workspace and surface management

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;
use uuid::Uuid;

use paneflow_core::split_tree::{Direction, SplitTree};
use paneflow_core::tab_manager::TabManager;
use paneflow_core::workspace::Workspace;

use crate::dispatcher::Dispatcher;
use crate::protocol::JsonRpcResponse;

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

/// Shared mutable state that all JSON-RPC handlers operate on.
pub struct AppState {
    /// Manages workspace (tab) list and selection.
    pub tab_manager: TabManager,
    /// Maps each workspace ID to its split-tree layout.
    pub split_trees: HashMap<Uuid, SplitTree>,
}

impl AppState {
    /// Create an empty application state.
    pub fn new() -> Self {
        Self {
            tab_manager: TabManager::new(),
            split_trees: HashMap::new(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Param helpers
// ---------------------------------------------------------------------------

/// Extract a string field from a JSON params object.
fn get_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Extract a UUID field from a JSON params object.
fn get_uuid(params: &Value, key: &str) -> Option<Uuid> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
}

/// Parse a direction string ("horizontal"/"vertical") into a `Direction`.
fn parse_direction(s: &str) -> Option<Direction> {
    match s.to_lowercase().as_str() {
        "horizontal" => Some(Direction::Horizontal),
        "vertical" => Some(Direction::Vertical),
        _ => None,
    }
}

/// Find the workspace that contains a given pane (surface) ID.
fn find_workspace_for_pane(state: &AppState, pane_id: Uuid) -> Option<Uuid> {
    for (ws_id, tree) in &state.split_trees {
        if tree.find_pane(pane_id).is_some() {
            return Some(*ws_id);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// All supported method names
// ---------------------------------------------------------------------------

const METHOD_LIST: &[&str] = &[
    "system.ping",
    "system.capabilities",
    "system.identify",
    "workspace.list",
    "workspace.create",
    "workspace.select",
    "workspace.close",
    "workspace.current",
    "surface.list",
    "surface.split",
    "surface.close",
    "surface.send_text",
    "surface.focus",
];

// ---------------------------------------------------------------------------
// Handler registration
// ---------------------------------------------------------------------------

/// Register all core method handlers on the given dispatcher.
///
/// The `state` is shared across all handlers via `Arc<Mutex<AppState>>`.
pub fn register_handlers(dispatcher: &mut Dispatcher, state: Arc<Mutex<AppState>>) {
    // ── system.capabilities ────────────────────────────────────────────
    {
        dispatcher.register(
            "system.capabilities",
            Arc::new(|_params| {
                Box::pin(async {
                    JsonRpcResponse::success(
                        None,
                        json!({
                            "version": "0.1.0",
                            "protocol": "v2",
                            "methods": METHOD_LIST,
                        }),
                    )
                })
            }),
        );
    }

    // ── system.identify ────────────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "system.identify",
            Arc::new(move |_params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let state = state.lock().await;
                    let workspace = state.tab_manager.selected();
                    match workspace {
                        Some(ws) => {
                            let first_pane = state
                                .split_trees
                                .get(&ws.id)
                                .map(|tree| tree.all_panes())
                                .and_then(|panes| panes.into_iter().next());
                            JsonRpcResponse::success(
                                None,
                                json!({
                                    "workspace_id": ws.id.to_string(),
                                    "pane_id": first_pane.map(|id| id.to_string()),
                                }),
                            )
                        }
                        None => JsonRpcResponse::success(
                            None,
                            json!({
                                "workspace_id": null,
                                "pane_id": null,
                            }),
                        ),
                    }
                })
            }),
        );
    }

    // ── workspace.list ─────────────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "workspace.list",
            Arc::new(move |_params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let state = state.lock().await;
                    let workspaces: Vec<Value> = state
                        .tab_manager
                        .workspaces()
                        .iter()
                        .map(|ws| {
                            json!({
                                "id": ws.id.to_string(),
                                "title": ws.display_title(),
                                "working_directory": ws.working_directory.to_string_lossy(),
                            })
                        })
                        .collect();
                    JsonRpcResponse::success(None, json!({ "workspaces": workspaces }))
                })
            }),
        );
    }

    // ── workspace.create ───────────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "workspace.create",
            Arc::new(move |params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let name = params
                        .as_ref()
                        .and_then(|p| get_str(p, "name"))
                        .unwrap_or_else(|| "workspace".to_string());
                    let cwd = params
                        .as_ref()
                        .and_then(|p| get_str(p, "cwd"))
                        .unwrap_or_else(|| {
                            std::env::current_dir()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|_| "/".to_string())
                        });

                    let mut state = state.lock().await;
                    let ws = Workspace::new(&name, &cwd);
                    let ws_id = ws.id;

                    // Create a root pane for the workspace's split tree.
                    let root_pane_id = Uuid::new_v4();
                    let tree = SplitTree::new(root_pane_id);

                    state.tab_manager.add_workspace(ws);
                    state.split_trees.insert(ws_id, tree);

                    JsonRpcResponse::success(
                        None,
                        json!({
                            "id": ws_id.to_string(),
                            "pane_id": root_pane_id.to_string(),
                        }),
                    )
                })
            }),
        );
    }

    // ── workspace.select ───────────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "workspace.select",
            Arc::new(move |params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let id = params.as_ref().and_then(|p| get_uuid(p, "id"));
                    let id = match id {
                        Some(id) => id,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "invalid_params",
                                "missing or invalid 'id' parameter",
                            );
                        }
                    };

                    let mut state = state.lock().await;
                    match state.tab_manager.select_workspace(id) {
                        Ok(()) => {
                            JsonRpcResponse::success(None, json!({ "ok": true }))
                        }
                        Err(_) => JsonRpcResponse::error(
                            None,
                            "not_found",
                            format!("workspace {id} not found"),
                        ),
                    }
                })
            }),
        );
    }

    // ── workspace.close ────────────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "workspace.close",
            Arc::new(move |params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let id = params.as_ref().and_then(|p| get_uuid(p, "id"));
                    let id = match id {
                        Some(id) => id,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "invalid_params",
                                "missing or invalid 'id' parameter",
                            );
                        }
                    };

                    let mut state = state.lock().await;
                    match state.tab_manager.close_workspace(id) {
                        Ok(()) => {
                            state.split_trees.remove(&id);
                            JsonRpcResponse::success(None, json!({ "ok": true }))
                        }
                        Err(e) => {
                            let code = if e.to_string().contains("last workspace") {
                                "protected"
                            } else {
                                "not_found"
                            };
                            JsonRpcResponse::error(None, code, e.to_string())
                        }
                    }
                })
            }),
        );
    }

    // ── workspace.current ──────────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "workspace.current",
            Arc::new(move |_params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let state = state.lock().await;
                    match state.tab_manager.selected() {
                        Some(ws) => JsonRpcResponse::success(
                            None,
                            json!({
                                "id": ws.id.to_string(),
                                "title": ws.display_title(),
                                "working_directory": ws.working_directory.to_string_lossy(),
                            }),
                        ),
                        None => {
                            JsonRpcResponse::error(None, "not_found", "no workspace selected")
                        }
                    }
                })
            }),
        );
    }

    // ── surface.list ───────────────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "surface.list",
            Arc::new(move |params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let state = state.lock().await;

                    // Determine which workspace to list surfaces for.
                    let ws_id = params
                        .as_ref()
                        .and_then(|p| get_uuid(p, "workspace_id"))
                        .or(state.tab_manager.selected_id);

                    let ws_id = match ws_id {
                        Some(id) => id,
                        None => {
                            return JsonRpcResponse::success(
                                None,
                                json!({ "surfaces": [] }),
                            );
                        }
                    };

                    // Verify workspace exists.
                    if state.tab_manager.get(ws_id).is_none() {
                        return JsonRpcResponse::error(
                            None,
                            "not_found",
                            format!("workspace {ws_id} not found"),
                        );
                    }

                    let panes = state
                        .split_trees
                        .get(&ws_id)
                        .map(|tree| tree.all_panes())
                        .unwrap_or_default();

                    let surfaces: Vec<Value> = panes
                        .iter()
                        .map(|id| json!({ "id": id.to_string() }))
                        .collect();

                    JsonRpcResponse::success(None, json!({ "surfaces": surfaces }))
                })
            }),
        );
    }

    // ── surface.split ──────────────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "surface.split",
            Arc::new(move |params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let params = match params {
                        Some(p) => p,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "invalid_params",
                                "missing parameters",
                            );
                        }
                    };

                    let surface_id = match get_uuid(&params, "surface_id") {
                        Some(id) => id,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "invalid_params",
                                "missing or invalid 'surface_id' parameter",
                            );
                        }
                    };

                    let direction_str = match get_str(&params, "direction") {
                        Some(d) => d,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "invalid_params",
                                "missing 'direction' parameter",
                            );
                        }
                    };

                    let direction = match parse_direction(&direction_str) {
                        Some(d) => d,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "invalid_params",
                                format!(
                                    "invalid direction '{}', expected 'horizontal' or 'vertical'",
                                    direction_str
                                ),
                            );
                        }
                    };

                    let mut state = state.lock().await;

                    let ws_id = match find_workspace_for_pane(&state, surface_id) {
                        Some(id) => id,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "not_found",
                                format!("surface {surface_id} not found"),
                            );
                        }
                    };

                    let tree = state.split_trees.get_mut(&ws_id).unwrap();
                    match tree.split(surface_id, direction) {
                        Ok(new_id) => JsonRpcResponse::success(
                            None,
                            json!({ "id": new_id.to_string() }),
                        ),
                        Err(e) => {
                            JsonRpcResponse::error(None, "not_found", e.to_string())
                        }
                    }
                })
            }),
        );
    }

    // ── surface.close ──────────────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "surface.close",
            Arc::new(move |params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let surface_id = params
                        .as_ref()
                        .and_then(|p| get_uuid(p, "surface_id"));
                    let surface_id = match surface_id {
                        Some(id) => id,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "invalid_params",
                                "missing or invalid 'surface_id' parameter",
                            );
                        }
                    };

                    let mut state = state.lock().await;

                    let ws_id = match find_workspace_for_pane(&state, surface_id) {
                        Some(id) => id,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "not_found",
                                format!("surface {surface_id} not found"),
                            );
                        }
                    };

                    let tree = state.split_trees.get_mut(&ws_id).unwrap();
                    match tree.close(surface_id) {
                        Ok(()) => {
                            JsonRpcResponse::success(None, json!({ "ok": true }))
                        }
                        Err(e) => {
                            let code = if e.to_string().contains("last pane") {
                                "protected"
                            } else {
                                "not_found"
                            };
                            JsonRpcResponse::error(None, code, e.to_string())
                        }
                    }
                })
            }),
        );
    }

    // ── surface.send_text (stub) ───────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "surface.send_text",
            Arc::new(move |params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let surface_id = params
                        .as_ref()
                        .and_then(|p| get_uuid(p, "surface_id"));
                    let surface_id = match surface_id {
                        Some(id) => id,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "invalid_params",
                                "missing or invalid 'surface_id' parameter",
                            );
                        }
                    };

                    let text = params
                        .as_ref()
                        .and_then(|p| get_str(p, "text"));
                    if text.is_none() {
                        return JsonRpcResponse::error(
                            None,
                            "invalid_params",
                            "missing 'text' parameter",
                        );
                    }

                    let state = state.lock().await;
                    if find_workspace_for_pane(&state, surface_id).is_none() {
                        return JsonRpcResponse::error(
                            None,
                            "not_found",
                            format!("surface {surface_id} not found"),
                        );
                    }

                    // Stub: actual PTY integration will come later.
                    JsonRpcResponse::success(None, json!({ "ok": true }))
                })
            }),
        );
    }

    // ── surface.focus (stub) ───────────────────────────────────────────
    {
        let state = Arc::clone(&state);
        dispatcher.register(
            "surface.focus",
            Arc::new(move |params| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let surface_id = params
                        .as_ref()
                        .and_then(|p| get_uuid(p, "surface_id"));
                    let surface_id = match surface_id {
                        Some(id) => id,
                        None => {
                            return JsonRpcResponse::error(
                                None,
                                "invalid_params",
                                "missing or invalid 'surface_id' parameter",
                            );
                        }
                    };

                    let state = state.lock().await;
                    if find_workspace_for_pane(&state, surface_id).is_none() {
                        return JsonRpcResponse::error(
                            None,
                            "not_found",
                            format!("surface {surface_id} not found"),
                        );
                    }

                    // Stub: actual focus tracking will come later.
                    JsonRpcResponse::success(None, json!({ "ok": true }))
                })
            }),
        );
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Helpers ────────────────────────────────────────────────────────

    /// Build a dispatcher with all handlers registered and a default workspace.
    async fn setup() -> (Dispatcher, Arc<Mutex<AppState>>) {
        let state = Arc::new(Mutex::new(AppState::new()));
        let mut dispatcher = Dispatcher::new();
        register_handlers(&mut dispatcher, Arc::clone(&state));
        (dispatcher, state)
    }

    /// Build a dispatcher with a pre-created workspace and return its id + root pane id.
    async fn setup_with_workspace() -> (Dispatcher, Arc<Mutex<AppState>>, Uuid, Uuid) {
        let (dispatcher, state) = setup().await;
        let resp = dispatch_json(
            &dispatcher,
            json!({"id":"s","method":"workspace.create","params":{"name":"test","cwd":"/tmp"}}),
        )
        .await;
        let ws_id = Uuid::parse_str(resp.result.as_ref().unwrap()["id"].as_str().unwrap()).unwrap();
        let pane_id =
            Uuid::parse_str(resp.result.as_ref().unwrap()["pane_id"].as_str().unwrap()).unwrap();
        (dispatcher, state, ws_id, pane_id)
    }

    /// Dispatch a JSON value as a request and parse the response.
    async fn dispatch_json(
        dispatcher: &Dispatcher,
        request: Value,
    ) -> crate::protocol::JsonRpcResponse {
        let raw = serde_json::to_string(&request).unwrap();
        let resp_str = dispatcher.dispatch(&raw).await;
        serde_json::from_str(&resp_str).unwrap()
    }

    // ── system.capabilities ────────────────────────────────────────────

    #[tokio::test]
    async fn capabilities_returns_version_and_methods() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"system.capabilities"}),
        )
        .await;

        assert!(resp.ok);
        let result = resp.result.unwrap();
        assert_eq!(result["version"], "0.1.0");
        assert_eq!(result["protocol"], "v2");

        let methods = result["methods"].as_array().unwrap();
        assert!(methods.contains(&json!("system.ping")));
        assert!(methods.contains(&json!("workspace.create")));
        assert!(methods.contains(&json!("surface.split")));
    }

    // ── system.identify ────────────────────────────────────────────────

    #[tokio::test]
    async fn identify_returns_null_when_no_workspace() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"system.identify"}),
        )
        .await;

        assert!(resp.ok);
        let result = resp.result.unwrap();
        assert!(result["workspace_id"].is_null());
        assert!(result["pane_id"].is_null());
    }

    #[tokio::test]
    async fn identify_returns_selected_workspace_and_pane() {
        let (d, _, ws_id, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"system.identify"}),
        )
        .await;

        assert!(resp.ok);
        let result = resp.result.unwrap();
        assert_eq!(result["workspace_id"], ws_id.to_string());
        assert_eq!(result["pane_id"], pane_id.to_string());
    }

    // ── workspace.list ─────────────────────────────────────────────────

    #[tokio::test]
    async fn list_empty_returns_empty_array() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.list"}),
        )
        .await;

        assert!(resp.ok);
        let result = resp.result.unwrap();
        assert_eq!(result["workspaces"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn list_returns_created_workspaces() {
        let (d, _) = setup().await;

        // Create two workspaces.
        dispatch_json(
            &d,
            json!({"id":"c1","method":"workspace.create","params":{"name":"alpha","cwd":"/tmp/a"}}),
        )
        .await;
        dispatch_json(
            &d,
            json!({"id":"c2","method":"workspace.create","params":{"name":"beta","cwd":"/tmp/b"}}),
        )
        .await;

        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.list"}),
        )
        .await;

        assert!(resp.ok);
        let workspaces = resp.result.unwrap()["workspaces"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0]["title"], "alpha");
        assert_eq!(workspaces[1]["title"], "beta");
    }

    // ── workspace.create ───────────────────────────────────────────────

    #[tokio::test]
    async fn create_workspace_returns_id_and_pane() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.create","params":{"name":"ws1","cwd":"/home"}}),
        )
        .await;

        assert!(resp.ok);
        let result = resp.result.unwrap();
        // Verify UUIDs are parseable.
        Uuid::parse_str(result["id"].as_str().unwrap()).unwrap();
        Uuid::parse_str(result["pane_id"].as_str().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn create_workspace_without_params_uses_defaults() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.create"}),
        )
        .await;

        assert!(resp.ok);
        let result = resp.result.unwrap();
        Uuid::parse_str(result["id"].as_str().unwrap()).unwrap();
    }

    // ── workspace.select ───────────────────────────────────────────────

    #[tokio::test]
    async fn select_workspace_succeeds() {
        let (d, _, ws_id, _) = setup_with_workspace().await;

        // Create a second workspace.
        let resp = dispatch_json(
            &d,
            json!({"id":"c","method":"workspace.create","params":{"name":"other"}}),
        )
        .await;
        let other_id = resp.result.unwrap()["id"].as_str().unwrap().to_string();

        // Select the first one.
        let resp = dispatch_json(
            &d,
            json!({"id":"s","method":"workspace.select","params":{"id": ws_id.to_string()}}),
        )
        .await;
        assert!(resp.ok);

        // Verify via workspace.current.
        let resp = dispatch_json(
            &d,
            json!({"id":"c","method":"workspace.current"}),
        )
        .await;
        assert_eq!(resp.result.unwrap()["id"], ws_id.to_string());

        // Verify we can select the other one too.
        let resp = dispatch_json(
            &d,
            json!({"id":"s2","method":"workspace.select","params":{"id": other_id}}),
        )
        .await;
        assert!(resp.ok);
    }

    #[tokio::test]
    async fn select_nonexistent_workspace_returns_not_found() {
        let (d, _) = setup().await;
        let bogus = Uuid::new_v4();
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.select","params":{"id": bogus.to_string()}}),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "not_found");
    }

    #[tokio::test]
    async fn select_without_id_returns_invalid_params() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.select","params":{}}),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "invalid_params");
    }

    // ── workspace.close ────────────────────────────────────────────────

    #[tokio::test]
    async fn close_workspace_succeeds() {
        let (d, _, ws_id, _) = setup_with_workspace().await;

        // Create a second workspace so the first can be closed.
        dispatch_json(
            &d,
            json!({"id":"c","method":"workspace.create","params":{"name":"other"}}),
        )
        .await;

        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.close","params":{"id": ws_id.to_string()}}),
        )
        .await;
        assert!(resp.ok);

        // Verify it was removed.
        let resp = dispatch_json(
            &d,
            json!({"id":"l","method":"workspace.list"}),
        )
        .await;
        let workspaces = resp.result.unwrap()["workspaces"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(workspaces.len(), 1);
        assert_ne!(workspaces[0]["id"], ws_id.to_string());
    }

    #[tokio::test]
    async fn close_last_workspace_returns_protected() {
        let (d, _, ws_id, _) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.close","params":{"id": ws_id.to_string()}}),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "protected");
    }

    #[tokio::test]
    async fn close_nonexistent_workspace_returns_not_found() {
        let (d, _) = setup().await;
        let bogus = Uuid::new_v4();
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.close","params":{"id": bogus.to_string()}}),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "not_found");
    }

    // ── workspace.current ──────────────────────────────────────────────

    #[tokio::test]
    async fn current_returns_selected_workspace() {
        let (d, _, ws_id, _) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.current"}),
        )
        .await;

        assert!(resp.ok);
        let result = resp.result.unwrap();
        assert_eq!(result["id"], ws_id.to_string());
        assert_eq!(result["title"], "test");
    }

    #[tokio::test]
    async fn current_returns_error_when_no_workspace() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.current"}),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "not_found");
    }

    // ── surface.list ───────────────────────────────────────────────────

    #[tokio::test]
    async fn surface_list_returns_panes() {
        let (d, _, ws_id, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"surface.list","params":{"workspace_id": ws_id.to_string()}}),
        )
        .await;

        assert!(resp.ok);
        let surfaces = resp.result.unwrap()["surfaces"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0]["id"], pane_id.to_string());
    }

    #[tokio::test]
    async fn surface_list_defaults_to_selected_workspace() {
        let (d, _, _, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"surface.list"}),
        )
        .await;

        assert!(resp.ok);
        let surfaces = resp.result.unwrap()["surfaces"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0]["id"], pane_id.to_string());
    }

    #[tokio::test]
    async fn surface_list_nonexistent_workspace_returns_not_found() {
        let (d, _) = setup().await;
        let bogus = Uuid::new_v4();
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"surface.list","params":{"workspace_id": bogus.to_string()}}),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "not_found");
    }

    // ── surface.split ──────────────────────────────────────────────────

    #[tokio::test]
    async fn surface_split_horizontal() {
        let (d, _, _, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.split",
                "params": {
                    "surface_id": pane_id.to_string(),
                    "direction": "horizontal"
                }
            }),
        )
        .await;

        assert!(resp.ok);
        let new_id = resp.result.unwrap()["id"].as_str().unwrap().to_string();
        Uuid::parse_str(&new_id).unwrap();

        // Surface list should now have 2 panes.
        let resp = dispatch_json(
            &d,
            json!({"id":"2","method":"surface.list"}),
        )
        .await;
        let surfaces = resp.result.unwrap()["surfaces"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(surfaces.len(), 2);
    }

    #[tokio::test]
    async fn surface_split_vertical() {
        let (d, _, _, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.split",
                "params": {
                    "surface_id": pane_id.to_string(),
                    "direction": "vertical"
                }
            }),
        )
        .await;

        assert!(resp.ok);
        Uuid::parse_str(resp.result.unwrap()["id"].as_str().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn surface_split_invalid_direction() {
        let (d, _, _, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.split",
                "params": {
                    "surface_id": pane_id.to_string(),
                    "direction": "diagonal"
                }
            }),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "invalid_params");
    }

    #[tokio::test]
    async fn surface_split_nonexistent_surface() {
        let (d, _) = setup().await;
        let bogus = Uuid::new_v4();
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.split",
                "params": {
                    "surface_id": bogus.to_string(),
                    "direction": "horizontal"
                }
            }),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "not_found");
    }

    #[tokio::test]
    async fn surface_split_missing_params() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"surface.split"}),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "invalid_params");
    }

    // ── surface.close ──────────────────────────────────────────────────

    #[tokio::test]
    async fn surface_close_succeeds() {
        let (d, _, _, pane_id) = setup_with_workspace().await;

        // Split to get a second pane.
        let resp = dispatch_json(
            &d,
            json!({
                "id": "s",
                "method": "surface.split",
                "params": {
                    "surface_id": pane_id.to_string(),
                    "direction": "horizontal"
                }
            }),
        )
        .await;
        let new_pane = resp.result.unwrap()["id"].as_str().unwrap().to_string();

        // Close the new pane.
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.close",
                "params": { "surface_id": new_pane }
            }),
        )
        .await;
        assert!(resp.ok);

        // Surface list should be back to 1.
        let resp = dispatch_json(
            &d,
            json!({"id":"l","method":"surface.list"}),
        )
        .await;
        let surfaces = resp.result.unwrap()["surfaces"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(surfaces.len(), 1);
    }

    #[tokio::test]
    async fn surface_close_last_pane_returns_protected() {
        let (d, _, _, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.close",
                "params": { "surface_id": pane_id.to_string() }
            }),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "protected");
    }

    #[tokio::test]
    async fn surface_close_nonexistent_returns_not_found() {
        let (d, _) = setup().await;
        let bogus = Uuid::new_v4();
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.close",
                "params": { "surface_id": bogus.to_string() }
            }),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "not_found");
    }

    // ── surface.send_text (stub) ───────────────────────────────────────

    #[tokio::test]
    async fn send_text_returns_ok() {
        let (d, _, _, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.send_text",
                "params": {
                    "surface_id": pane_id.to_string(),
                    "text": "echo hello\n"
                }
            }),
        )
        .await;

        assert!(resp.ok);
    }

    #[tokio::test]
    async fn send_text_missing_surface_id_returns_invalid_params() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.send_text",
                "params": { "text": "hello" }
            }),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "invalid_params");
    }

    #[tokio::test]
    async fn send_text_missing_text_returns_invalid_params() {
        let (d, _, _, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.send_text",
                "params": { "surface_id": pane_id.to_string() }
            }),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "invalid_params");
    }

    #[tokio::test]
    async fn send_text_nonexistent_surface_returns_not_found() {
        let (d, _) = setup().await;
        let bogus = Uuid::new_v4();
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.send_text",
                "params": {
                    "surface_id": bogus.to_string(),
                    "text": "hello"
                }
            }),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "not_found");
    }

    // ── surface.focus (stub) ───────────────────────────────────────────

    #[tokio::test]
    async fn focus_returns_ok() {
        let (d, _, _, pane_id) = setup_with_workspace().await;
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.focus",
                "params": { "surface_id": pane_id.to_string() }
            }),
        )
        .await;

        assert!(resp.ok);
    }

    #[tokio::test]
    async fn focus_nonexistent_surface_returns_not_found() {
        let (d, _) = setup().await;
        let bogus = Uuid::new_v4();
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.focus",
                "params": { "surface_id": bogus.to_string() }
            }),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "not_found");
    }

    #[tokio::test]
    async fn focus_missing_surface_id_returns_invalid_params() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({
                "id": "1",
                "method": "surface.focus",
                "params": {}
            }),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "invalid_params");
    }

    // ── Integration: full workflow ─────────────────────────────────────

    #[tokio::test]
    async fn full_workflow_create_split_close() {
        let (d, _) = setup().await;

        // 1. Create workspace.
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.create","params":{"name":"dev","cwd":"/home"}}),
        )
        .await;
        assert!(resp.ok);
        let ws_id = resp.result.as_ref().unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let root_pane = resp.result.as_ref().unwrap()["pane_id"]
            .as_str()
            .unwrap()
            .to_string();

        // 2. Current should return it.
        let resp = dispatch_json(
            &d,
            json!({"id":"2","method":"workspace.current"}),
        )
        .await;
        assert!(resp.ok);
        assert_eq!(resp.result.as_ref().unwrap()["id"], ws_id);

        // 3. Split horizontally.
        let resp = dispatch_json(
            &d,
            json!({
                "id": "3",
                "method": "surface.split",
                "params": { "surface_id": root_pane, "direction": "horizontal" }
            }),
        )
        .await;
        assert!(resp.ok);
        let pane_b = resp.result.as_ref().unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        // 4. Split the new pane vertically.
        let resp = dispatch_json(
            &d,
            json!({
                "id": "4",
                "method": "surface.split",
                "params": { "surface_id": pane_b, "direction": "vertical" }
            }),
        )
        .await;
        assert!(resp.ok);
        let pane_c = resp.result.as_ref().unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        // 5. List surfaces: should have 3.
        let resp = dispatch_json(
            &d,
            json!({"id":"5","method":"surface.list"}),
        )
        .await;
        assert_eq!(
            resp.result.unwrap()["surfaces"].as_array().unwrap().len(),
            3
        );

        // 6. Send text to a pane (stub).
        let resp = dispatch_json(
            &d,
            json!({
                "id": "6",
                "method": "surface.send_text",
                "params": { "surface_id": pane_c, "text": "ls -la\n" }
            }),
        )
        .await;
        assert!(resp.ok);

        // 7. Focus a pane (stub).
        let resp = dispatch_json(
            &d,
            json!({
                "id": "7",
                "method": "surface.focus",
                "params": { "surface_id": root_pane }
            }),
        )
        .await;
        assert!(resp.ok);

        // 8. Close pane_c.
        let resp = dispatch_json(
            &d,
            json!({
                "id": "8",
                "method": "surface.close",
                "params": { "surface_id": pane_c }
            }),
        )
        .await;
        assert!(resp.ok);

        // 9. Surfaces should be back to 2.
        let resp = dispatch_json(
            &d,
            json!({"id":"9","method":"surface.list"}),
        )
        .await;
        assert_eq!(
            resp.result.unwrap()["surfaces"].as_array().unwrap().len(),
            2
        );

        // 10. Create second workspace, close the first.
        dispatch_json(
            &d,
            json!({"id":"10","method":"workspace.create","params":{"name":"other"}}),
        )
        .await;
        let resp = dispatch_json(
            &d,
            json!({"id":"11","method":"workspace.close","params":{"id": ws_id}}),
        )
        .await;
        assert!(resp.ok);

        // 11. Only one workspace remains.
        let resp = dispatch_json(
            &d,
            json!({"id":"12","method":"workspace.list"}),
        )
        .await;
        assert_eq!(
            resp.result.unwrap()["workspaces"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    // ── Edge cases ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn workspace_close_removes_split_tree() {
        let (d, state, ws_id, _) = setup_with_workspace().await;

        // Create a second workspace so the first can be closed.
        dispatch_json(
            &d,
            json!({"id":"c","method":"workspace.create","params":{"name":"other"}}),
        )
        .await;

        // Close first workspace.
        dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.close","params":{"id": ws_id.to_string()}}),
        )
        .await;

        // Verify split tree was cleaned up.
        let state = state.lock().await;
        assert!(!state.split_trees.contains_key(&ws_id));
    }

    #[tokio::test]
    async fn workspace_close_without_id_returns_invalid_params() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"workspace.close","params":{}}),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "invalid_params");
    }

    #[tokio::test]
    async fn surface_close_without_surface_id_returns_invalid_params() {
        let (d, _) = setup().await;
        let resp = dispatch_json(
            &d,
            json!({"id":"1","method":"surface.close","params":{}}),
        )
        .await;

        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "invalid_params");
    }
}
