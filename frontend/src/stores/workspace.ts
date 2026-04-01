import { createStore, produce } from "solid-js/store";

// ── Split Tree Types ────────────────────────────────────────────────────

export type Direction = "horizontal" | "vertical";

export type SplitNode =
  | { type: "leaf"; paneId: string }
  | {
      type: "split";
      direction: Direction;
      ratio: number;
      first: SplitNode;
      second: SplitNode;
    };

// ── Workspace Types ─────────────────────────────────────────────────────

export interface Workspace {
  id: string;
  title: string;
  customTitle?: string;
  workingDirectory: string;
  isPinned: boolean;
  color?: string;
  splitTree: SplitNode;
  focusedPaneId: string | null;
}

export interface WorkspaceState {
  workspaces: Workspace[];
  selectedId: string | null;
}

// ── Helpers ─────────────────────────────────────────────────────────────

type LeafNode = SplitNode & { type: "leaf" };

function createLeaf(): LeafNode {
  return { type: "leaf", paneId: crypto.randomUUID() };
}

function createDefaultWorkspace(): Workspace {
  const leaf = createLeaf();
  return {
    id: crypto.randomUUID(),
    title: "Workspace 1",
    workingDirectory: "~",
    isPinned: false,
    splitTree: leaf,
    focusedPaneId: leaf.paneId,
  };
}

/**
 * Recursively find and replace a node in the split tree.
 * Returns a new tree with the replacement applied, or null if the target
 * was not found.
 */
function replaceNode(
  tree: SplitNode,
  targetPaneId: string,
  replacer: (node: SplitNode) => SplitNode,
): SplitNode | null {
  if (tree.type === "leaf") {
    if (tree.paneId === targetPaneId) {
      return replacer(tree);
    }
    return null;
  }

  const firstResult = replaceNode(tree.first, targetPaneId, replacer);
  if (firstResult !== null) {
    return { ...tree, first: firstResult };
  }

  const secondResult = replaceNode(tree.second, targetPaneId, replacer);
  if (secondResult !== null) {
    return { ...tree, second: secondResult };
  }

  return null;
}

/**
 * Remove a leaf from the tree and collapse its parent split.
 * Returns the new tree, or null if the leaf was not found.
 * If the tree itself is the target leaf, returns undefined (tree is empty).
 */
function removeLeaf(
  tree: SplitNode,
  targetPaneId: string,
): SplitNode | null | undefined {
  if (tree.type === "leaf") {
    if (tree.paneId === targetPaneId) {
      // The root itself is the target — signal that tree is now empty
      return undefined;
    }
    return null; // not found
  }

  // Check if the target is a direct child of this split
  if (tree.first.type === "leaf" && tree.first.paneId === targetPaneId) {
    return tree.second; // collapse: keep sibling
  }
  if (tree.second.type === "leaf" && tree.second.paneId === targetPaneId) {
    return tree.first; // collapse: keep sibling
  }

  // Recurse into children
  const firstResult = removeLeaf(tree.first, targetPaneId);
  if (firstResult !== null && firstResult !== undefined) {
    return { ...tree, first: firstResult };
  }

  const secondResult = removeLeaf(tree.second, targetPaneId);
  if (secondResult !== null && secondResult !== undefined) {
    return { ...tree, second: secondResult };
  }

  return null;
}

/**
 * Collect all pane IDs from a split tree.
 */
function collectPaneIds(tree: SplitNode): string[] {
  if (tree.type === "leaf") {
    return [tree.paneId];
  }
  return [...collectPaneIds(tree.first), ...collectPaneIds(tree.second)];
}

// ── Store ───────────────────────────────────────────────────────────────

const defaultWs = createDefaultWorkspace();

const [state, setState] = createStore<WorkspaceState>({
  workspaces: [defaultWs],
  selectedId: defaultWs.id,
});

// ── Title Generation ────────────────────────────────────────────────────

function nextTitle(): string {
  const existing = state.workspaces.map((ws) => {
    const match = ws.title.match(/^Workspace (\d+)$/);
    return match ? parseInt(match[1], 10) : 0;
  });
  const max = existing.length > 0 ? Math.max(...existing) : 0;
  return `Workspace ${max + 1}`;
}

// ── Workspace Operations ────────────────────────────────────────────────

