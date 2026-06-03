//! Config file I/O: read-modify-write helpers for `paneflow.json`.
//!
//! All functions operate on raw JSON to preserve unknown fields and formatting.

use std::path::PathBuf;

/// Load the raw JSON config, or an empty object if missing/invalid.
fn load_raw_config(path: &PathBuf) -> serde_json::Value {
    if path.exists() {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                serde_json::from_str(&contents).unwrap_or_else(|_| serde_json::json!({}))
            }
            Err(_) => serde_json::json!({}),
        }
    } else {
        serde_json::json!({})
    }
}

/// Write a JSON value back to the config file, creating parent dirs if needed.
fn write_config(path: &PathBuf, value: &serde_json::Value) {
    let _ = write_config_checked(path, value);
}

/// Result-returning variant of [`write_config`]. Returns `true` on
/// successful write, `false` otherwise (serialization or I/O error —
/// logged at WARN in both cases).
fn write_config_checked(path: &PathBuf, value: &serde_json::Value) -> bool {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(value) {
        Ok(json_str) => match std::fs::write(path, json_str) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("config: failed to write: {e}");
                false
            }
        },
        Err(e) => {
            log::warn!("config: failed to serialize: {e}");
            false
        }
    }
}

/// Save a top-level config field (e.g. `"font_size"`, `"line_height"`).
pub fn save_config_value(key: &str, value: serde_json::Value) {
    let _ = save_config_value_checked(key, value);
}

/// Same as `save_config_value`, but returns `true` on success and `false`
/// when the config path could not be resolved or the file write failed.
///
/// Callers that need to surface persistence failures to the user (e.g. the
/// telemetry consent modal in US-011, which must honor the choice
/// in-memory and show a toast when the disk write fails) should use this
/// variant. The void `save_config_value` wrapper above is kept for
/// fire-and-forget call sites that already accept best-effort writes.
pub fn save_config_value_checked(key: &str, value: serde_json::Value) -> bool {
    let Some(path) = paneflow_config::loader::config_path() else {
        log::warn!("config: cannot determine config path, not saving");
        return false;
    };
    let mut json = load_raw_config(&path);
    if let Some(root) = json.as_object_mut() {
        if value.is_null() {
            root.remove(key);
        } else {
            root.insert(key.to_string(), value);
        }
    }
    write_config_checked(&path, &json)
}

/// Save a single shortcut override to `paneflow.json`.
///
/// Merges the new binding into `shortcuts`, removing any previous key for the same action.
pub fn save_shortcut(new_key: &str, action_name: &str) {
    let Some(path) = paneflow_config::loader::config_path() else {
        log::warn!("config: cannot determine config path, not saving");
        return;
    };
    let mut json = load_raw_config(&path);

    // A user's paneflow.json can be valid JSON but not an object (`[]`, `"x"`,
    // `42`); `load_raw_config` returns it verbatim, so guard rather than
    // `.expect()` (which would panic on the UI thread). Mirrors the graceful
    // `if let Some(root)` idiom used by every other writer in this file.
    let Some(root) = json.as_object_mut() else {
        log::warn!("config: root is not a JSON object, not saving shortcut");
        return;
    };
    // Ensure `shortcuts` exists and is an object (replace a non-object).
    let shortcuts = root
        .entry("shortcuts")
        .or_insert_with(|| serde_json::json!({}));
    if !shortcuts.is_object() {
        *shortcuts = serde_json::json!({});
    }
    let Some(shortcuts_obj) = shortcuts.as_object_mut() else {
        return;
    };

    // Remove any existing binding for this action (avoid duplicate keys for same action)
    let keys_to_remove: Vec<String> = shortcuts_obj
        .iter()
        .filter(|(_, v)| v.as_str() == Some(action_name))
        .map(|(k, _)| k.clone())
        .collect();
    for k in keys_to_remove {
        shortcuts_obj.remove(&k);
    }

    shortcuts_obj.insert(
        new_key.to_string(),
        serde_json::Value::String(action_name.to_string()),
    );

    write_config(&path, &json);
}

/// Remove all user shortcut overrides from `paneflow.json`, restoring defaults.
pub fn reset_shortcuts() {
    let Some(path) = paneflow_config::loader::config_path() else {
        return;
    };
    let mut json = load_raw_config(&path);
    if let Some(root) = json.as_object_mut() {
        root.remove("shortcuts");
    }
    write_config(&path, &json);
}

/// Pure read-modify-write of the `"terminal"` sub-object. Extracted from
/// [`save_terminal_field`] so the nesting/removal semantics can be unit-tested
/// without resolving (or touching) the real config path.
fn apply_terminal_field(json: &mut serde_json::Value, key: &str, value: serde_json::Value) {
    let Some(root) = json.as_object_mut() else {
        return;
    };
    // Ensure `terminal` exists and is an object (replace a non-object).
    let terminal = root
        .entry("terminal")
        .or_insert_with(|| serde_json::json!({}));
    if !terminal.is_object() {
        *terminal = serde_json::json!({});
    }
    if let Some(obj) = terminal.as_object_mut() {
        if value.is_null() {
            obj.remove(key);
        } else {
            obj.insert(key.to_string(), value);
        }
    }
}

/// Save a single field inside the `"terminal": { ... }` block in `paneflow.json`
/// (US-016 Terminal settings tab). A `Null` value removes the key (restoring
/// the schema default on next load); the `"terminal"` object itself is left in
/// place (an empty block is harmless — `#[serde(default)]` handles it).
pub fn save_terminal_field(key: &str, value: serde_json::Value) {
    let Some(path) = paneflow_config::loader::config_path() else {
        log::warn!("config: cannot determine config path, not saving");
        return;
    };
    let mut json = load_raw_config(&path);
    apply_terminal_field(&mut json, key, value);
    write_config(&path, &json);
}

#[cfg(test)]
mod tests {
    use super::apply_terminal_field;
    use serde_json::{Value, json};

    #[test]
    fn upserts_into_terminal_block_creating_it() {
        let mut j = json!({});
        apply_terminal_field(&mut j, "ligatures", json!(true));
        assert_eq!(j["terminal"]["ligatures"], json!(true));
    }

    #[test]
    fn preserves_other_terminal_keys() {
        let mut j = json!({"terminal": {"bell": "audible"}});
        apply_terminal_field(&mut j, "cursor_shape", json!("beam"));
        assert_eq!(j["terminal"]["bell"], json!("audible"));
        assert_eq!(j["terminal"]["cursor_shape"], json!("beam"));
    }

    #[test]
    fn null_removes_key_but_keeps_block() {
        let mut j = json!({"terminal": {"bell": "audible", "ligatures": true}});
        apply_terminal_field(&mut j, "bell", Value::Null);
        assert!(j["terminal"].get("bell").is_none());
        assert_eq!(j["terminal"]["ligatures"], json!(true));
        assert!(j["terminal"].is_object());
    }

    #[test]
    fn replaces_non_object_terminal_value() {
        let mut j = json!({"terminal": "garbage"});
        apply_terminal_field(&mut j, "bell", json!("off"));
        assert_eq!(j["terminal"]["bell"], json!("off"));
    }

    #[test]
    fn leaves_top_level_keys_untouched() {
        let mut j = json!({"theme": "One Dark", "font_size": 14.0});
        apply_terminal_field(&mut j, "scrollback_lines", json!(5000));
        assert_eq!(j["theme"], json!("One Dark"));
        assert_eq!(j["font_size"], json!(14.0));
        assert_eq!(j["terminal"]["scrollback_lines"], json!(5000));
    }
}
