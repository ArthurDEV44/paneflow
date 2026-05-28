//! US-116 (prd-agent-ui-refactor-2026-Q3.md): cross-platform OS
//! notifications fired when an agent turn ends, refuses, or errors out
//! while the Paneflow panel is not the user-focused surface.
//!
//! Three triggers feed this module from [`crate::agents::composer`]:
//! - [`on_turn_ended`] — every `RuntimeEvent::TurnEnded`. Body text
//!   depends on the `StopReasonKind` (success / refusal) and on
//!   whether any tools ran during the turn (AC #3 / #4).
//! - [`on_fatal`] — every `RuntimeEvent::Fatal`. Maps 1:1 to Zed's
//!   "Agent stopped due to an error" notification (AC #5).
//!
//! All decisions land here in a single place — gate, body builder,
//! `notify-rust` dispatch — so the composer side stays at one line per
//! call.
//!
//! ## Visibility model
//!
//! Notifications only fire when the Paneflow window is NOT
//! visible+focused (AC #6). We track two booleans via
//! [`AtomicBool`]s updated by:
//! - [`set_window_active`] — from
//!   `cx.observe_window_activation` in `main.rs`.
//! - [`set_agents_panel_visible`] — from the app-mode switcher in
//!   `app::agents_view_actions::toggle_mode` (and the bootstrap
//!   initial-mode hydrate).
//!
//! When both are `true`, the panel is "front-and-center" so the
//! notification is suppressed. Any other combination fires.
//!
//! ## Failure handling
//!
//! `notify-rust` returns an error when the underlying notification
//! daemon is unavailable (Linux without freedesktop daemon, macOS
//! without notarized .app, Windows without WinRT support). The error
//! is logged at `warn!` and swallowed -- never panics, never bubbles
//! up to the GPUI render path (AC #8 unhappy path).

use std::sync::atomic::{AtomicBool, Ordering};

use paneflow_config::schema::NotifyWhenAgentWaiting;

use super::panel_config::active_notify_when_agent_waiting;
use super::runtime::StopReasonKind;

/// Window-active gate updated by `cx.observe_window_activation`.
/// `true` while the OS reports the Paneflow window as the focused
/// one. Defaults to `true` so an early startup notification (before
/// the observer fires the first activation tick) is suppressed --
/// the user is staring at the window we just opened.
static WINDOW_ACTIVE: AtomicBool = AtomicBool::new(true);

/// Agents-panel gate updated by `app::agents_view_actions`. `true`
/// while the user is in `AppMode::Agents`. Defaults to `false` because
/// the bootstrap starts in `AppMode::Cli` (the persisted session
/// override flips it before the first runtime tick).
static AGENTS_PANEL_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Update the window-active flag. Called from
/// `cx.observe_window_activation` and from the initial activation
/// tick that GPUI fires when the observer registers.
pub fn set_window_active(active: bool) {
    WINDOW_ACTIVE.store(active, Ordering::Relaxed);
}

/// Update the agents-panel-visible flag. Called from the mode toggle
/// and from the bootstrap when the persisted session restores into
/// agents mode.
pub fn set_agents_panel_visible(visible: bool) {
    AGENTS_PANEL_VISIBLE.store(visible, Ordering::Relaxed);
}

/// Whether the panel is currently "front-and-center" -- i.e. the
/// window is the OS-focused surface AND the user is in agents mode.
/// `false` in every other combination (other window focused, CLI mode
/// active, ...). Used by the helpers below to decide whether to fire.
fn panel_is_focused() -> bool {
    panel_is_focused_pure(
        WINDOW_ACTIVE.load(Ordering::Relaxed),
        AGENTS_PANEL_VISIBLE.load(Ordering::Relaxed),
    )
}

/// Pure helper exposed for unit testing -- the atomics are
/// process-global and would race across parallel tests, so the gate
/// itself lives in a side-effect-free function the tests can call
/// with explicit values.
fn panel_is_focused_pure(window_active: bool, agents_visible: bool) -> bool {
    window_active && agents_visible
}

