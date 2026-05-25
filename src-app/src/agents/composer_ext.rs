//! US-019 (prd-agents-view.md): support types + helpers for the
//! Composer's attachment, `@`-mention and `/`-slash machinery. Lives
//! in its own module so [`super::composer`] stays focused on the
//! send/stop/picker mechanics from US-016.
//!
//! Nothing here renders directly -- the Composer owns its own
//! rendering for popups (they share state with the textarea, agent
//! pills, and the attachment chip row). This module hosts:
//!
//! - [`PendingAttachment`]: a chip pinned above the textarea that
//!   carries the ACP [`ContentBlock`] to splice into the next prompt.
//! - [`MentionState`] / [`SlashState`]: ephemeral per-query popup
//!   state. Idle when `None`.
//! - [`scan_files`]: gitignore-respecting walk used by `@`-mention.
//! - [`built_in_slash_commands`]: Paneflow-side `/`-commands (the
//!   agent's own commands are not exposed over ACP yet -- see US-019
//!   AC fallback behaviour).
//! - [`token_before_cursor`]: shared helper that locates an in-flight
//!   `@` / `/` token in the textarea content.
//!
//! Sandboxing: the file walk and the attachment helpers refuse paths
//! outside the thread cwd, mirroring the file-ops handler from
//! US-018. The Composer never persists an absolute path -- attachment
//! chips and `@`-mentions store paths relative to the thread cwd.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::{ContentBlock, ImageContent, ResourceLink, TextContent};
use base64::Engine as _;

/// AC: "Given the user attaches a 50MB image, when uploaded, then a
/// clear error message appears ("Image too large, max 10MB") and the
/// attachment is not added to the prompt".
pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

/// Cap the file walk so a huge monorepo cwd does not block the pump
/// thread. The popup only displays the top results anyway.
pub const MAX_FILE_RESULTS: usize = 50;

/// AC: "popup ... debounced 200ms".
pub const MENTION_DEBOUNCE: Duration = Duration::from_millis(200);

/// US-019: an attachment displayed as a chip above the textarea and
/// spliced into the next `session/prompt`. The `block` is what gets
/// sent over ACP; the `label` is what the chip renders.
#[derive(Debug, Clone)]
pub struct PendingAttachment {
    pub label: String,
    pub kind: AttachmentKind,
    pub block: ContentBlock,
}

/// Distinguishes image attachments from generic file/resource links
/// so the chip renderer can pick the right icon (and so the
/// composer's `send_prompt_blocks` plumbing can flatten the mixed
/// list deterministically).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    /// `ContentBlock::Image` (base64-encoded body + mime). The image
    /// rendered inline in the user bubble (US-014 + future work).
    Image,
    /// `ContentBlock::ResourceLink` pointing at a path inside the
    /// thread cwd. The agent reads it via `ReadTextFile` (US-018).
    File,
}

/// State of the `@`-mention popup. `None` when no `@`-token is being
/// edited at the cursor.
#[derive(Debug, Clone)]
pub struct MentionState {
    /// Byte offset of the `@` character in the textarea content. The
    /// completion list replaces `content[anchor..cursor]` when a row
    /// is picked.
    pub anchor: usize,
    /// Text after the `@` (case-insensitive substring match).
    pub query: String,
    /// Filesystem walk result. Empty when no scan has completed yet
    /// or when no file matches (AC empty state).
    pub results: Vec<PathBuf>,
    /// When the current query was set. Used to debounce
    /// (`MENTION_DEBOUNCE`).
    pub query_started_at: Instant,
    /// `true` once the debounced walk has run for the current query
    /// (so the renderer can show "No files match" instead of an
    /// empty list while the debounce is still pending).
    pub scanned: bool,
}

/// State of the `/`-slash popup. Same shape as [`MentionState`]
/// minus the debounced filesystem walk (slash commands are static
/// for v1).
#[derive(Debug, Clone)]
pub struct SlashState {
    pub anchor: usize,
    pub query: String,
    pub results: Vec<SlashCommand>,
}

/// Origin of a slash command surfaced in the picker.
///
/// US-112 splits the source so the picker can route the click:
/// built-ins run a local action inside the composer; agent commands
/// are inserted into the textarea and sent verbatim as a
/// `ContentBlock::Text` so the agent handles them natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommandSource {
    /// Paneflow-side command (`/clear`, `/export`). Handled locally
    /// inside [`super::composer::Composer::pick_slash_command`].
    BuiltIn,
    /// Command advertised by the active ACP session via
    /// `session/update` -> `available_commands_update`. Sent verbatim
    /// as `/<name>` to the agent on the next prompt.
    Agent,
}

