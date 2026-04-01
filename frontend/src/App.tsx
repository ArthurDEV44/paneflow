import type { Component } from "solid-js";
import { Show } from "solid-js";
import Sidebar from "./components/Sidebar";
import { workspaceState } from "./stores/workspace";

const WorkspaceView: Component = () => {
  const selected = () =>
    workspaceState.workspaces.find((ws) => ws.id === workspaceState.selectedId);

  return (
    <div class="workspace-view">
      <Show when={selected()} fallback={<p>No workspace selected</p>}>
        {(ws) => (
          <>
            <h1 class="workspace-view__title">{ws().title}</h1>
            <p class="workspace-view__hint">Terminal panes will render here</p>
          </>
        )}
      </Show>
    </div>
  );
};

const App: Component = () => {
  return (
    <div class="app">
      <Sidebar />
      <div class="main-area">
        <WorkspaceView />
      </div>
    </div>
  );
};

export default App;
