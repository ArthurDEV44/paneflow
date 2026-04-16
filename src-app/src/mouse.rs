//! Mouse event encoding for terminal mouse reporting protocols.
//!
//! Encodes GPUI mouse events into SGR and X10 (Normal) escape sequences
//! for forwarding to the PTY when terminal mouse modes are active.

use alacritty_terminal::index::Point as AlacPoint;
use alacritty_terminal::term::TermMode;
use gpui::{Modifiers, MouseButton};

/// Mouse report encoding format, selected based on the terminal's active mode.
pub enum MouseFormat {
    /// SGR encoding (mode 1006): `\e[<btn;col;row{M|m}`. Unbounded coordinates.
    Sgr,
    /// Normal/X10 encoding: `\e[M{cb}{cx}{cy}`. Limited coordinates.
    Normal { utf8: bool },
}

impl MouseFormat {
    /// Select encoding format from the terminal's current mode flags.
    pub fn from_mode(mode: TermMode) -> Self {
        if mode.contains(TermMode::SGR_MOUSE) {
            MouseFormat::Sgr
        } else if mode.contains(TermMode::UTF8_MOUSE) {
            MouseFormat::Normal { utf8: true }
        } else {
            MouseFormat::Normal { utf8: false }
        }
    }
}

/// Scroll direction for mouse scroll reports.
pub enum ScrollDirection {
    Up,
    Down,
}

/// Map a GPUI mouse button + modifier keys to the terminal mouse button byte.
///
/// Button codes: left=0, middle=1, right=2.
/// Modifier bits: shift=+4, alt=+8, ctrl=+16.
pub fn mouse_button_code(button: MouseButton, modifiers: Modifiers) -> u8 {
    let base = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Navigate(_) => 0,
    };
    base + modifier_bits(modifiers)
}

/// Map a scroll direction + modifier keys to the terminal scroll button byte.
///
/// Scroll codes: up=64, down=65.
/// Modifier bits: shift=+4, alt=+8, ctrl=+16.
pub fn scroll_button_code(direction: ScrollDirection, modifiers: Modifiers) -> u8 {
    let base = match direction {
        ScrollDirection::Up => 64,
        ScrollDirection::Down => 65,
    };
    base + modifier_bits(modifiers)
}

fn modifier_bits(modifiers: Modifiers) -> u8 {
    let mut bits = 0u8;
    if modifiers.shift {
        bits += 4;
    }
    if modifiers.alt {
        bits += 8;
    }
    if modifiers.control {
        bits += 16;
    }
    bits
}

/// Generate an SGR mouse report: `\e[<{button};{col+1};{row+1}{M|m}`.
///
/// `M` for press, `m` for release. Coordinates are 1-based (no upper limit).
/// `button` should be the pre-encoded button code from [`mouse_button_code`].
pub fn sgr_mouse_report(point: AlacPoint, button: u8, pressed: bool) -> String {
    let col = point.column.0 + 1;
    let row = point.line.0.max(0) + 1; // clamp negative scrollback lines to 0
    let suffix = if pressed { 'M' } else { 'm' };
    format!("\x1b[<{button};{col};{row}{suffix}")
}

/// Generate a Normal (X10) mouse report: `\e[M{cb}{cx}{cy}`.
///
/// Each coordinate is encoded as `position + 33`. Without UTF-8, values are
/// single bytes (max position 223). With UTF-8, values are multi-byte encoded
/// (max position 2015). Returns `None` if any coordinate exceeds the limit.
///
/// `button` should be the pre-encoded button code from [`mouse_button_code`].
pub fn normal_mouse_report(point: AlacPoint, button: u8, utf8: bool) -> Option<Vec<u8>> {
    let col = point.column.0 as u32;
    let row = point.line.0.max(0) as u32;
    let max = if utf8 { 2015 } else { 222 };
    if col > max || row > max {
        return None;
    }

    let mut report = vec![0x1b, b'[', b'M', button.wrapping_add(32)];

    if utf8 {
        push_utf8_coord(col + 33, &mut report)?;
        push_utf8_coord(row + 33, &mut report)?;
    } else {
        report.push((col + 33) as u8);
        report.push((row + 33) as u8);
    }

    Some(report)
}

