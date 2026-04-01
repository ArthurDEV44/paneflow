use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::warn;

/// Actions that can be triggered by keyboard shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    NewWorkspace,
    CloseWorkspace,
    SplitRight,
    SplitDown,
    NextWorkspace,
    PrevWorkspace,
    FocusPaneLeft,
    FocusPaneDown,
    FocusPaneUp,
    FocusPaneRight,
    ClosePane,
}

/// Keyboard modifier flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    /// No modifiers pressed.
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
    };
}

/// A keyboard shortcut: a combination of modifier keys and a primary key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Shortcut {
    pub modifiers: Modifiers,
    /// Lowercase key name (e.g. `"t"`, `"tab"`, `"enter"`).
    pub key: String,
}

impl Shortcut {
    /// Create a new shortcut from modifiers and a key name.
    pub fn new(modifiers: Modifiers, key: impl Into<String>) -> Self {
        Self {
            modifiers,
            key: key.into().to_lowercase(),
        }
    }

    /// Parse a shortcut string like `"ctrl+shift+t"`.
    ///
    /// Modifier tokens (`ctrl`, `shift`, `alt`, `meta`) are extracted in any
    /// order; the remaining non-modifier token becomes the key. Returns `None`
    /// if the string contains no key or is otherwise invalid.
    pub fn parse(s: &str) -> Option<Self> {
        let mut modifiers = Modifiers::NONE;
        let mut key: Option<String> = None;

        for token in s.split('+') {
            let token = token.trim().to_lowercase();
            if token.is_empty() {
                continue;
            }
            match token.as_str() {
                "ctrl" => modifiers.ctrl = true,
                "shift" => modifiers.shift = true,
                "alt" => modifiers.alt = true,
                "meta" => modifiers.meta = true,
                _ => {
                    if key.is_some() {
                        // More than one non-modifier token is ambiguous.
                        return None;
                    }
                    key = Some(token);
                }
            }
        }

        key.map(|k| Shortcut::new(modifiers, k))
    }
}

/// Error returned when parsing a shortcut config entry fails.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ShortcutError {
    #[error("invalid shortcut string: {0:?}")]
    InvalidShortcutString(String),
    #[error("unknown action name: {0:?}")]
    UnknownAction(String),
}

/// Maps keyboard shortcuts to actions.
///
/// Use [`ShortcutRegistry::default()`] for the built-in key bindings, or
/// [`ShortcutRegistry::from_config`] to overlay user overrides on top of
/// defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutRegistry {
    #[serde(with = "bindings_serde")]
    bindings: HashMap<Shortcut, Action>,
}

/// Custom serde for `HashMap<Shortcut, Action>` — serialized as a vec of pairs
/// because JSON map keys must be strings.
mod bindings_serde {
    use super::*;
    use serde::de::Deserializer;
    use serde::ser::Serializer;

    pub fn serialize<S: Serializer>(
        map: &HashMap<Shortcut, Action>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let pairs: Vec<(&Shortcut, &Action)> = map.iter().collect();
        pairs.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HashMap<Shortcut, Action>, D::Error> {
        let pairs: Vec<(Shortcut, Action)> = Vec::deserialize(deserializer)?;
        Ok(pairs.into_iter().collect())
    }
}

impl ShortcutRegistry {
    /// Look up the action bound to a key press, if any.
    pub fn lookup(&self, modifiers: Modifiers, key: &str) -> Option<Action> {
        let shortcut = Shortcut::new(modifiers, key);
        self.bindings.get(&shortcut).copied()
    }

    /// Build a registry from default shortcuts with user-provided config
    /// overrides applied on top.
    ///
    /// `shortcuts` maps action names (e.g. `"NewWorkspace"`) to shortcut
    /// strings (e.g. `"ctrl+shift+n"`). Invalid entries are logged as
    /// warnings and skipped.
    pub fn from_config(shortcuts: &HashMap<String, String>) -> Self {
        let mut registry = Self::default();

        for (action_name, shortcut_str) in shortcuts {
            let action = match parse_action_name(action_name) {
                Some(a) => a,
                None => {
                    warn!(
                        action = %action_name,
                        "ignoring unknown action name in shortcut config"
                    );
                    continue;
                }
            };

            let shortcut = match Shortcut::parse(shortcut_str) {
                Some(s) => s,
                None => {
                    warn!(
                        shortcut = %shortcut_str,
                        action = %action_name,
                        "ignoring invalid shortcut string in config"
                    );
                    continue;
                }
            };

            // Remove any previous binding that pointed to the same action so
            // one action never maps to two shortcuts simultaneously.
            registry.bindings.retain(|_, v| *v != action);
            registry.bindings.insert(shortcut, action);
        }

        registry
    }

