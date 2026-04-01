import type { Component } from "solid-js";
import { Show, onMount, onCleanup } from "solid-js";
import Sidebar from "./components/Sidebar";
import SplitLayout from "./components/SplitLayout";
import { workspaceState, splitPane, selectWorkspaceByIndex } from "./stores/workspace";

const App: Component = () => {
  const selectedWorkspace = () =>
    workspaceState.workspaces.find(
      (ws) => ws.id === workspaceState.selectedId,
    );

  // ── Keyboard shortcuts ──────────────────────────────────────────────
  const handleKeyDown = (e: KeyboardEvent) => {
    const ws = selectedWorkspace();
    if (!ws || !ws.focusedPaneId) return;

    // Ctrl+Shift+D → split right (horizontal)
    if (e.ctrlKey && e.shiftKey && e.key === "D") {
      e.preventDefault();
      splitPane(ws.id, ws.focusedPaneId, "horizontal");
      return;
    }

    // Ctrl+Shift+E → split down (vertical)
    if (e.ctrlKey && e.shiftKey && e.key === "E") {
      e.preventDefault();
      splitPane(ws.id, ws.focusedPaneId, "vertical");
      return;
    }

    // Ctrl+1-9 → switch workspace by index
    if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key >= "1" && e.key <= "9") {
      e.preventDefault();
      selectWorkspaceByIndex(parseInt(e.key, 10) - 1);
      return;
    }
  };

  onMount(() => {
    document.addEventListener("keydown", handleKeyDown);
  });

  onCleanup(() => {
    document.removeEventListener("keydown", handleKeyDown);
  });

  return (
    <div class="app">
      <Sidebar />
      <div class="main-area">
        <Show
          when={selectedWorkspace()}
          fallback={
            <div class="workspace-view">
              <p>No workspace selected</p>
            </div>
          }
        >
          {(ws) => (
            <SplitLayout
              node={ws().splitTree}
              workspaceId={ws().id}
              focusedPaneId={ws().focusedPaneId}
            />
          )}
        </Show>
      </div>
    </div>
  );
};

export default App;