/// Encode a coordinate value as a UTF-8 character and append to `out`.
fn push_utf8_coord(val: u32, out: &mut Vec<u8>) -> Option<()> {
    let c = char::from_u32(val)?;
    let mut buf = [0u8; 4];
    let encoded = c.encode_utf8(&mut buf);
    out.extend_from_slice(encoded.as_bytes());
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line, Point};

    fn pt(col: usize, line: i32) -> AlacPoint {
        Point::new(Line(line), Column(col))
    }

    // --- button code tests ---

    #[test]
    fn button_left() {
        assert_eq!(
            mouse_button_code(MouseButton::Left, Modifiers::default()),
            0
        );
    }

    #[test]
    fn button_middle() {
        assert_eq!(
            mouse_button_code(MouseButton::Middle, Modifiers::default()),
            1
        );
    }

    #[test]
    fn button_right() {
        assert_eq!(
            mouse_button_code(MouseButton::Right, Modifiers::default()),
            2
        );
    }

    #[test]
    fn button_with_shift() {
        let mods = Modifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(mouse_button_code(MouseButton::Left, mods), 4);
    }

    #[test]
    fn button_with_alt() {
        let mods = Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(mouse_button_code(MouseButton::Left, mods), 8);
    }

    #[test]
    fn button_with_ctrl() {
        let mods = Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(mouse_button_code(MouseButton::Left, mods), 16);
    }

    #[test]
    fn button_all_modifiers() {
        let mods = Modifiers {
            shift: true,
            alt: true,
            control: true,
            ..Default::default()
        };
        assert_eq!(mouse_button_code(MouseButton::Left, mods), 28);
    }

    // --- scroll code tests ---

    #[test]
    fn scroll_up_code() {
        assert_eq!(
            scroll_button_code(ScrollDirection::Up, Modifiers::default()),
            64
        );
    }

    #[test]
    fn scroll_down_code() {
        assert_eq!(
            scroll_button_code(ScrollDirection::Down, Modifiers::default()),
            65
        );
    }

    #[test]
    fn scroll_with_shift() {
        let mods = Modifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(scroll_button_code(ScrollDirection::Up, mods), 68);
    }

    // --- SGR format tests ---

    #[test]
    fn sgr_left_press() {
        assert_eq!(sgr_mouse_report(pt(10, 5), 0, true), "\x1b[<0;11;6M");
    }

    #[test]
    fn sgr_left_release() {
        assert_eq!(sgr_mouse_report(pt(10, 5), 0, false), "\x1b[<0;11;6m");
    }

    #[test]
    fn sgr_right_press() {
        assert_eq!(sgr_mouse_report(pt(0, 0), 2, true), "\x1b[<2;1;1M");
    }

    #[test]
    fn sgr_with_modifiers() {
        // button=0 + shift(4) + ctrl(16) = 20
        assert_eq!(sgr_mouse_report(pt(5, 3), 20, true), "\x1b[<20;6;4M");
    }

    // --- Normal (X10) format tests ---

    #[test]
    fn normal_basic() {
        let result = normal_mouse_report(pt(10, 5), 0, false).unwrap();
        assert_eq!(result, vec![0x1b, b'[', b'M', 32, 43, 38]);
    }

    #[test]
    fn normal_boundary_col_94() {
        let result = normal_mouse_report(pt(94, 0), 0, false).unwrap();
        assert_eq!(result[4], 127); // 94 + 33
    }

    #[test]
    fn normal_boundary_col_95() {
        let result = normal_mouse_report(pt(95, 0), 0, false).unwrap();
        assert_eq!(result[4], 128); // 95 + 33
    }

    #[test]
    fn normal_boundary_col_96() {
        let result = normal_mouse_report(pt(96, 0), 0, false).unwrap();
        assert_eq!(result[4], 129); // 96 + 33
    }

    #[test]
    fn normal_max_no_utf8() {
        // Position 222 is the max without UTF-8 (222 + 33 = 255 fits in u8)
        let result = normal_mouse_report(pt(222, 0), 0, false);
        assert!(result.is_some());
        // Verify the byte value is 255 (not wrapped)
        assert_eq!(result.unwrap()[4], 255);
    }

    #[test]
    fn normal_overflow_no_utf8() {
        // Position 223 exceeds the limit without UTF-8 (223 + 33 = 256 overflows u8)
        let result = normal_mouse_report(pt(223, 0), 0, false);
        assert!(result.is_none());
    }

    #[test]
    fn normal_utf8_high_coord() {
        // Position 200 with UTF-8 — should produce multi-byte encoding
        let result = normal_mouse_report(pt(200, 0), 0, true).unwrap();
        // 200 + 33 = 233 → UTF-8: 0xC3 0xA9
        assert!(result.len() > 5); // prefix(4) + multi-byte col + single-byte row
    }

    #[test]
    fn normal_utf8_overflow() {
        // Position 2016 exceeds the UTF-8 limit
        let result = normal_mouse_report(pt(2016, 0), 0, true);
        assert!(result.is_none());
    }

    #[test]
    fn normal_utf8_max() {
        // Position 2015 is the max with UTF-8
        let result = normal_mouse_report(pt(2015, 0), 0, true);
        assert!(result.is_some());
    }

    // --- MouseFormat tests ---

    #[test]
    fn format_sgr_from_mode() {
        let mode = TermMode::SGR_MOUSE;
        assert!(matches!(MouseFormat::from_mode(mode), MouseFormat::Sgr));
    }

    #[test]
    fn format_utf8_from_mode() {
        let mode = TermMode::UTF8_MOUSE;
        assert!(matches!(
            MouseFormat::from_mode(mode),
            MouseFormat::Normal { utf8: true }
        ));
    }

    #[test]
    fn format_normal_from_mode() {
        let mode = TermMode::empty();
        assert!(matches!(
            MouseFormat::from_mode(mode),
            MouseFormat::Normal { utf8: false }
        ));
    }
}