/// Whether the user's preference allows a fire at all. `Never` blocks
/// everything; `PrimaryScreen` and `AllScreens` both pass through to
/// `notify-rust` (the per-screen filter is OS-managed; we surface the
/// toggle for parity).
fn user_opted_in() -> bool {
    !matches!(
        active_notify_when_agent_waiting(),
        NotifyWhenAgentWaiting::Never,
    )
}

/// US-116 AC #3 / #4: surface a turn-end notification.
///
/// - `reason` — categorises the body text. `EndTurn` reads "Finished
///   running tools" (when `ran_tools`) or "New message" (when not).
///   `Refusal` reads "{model} refused to respond to this request".
///   `Other` (Cancelled / MaxTokens / future variants) is a no-op --
///   user-initiated cancels should not surface a desktop toast.
/// - `ran_tools` — gates EndTurn between the "tools" / "message" body.
///   Sourced from the composer's per-turn tool counter.
/// - `model_label` — fed to the Refusal body when the model id is
///   known. Falls back to "Agent" when `None`.
pub fn on_turn_ended(reason: StopReasonKind, ran_tools: bool, model_label: Option<&str>) {
    if !user_opted_in() || panel_is_focused() {
        return;
    }
    let Some(body) = turn_ended_body(reason, ran_tools, model_label) else {
        return;
    };
    fire("Paneflow", &body);
}

/// Pure body builder. Extracted from [`on_turn_ended`] so tests can
/// exercise the exact production logic (including the `Refusal`
/// branch's call into [`sanitize_model_label`]) without firing the
/// notify-rust path. Returns `None` for branches that intentionally
/// stay silent.
fn turn_ended_body(
    reason: StopReasonKind,
    ran_tools: bool,
    model_label: Option<&str>,
) -> Option<String> {
    match reason {
        StopReasonKind::EndTurn => Some(if ran_tools {
            "Finished running tools".to_string()
        } else {
            "New message".to_string()
        }),
        StopReasonKind::Refusal => {
            let model_owned = model_label
                .map(sanitize_model_label)
                .filter(|s| !s.is_empty());
            let model = model_owned.as_deref().unwrap_or("Agent");
            Some(format!("{model} refused to respond to this request"))
        }
        // MaxTokens / MaxTurnRequests: surface the truncation reason
        // so a user who alt-tabbed away learns why the response is
        // incomplete. Cancelled / Other stay silent (user-initiated
        // or unknown -- a notification would just add noise).
        StopReasonKind::MaxTokens => {
            Some("Response truncated: max output tokens reached".to_string())
        }
        StopReasonKind::MaxTurnRequests => {
            Some("Stopped: agent reached the per-turn request limit".to_string())
        }
        StopReasonKind::Cancelled | StopReasonKind::Other => None,
    }
}

/// US-116 AC #5: surface a fatal-runtime notification when the panel
/// is not focused.
pub fn on_fatal() {
    if !user_opted_in() || panel_is_focused() {
        return;
    }
    fire("Paneflow", "Agent stopped due to an error");
}

/// Maximum number of characters preserved from an agent-supplied model
/// id before [`sanitize_model_label`] truncates with an ellipsis.
/// Legitimate model ids ("claude-sonnet-4-5", "gpt-5", "codex-1") fit
/// in well under this cap; anything longer is either malformed or
/// agent-attacker-crafted phishing.
const MODEL_LABEL_MAX_CHARS: usize = 32;