export function addWorkspace(): void {
  const leaf = createLeaf();
  const ws: Workspace = {
    id: crypto.randomUUID(),
    title: nextTitle(),
    workingDirectory: "~",
    isPinned: false,
    splitTree: leaf,
    focusedPaneId: leaf.paneId,
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

  const clampedIndex = Math.max(
    0,
    Math.min(newIndex, state.workspaces.length - 1),
  );
  if (currentIdx === clampedIndex) return;

  const updated = [...state.workspaces];
  const [moved] = updated.splice(currentIdx, 1);
  updated.splice(clampedIndex, 0, moved);
  setState("workspaces", updated);
}

// ── Split Tree Operations ───────────────────────────────────────────────

export function splitPane(
  workspaceId: string,
  paneId: string,
  direction: Direction,
): void {
  const wsIdx = state.workspaces.findIndex((ws) => ws.id === workspaceId);
  if (wsIdx === -1) return;

  const workspace = state.workspaces[wsIdx];
  const newLeaf = createLeaf();

  const newTree = replaceNode(workspace.splitTree, paneId, (existing) => ({
    type: "split" as const,
    direction,
    ratio: 0.5,
    first: existing,
    second: newLeaf,
  }));

  if (newTree === null) return;

  setState(
    produce((draft) => {
      draft.workspaces[wsIdx].splitTree = newTree;
      draft.workspaces[wsIdx].focusedPaneId = newLeaf.paneId;
    }),
  );
}

export function closePane(workspaceId: string, paneId: string): void {
  const wsIdx = state.workspaces.findIndex((ws) => ws.id === workspaceId);
  if (wsIdx === -1) return;

  const workspace = state.workspaces[wsIdx];
  const result = removeLeaf(workspace.splitTree, paneId);

  if (result === undefined) {
    // The entire tree was just this one leaf — do nothing (or close workspace)
    // For now, replace with a fresh leaf so the workspace stays alive
    const freshLeaf = createLeaf();
    setState(
      produce((draft) => {
        draft.workspaces[wsIdx].splitTree = freshLeaf;
        draft.workspaces[wsIdx].focusedPaneId = freshLeaf.paneId;
      }),
    );
    return;
  }

  if (result === null) return; // pane not found

  // Pick a new focused pane from the remaining tree
  const remainingPanes = collectPaneIds(result);
  const newFocused = remainingPanes.length > 0 ? remainingPanes[0] : null;

  setState(
    produce((draft) => {
      draft.workspaces[wsIdx].splitTree = result;
      draft.workspaces[wsIdx].focusedPaneId = newFocused;
    }),
  );
}

export function resizeSplit(
  workspaceId: string,
  paneId: string,
  newRatio: number,
): void {
  const wsIdx = state.workspaces.findIndex((ws) => ws.id === workspaceId);
  if (wsIdx === -1) return;

  const clamped = Math.max(0.1, Math.min(0.9, newRatio));
  const workspace = state.workspaces[wsIdx];

  // Find the parent split that contains this pane and update its ratio
  const updated = updateParentRatio(workspace.splitTree, paneId, clamped);
  if (updated === null) return;

  setState(
    produce((draft) => {
      draft.workspaces[wsIdx].splitTree = updated;
    }),
  );
}

/**
 * Walk up from a pane to find its parent split and update the ratio.
 * Returns the new tree or null if not found.
 */
function updateParentRatio(
  tree: SplitNode,
  paneId: string,
  newRatio: number,
): SplitNode | null {
  if (tree.type === "leaf") return null;

  // Check if either direct child contains or is the target pane
  const firstContains = containsPane(tree.first, paneId);
  const secondContains = containsPane(tree.second, paneId);

  if (firstContains || secondContains) {
    // If a direct child is the target, update this split's ratio
    if (
      (tree.first.type === "leaf" && tree.first.paneId === paneId) ||
      (tree.second.type === "leaf" && tree.second.paneId === paneId)
    ) {
      return { ...tree, ratio: newRatio };
    }

    // Otherwise recurse into the child that contains the target
    if (firstContains) {
      const updatedFirst = updateParentRatio(tree.first, paneId, newRatio);
      if (updatedFirst !== null) {
        return { ...tree, first: updatedFirst };
      }
    }
    if (secondContains) {
      const updatedSecond = updateParentRatio(tree.second, paneId, newRatio);
      if (updatedSecond !== null) {
        return { ...tree, second: updatedSecond };
      }
    }
  }

  return null;
}

function containsPane(tree: SplitNode, paneId: string): boolean {
  if (tree.type === "leaf") return tree.paneId === paneId;
  return containsPane(tree.first, paneId) || containsPane(tree.second, paneId);
}

export function focusPane(workspaceId: string, paneId: string): void {
  const wsIdx = state.workspaces.findIndex((ws) => ws.id === workspaceId);
  if (wsIdx === -1) return;

  setState(
    produce((draft) => {
      draft.workspaces[wsIdx].focusedPaneId = paneId;
    }),
  );
}

// ── Workspace Metadata Operations ──────────────────────────────────────

export function renameWorkspace(id: string, newTitle: string): void {
  const wsIdx = state.workspaces.findIndex((ws) => ws.id === id);
  if (wsIdx === -1) return;
  const trimmed = newTitle.trim();
  if (!trimmed) return;
  setState(
    produce((draft) => {
      draft.workspaces[wsIdx].customTitle = trimmed;
    }),
  );
}

export function clearCustomTitle(id: string): void {
  const wsIdx = state.workspaces.findIndex((ws) => ws.id === id);
  if (wsIdx === -1) return;
  setState(
    produce((draft) => {
      draft.workspaces[wsIdx].customTitle = undefined;
    }),
  );
}

export function togglePin(id: string): void {
  const wsIdx = state.workspaces.findIndex((ws) => ws.id === id);
  if (wsIdx === -1) return;
  setState(
    produce((draft) => {
      draft.workspaces[wsIdx].isPinned = !draft.workspaces[wsIdx].isPinned;
    }),
  );
}

export function setWorkspaceColor(id: string, color: string | undefined): void {
  const wsIdx = state.workspaces.findIndex((ws) => ws.id === id);
  if (wsIdx === -1) return;
  setState(
    produce((draft) => {
      draft.workspaces[wsIdx].color = color;
    }),
  );
}

export function closeOtherWorkspaces(id: string): void {
  const kept = state.workspaces.filter((ws) => ws.id === id || ws.isPinned);
  if (kept.length === 0) return;
  setState("workspaces", kept);
  setState("selectedId", id);
}

export function selectWorkspaceByIndex(index: number): void {
  if (index >= 0 && index < state.workspaces.length) {
    setState("selectedId", state.workspaces[index].id);
  }
}

/** Get display title: custom title > generated title */
export function displayTitle(ws: Workspace): string {
  return ws.customTitle || ws.title;
}

/** Count panes in a workspace's split tree */
export function paneCount(ws: Workspace): number {
  return collectPaneIds(ws.splitTree).length;
}

// ── Export ───────────────────────────────────────────────────────────────

export { state as workspaceState };
