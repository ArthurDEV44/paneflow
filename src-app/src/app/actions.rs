//! GPUI action types dispatched through the focus chain.
//!
//! The `actions!` macro generates zero-sized types in the enclosing module,
//! all publicly visible, and registers them under the `paneflow` namespace
//! for JSON dispatch via `cx.dispatch_action`.

use gpui::actions;

actions!(
    paneflow,
    [
        SplitHorizontally,
        SplitVertically,
        ClosePane,
        NewTab,
        CloseTab,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        JumpNextWaiting,
        NewWorkspace,
        CloseWorkspace,
        CopyWorkspacePath,
        RevealWorkspaceInFileManager,
        OpenWorkspaceInZed,
        OpenWorkspaceInCursor,
        OpenWorkspaceInVsCode,
        OpenWorkspaceInWindsurf,
        NextWorkspace,
        SelectWorkspace1,
        SelectWorkspace2,
        SelectWorkspace3,
        SelectWorkspace4,
        SelectWorkspace5,
        SelectWorkspace6,
        SelectWorkspace7,
        SelectWorkspace8,
        SelectWorkspace9,
        TerminalCopy,
        TerminalPaste,
        ScrollPageUp,
        ScrollPageDown,
        CloseWindow,
        ToggleZoom,
        LayoutEvenHorizontal,
        LayoutEvenVertical,
        LayoutMainVertical,
        LayoutTiled,
        SplitEqualize,
        SwapPane,
        ToggleSearch,
        ToggleSearchRegex,
        UndoClosePane,
        SearchNext,
        SearchPrev,
        DismissSearch,
        ToggleCopyMode,
        ClearScrollHistory,
        ResetTerminal,
        StartSelfUpdate,
        /// US-007 AC3: dismiss the update pill for the current launch
        /// (no persistence — re-prompts on next start). Dispatched by
        /// the `×` button on the Idle / Errored pill states.
        DismissUpdate,
        // US-012: macOS native menu-bar actions. Dispatched by `cx.set_menus`
        // via GPUI's `on_app_menu_action` → `cx.dispatch_action`, then caught
        // by the `.on_action(...)` handlers on the PaneFlowApp render root.
        Quit,
        About,
        Copy,
        Paste,
        SelectAll,
        OpenHelp,
        // US-022 (cmux port 2026-Q2) — markdown pane navigation. Scoped to
        // the `Markdown` key context (root) and `MarkdownSearch` (when the
        // find overlay is open). Defined as separate actions from terminal
        // scroll/copy so the keybinding registry can scope them cleanly.
        MarkdownScrollPageUp,
        MarkdownScrollPageDown,
        MarkdownFindOpen,
        MarkdownFindNext,
        MarkdownFindPrev,
        MarkdownFindDismiss,
        MarkdownCopy,
        // US-005 of tasks/prd-agents-view.md — toggles the lightweight
        // Agents-view shell that hosts the auth-required card and the
        // missing-agents empty state. US-008 will repurpose the same
        // action as the full AppMode toggle.
        OpenAgentsView,
        // US-003 of tasks/prd-multi-worktree-diff-2026-Q3.md — open the
        // multi-worktree diff view for the active workspace's repo. Resolves
        // the repo from `active_idx`'s `repo_root` and opens a `DiffView` tab
        // seeded with every sibling worktree. Also invoked directly by the
        // sidebar group header's "Diff all" button.
        OpenMultiDiff,
        // US-003 of tasks/prd-git-diff-mode-2026-Q3.md — toggle the
        // dedicated Git Diff mode (AppMode::Diff): a full-screen diff
        // surface entered via the CLI / Diff / Agents sidebar toggle.
        // Distinct from `OpenMultiDiff` (the ephemeral tab path), which
        // stays alive as a secondary entry.
        OpenDiffView,
        // US-003 of tasks/prd-ai-in-diff-2026-Q3.md — copy the hunk under the
        // cursor as a unified diff (Ctrl+Shift+C inside the DiffView context).
        CopyDiffHunk,
        // US-011 of tasks/prd-agents-ui-codex-redesign-2026-Q3.md — open the
        // overflow (`⋯`) menu for the current Agents thread/chat. Dispatched
        // by the title-bar `⋯` button (a SEPARATE `TitleBar` entity with no
        // access to agents state), caught by `PaneFlowApp` which resolves the
        // selected target and opens the shared thread context menu. The
        // button never calls agents methods directly — it dispatches this
        // typed action, mirroring the update pill's `StartSelfUpdate`.
        OpenAgentsThreadMenu
    ]
);
