import { For, type Component } from "solid-js";
import {
  workspaceState,
  selectWorkspace,
  addWorkspace,
  closeWorkspace,
} from "../stores/workspace";

const Sidebar: Component = () => {
  return (
    <div class="sidebar">
      <h2>PaneFlow</h2>

      <div class="workspace-list">
        <For each={workspaceState.workspaces}>
          {(ws) => (
            <div
              class="workspace-item"
              classList={{ "workspace-item--active": ws.id === workspaceState.selectedId }}
              onClick={() => selectWorkspace(ws.id)}
            >
              <span class="workspace-item__title">{ws.title}</span>
              <button
                class="workspace-item__close"
                onClick={(e) => {
                  e.stopPropagation();
                  closeWorkspace(ws.id);
                }}
                aria-label={`Close ${ws.title}`}
              >
                &times;
              </button>
            </div>
          )}
        </For>
      </div>

      <button class="sidebar__add-btn" onClick={() => addWorkspace()}>
        + New Workspace
      </button>
    </div>
  );
};

export default Sidebar;
