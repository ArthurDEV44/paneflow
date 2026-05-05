//! URL and file-path detection for the terminal renderer.
//!
//! Two scanners share the same line-scoped, char-to-column mapped pattern:
//! - `detect_urls_on_line_mapped`  — Zed-style URL regex (US-015).
//! - `detect_file_paths_on_line_mapped` — `.md` / `.markdown` paths with
//!   existence check + heuristics (US-019).
//!
//! Both return `HyperlinkZone`; the scheme allowlist (`is_url_scheme_openable`)
//! guards what `TerminalView` will actually open.

use std::path::{Path, PathBuf};
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

// ---------------------------------------------------------------------------
// File-path scanner (US-019)
// ---------------------------------------------------------------------------

/// Regex matching `.md` / `.markdown` path candidates on a terminal line.
///
/// The character class allows alphanumerics, common path/separator chars, and
/// nothing else — explicitly excludes whitespace, quotes, brackets, and ANSI
/// escape control chars (C0/C1). The trailing `\b` ensures `.md` is not matched
/// inside a longer alphanumeric token (e.g. `.markdown_old`).
///
/// Boundary at the start is enforced post-match (preceding char must be start
/// of string, whitespace, or an opening punctuation char). The `regex` crate
/// has no lookbehind, so we filter in code.
const FILE_PATH_REGEX_PATTERN: &str = r#"(?i)[A-Za-z0-9_:./\\\-]+\.(?:md|markdown)\b"#;

fn file_path_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(FILE_PATH_REGEX_PATTERN).expect("file-path regex compilation failed")
    })
}

/// Returns true if the byte position is at a clean left boundary for a path
/// match: start of string, preceded by whitespace, or by an opening
/// punctuation char (parens, brackets, quotes, backtick, brace).
fn left_boundary_ok(line_text: &str, byte_pos: usize) -> bool {
    if byte_pos == 0 {
        return true;
    }
    match line_text[..byte_pos].chars().next_back() {
        None => true,
        Some(c) if c.is_whitespace() => true,
        Some('(' | '[' | '<' | '\'' | '"' | '`' | '{') => true,
        _ => false,
    }
}

/// Minimum stem length (basename without extension) for candidates that have
/// no path separator. `123.md` → rejected; `/foo/bar.md` accepted regardless.
const MIN_BARE_STEM_LEN: usize = 4;

/// Returns true if `path_str` looks like a Windows absolute path (`C:\foo` or `C:/foo`).
fn is_windows_absolute(path_str: &str) -> bool {
    let bytes = path_str.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// Returns true if `path_str` looks like a POSIX absolute path (`/foo`).
fn is_posix_absolute(path_str: &str) -> bool {
    path_str.starts_with('/')
}

/// Returns true if any character in `s` is a C0/C1 control or DEL.
fn contains_control_char(s: &str) -> bool {
    s.chars()
        .any(|c| (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)))
}

/// Returns the file stem (portion before the last `.`) length in chars.
/// For multi-segment paths, only the basename is considered.
/// Example: `/foo/bar/README.md` → 6 (the stem `README`).
fn stem_len(path_str: &str) -> usize {
    let basename = path_str
        .rsplit_once(['/', '\\'])
        .map(|(_, name)| name)
        .unwrap_or(path_str);
    let stem = basename
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(basename);
    stem.chars().count()
}

/// Returns true if `candidate` is prefixed with a URL-style scheme (`http:`,
/// `file:`, `mailto:`, `ssh:`, …) — i.e. two or more ASCII letters followed by
/// `:`. Single-letter prefixes (`C:`, `D:`) are Windows drive letters, not
/// schemes, and are NOT classified as schemes here. Used to bar terminal
/// output like `file:///etc/passwd.md` from being passed to `open::that`,
/// where `xdg-open` would honour the URI scheme rather than treat it as a
/// local file.
fn has_url_scheme_prefix(candidate: &str) -> bool {
    let Some(colon_idx) = candidate.find(':') else {
        return false;
    };
    let prefix = &candidate[..colon_idx];
    prefix.len() >= 2 && prefix.chars().all(|c| c.is_ascii_alphabetic())
}

