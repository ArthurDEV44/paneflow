//! URL detection and scheme allowlisting for the terminal renderer.
//!
//! Matches Zed's `terminal_hyperlinks.rs` behaviour: a regex sweep over each
//! visible line that respects wide-char column mapping, plus a scheme
//! allowlist guarding what `terminal::TerminalView::open_hyperlink` may open.
//!
//! Extracted from `terminal_element.rs` per US-010 of the src-app refactor PRD.

use std::sync::OnceLock;

pub use crate::terminal::types::{HyperlinkSource, HyperlinkZone};

/// URL regex pattern matching Zed's terminal_hyperlinks.rs.
/// Excludes C0/C1 control chars, whitespace, angle brackets, quotes, and other
/// non-URL characters. Box-drawing chars (U+2500-U+257F) are not valid URL
/// characters and won't match the allowed character class.
pub(super) const URL_REGEX_PATTERN: &str = r#"(mailto:|gemini://|gopher://|https://|http://|news:|file://|git://|ssh:|ftp://)[^\x00-\x1f\x7f-\x9f<>"\s{}\^⟨⟩`']+"#;

/// Lazily compiled URL regex (compiled once, reused across all calls).
pub(super) fn url_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(URL_REGEX_PATTERN).expect("URL regex compilation failed"))
}

/// Detect URLs on a single terminal line via regex with char-to-column mapping.
/// `char_to_col` maps each character index in `line_text` to its grid column,
/// accounting for wide-char spacers that were skipped during text extraction.
pub fn detect_urls_on_line_mapped(
    line_text: &str,
    line: alacritty_terminal::index::Line,
    char_to_col: &[usize],
) -> Vec<HyperlinkZone> {
    let re = url_regex();
    re.find_iter(line_text)
        .filter_map(|m| {
            // Convert byte offsets to char indices for column lookup
            let char_start = line_text[..m.start()].chars().count();
            let char_end = line_text[..m.end()].chars().count().saturating_sub(1);
            let col_start = char_to_col.get(char_start)?;
            let col_end = char_to_col.get(char_end)?;
            let uri = m.as_str().to_string();
            let is_openable = is_url_scheme_openable(&uri);
            Some(HyperlinkZone {
                uri,
                id: String::new(),
                start: alacritty_terminal::index::Point::new(
                    line,
                    alacritty_terminal::index::Column(*col_start),
                ),
                end: alacritty_terminal::index::Point::new(
                    line,
                    alacritty_terminal::index::Column(*col_end),
                ),
                is_openable,
                source: HyperlinkSource::Regex,
            })
        })
        .collect()
}

/// Check if a URL scheme is in the allowlist for opening.
/// Allowed: http, https, mailto, file (with localhost/empty host validation).
pub fn is_url_scheme_openable(uri: &str) -> bool {
    if uri.starts_with("http://") || uri.starts_with("https://") || uri.starts_with("mailto:") {
        return true;
    }
    if let Some(rest) = uri.strip_prefix("file://") {
        // file:// must have empty host or localhost
        return rest.starts_with('/')
            || rest.starts_with("localhost/")
            || rest.starts_with("localhost:");
    }
    false
}
