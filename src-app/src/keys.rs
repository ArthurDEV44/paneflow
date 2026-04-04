//! Keystroke-to-escape-sequence mapping for terminal input.
//!
//! Returns `Cow::Borrowed` for all static sequences (zero allocation).
//! Only modifier+key combos that require formatting allocate via `Cow::Owned`.

use std::borrow::Cow;

use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

/// Map a GPUI keystroke to a terminal escape sequence.
///
/// Returns `Some(Cow::Borrowed(...))` for static keys (zero-alloc),
/// `Some(Cow::Owned(...))` for modifier combos (one alloc),
/// or `None` if the keystroke should be handled as printable character input.
pub fn to_esc_str(keystroke: &Keystroke, mode: &TermMode) -> Option<Cow<'static, str>> {
    let key = keystroke.key.as_str();
    let ctrl = keystroke.modifiers.control;
    let shift = keystroke.modifiers.shift;
    let alt = keystroke.modifiers.alt;

    // Ctrl+letter → control byte (zero alloc via static strings)
    if ctrl && !shift && !alt {
        let seq: Option<&'static str> = match key {
            "a" => Some("\x01"),
            "b" => Some("\x02"),
            "c" => Some("\x03"),
            "d" => Some("\x04"),
            "e" => Some("\x05"),
            "f" => Some("\x06"),
            "g" => Some("\x07"),
            "h" => Some("\x08"),
            "i" => Some("\x09"),
            "j" => Some("\x0a"),
            "k" => Some("\x0b"),
            "l" => Some("\x0c"),
            "m" => Some("\x0d"),
            "n" => Some("\x0e"),
            "o" => Some("\x0f"),
            "p" => Some("\x10"),
            "q" => Some("\x11"),
            "r" => Some("\x12"),
            "s" => Some("\x13"),
            "t" => Some("\x14"),
            "u" => Some("\x15"),
            "v" => Some("\x16"),
            "w" => Some("\x17"),
            "x" => Some("\x18"),
            "y" => Some("\x19"),
            "z" => Some("\x1a"),
            "[" => Some("\x1b"), // Same as Escape — standard ANSI behavior
            "\\" => Some("\x1c"),
            "]" => Some("\x1d"),
            "^" => Some("\x1e"),
            "_" => Some("\x1f"),
            _ => None,
        };
        if let Some(s) = seq {
            return Some(Cow::Borrowed(s));
        }
    }

    // Special keys — no modifiers
    if !ctrl && !shift && !alt {
        let app_cursor = mode.contains(TermMode::APP_CURSOR);
        let seq: Option<&'static str> = match key {
            "enter" => Some("\r"),
            "tab" => Some("\t"),
            "escape" => Some("\x1b"),
            "backspace" => Some("\x7f"),
            "delete" => Some("\x1b[3~"),
            "insert" => Some("\x1b[2~"),
            "space" => Some(" "),
            // Cursor keys: application mode (SS3) vs normal mode (CSI)
            "up" if app_cursor => Some("\x1bOA"),
            "down" if app_cursor => Some("\x1bOB"),
            "right" if app_cursor => Some("\x1bOC"),
            "left" if app_cursor => Some("\x1bOD"),
            "up" => Some("\x1b[A"),
            "down" => Some("\x1b[B"),
            "right" => Some("\x1b[C"),
            "left" => Some("\x1b[D"),
            "home" if app_cursor => Some("\x1bOH"),
            "end" if app_cursor => Some("\x1bOF"),
            "home" => Some("\x1b[H"),
            "end" => Some("\x1b[F"),
            "pageup" => Some("\x1b[5~"),
            "pagedown" => Some("\x1b[6~"),
            // Function keys
            "f1" => Some("\x1bOP"),
            "f2" => Some("\x1bOQ"),
            "f3" => Some("\x1bOR"),
            "f4" => Some("\x1bOS"),
            "f5" => Some("\x1b[15~"),
            "f6" => Some("\x1b[17~"),
            "f7" => Some("\x1b[18~"),
            "f8" => Some("\x1b[19~"),
            "f9" => Some("\x1b[20~"),
            "f10" => Some("\x1b[21~"),
            "f11" => Some("\x1b[23~"),
            "f12" => Some("\x1b[24~"),
            _ => None,
        };
        if let Some(s) = seq {
            return Some(Cow::Borrowed(s));
        }
    }

    // Shift+special keys
    if shift && !ctrl && !alt {
        let seq: Option<&'static str> = match key {
            "tab" => Some("\x1b[Z"), // Back-tab
            _ => None,
        };
        if let Some(s) = seq {
            return Some(Cow::Borrowed(s));
        }
    }

    // Alt+key → ESC prefix
    if alt && !ctrl && !shift && key.len() == 1 {
        return Some(Cow::Owned(format!("\x1b{key}")));
    }

    // Modifier+cursor combos (CSI 1;N sequences) — one allocation
    let modifier_code = match (shift, alt, ctrl) {
        (true, false, false) => Some(2),
        (false, true, false) => Some(3),
        (true, true, false) => Some(4),
        (false, false, true) => Some(5),
        (true, false, true) => Some(6),
        (false, true, true) => Some(7),
        (true, true, true) => Some(8),
        _ => None,
    };

    if let Some(m) = modifier_code {
        let base = match key {
            "up" => Some("A"),
            "down" => Some("B"),
            "right" => Some("C"),
            "left" => Some("D"),
            "home" => Some("H"),
            "end" => Some("F"),
            _ => None,
        };
        if let Some(b) = base {
            return Some(Cow::Owned(format!("\x1b[1;{m}{b}")));
        }
    }

    // Not a special key — caller should handle as printable character input
    None
}
