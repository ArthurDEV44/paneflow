//! Config file I/O: read-modify-write helpers for `paneflow.json`.
//!
//! All functions operate on raw JSON to preserve unknown fields and formatting.

use std::collections::BTreeSet;
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
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(value) {
        Ok(json_str) => {
            if let Err(e) = std::fs::write(path, json_str) {
                log::warn!("config: failed to write: {e}");
            }
        }
        Err(e) => log::warn!("config: failed to serialize: {e}"),
    }
}

/// Save a top-level config field (e.g. `"font_size"`, `"line_height"`).
pub fn save_config_value(key: &str, value: serde_json::Value) {
    let Some(path) = paneflow_config::loader::config_path() else {
        log::warn!("config: cannot determine config path, not saving");
        return;
    };
    let mut json = load_raw_config(&path);
    if let Some(root) = json.as_object_mut() {
        if value.is_null() {
            root.remove(key);
        } else {
            root.insert(key.to_string(), value);
        }
    }
    write_config(&path, &json);
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

    let root = json.as_object_mut().expect("config root must be an object");
    if !root.contains_key("shortcuts") || !root.get("shortcuts").unwrap().is_object() {
        root.insert("shortcuts".to_string(), serde_json::json!({}));
    }
    let shortcuts_obj = root.get_mut("shortcuts").unwrap().as_object_mut().unwrap();

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

/// Enumerate monospace font families installed on the system via `fontconfig`.
///
/// Falls back to an empty list if `fc-list` is not available.
pub fn load_mono_fonts() -> Vec<String> {
    let output = match std::process::Command::new("fc-list")
        .args([":spacing=mono", "family"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("config: fc-list failed: {e}");
            return Vec::new();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut families = BTreeSet::new();

    for line in stdout.lines() {
        // fc-list may output "Family1,Family2" on a single line
        for part in line.split(',') {
            let name = part.trim();
            if !name.is_empty() {
                families.insert(name.to_string());
            }
        }
    }

    families.into_iter().collect()
}