/// One `/` command surfaced in the picker. The Composer keeps a
/// merged list of [`SlashCommandSource::BuiltIn`] commands +
/// [`SlashCommandSource::Agent`] commands from the active session
/// (US-112). Agent-advertised commands win on name collision.
#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub source: SlashCommandSource,
    /// Argument hint for commands that require user input (e.g.
    /// `"<file>"`). `None` means the command takes no argument and
    /// can be auto-submitted on pick (built-ins always carry `None`).
    pub argument_hint: Option<String>,
}

/// Built-in `/`-commands. Two are exposed by US-019:
/// - `/clear` -- the Composer wipes its in-memory transcript (does
///   NOT delete the thread row on disk; US-006 `delete_thread` is the
///   destructive path).
/// - `/export` -- writes the active thread to a Markdown file via
///   `rfd::AsyncFileDialog`.
///
/// Returns owned values so the merged list with agent commands
/// (US-112) is a single `Vec<SlashCommand>`.
pub fn built_in_slash_commands() -> Vec<SlashCommand> {
    vec![
        SlashCommand {
            name: "clear".to_string(),
            description: "Clear the thread display".to_string(),
            source: SlashCommandSource::BuiltIn,
            argument_hint: None,
        },
        SlashCommand {
            name: "export".to_string(),
            description: "Export thread to markdown".to_string(),
            source: SlashCommandSource::BuiltIn,
            argument_hint: None,
        },
    ]
}

/// US-112: convert an ACP `AvailableCommand` into the composer's
/// picker shape. The `requires_argument` signal comes from the
/// optional `input` field — when present, the hint string is what
/// the picker leaves in the textarea for the user to fill in.
pub fn agent_slash_command_from_acp(
    cmd: &agent_client_protocol::schema::AvailableCommand,
) -> SlashCommand {
    use agent_client_protocol::schema::AvailableCommandInput;
    let argument_hint = match cmd.input.as_ref() {
        Some(AvailableCommandInput::Unstructured(input)) => Some(input.hint.clone()),
        _ => None,
    };
    SlashCommand {
        name: cmd.name.clone(),
        description: cmd.description.clone(),
        source: SlashCommandSource::Agent,
        argument_hint,
    }
}

/// US-112: merge built-in and agent-advertised commands, then filter
/// against `query` (case-insensitive substring match on name).
///
/// Collision policy (AC #6): when an agent command shares a name with
/// a built-in, the agent's version wins and the built-in is filtered
/// out — agent semantics are more authoritative for the current thread
/// context.
pub fn merge_and_filter_slash_commands(
    built_ins: &[SlashCommand],
    agent_commands: &[SlashCommand],
    query: &str,
) -> Vec<SlashCommand> {
    let mut out: Vec<SlashCommand> = Vec::with_capacity(built_ins.len() + agent_commands.len());
    // Agent commands first so the collision check below has them
    // already present.
    out.extend(agent_commands.iter().cloned());
    let agent_names: std::collections::HashSet<&str> =
        agent_commands.iter().map(|c| c.name.as_str()).collect();
    for built_in in built_ins {
        if agent_names.contains(built_in.name.as_str()) {
            continue;
        }
        out.push(built_in.clone());
    }
    if query.is_empty() {
        return out;
    }
    let q = query.to_lowercase();
    out.into_iter()
        .filter(|c| c.name.to_lowercase().contains(&q))
        .collect()
}

