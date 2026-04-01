import {
  type Component,
  For,
  Show,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import {
  type Workspace,
  workspaceState,
  addWorkspace,
  closeWorkspace,
  selectWorkspace,
  reorderWorkspace,
  renameWorkspace,
  togglePin,
  closeOtherWorkspaces,
  displayTitle,
  paneCount,
} from "../stores/workspace";

// ── Context Menu ────────────────────────────────────────────────────────

interface ContextMenuProps {
  x: number;
  y: number;
  workspace: Workspace;
  onClose: () => void;
  onRename: (id: string) => void;
}

const ContextMenu: Component<ContextMenuProps> = (props) => {
  let menuRef: HTMLDivElement | undefined;

  onMount(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef && !menuRef.contains(e.target as Node)) props.onClose();
    };
    document.addEventListener("mousedown", handler);
    onCleanup(() => document.removeEventListener("mousedown", handler));
  });

  const item = (label: string, action: () => void, danger = false) => (
    <button
      class="ctx-item"
      classList={{ "ctx-item--danger": danger }}
      onClick={() => {
        action();
        props.onClose();
      }}
    >
      {label}
    </button>
  );

  return (
    <div ref={menuRef} class="ctx-menu" style={{ left: `${props.x}px`, top: `${props.y}px` }}>
      {item("Rename Workspace", () => props.onRename(props.workspace.id))}
      {item(
        props.workspace.isPinned ? "Unpin Workspace" : "Pin Workspace",
        () => togglePin(props.workspace.id),
      )}
      <div class="ctx-sep" />
      {item("Close Other Workspaces", () => closeOtherWorkspaces(props.workspace.id))}
      {item("Close Workspace", () => closeWorkspace(props.workspace.id), true)}
    </div>
  );
};

// ── Sidebar Item ────────────────────────────────────────────────────────

interface SidebarItemProps {
  workspace: Workspace;
  index: number;
  isActive: boolean;
  isRenaming: boolean;
  onStartRename: () => void;
  onFinishRename: (title: string) => void;
  onCancelRename: () => void;
}

const SidebarItem: Component<SidebarItemProps> = (props) => {
  let inputRef: HTMLInputElement | undefined;
  const [dragOver, setDragOver] = createSignal(false);

  const title = () => displayTitle(props.workspace);
  const panes = () => paneCount(props.workspace);
  const hint = () => (props.index < 9 ? String(props.index + 1) : "");

  return (
    <div
      class="sb-item"
      classList={{
        "sb-item--active": props.isActive,
        "sb-item--drop": dragOver(),
      }}
      onClick={() => selectWorkspace(props.workspace.id)}
      onDblClick={(e) => {
        e.preventDefault();
        props.onStartRename();
        requestAnimationFrame(() => {
          inputRef?.focus();
          inputRef?.select();
        });
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        document.dispatchEvent(
          new CustomEvent("pf:ctx", {
            detail: { x: e.clientX, y: e.clientY, ws: props.workspace },
          }),
        );
      }}
      draggable={!props.isRenaming}
      onDragStart={(e) => {
        e.dataTransfer?.setData("text/plain", props.workspace.id);
        if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
      }}
      onDragOver={(e) => {
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragOver(false);
        const id = e.dataTransfer?.getData("text/plain");
        if (id && id !== props.workspace.id) reorderWorkspace(id, props.index);
      }}
    >
      {/* Left accent rail */}
      <div
        class="sb-item__rail"
        classList={{ "sb-item__rail--visible": props.isActive }}
        style={props.workspace.color ? { background: props.workspace.color } : undefined}
      />

      <div class="sb-item__body">
        <div class="sb-item__top">
          <Show when={props.workspace.isPinned}>
            <svg class="sb-item__pin" viewBox="0 0 16 16" width="12" height="12">
              <path
                fill="currentColor"
                d="M9.828.722a.5.5 0 0 1 .354.146l4.95 4.95a.5.5 0 0 1-.707.707l-.71-.71-3.18 3.18a3.01 3.01 0 0 1-.39 3.14l-.708.708a.5.5 0 0 1-.707 0L6.5 10.61l-3.646 3.647a.5.5 0 1 1-.708-.708L5.793 9.9 3.535 7.643a.5.5 0 0 1 0-.708l.707-.707a3.01 3.01 0 0 1 3.14-.39l3.18-3.18-.71-.71a.5.5 0 0 1 .147-.854z"
              />
            </svg>
          </Show>

          <Show
            when={!props.isRenaming}
            fallback={
              <input
                ref={inputRef}
                class="sb-item__input"
                value={title()}
                onKeyDown={(e) => {
                  if (e.key === "Enter") props.onFinishRename(e.currentTarget.value);
                  else if (e.key === "Escape") props.onCancelRename();
                }}
                onBlur={(e) => props.onFinishRename(e.currentTarget.value)}
                onClick={(e) => e.stopPropagation()}
              />
            }
          >
            <span class="sb-item__title">{title()}</span>
          </Show>

          <Show when={hint()}>
            <span class="sb-item__hint">{hint()}</span>
          </Show>

          <button
            class="sb-item__close"
            title={props.workspace.isPinned ? "Pinned" : "Close"}
            onClick={(e) => {
              e.stopPropagation();
              if (!props.workspace.isPinned) closeWorkspace(props.workspace.id);
            }}
          >
            <svg viewBox="0 0 12 12" width="10" height="10">
              <path
                fill="currentColor"
                d="M2.22 2.22a.75.75 0 0 1 1.06 0L6 4.94l2.72-2.72a.75.75 0 1 1 1.06 1.06L7.06 6l2.72 2.72a.75.75 0 1 1-1.06 1.06L6 7.06 3.28 9.78a.75.75 0 0 1-1.06-1.06L4.94 6 2.22 3.28a.75.75 0 0 1 0-1.06z"
              />
            </svg>
          </button>
        </div>

        <div class="sb-item__meta">
          <span class="sb-item__dir">{props.workspace.workingDirectory}</span>
          <Show when={panes() > 1}>
            <span class="sb-item__panes">
              <svg viewBox="0 0 16 16" width="11" height="11">
                <path
                  fill="currentColor"
                  d="M2.5 0A2.5 2.5 0 0 0 0 2.5v11A2.5 2.5 0 0 0 2.5 16h11a2.5 2.5 0 0 0 2.5-2.5v-11A2.5 2.5 0 0 0 13.5 0h-11zM1 2.5A1.5 1.5 0 0 1 2.5 1H7v14H2.5A1.5 1.5 0 0 1 1 13.5v-11zM8 15V1h5.5A1.5 1.5 0 0 1 15 2.5v11a1.5 1.5 0 0 1-1.5 1.5H8z"
                />
              </svg>
              {panes()}
            </span>
          </Show>
        </div>
      </div>
    </div>
  );
};