/// Neutralise an agent-supplied `model_label` before it lands in the
/// notification body. Strips ASCII / Unicode `Cc` control chars (ESC,
/// BEL, etc.), Pango / HTML tags (`<...>`) and entities (`&...;`) and
/// caps the result at [`MODEL_LABEL_MAX_CHARS`]. The GNOME/libnotify
/// notification daemon renders Pango markup by default, so without
/// this an attacker-controlled `current_model_id` could forge a fake
/// "system" message in the desktop toast. The control-char strip is
/// defense-in-depth -- libnotify has no VT parser today, but a stray
/// OSC sequence (`\x1b]2;...\x07`) has no business in a model id.
///
/// On a malformed entity (`&` followed by 13+ chars without a `;`) we
/// stop processing entirely and return what we have. A legitimate
/// model id has no `&`; the truncation is the safe direction.
fn sanitize_model_label(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    let mut in_entity = false;
    let mut entity_len = 0usize;
    for ch in raw.chars() {
        // Drop control characters up front so they cannot land in tag
        // content, entity payloads, or the final output.
        if ch.is_control() {
            continue;
        }
        if in_tag {
            if ch == '>' {
                in_tag = false;
            }
            continue;
        }
        if in_entity {
            entity_len += 1;
            if ch == ';' {
                in_entity = false;
                entity_len = 0;
                continue;
            }
            if entity_len > 12 {
                // Malformed entity -- treat the remainder of the input
                // as untrusted. Keeping going would let an attacker
                // craft `&xxxxxxxxxxxx<b>evil</b>` and have `evil` leak
                // through the tag stripper.
                break;
            }
            continue;
        }
        match ch {
            '<' => in_tag = true,
            '&' => {
                in_entity = true;
                entity_len = 0;
            }
            _ => out.push(ch),
        }
    }
    if out.chars().count() <= MODEL_LABEL_MAX_CHARS {
        return out;
    }
    let mut capped: String = out.chars().take(MODEL_LABEL_MAX_CHARS).collect();
    capped.push('…');
    capped
}