/// Locate the in-flight `@` / `/` token at the cursor, if any. Walks
/// backward from `cursor` to find a trigger character that:
/// - sits at the start of a line OR is preceded by whitespace,
/// - has only non-whitespace characters between it and the cursor.
///
/// Returns `(anchor, query)` where `anchor` is the byte offset of the
/// trigger char and `query` is `content[anchor + 1 .. cursor]`.
/// Returns `None` when no active token is being edited.
pub fn token_before_cursor(content: &str, cursor: usize, trigger: char) -> Option<(usize, String)> {
    let cursor = cursor.min(content.len());
    let head = &content[..cursor];
    let mut anchor: Option<usize> = None;
    for (idx, ch) in head.char_indices().rev() {
        if ch == trigger {
            // For `/` we additionally enforce start-of-line: AC says
            // "Typing `/` at the start of a line triggers a slash
            // command popup". For `@` we allow anywhere preceded by
            // whitespace OR start-of-input.
            let prev = head[..idx].chars().next_back();
            let line_start = prev.map(|c| c == '\n').unwrap_or(true);
            let word_start = prev.map(|c| c.is_whitespace()).unwrap_or(true);
            let ok = if trigger == '/' {
                line_start
            } else {
                word_start
            };
            if ok {
                anchor = Some(idx);
            }
            break;
        }
        if ch.is_whitespace() {
            return None;
        }
    }
    let anchor = anchor?;
    let query_start = anchor + trigger.len_utf8();
    let query = content[query_start..cursor].to_string();
    Some((anchor, query))
}

/// Scan `cwd` recursively, gitignore-respecting (via the `ignore`
/// crate), and return up to [`MAX_FILE_RESULTS`] paths whose file
/// component contains `query` (case-insensitive substring).
///
/// Returns paths RELATIVE to `cwd` so the textarea splices in a short
/// chip-like string. Hidden + gitignored files are filtered.
pub fn scan_files(cwd: &Path, query: &str) -> Vec<PathBuf> {
    let query_lc = query.to_lowercase();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut walker = ignore::WalkBuilder::new(cwd);
    // The `ignore` crate's defaults are sensible: it honors
    // .gitignore, .ignore, .git/info/exclude, and hides "hidden"
    // entries. We do not flip any of those off.
    walker.max_depth(Some(8)); // 8 levels is plenty for chat completion
    walker.threads(1); // pump thread already cheap; avoid burning cores
    for result in walker.build() {
        let Ok(entry) = result else {
            continue;
        };
        // Skip directories and non-file entries; the `@`-mention only
        // makes sense for files (the agent can list directories via
        // its own tools).
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel = match path.strip_prefix(cwd) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => continue,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        if !query_lc.is_empty() {
            let name_match = path
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase().contains(&query_lc))
                .unwrap_or(false);
            let path_match = rel
                .to_str()
                .map(|s| s.to_lowercase().contains(&query_lc))
                .unwrap_or(false);
            if !(name_match || path_match) {
                continue;
            }
        }
        out.push(rel);
        if out.len() >= MAX_FILE_RESULTS {
            break;
        }
    }
    // Stable order so the popup does not jitter between identical
    // walks. `ignore` is already deterministic for a single thread,
    // but we sort as a defensive guarantee.
    out.sort();
    out
}

/// Build a `ContentBlock::Image` from raw bytes. Returns `None` if
/// the file would exceed [`MAX_IMAGE_BYTES`] (AC: 10MB cap with a
/// clear error message -- the Composer surfaces the error; this
/// helper just gates the encoding).
pub fn image_block_from_bytes(bytes: &[u8], mime: &str) -> Option<ContentBlock> {
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return None;
    }
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(ContentBlock::Image(ImageContent::new(data, mime)))
}

/// Build a `ContentBlock::ResourceLink` from a path. The `uri` is the
/// `file://` URL form so ACP servers that resolve resource links via
/// MCP get a stable, scheme-prefixed reference; the `name` is the
/// last path component (or the full path when the file is at the cwd
/// root).
pub fn resource_block_for_path(path: &Path) -> ContentBlock {
    let display_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string());
    // `file://` URI keeps round-tripping through MCP resource servers
    // tidy. We do NOT URL-encode every char of the path -- ACP
    // servers we target (`claude-code-acp`, `codex-acp`) read the
    // path verbatim and apply their own resolution.
    let uri = format!("file://{}", path.display());
    ContentBlock::ResourceLink(ResourceLink::new(display_name, uri))
}

/// AC: "Image too large, max 10MB". Plain text helper so the
/// Composer's error row + the unit tests speak the same string.
pub fn image_too_large_message() -> String {
    "Image too large, max 10MB".to_string()
}

/// US-117: invert [`image_block_from_bytes`] for the preview tooltip.
/// Returns `(bytes, mime)` so the caller can wrap them into a
/// `gpui::Image` of the right format. `None` when the block is not an
/// image variant or the base64 payload is malformed (unhappy path --
/// surfaced as the file-only tooltip fallback, never a panic).
pub fn decode_image_block(block: &ContentBlock) -> Option<(Vec<u8>, &str)> {
    let ContentBlock::Image(image) = block else {
        return None;
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image.data.as_bytes())
        .ok()?;
    Some((bytes, image.mime_type.as_str()))
}