    /// Return a reference to the inner bindings map.
    pub fn bindings(&self) -> &HashMap<Shortcut, Action> {
        &self.bindings
    }
}

impl Default for ShortcutRegistry {
    fn default() -> Self {
        let ctrl_shift = |key: &str| {
            Shortcut::new(
                Modifiers {
                    ctrl: true,
                    shift: true,
                    alt: false,
                    meta: false,
                },
                key,
            )
        };

        let bindings = HashMap::from([
            (ctrl_shift("t"), Action::NewWorkspace),
            (ctrl_shift("w"), Action::CloseWorkspace),
            (ctrl_shift("d"), Action::SplitRight),
            (ctrl_shift("e"), Action::SplitDown),
            (
                Shortcut::new(
                    Modifiers {
                        ctrl: true,
                        shift: false,
                        alt: false,
                        meta: false,
                    },
                    "tab",
                ),
                Action::NextWorkspace,
            ),
            (ctrl_shift("tab"), Action::PrevWorkspace),
            (ctrl_shift("h"), Action::FocusPaneLeft),
            (ctrl_shift("j"), Action::FocusPaneDown),
            (ctrl_shift("k"), Action::FocusPaneUp),
            (ctrl_shift("l"), Action::FocusPaneRight),
        ]);

        Self { bindings }
    }
}

/// Map an action name string (case-insensitive) to an `Action` variant.
fn parse_action_name(name: &str) -> Option<Action> {
    match name.to_lowercase().as_str() {
        "newworkspace" => Some(Action::NewWorkspace),
        "closeworkspace" => Some(Action::CloseWorkspace),
        "splitright" => Some(Action::SplitRight),
        "splitdown" => Some(Action::SplitDown),
        "nextworkspace" => Some(Action::NextWorkspace),
        "prevworkspace" => Some(Action::PrevWorkspace),
        "focuspaneleft" => Some(Action::FocusPaneLeft),
        "focuspanedown" => Some(Action::FocusPaneDown),
        "focuspaneup" => Some(Action::FocusPaneUp),
        "focuspaneright" => Some(Action::FocusPaneRight),
        "closepane" => Some(Action::ClosePane),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Shortcut parsing ────────────────────────────────────────────

    #[test]
    fn parse_simple_shortcut() {
        let s = Shortcut::parse("ctrl+shift+t").unwrap();
        assert!(s.modifiers.ctrl);
        assert!(s.modifiers.shift);
        assert!(!s.modifiers.alt);
        assert!(!s.modifiers.meta);
        assert_eq!(s.key, "t");
    }

    #[test]
    fn parse_single_modifier() {
        let s = Shortcut::parse("ctrl+tab").unwrap();
        assert!(s.modifiers.ctrl);
        assert!(!s.modifiers.shift);
        assert_eq!(s.key, "tab");
    }

    #[test]
    fn parse_all_modifiers() {
        let s = Shortcut::parse("ctrl+shift+alt+meta+x").unwrap();
        assert!(s.modifiers.ctrl);
        assert!(s.modifiers.shift);
        assert!(s.modifiers.alt);
        assert!(s.modifiers.meta);
        assert_eq!(s.key, "x");
    }

    #[test]
    fn parse_case_insensitive() {
        let s = Shortcut::parse("Ctrl+Shift+T").unwrap();
        assert!(s.modifiers.ctrl);
        assert!(s.modifiers.shift);
        assert_eq!(s.key, "t");
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(Shortcut::parse("").is_none());
    }

    #[test]
    fn parse_rejects_modifiers_only() {
        assert!(Shortcut::parse("ctrl+shift").is_none());
    }

    #[test]
    fn parse_rejects_multiple_keys() {
        assert!(Shortcut::parse("ctrl+a+b").is_none());
    }

    // ── Default registry ────────────────────────────────────────────

    #[test]
    fn default_new_workspace() {
        let reg = ShortcutRegistry::default();
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "t"), Some(Action::NewWorkspace));
    }

