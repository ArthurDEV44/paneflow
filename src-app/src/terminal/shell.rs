//! Shell resolution + automatic OSC 7 injection.
//!
//! `resolve_default_shell` picks the shell binary to launch in every PTY,
//! following a platform-specific fallback chain. `setup_shell_integration`
//! writes small per-shell rc scripts into `$XDG_DATA_HOME/paneflow/shell/`
//! (or `%APPDATA%\paneflow\shell\` on Windows) and returns the extra CLI
//! args/env needed to wire them in.
//!
//! Keep this module shell-specific: no terminal state, no GPUI.

use std::collections::HashMap;

/// zsh: ZDOTDIR-based injection. Our `.zshenv` restores the original ZDOTDIR
/// so all other dotfiles (`.zshrc`, `.zprofile`) load from `$HOME` as usual.
///
/// AI-hook PATH-prepend (re-applied via `precmd`): the PTY-level
/// `$PATH` prepend in `pty_session::inject_ai_hook_env` is invariably
/// undone by user `.zshrc`/`.bashrc` lines like
/// `export PATH="$HOME/.local/bin:$PATH"`, which demote PaneFlow's bin
/// dir behind the user's `~/.local/bin/claude` and bypass the shim
/// entirely. We re-prepend before every prompt — first invocation runs
/// after `.zshrc` finishes, so the first `claude` typed at the prompt
/// resolves to the shim. Idempotent + O(1) string work, invisible cost.
const ZSH_OSC7: &str = r#"# PaneFlow shell integration — OSC 7 CWD reporting
if [[ -n "${PANEFLOW_ORIG_ZDOTDIR+x}" ]]; then
    ZDOTDIR="${PANEFLOW_ORIG_ZDOTDIR}"
    unset PANEFLOW_ORIG_ZDOTDIR
else
    unset ZDOTDIR
fi
[[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]] && source "${ZDOTDIR:-$HOME}/.zshenv"
__paneflow_osc7() { printf '\e]7;file://%s%s\a' "${HOST}" "${PWD}"; }
__paneflow_path_prepend() {
    [[ -z "${PANEFLOW_BIN_DIR-}" ]] && return
    # Strip every existing occurrence then prepend, keeping our dir first
    # regardless of what `.zshrc`/`.zprofile` did. Uses zsh's `path` tied
    # array so the change propagates to `$PATH` automatically.
    path=("${PANEFLOW_BIN_DIR}" "${(@)path:#${PANEFLOW_BIN_DIR}}")
}
autoload -Uz add-zsh-hook
add-zsh-hook chpwd __paneflow_osc7
add-zsh-hook precmd __paneflow_path_prepend
__paneflow_osc7
__paneflow_path_prepend
"#;

/// bash: `--rcfile` replacement. Sources the real `.bashrc`, then appends
/// our OSC 7 function to PROMPT_COMMAND (preserving starship/oh-my-bash/etc.).
/// Same AI-hook PATH-prepend rationale as ZSH_OSC7 — PROMPT_COMMAND fires
/// before each prompt, after `.bashrc` has run.
const BASH_OSC7: &str = r#"# PaneFlow shell integration — OSC 7 CWD reporting
[[ -f ~/.bashrc ]] && source ~/.bashrc
__paneflow_osc7() { printf '\e]7;file://%s%s\a' "${HOSTNAME}" "${PWD}"; }
__paneflow_path_prepend() {
    [[ -z "${PANEFLOW_BIN_DIR-}" ]] && return
    local p=":${PATH}:"
    p="${p//:${PANEFLOW_BIN_DIR}:/:}"
    p="${p#:}"; p="${p%:}"
    PATH="${PANEFLOW_BIN_DIR}:${p}"
    export PATH
}
PROMPT_COMMAND="__paneflow_osc7;__paneflow_path_prepend${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
__paneflow_path_prepend
"#;

/// fish: `--init-command` sourced script. Uses `--on-variable PWD` so it
/// fires on every directory change independently of the prompt function.
/// fish `--init-command` runs AFTER `config.fish`, so a one-shot prepend
/// is sufficient — but `fish_add_path -gp` is idempotent so a re-source
/// of this file is also safe.
const FISH_OSC7: &str = r#"# PaneFlow shell integration — OSC 7 CWD reporting
function __paneflow_osc7 --on-variable PWD
    printf '\e]7;file://%s%s\a' (hostname) "$PWD"
end
__paneflow_osc7
if set -q PANEFLOW_BIN_DIR; and test -n "$PANEFLOW_BIN_DIR"
    fish_add_path -gp $PANEFLOW_BIN_DIR
end
"#;

