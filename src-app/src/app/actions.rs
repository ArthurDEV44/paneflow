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
        MarkdownCopy
    ]
);