/// Send the notification through `notify-rust`. Per AC #8, failure is
/// logged + swallowed -- the desktop notification surface is
/// best-effort and must never destabilise the GPUI render path.
fn fire(summary: &str, body: &str) {
    let result = notify_rust::Notification::new()
        .summary(summary)
        .body(body)
        .appname("Paneflow")
        .show();
    if let Err(err) = result {
        tracing::warn!(
            target: "paneflow_app::agents::notifications",
            "desktop notification failed: {err}",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC #6: when both flags are `true` (window focused + agents
    /// mode), the gate returns `true` so the notification is
    /// suppressed.
    #[test]
    fn panel_focused_suppresses_when_both_flags_true() {
        assert!(panel_is_focused_pure(true, true));
    }

    /// AC #3: window focused but user is in CLI mode -> the gate
    /// reports "not focused" so the notification fires.
    #[test]
    fn cli_mode_does_not_count_as_focused() {
        assert!(!panel_is_focused_pure(true, false));
    }

    /// AC #3: window unfocused -> the gate reports "not focused"
    /// regardless of mode, so the notification fires either way.
    #[test]
    fn window_blurred_does_not_count_as_focused() {
        assert!(!panel_is_focused_pure(false, true));
        assert!(!panel_is_focused_pure(false, false));
    }

    /// AC #3: EndTurn with tools picks the "Finished running tools"
    /// body. Exercises the production [`turn_ended_body`] helper
    /// directly so refactors to `on_turn_ended` cannot drift this
    /// branch logic out of sync with the test.
    #[test]
    fn end_turn_body_switches_on_ran_tools() {
        assert_eq!(
            turn_ended_body(StopReasonKind::EndTurn, true, None).as_deref(),
            Some("Finished running tools"),
        );
        assert_eq!(
            turn_ended_body(StopReasonKind::EndTurn, false, None).as_deref(),
            Some("New message"),
        );
        assert_eq!(
            turn_ended_body(StopReasonKind::Refusal, false, Some("claude-sonnet-4-5")).as_deref(),
            Some("claude-sonnet-4-5 refused to respond to this request"),
        );
        assert_eq!(
            turn_ended_body(StopReasonKind::Refusal, false, None).as_deref(),
            Some("Agent refused to respond to this request"),
        );
        // Other = no notification body at all.
        assert!(turn_ended_body(StopReasonKind::Other, true, None).is_none());
    }

    /// US-007 (review follow-up): the Refusal branch of
    /// [`turn_ended_body`] MUST apply [`sanitize_model_label`] to the
    /// agent-supplied label. Regression guard against an unwired call
    /// site -- the dedicated sanitize_model_label_* tests only cover
    /// the function itself, not the production wiring.
    #[test]
    fn refusal_body_sanitises_model_label() {
        let body = turn_ended_body(StopReasonKind::Refusal, false, Some("<b>system</b>")).unwrap();
        assert!(
            !body.contains('<') && !body.contains('>'),
            "refusal body must not carry Pango/HTML markup, got {body:?}",
        );
        assert_eq!(body, "system refused to respond to this request");

        // Empty-after-sanitisation label falls back to "Agent" so the
        // body never reads " refused to respond to this request".
        let stripped = turn_ended_body(StopReasonKind::Refusal, false, Some("<b></b>")).unwrap();
        assert_eq!(stripped, "Agent refused to respond to this request");
    }

    /// US-007 AC #1: HTML/Pango tags supplied by the agent must be
    /// stripped so a phishing notification body cannot impersonate the
    /// system or render bold/italic styling.
    #[test]
    fn sanitize_model_label_strips_tags() {
        assert_eq!(
            sanitize_model_label("<b>system</b>: <i>evil</i>"),
            "system: evil"
        );
        assert_eq!(sanitize_model_label("<span size=\"large\">x</span>"), "x");
        assert_eq!(sanitize_model_label("<unclosed-tag"), "");
    }

    /// US-007: HTML entities are also rendered by libnotify when Pango
    /// is enabled; strip them too.
    #[test]
    fn sanitize_model_label_strips_entities() {
        assert_eq!(sanitize_model_label("a&amp;b"), "ab");
        assert_eq!(sanitize_model_label("&lt;not-a-tag&gt;"), "not-a-tag");
        // Numeric entity is stripped wholesale.
        assert_eq!(sanitize_model_label("x&#65;y"), "xy");
        // Unterminated entity (no `;` within 12 chars) aborts processing
        // so a payload like `&xxxxxxxxxxxx<b>evil</b>` cannot leak `evil`
        // back through the tag stripper. Output is empty when the `&`
        // begins the string and no `;` arrives in time.
        assert_eq!(sanitize_model_label("&unterminatedlongtail"), "");
        // A short stray `&` at end-of-input also aborts cleanly: out
        // is whatever was accumulated before the `&`.
        assert_eq!(sanitize_model_label("safe&"), "safe");
        // The historic bypass payload: prefix output is preserved, but
        // the post-entity `<b>evil</b>` content does NOT leak through.
        assert_eq!(sanitize_model_label("safe&xxxxxxxxxxxx<b>evil</b>"), "safe",);
    }

    /// US-007 (review follow-up): control characters supplied by the
    /// agent must be stripped before the Pango pass so an OSC sequence
    /// (`\x1b]2;...\x07`) cannot reach `notify_rust::body`. libnotify
    /// has no VT parser today, but the strip is defense-in-depth.
    /// Only the control bytes themselves are stripped -- non-control
    /// punctuation that happens to participate in an escape sequence
    /// (`]`, `;`) survives, which is fine since they are not Pango
    /// markup and cannot drive a terminal emulator without ESC.
    #[test]
    fn sanitize_model_label_strips_control_chars() {
        // ESC + BEL discarded; the surrounding bytes survive as plain
        // text. An OSC sequence without ESC + ST is harmless ASCII.
        assert_eq!(
            sanitize_model_label("claude\x1b]2;evil\x07-sonnet"),
            "claude]2;evil-sonnet",
        );
        // Bare ESC at start: dropped, prefix unaffected.
        assert_eq!(sanitize_model_label("\x1bgpt-5"), "gpt-5");
        // Newline / CR inside a model id is non-sensical -- drop them.
        assert_eq!(sanitize_model_label("gpt\n5\r"), "gpt5");
    }

    /// US-007 AC #2: the result is capped at 32 chars with an ellipsis
    /// appended (33-char visual cap including the ellipsis).
    #[test]
    fn sanitize_model_label_truncates_over_cap() {
        let long = "a".repeat(64);
        let out = sanitize_model_label(&long);
        assert_eq!(out.chars().count(), MODEL_LABEL_MAX_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    /// US-007 AC #3: a legitimate model id passes through unchanged
    /// (idempotent on safe input).
    #[test]
    fn sanitize_model_label_idempotent_on_safe_input() {
        assert_eq!(
            sanitize_model_label("claude-sonnet-4-5"),
            "claude-sonnet-4-5"
        );
        assert_eq!(sanitize_model_label("gpt-5"), "gpt-5");
        assert_eq!(sanitize_model_label("codex-1"), "codex-1");
        let safe = "claude-opus-4-7";
        assert_eq!(sanitize_model_label(safe), safe);
        assert_eq!(sanitize_model_label(&sanitize_model_label(safe)), safe);
    }
}