/// PowerShell 5.1 / 7 (pwsh): dot-sourced via `-NoExit -Command ". <path>"`,
/// which runs AFTER the user's `$PROFILE`, so any `prompt` function they
/// defined is already in place. We capture it as a ScriptBlock and wrap it
/// non-destructively so their prompt still renders while we emit OSC 7.
///
/// BEL terminator (``a``) matches the zsh/bash/fish emitters so PaneFlow's
/// shared OSC 7 parser handles Windows and Unix identically.
///
/// US-012 — prd-windows-port.md.
const PWSH_OSC7: &str = r#"# PaneFlow shell integration - OSC 7 CWD reporting (US-012)
# Non-destructive: wraps the existing `prompt` function so the user's
# prompt still renders. Loaded via `pwsh -NoExit -Command ". <this>"`.
# Dot-sourcing happens AFTER $PROFILE, so any user PATH mutations there
# have already run -- a one-shot prepend is sufficient. The `prompt`
# wrapper additionally re-asserts the prepend on every prompt for users
# who modify $env:PATH at runtime.

$__paneflow_prev_prompt = Get-Item function:prompt
function global:__paneflow_path_prepend {
    if ([string]::IsNullOrEmpty($env:PANEFLOW_BIN_DIR)) { return }
    $sep = [System.IO.Path]::PathSeparator
    $entries = $env:PATH -split [regex]::Escape($sep) | Where-Object { $_ -ne $env:PANEFLOW_BIN_DIR }
    $env:PATH = (@($env:PANEFLOW_BIN_DIR) + $entries) -join $sep
}
function global:prompt {
    $cwd = (Get-Location).ProviderPath
    # OSC 7 with BEL terminator (matches zsh/bash/fish emitters).
    [Console]::Write("`e]7;file://$env:COMPUTERNAME$cwd`a")
    __paneflow_path_prepend
    & $__paneflow_prev_prompt.ScriptBlock
}
__paneflow_path_prepend
"#;

/// Resolve the default shell path following a platform-specific fallback chain
/// (US-006 — prd-windows-port.md). Returns the path that should be passed to
/// `portable-pty`'s `CommandBuilder::new`.
///
/// Unix chain: configured (if executable) → `$SHELL` → `/bin/sh`.
/// Windows chain: configured (if present, resolved via PATH when it has no
/// separators) → `%ComSpec%` → `C:\Windows\System32\cmd.exe` → `powershell.exe`
/// on PATH → bare `"cmd.exe"` (last-ditch; the spawner will search PATH and
/// surface a clearly-located error if even this fails).
pub(super) fn resolve_default_shell(configured: Option<&str>) -> String {
    if let Some(path) = configured {
        if let Some(resolved) = configured_shell_if_usable(path) {
            return resolved;
        }
        log::warn!(
            "Configured default_shell {:?} not found or not executable, \
             falling back to platform defaults",
            path
        );
    }
    resolve_default_shell_fallback()
}

/// Validate that a user-configured shell entry resolves to an executable file.
/// Bare names (no path separators) are searched on PATH via `which` — this is
/// what lets `"default_shell": "pwsh.exe"` work on Windows without the user
/// having to hard-code `C:\Program Files\PowerShell\7\pwsh.exe`.
fn configured_shell_if_usable(path: &str) -> Option<String> {
    let has_separator = path.contains('/') || path.contains('\\');
    let candidate: std::path::PathBuf = if has_separator {
        std::path::PathBuf::from(path)
    } else {
        which::which(path).ok()?
    };
    let is_executable = candidate.is_file() && {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(&candidate)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }
        #[cfg(windows)]
        {
            std::fs::metadata(&candidate).is_ok()
        }
    };
    if is_executable {
        Some(candidate.to_string_lossy().into_owned())
    } else {
        None
    }
}

#[cfg(unix)]
fn resolve_default_shell_fallback() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

#[cfg(windows)]
fn resolve_default_shell_fallback() -> String {
    // %ComSpec% — Windows convention for "the command interpreter",
    // respected by every console app on the platform.
    if let Ok(com_spec) = std::env::var("ComSpec")
        && std::path::Path::new(&com_spec).is_file()
    {
        return com_spec;
    }
    // Canonical cmd.exe location (works on every supported Windows since
    // 10 1809; we pin the 64-bit System32 path — WOW64 users still see
    // cmd.exe there via redirection).
    const CMD_FALLBACK: &str = r"C:\Windows\System32\cmd.exe";
    if std::path::Path::new(CMD_FALLBACK).is_file() {
        return CMD_FALLBACK.to_string();
    }
    // PowerShell 5.1 (bundled with Windows) or pwsh.exe (PowerShell 7) —
    // `which` appends PATHEXT extensions when resolving.
    if let Ok(pwsh) = which::which("powershell.exe") {
        return pwsh.to_string_lossy().into_owned();
    }
    // Last-ditch: return bare "cmd.exe" and let the spawner search PATH.
    log::error!(
        "Windows shell fallback chain exhausted: %ComSpec%, C:\\Windows\\System32\\cmd.exe, \
         and powershell.exe on PATH all unavailable. Falling back to bare 'cmd.exe'; \
         PTY spawn will fail with a clear error if even this is missing."
    );
    "cmd.exe".to_string()
}