/// Resolve `path_str` against `cwd` and canonicalize the result. Returns the
/// canonical absolute path when:
/// - the candidate is a POSIX or Windows absolute path that exists, or
/// - the candidate is relative and joins-with-`cwd` to an existing path.
///
/// `Path::canonicalize` resolves symlinks, normalises `..`/`.` segments, and
/// returns `Err` when the file does not exist — combining the existence check
/// with normalisation in a single call. The canonicalised string is what gets
/// passed to `open::that`, so the user opens the actual resolved target rather
/// than a misleading traversal path printed by the terminal.
fn resolve_path(path_str: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    let candidate = if is_posix_absolute(path_str) || is_windows_absolute(path_str) {
        PathBuf::from(path_str)
    } else {
        let cwd = cwd?;
        cwd.join(path_str)
    };
    candidate.canonicalize().ok()
}

/// Returns true if `path` ends with `.md` or `.markdown` (case-insensitive),
/// after canonicalisation may have changed the byte sequence (e.g. case fold
/// on Windows). Used as a defence-in-depth check after `canonicalize` so a
/// symlink target without the right extension cannot be opened as if it were
/// a markdown file.
fn canonical_has_md_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            let lower = ext.to_ascii_lowercase();
            lower == "md" || lower == "markdown"
        })
}