    #[test]
    fn default_close_workspace() {
        let reg = ShortcutRegistry::default();
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "w"), Some(Action::CloseWorkspace));
    }

    #[test]
    fn default_split_right() {
        let reg = ShortcutRegistry::default();
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "d"), Some(Action::SplitRight));
    }

    #[test]
    fn default_split_down() {
        let reg = ShortcutRegistry::default();
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "e"), Some(Action::SplitDown));
    }

    #[test]
    fn default_next_workspace() {
        let reg = ShortcutRegistry::default();
        let mods = Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "tab"), Some(Action::NextWorkspace));
    }

    #[test]
    fn default_prev_workspace() {
        let reg = ShortcutRegistry::default();
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "tab"), Some(Action::PrevWorkspace));
    }

    #[test]
    fn default_focus_pane_directions() {
        let reg = ShortcutRegistry::default();
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "h"), Some(Action::FocusPaneLeft));
        assert_eq!(reg.lookup(mods, "j"), Some(Action::FocusPaneDown));
        assert_eq!(reg.lookup(mods, "k"), Some(Action::FocusPaneUp));
        assert_eq!(reg.lookup(mods, "l"), Some(Action::FocusPaneRight));
    }

    #[test]
    fn lookup_returns_none_for_unbound_key() {
        let reg = ShortcutRegistry::default();
        assert_eq!(reg.lookup(Modifiers::NONE, "z"), None);
    }

    // ── Config overrides ────────────────────────────────────────────

    #[test]
    fn config_override_replaces_default() {
        let mut config = HashMap::new();
        config.insert("NewWorkspace".to_string(), "ctrl+n".to_string());

        let reg = ShortcutRegistry::from_config(&config);

        // New binding works.
        let mods = Modifiers {
            ctrl: true,
            shift: false,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "n"), Some(Action::NewWorkspace));

        // Old default binding is removed.
        let old_mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(old_mods, "t"), None);
    }

    #[test]
    fn config_override_keeps_other_defaults() {
        let mut config = HashMap::new();
        config.insert("NewWorkspace".to_string(), "ctrl+n".to_string());

        let reg = ShortcutRegistry::from_config(&config);

        // CloseWorkspace should still be at its default.
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "w"), Some(Action::CloseWorkspace));
    }

    #[test]
    fn config_ignores_unknown_action() {
        let mut config = HashMap::new();
        config.insert("DoSomethingWeird".to_string(), "ctrl+z".to_string());

        // Should not panic; the unknown action is silently skipped.
        let reg = ShortcutRegistry::from_config(&config);

        // Defaults remain intact.
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "t"), Some(Action::NewWorkspace));
    }

    #[test]
    fn config_ignores_invalid_shortcut_string() {
        let mut config = HashMap::new();
        config.insert("NewWorkspace".to_string(), "ctrl+shift".to_string());

        // Invalid shortcut (no key) is skipped; default remains.
        let reg = ShortcutRegistry::from_config(&config);

        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "t"), Some(Action::NewWorkspace));
    }

    #[test]
    fn config_action_name_is_case_insensitive() {
        let mut config = HashMap::new();
        config.insert("closepane".to_string(), "alt+w".to_string());

        let reg = ShortcutRegistry::from_config(&config);

        let mods = Modifiers {
            ctrl: false,
            shift: false,
            alt: true,
            meta: false,
        };
        assert_eq!(reg.lookup(mods, "w"), Some(Action::ClosePane));
    }

    // ── Serialization round-trip ────────────────────────────────────

    #[test]
    fn shortcut_serialization_roundtrip() {
        let original = Shortcut::parse("ctrl+shift+t").unwrap();
        let json = serde_json::to_string(&original).unwrap();
        let restored: Shortcut = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn action_serialization_roundtrip() {
        let action = Action::SplitRight;
        let json = serde_json::to_string(&action).unwrap();
        let restored: Action = serde_json::from_str(&json).unwrap();
        assert_eq!(action, restored);
    }

    #[test]
    fn registry_serialization_roundtrip() {
        let original = ShortcutRegistry::default();
        let json = serde_json::to_string(&original).unwrap();
        let restored: ShortcutRegistry = serde_json::from_str(&json).unwrap();

        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(restored.lookup(mods, "t"), original.lookup(mods, "t"));
    }
}