/// Write OSC 7 shell integration scripts and return the extra shell args
/// and env vars needed to activate them. Scripts are written to
/// `$XDG_DATA_HOME/paneflow/shell/{zsh,bash,fish,pwsh}/` (`%APPDATA%\paneflow\shell\`
/// on Windows).
///
/// Supported shells:
/// - **zsh, bash, fish** — BEL-terminated OSC 7 via per-prompt hooks.
/// - **PowerShell 5.1 / pwsh 7** (US-012) — `prompt` function wrapper,
///   dot-sourced so the user's `$PROFILE`-defined prompt still renders.
/// - **cmd.exe** — `info!` log only; cmd has no per-prompt scripting hook,
///   so split-pane CWD inheritance from a cmd.exe pane is v1-unsupported
///   (documented in `docs/WINDOWS.md` per US-022).
/// - **Shells without injection** (nushell, elvish, xonsh): rely on
///   `cwd_now()` fallback. On macOS this requires `proc_pidinfo()`.
pub(super) fn setup_shell_integration(
    shell: &str,
    env: &mut HashMap<String, String>,
) -> Vec<String> {
    let Some(base) = dirs::data_dir().map(|d| d.join("paneflow").join("shell")) else {
        return vec![];
    };

    // US-006 — `Path::file_name()` is path-separator-agnostic:
    //   /bin/zsh  → "zsh"      (Unix)
    //   C:\Windows\System32\cmd.exe → "cmd.exe"  (Windows)
    //   zsh (bare) → "zsh"     (either platform)
    let basename = std::path::Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(shell);
    // US-012 — normalize for case-insensitive match + optional `.exe`
    // suffix. Windows allows `pwsh` and `pwsh.exe` interchangeably on
    // PATH; Unix shell names (lowercase, no suffix) are unaffected.
    let normalized = basename.to_ascii_lowercase();
    let key = normalized.trim_end_matches(".exe");
    match key {
        "zsh" => {
            let dir = base.join("zsh");
            if std::fs::create_dir_all(&dir).is_err() {
                return vec![];
            }
            let _ = std::fs::write(dir.join(".zshenv"), ZSH_OSC7);
            if let Ok(orig) = std::env::var("ZDOTDIR") {
                env.insert("PANEFLOW_ORIG_ZDOTDIR".into(), orig);
            }
            env.insert("ZDOTDIR".into(), dir.display().to_string());
            vec![]
        }
        "bash" => {
            let dir = base.join("bash");
            if std::fs::create_dir_all(&dir).is_err() {
                return vec![];
            }
            let rcfile = dir.join("bashrc");
            let _ = std::fs::write(&rcfile, BASH_OSC7);
            vec!["--rcfile".into(), rcfile.display().to_string()]
        }
        "fish" => {
            let dir = base.join("fish");
            if std::fs::create_dir_all(&dir).is_err() {
                return vec![];
            }
            let initfile = dir.join("osc7.fish");
            let _ = std::fs::write(&initfile, FISH_OSC7);
            vec![
                "--init-command".into(),
                format!("source {}", initfile.display()),
            ]
        }
        // US-012 — PowerShell 7 (pwsh) and Windows PowerShell 5.1 share
        // the same `function prompt { ... }` hook mechanism, so one
        // script serves both. `-NoExit` keeps the shell interactive after
        // the init command; `-Command ". 'path'"` dot-sources our script
        // AFTER the user's `$PROFILE` has loaded any `prompt` they
        // defined (so we can wrap rather than replace it).
        "pwsh" | "powershell" => {
            let dir = base.join("pwsh");
            if std::fs::create_dir_all(&dir).is_err() {
                return vec![];
            }
            let initfile = dir.join("osc7.ps1");
            let _ = std::fs::write(&initfile, PWSH_OSC7);
            // Single-quote the path and escape any embedded single
            // quotes ('' is the literal single-quote inside a single-
            // quoted PowerShell string). Guards against pathological
            // usernames without breaking the common case.
            let escaped = initfile.display().to_string().replace('\'', "''");
            vec![
                "-NoExit".into(),
                "-Command".into(),
                format!(". '{escaped}'"),
            ]
        }
        // US-012 AC-5 — cmd.exe has no scripting hook for per-prompt
        // actions (its `$PROMPT` env var controls only the displayed
        // text, not arbitrary execution). Split-pane CWD inheritance
        // from cmd.exe panes is v1-unsupported; users can `cd` manually
        // or switch to PowerShell for the integrated experience.
        "cmd" => {
            log::info!(
                "paneflow: cmd.exe has no OSC 7 scripting hook; split-pane CWD \
                 inheritance from cmd.exe panes is v1-unsupported (docs/WINDOWS.md)"
            );
            vec![]
        }
        _ => vec![],
    }
}
