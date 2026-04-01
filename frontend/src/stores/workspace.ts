import { createStore } from "solid-js/store";

export interface Workspace {
  id: string;
  title: string;
  customTitle?: string;
  workingDirectory: string;
}

export interface WorkspaceState {
  workspaces: Workspace[];
  selectedId: string | null;
}

function createDefaultWorkspace(): Workspace {
  return {
    id: crypto.randomUUID(),
    title: "Workspace 1",
    workingDirectory: "~",
  };
}

const defaultWs = createDefaultWorkspace();

const [state, setState] = createStore<WorkspaceState>({
  workspaces: [defaultWs],
  selectedId: defaultWs.id,
});

function nextTitle(): string {
  const existing = state.workspaces.map((ws) => {
    const match = ws.title.match(/^Workspace (\d+)$/);
    return match ? parseInt(match[1], 10) : 0;
  });
  const max = existing.length > 0 ? Math.max(...existing) : 0;
  return `Workspace ${max + 1}`;
}

export function addWorkspace(): void {
  const ws: Workspace = {
    id: crypto.randomUUID(),
    title: nextTitle(),
    workingDirectory: "~",
  };
  setState("workspaces", (prev) => [...prev, ws]);
  setState("selectedId", ws.id);
}

export function closeWorkspace(id: string): void {
  const idx = state.workspaces.findIndex((ws) => ws.id === id);
  if (idx === -1) return;

  const remaining = state.workspaces.filter((ws) => ws.id !== id);

  if (remaining.length === 0) {
    // Last workspace closed -- auto-create a default
    const ws = createDefaultWorkspace();
    setState("workspaces", [ws]);
    setState("selectedId", ws.id);
    return;
  }

  setState("workspaces", remaining);

  // If the closed workspace was selected, select an adjacent one
  if (state.selectedId === id) {
    const nextIdx = Math.min(idx, remaining.length - 1);
    setState("selectedId", remaining[nextIdx].id);
  }
}

export function selectWorkspace(id: string): void {
  const exists = state.workspaces.some((ws) => ws.id === id);
  if (exists) {
    setState("selectedId", id);
  }
}

export function reorderWorkspace(id: string, newIndex: number): void {
  const currentIdx = state.workspaces.findIndex((ws) => ws.id === id);
  if (currentIdx === -1) return;

  const clampedIndex = Math.max(0, Math.min(newIndex, state.workspaces.length - 1));
  if (currentIdx === clampedIndex) return;

  const updated = [...state.workspaces];
  const [moved] = updated.splice(currentIdx, 1);
  updated.splice(clampedIndex, 0, moved);
  setState("workspaces", updated);
}

export { state as workspaceState };