/// Heuristic mime type detection from the file extension. Conservative:
/// returns `None` when the extension is missing or unknown so the
/// Composer can fall back to "Attach file" semantics rather than
/// uploading bytes the agent cannot decode.
pub fn detect_image_mime(path: &Path) -> Option<&'static str> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())?;
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// US-114: classification verdict for a single dropped path.
/// `complete_drop_paths` produces one of these per entry before
/// touching the composer state, so the per-file decision is
/// unit-testable without spinning up a GPUI context. Side effects
/// (chip push, error queue) stay on the composer side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropClassification {
    /// Image extension under the 10 MB cap. The composer reads the
    /// bytes again on the main thread and builds the
    /// `ContentBlock::Image`; this verdict only carries the mime
    /// hint so the byte read can pass it back into
    /// [`image_block_from_bytes`].
    Image { mime: &'static str },
    /// Non-image, non-directory entry. The composer wraps it as a
    /// `ResourceLink` regardless of worktree membership (AC #6 --
    /// "the attachment is still accepted as an absolute path").
    File,
    /// Directory entry. Rejected per AC #3; the composer tallies a
    /// single combined toast at the end of the batch.
    DirectoryRejected,
    /// `std::fs::metadata` failed for this entry. The composer
    /// surfaces the OS error to the user.
    Unreadable { reason: String },
}

/// US-114: pure-data per-path classification. Returns the
/// [`DropClassification`] verdict so the composer can apply it.
///
/// Folders are rejected outright (AC #3); image extensions return
/// an `Image { mime }` verdict regardless of file size — the byte
/// read + 10 MB cap happen on the composer side where the
/// in-memory budget is checked once per chip. Anything else falls
/// through to a generic `File` attachment.
pub fn classify_dropped_path(path: &std::path::Path) -> DropClassification {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(err) => {
            return DropClassification::Unreadable {
                reason: err.to_string(),
            };
        }
    };
    if metadata.is_dir() {
        return DropClassification::DirectoryRejected;
    }
    if let Some(mime) = detect_image_mime(path) {
        return DropClassification::Image { mime };
    }
    DropClassification::File
}

