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
        JumpToPromptPrev,
        JumpToPromptNext,
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
        // Zed-parity (2026-05-25): review actions for AI-applied file
        // edits. KeepEdits accepts every unreviewed change in the
        // current thread; RejectEdits reverts each one on disk
        // (fs::write old_text, or fs::remove_file when old_text is
        // None). Bound to Shift+Alt+Y / Shift+Alt+Z to match Zed's
        // default keymap so muscle memory transfers.
        KeepEdits,
        RejectEdits
    ]
);