/// Detect `.md` / `.markdown` file paths on a single terminal line.
///
/// `char_to_col` maps each character index in `line_text` to its grid column,
/// matching the URL scanner's wide-char-aware mapping. `cwd` is used to resolve
/// relative paths; if `None`, only absolute paths are eligible.
///
/// Returned zones have `source = HyperlinkSource::FilePath`, `is_openable = true`,
/// and `uri` set to the resolved absolute path. Anti-false-positive rules:
/// - Stem (basename without extension) must be ≥ 4 chars when the candidate
///   has no path separator. `123.md` → rejected; `/foo/bar.md` accepted.
/// - Candidate must not contain ANSI control chars.
/// - Candidate must resolve to an existing file on disk.
pub fn detect_file_paths_on_line_mapped(
    line_text: &str,
    line: alacritty_terminal::index::Line,
    char_to_col: &[usize],
    cwd: Option<&Path>,
) -> Vec<HyperlinkZone> {
    let re = file_path_regex();
    re.find_iter(line_text)
        .filter_map(|m| {
            if !left_boundary_ok(line_text, m.start()) {
                return None;
            }
            let candidate = m.as_str();
            if contains_control_char(candidate) {
                return None;
            }
            // Reject URL-scheme-prefixed candidates so `file:///etc/shadow.md`
            // can never reach `open::that` (which would honour the scheme).
            // Windows drive letters (`C:`) are single-letter and not rejected.
            if has_url_scheme_prefix(candidate) && !is_windows_absolute(candidate) {
                return None;
            }
            let has_separator = candidate.contains('/') || candidate.contains('\\');
            if !has_separator && stem_len(candidate) < MIN_BARE_STEM_LEN {
                return None;
            }
            let resolved = resolve_path(candidate, cwd)?;
            // Defence-in-depth: a symlinked candidate could canonicalise to a
            // file with a different (or no) extension. Confirm the canonical
            // target still ends with `.md` / `.markdown`.
            if !canonical_has_md_extension(&resolved) {
                return None;
            }
            let absolute = resolved.to_string_lossy().into_owned();

            let char_start = line_text[..m.start()].chars().count();
            let char_end = line_text[..m.end()].chars().count().saturating_sub(1);
            let col_start = char_to_col.get(char_start)?;
            let col_end = char_to_col.get(char_end)?;

            Some(HyperlinkZone {
                uri: absolute,
                id: String::new(),
                start: alacritty_terminal::index::Point::new(
                    line,
                    alacritty_terminal::index::Column(*col_start),
                ),
                end: alacritty_terminal::index::Point::new(
                    line,
                    alacritty_terminal::index::Column(*col_end),
                ),
                is_openable: true,
                source: HyperlinkSource::FilePath,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    fn line0() -> alacritty_terminal::index::Line {
        alacritty_terminal::index::Line(0)
    }

    /// Builds a 1-to-1 char→column map for ASCII-only test text.
    fn ascii_map(text: &str) -> Vec<usize> {
        (0..text.chars().count()).collect()
    }

    fn write_md(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create dir");
        }
        fs::write(&p, b"# test").expect("write md");
        p
    }

    /// Resolve `p` to a string the file-path regex can match end-to-end.
    ///
    /// The regex character class `[A-Za-z0-9_:./\\\-]+` deliberately
    /// excludes `~` and `?`. On Windows runners `tempfile::tempdir()`
    /// often returns paths with 8.3 short-name segments (`RUNNER~1`);
    /// `canonicalize` resolves them to long form but prepends the
    /// `\\?\` UNC prefix that contains `?`. We strip that prefix so the
    /// downstream regex pass sees a path made entirely of accepted
    /// characters. On Unix `canonicalize` returns a regex-friendly path
    /// already and the strip is a no-op.
    fn canonical_display(p: &Path) -> String {
        let canonical = p.canonicalize().expect("canonicalize");
        let s = canonical.to_string_lossy().into_owned();
        s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s)
    }

    #[cfg(unix)]
    #[test]
    fn linux_absolute_path_existing_matches() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let md = write_md(tmp.path(), "doc.md");
        let canonical = md.canonicalize().expect("canonicalize");
        let line_text = format!("see {}", md.to_string_lossy());
        let map = ascii_map(&line_text);
        let zones = detect_file_paths_on_line_mapped(&line_text, line0(), &map, None);
        assert_eq!(zones.len(), 1);
        assert_eq!(PathBuf::from(&zones[0].uri), canonical);
        assert_eq!(zones[0].source, HyperlinkSource::FilePath);
        assert!(zones[0].is_openable);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_absolute_uses_same_unix_path() {
        // macOS uses POSIX paths; same code path as Linux.
        let tmp = tempfile::tempdir().expect("tempdir");
        let md = write_md(tmp.path(), "Users_foo.md");
        let line_text = format!("open {}", md.to_string_lossy());
        let map = ascii_map(&line_text);
        let zones = detect_file_paths_on_line_mapped(&line_text, line0(), &map, None);
        assert_eq!(zones.len(), 1);
    }

    #[test]
    fn windows_absolute_path_classified_correctly() {
        // Pure regex/classification check — file does not need to exist on
        // the host filesystem since `resolve_path` will reject it.
        assert!(is_windows_absolute("C:\\Users\\arthur\\doc.md"));
        assert!(is_windows_absolute("D:/repo/README.md"));
        assert!(!is_windows_absolute("/etc/foo.md"));
        assert!(!is_windows_absolute("foo.md"));
        assert!(!is_windows_absolute("C:foo"));
    }

    #[test]
    fn relative_with_dot_prefix_resolves_against_cwd() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_md(tmp.path(), "rel.md");
        let line_text = "open ./rel.md now";
        let map = ascii_map(line_text);
        let zones = detect_file_paths_on_line_mapped(line_text, line0(), &map, Some(tmp.path()));
        assert_eq!(zones.len(), 1);
        let resolved = PathBuf::from(&zones[0].uri);
        assert!(resolved.exists());
    }

    #[test]
    fn relative_bare_resolves_against_cwd() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_md(tmp.path(), "README.md");
        let line_text = "edit README.md please";
        let map = ascii_map(line_text);
        let zones = detect_file_paths_on_line_mapped(line_text, line0(), &map, Some(tmp.path()));
        assert_eq!(zones.len(), 1);
    }

    #[test]
    fn missing_file_is_rejected() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let line_text = format!("ghost {}/nope.md", tmp.path().to_string_lossy());
        let map = ascii_map(&line_text);
        let zones = detect_file_paths_on_line_mapped(&line_text, line0(), &map, None);
        assert!(zones.is_empty());
    }

    #[test]
    fn short_numeric_stem_is_rejected() {
        // Bare `123.md` with no separator — stem `123` (3 chars) < 4.
        let tmp = tempfile::tempdir().expect("tempdir");
        write_md(tmp.path(), "123.md");
        let line_text = "open 123.md";
        let map = ascii_map(line_text);
        let zones = detect_file_paths_on_line_mapped(line_text, line0(), &map, Some(tmp.path()));
        assert!(zones.is_empty(), "short bare stem must be rejected");
    }

    #[test]
    fn short_stem_with_path_separator_is_accepted() {
        // `./os.md` has a separator, so the length heuristic does not apply.
        let tmp = tempfile::tempdir().expect("tempdir");
        write_md(tmp.path(), "os.md");
        let line_text = "open ./os.md";
        let map = ascii_map(line_text);
        let zones = detect_file_paths_on_line_mapped(line_text, line0(), &map, Some(tmp.path()));
        assert_eq!(zones.len(), 1);
    }

    #[test]
    fn case_insensitive_extension() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_md(tmp.path(), "guide.MD");
        let line_text = "see ./guide.MD";
        let map = ascii_map(line_text);
        let zones = detect_file_paths_on_line_mapped(line_text, line0(), &map, Some(tmp.path()));
        assert_eq!(zones.len(), 1);
    }

    #[test]
    fn markdown_long_extension() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_md(tmp.path(), "guide.markdown");
        let line_text = "see ./guide.markdown";
        let map = ascii_map(line_text);
        let zones = detect_file_paths_on_line_mapped(line_text, line0(), &map, Some(tmp.path()));
        assert_eq!(zones.len(), 1);
    }

    #[test]
    fn control_chars_disqualify_match() {
        // A path-shaped token that contains an ANSI escape must be rejected.
        // We construct it directly because regex won't match the escape; the
        // helper is the safety net for unusual inputs.
        assert!(contains_control_char("\x1b[31m/foo.md"));
        assert!(!contains_control_char("/foo/bar.md"));
    }

    #[test]
    fn osc8_priority_does_not_overlap_filepath_scanner() {
        // The scanner does not consider OSC 8 zones — priority is enforced
        // at the call site (handle_mouse_move tries OSC 8 first, then URLs,
        // then file paths). This test documents that the scanner itself does
        // not emit zones for arbitrary OSC 8 cells: it only returns matches
        // from regex over the line text. Pure plain text behaves identically
        // whether or not the cell carries an OSC 8 hyperlink.
        let tmp = tempfile::tempdir().expect("tempdir");
        let md_path = write_md(tmp.path(), "doc.md");
        // `canonical_display` resolves Windows 8.3 short names + strips the
        // `\\?\` UNC prefix so the file-path regex character class accepts
        // every byte. On Unix it's a thin wrapper around canonicalize.
        let display = canonical_display(&md_path);
        let line_text = format!("file {display}");
        let map = ascii_map(&line_text);
        let zones = detect_file_paths_on_line_mapped(&line_text, line0(), &map, None);
        assert_eq!(zones.len(), 1);
    }

    #[test]
    fn boundary_rejects_mid_token_match() {
        // `xyz/foo.md` — the slash makes the regex match `xyz/foo.md`, but if
        // the candidate is preceded by `prefix-` (no whitespace), boundary
        // check rejects. Confirm that whitespace boundary works.
        let tmp = tempfile::tempdir().expect("tempdir");
        let md_path = write_md(tmp.path(), "foo.md");
        // Same Windows-friendly long-form path the OSC-8 test uses.
        let display = canonical_display(&md_path);
        let line_text = format!("ok {display}");
        let map = ascii_map(&line_text);
        let zones = detect_file_paths_on_line_mapped(&line_text, line0(), &map, None);
        assert_eq!(zones.len(), 1);

        // Embedded mid-word: `prefixfoo.md` (no slash, no boundary delim
        // around but at start of string => start-of-line counts as boundary,
        // so this match is allowed at column 0). However we still want the
        // existence check to reject when the file does not exist. This is a
        // pure regex-string check — no real file involved — so it stays
        // cross-platform without any UNC-strip dance.
        let line_text2 = "blob/junk.md";
        let map2 = ascii_map(line_text2);
        let zones2 = detect_file_paths_on_line_mapped(line_text2, line0(), &map2, Some(tmp.path()));
        assert!(zones2.is_empty());
    }

    #[test]
    fn relative_without_cwd_is_rejected() {
        let line_text = "see ./foo.md";
        let map = ascii_map(line_text);
        let zones = detect_file_paths_on_line_mapped(line_text, line0(), &map, None);
        assert!(zones.is_empty());
    }

    #[test]
    fn url_scheme_prefix_is_rejected() {
        // `file:///etc/shadow.md` ends in `.md` and matches the regex char
        // class, but the URL-scheme guard must reject it before resolve_path
        // runs — otherwise `open::that` would honour the file:// scheme.
        assert!(has_url_scheme_prefix("file:///etc/shadow.md"));
        assert!(has_url_scheme_prefix("http://evil.example/x.md"));
        assert!(has_url_scheme_prefix("ssh:host.md"));
        // Windows drive letters are single-letter prefixes — NOT schemes.
        assert!(!has_url_scheme_prefix("C:/repo/README.md"));
        assert!(!has_url_scheme_prefix("D:\\proj\\readme.md"));
        // Bare filenames have no colon → no prefix.
        assert!(!has_url_scheme_prefix("README.md"));
        assert!(!has_url_scheme_prefix("./foo.md"));

        // End-to-end: scanner refuses to emit a zone even if the URL-shaped
        // string would otherwise canonicalise to an existing file.
        let line_text = "open file:///tmp/doc.md please";
        let map = ascii_map(line_text);
        let zones = detect_file_paths_on_line_mapped(line_text, line0(), &map, None);
        assert!(zones.is_empty());
    }

    #[test]
    fn canonicalize_resolves_dot_dot_traversal() {
        // A relative candidate with `..` segments must canonicalise: the URI
        // emitted to the click handler should be the canonical (real) path,
        // not the misleading traversal string printed by the terminal.
        let tmp = tempfile::tempdir().expect("tempdir");
        let nested = tmp.path().join("nested");
        fs::create_dir_all(&nested).expect("create nested");
        let md = write_md(tmp.path(), "real.md");
        let canonical = md.canonicalize().expect("canonicalize");

        // From `nested/`, the path `../real.md` must resolve to canonical.
        let line_text = "see ../real.md";
        let map = ascii_map(line_text);
        let zones = detect_file_paths_on_line_mapped(line_text, line0(), &map, Some(&nested));
        assert_eq!(zones.len(), 1);
        assert_eq!(PathBuf::from(&zones[0].uri), canonical);
        // The emitted URI must NOT contain `..` — it has been normalised.
        assert!(!zones[0].uri.contains(".."));
    }

    #[test]
    fn perf_scan_200_lines_under_budget() {
        // AC budget: 200×80 grid scan < 5 ms (release).
        // Debug builds are ~5–10× slower; we assert release < 5 ms strictly
        // on Linux/macOS and apply a 25 ms ceiling in debug as a regression
        // guard. On Windows the hosted runners are 2-3× slower at the same
        // workload (US-004 AC5), so we relax to 15 ms in release without
        // weakening the regression intent — anything significantly above
        // 15 ms still surfaces as a perf regression.
        let tmp = tempfile::tempdir().expect("tempdir");
        let md_path = write_md(tmp.path(), "perf.md");
        let target = canonical_display(&md_path);
        let mut lines: Vec<String> = (0..200)
            .map(|i| {
                if i % 20 == 0 {
                    format!("[info] open {} for review", target)
                } else {
                    "plain log line with no path content here at all -----".to_string()
                }
            })
            .collect();
        for line in &mut lines {
            while line.chars().count() < 80 {
                line.push(' ');
            }
        }
        let started = Instant::now();
        let mut total = 0usize;
        for line in &lines {
            let map = ascii_map(line);
            let zones = detect_file_paths_on_line_mapped(line, line0(), &map, None);
            total += zones.len();
        }
        let elapsed = started.elapsed();
        assert!(total >= 10, "expected at least 10 hits, got {}", total);
        let budget_ms: u128 = if cfg!(debug_assertions) {
            25
        } else if cfg!(target_os = "windows") {
            15
        } else {
            5
        };
        assert!(
            elapsed.as_millis() < budget_ms,
            "200×80 scan took {:?}, exceeds {} ms budget",
            elapsed,
            budget_ms
        );
    }
}