/// US-019: convenience used by the Composer's `send_prompt_blocks`
/// path. Combines `text` (the value of the textarea) with the
/// pending attachments into a single block list. Empty text is
/// dropped so attachments-only prompts are valid. Attachments
/// preserve insertion order so the user sees the chips in the same
/// order they appear in the persisted message.
pub fn combine_prompt(text: &str, attachments: &[PendingAttachment]) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = Vec::new();
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        blocks.push(ContentBlock::Text(TextContent::new(text)));
    }
    for att in attachments {
        blocks.push(att.block.clone());
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_before_cursor_finds_at_anchor_after_whitespace() {
        let text = "hello @foo";
        let r = token_before_cursor(text, text.len(), '@');
        assert!(r.is_some());
        let (anchor, query) = r.unwrap();
        assert_eq!(anchor, 6);
        assert_eq!(query, "foo");
    }

    #[test]
    fn token_before_cursor_rejects_at_inside_word() {
        // `email@example.com`: the `@` is not preceded by whitespace
        // so the mention popup must NOT trigger.
        let text = "email@example.com";
        let r = token_before_cursor(text, text.len(), '@');
        assert!(r.is_none());
    }

    #[test]
    fn token_before_cursor_finds_slash_at_line_start() {
        let text = "/cle";
        let r = token_before_cursor(text, text.len(), '/');
        assert!(r.is_some());
        let (anchor, query) = r.unwrap();
        assert_eq!(anchor, 0);
        assert_eq!(query, "cle");
    }

    #[test]
    fn token_before_cursor_rejects_slash_in_middle_of_word() {
        let text = "foo /bar";
        // `/` is preceded by space (a non-newline whitespace), which
        // does NOT count as start of line.
        let r = token_before_cursor(text, text.len(), '/');
        assert!(r.is_none());
    }

    #[test]
    fn token_before_cursor_finds_slash_after_newline() {
        let text = "first line\n/help";
        let r = token_before_cursor(text, text.len(), '/');
        assert!(r.is_some());
        let (anchor, query) = r.unwrap();
        assert_eq!(query, "help");
        assert_eq!(&text[anchor..anchor + 1], "/");
    }

    #[test]
    fn image_block_from_bytes_caps_at_10mb() {
        // 10MB + 1 byte over the cap.
        let bytes = vec![0u8; (MAX_IMAGE_BYTES as usize) + 1];
        assert!(image_block_from_bytes(&bytes, "image/png").is_none());
    }

    #[test]
    fn image_block_from_bytes_encodes_under_cap() {
        let bytes = b"PNGDATA";
        let block = image_block_from_bytes(bytes, "image/png");
        match block {
            Some(ContentBlock::Image(img)) => {
                assert_eq!(img.mime_type, "image/png");
                assert!(!img.data.is_empty());
            }
            _ => panic!("expected Image variant"),
        }
    }

    #[test]
    fn combine_prompt_drops_empty_text() {
        let blocks = combine_prompt("", &[]);
        assert!(blocks.is_empty());
    }

    #[test]
    fn combine_prompt_includes_attachments_after_text() {
        let att = PendingAttachment {
            label: "x".into(),
            kind: AttachmentKind::File,
            block: resource_block_for_path(Path::new("x.txt")),
        };
        let blocks = combine_prompt("hi", std::slice::from_ref(&att));
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t.text, "hi"),
            _ => panic!("text first"),
        }
        match &blocks[1] {
            ContentBlock::ResourceLink(_) => {}
            _ => panic!("attachment second"),
        }
    }

    #[test]
    fn detect_image_mime_known_extensions() {
        assert_eq!(detect_image_mime(Path::new("a.png")), Some("image/png"));
        assert_eq!(detect_image_mime(Path::new("a.JPG")), Some("image/jpeg"));
        assert_eq!(detect_image_mime(Path::new("a.gif")), Some("image/gif"));
        assert_eq!(detect_image_mime(Path::new("a.webp")), Some("image/webp"));
        assert_eq!(detect_image_mime(Path::new("a.svg")), None);
    }

    #[test]
    fn scan_files_filters_query_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("foo.rs"), "// rust").unwrap();
        std::fs::write(tmp.path().join("bar.md"), "# md").unwrap();
        let results = scan_files(tmp.path(), "FOO");
        assert_eq!(results, vec![PathBuf::from("foo.rs")]);
    }

    #[test]
    fn scan_files_returns_empty_when_no_match() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("foo.rs"), "x").unwrap();
        let results = scan_files(tmp.path(), "zzz");
        assert!(results.is_empty());
    }

    /// US-112 AC #3: empty agent commands -> only built-ins surface.
    /// The merge must still produce both built-ins when the agent
    /// hasn't reported any (or reports an empty vec).
    #[test]
    fn slash_merge_empty_agent_returns_built_ins() {
        let built_ins = built_in_slash_commands();
        let merged = merge_and_filter_slash_commands(&built_ins, &[], "");
        let names: Vec<&str> = merged.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["clear", "export"]);
        assert!(
            merged
                .iter()
                .all(|c| matches!(c.source, SlashCommandSource::BuiltIn))
        );
    }

    /// US-112 AC #1: agent commands are merged with built-ins and
    /// both surface in the picker when names don't collide.
    #[test]
    fn slash_merge_non_colliding_agent_commands_concat() {
        let built_ins = built_in_slash_commands();
        let agent = vec![
            SlashCommand {
                name: "init".to_string(),
                description: "Initialize the project".to_string(),
                source: SlashCommandSource::Agent,
                argument_hint: None,
            },
            SlashCommand {
                name: "cost".to_string(),
                description: "Show token cost".to_string(),
                source: SlashCommandSource::Agent,
                argument_hint: None,
            },
        ];
        let merged = merge_and_filter_slash_commands(&built_ins, &agent, "");
        let names: Vec<&str> = merged.iter().map(|c| c.name.as_str()).collect();
        // Agent commands come first (added first in the merge);
        // built-ins follow in their declared order.
        assert_eq!(names, vec!["init", "cost", "clear", "export"]);
    }

    /// US-112 AC #6: on name collision, the agent version wins and
    /// the built-in is filtered out -- agent semantics take precedence
    /// in the active thread context.
    #[test]
    fn slash_merge_collision_agent_wins_over_built_in() {
        let built_ins = built_in_slash_commands();
        let agent = vec![SlashCommand {
            name: "clear".to_string(),
            description: "Clear the agent's own context window".to_string(),
            source: SlashCommandSource::Agent,
            argument_hint: None,
        }];
        let merged = merge_and_filter_slash_commands(&built_ins, &agent, "");
        let clear_entries: Vec<_> = merged.iter().filter(|c| c.name == "clear").collect();
        assert_eq!(clear_entries.len(), 1, "only one /clear should surface");
        assert!(
            matches!(clear_entries[0].source, SlashCommandSource::Agent),
            "agent's /clear must win over the built-in"
        );
        // The built-in /export must still be present (no collision).
        assert!(
            merged
                .iter()
                .any(|c| c.name == "export" && matches!(c.source, SlashCommandSource::BuiltIn))
        );
    }

    /// US-112 AC #1: substring filter is case-insensitive and matches
    /// against the command name only (not description).
    #[test]
    fn slash_merge_query_filters_by_name_substring() {
        let built_ins = built_in_slash_commands();
        let agent = vec![SlashCommand {
            name: "init".to_string(),
            description: "Initialize the project".to_string(),
            source: SlashCommandSource::Agent,
            argument_hint: None,
        }];
        let merged = merge_and_filter_slash_commands(&built_ins, &agent, "EX");
        let names: Vec<&str> = merged.iter().map(|c| c.name.as_str()).collect();
        // Only "export" matches "EX" case-insensitively. "init" does
        // not match even though its description contains "I" since
        // the filter is name-only.
        assert_eq!(names, vec!["export"]);
    }

    /// US-114 AC #1: image extensions get the matching mime hint.
    #[test]
    fn classify_image_extension_returns_image_with_mime() {
        let tmp = tempfile::tempdir().unwrap();
        let png = tmp.path().join("shot.png");
        std::fs::write(&png, b"PNGDATA").unwrap();
        match classify_dropped_path(&png) {
            DropClassification::Image { mime } => assert_eq!(mime, "image/png"),
            other => panic!("expected Image, got {other:?}"),
        }
    }

    /// US-114 AC #1 fallthrough: an unrecognised extension lands as
    /// the generic File verdict; the composer then wraps it as a
    /// `ResourceLink` chip without trying to read the bytes into a
    /// base64 image payload.
    #[test]
    fn classify_unknown_extension_returns_file() {
        let tmp = tempfile::tempdir().unwrap();
        let pdf = tmp.path().join("report.pdf");
        std::fs::write(&pdf, b"%PDF-1.7").unwrap();
        assert_eq!(classify_dropped_path(&pdf), DropClassification::File);
    }

    /// US-114 AC #3: folders must be rejected outright -- the
    /// composer aggregates these into a single "folders skipped"
    /// toast at the end of the batch.
    #[test]
    fn classify_directory_returns_directory_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            classify_dropped_path(tmp.path()),
            DropClassification::DirectoryRejected,
        );
    }

    /// US-114: unreadable paths surface as the explicit Unreadable
    /// verdict so the composer can report the OS error rather than
    /// silently dropping the file.
    #[test]
    fn classify_missing_path_returns_unreadable() {
        let nonexistent = std::path::PathBuf::from("/this/path/does/not/exist/honest.png");
        match classify_dropped_path(&nonexistent) {
            DropClassification::Unreadable { .. } => {}
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    /// US-112 AC #5: argument hint round-trips from the ACP
    /// `AvailableCommandInput::Unstructured` shape into the composer's
    /// `argument_hint` field so the picker can position the cursor.
    #[test]
    fn agent_command_round_trips_argument_hint() {
        use agent_client_protocol::schema::{
            AvailableCommand, AvailableCommandInput, UnstructuredCommandInput,
        };
        let no_input = AvailableCommand::new("help", "Show help");
        let mapped = agent_slash_command_from_acp(&no_input);
        assert!(mapped.argument_hint.is_none());
        assert_eq!(mapped.name, "help");
        assert!(matches!(mapped.source, SlashCommandSource::Agent));

        let with_input = AvailableCommand::new("init", "Initialize a project").input(
            AvailableCommandInput::Unstructured(UnstructuredCommandInput::new("<path>")),
        );
        let mapped = agent_slash_command_from_acp(&with_input);
        assert_eq!(mapped.argument_hint.as_deref(), Some("<path>"));
    }
}