// ── Sidebar ─────────────────────────────────────────────────────────────

const Sidebar: Component = () => {
  const [ctxMenu, setCtxMenu] = createSignal<{
    x: number;
    y: number;
    workspace: Workspace;
  } | null>(null);
  const [renamingId, setRenamingId] = createSignal<string | null>(null);

  onMount(() => {
    const onCtx = (e: Event) => {
      const d = (e as CustomEvent).detail;
      setCtxMenu({ x: d.x, y: d.y, workspace: d.ws });
    };
    document.addEventListener("pf:ctx", onCtx);
    onCleanup(() => document.removeEventListener("pf:ctx", onCtx));
  });

  return (
    <div class="sidebar">
      <div class="sidebar__head">
        <span class="sidebar__logo">PaneFlow</span>
      </div>

      <div class="sidebar__list">
        <For each={workspaceState.workspaces}>
          {(ws, i) => (
            <SidebarItem
              workspace={ws}
              index={i()}
              isActive={ws.id === workspaceState.selectedId}
              isRenaming={renamingId() === ws.id}
              onStartRename={() => setRenamingId(ws.id)}
              onFinishRename={(t) => {
                renameWorkspace(ws.id, t);
                setRenamingId(null);
              }}
              onCancelRename={() => setRenamingId(null)}
            />
          )}
        </For>
      </div>

      <div class="sidebar__foot">
        <button class="sidebar__add" onClick={() => addWorkspace()}>
          <svg viewBox="0 0 16 16" width="14" height="14">
            <path
              fill="currentColor"
              d="M8 2a.75.75 0 0 1 .75.75v4.5h4.5a.75.75 0 0 1 0 1.5h-4.5v4.5a.75.75 0 0 1-1.5 0v-4.5h-4.5a.75.75 0 0 1 0-1.5h4.5v-4.5A.75.75 0 0 1 8 2z"
            />
          </svg>
          New Workspace
        </button>
      </div>

      <Show when={ctxMenu()}>
        {(m) => (
          <ContextMenu
            x={m().x}
            y={m().y}
            workspace={m().workspace}
            onClose={() => setCtxMenu(null)}
            onRename={(id) => {
              setCtxMenu(null);
              setRenamingId(id);
            }}
          />
        )}
      </Show>
    </div>
  );
};

export default Sidebar;
