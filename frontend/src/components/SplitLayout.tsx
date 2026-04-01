import { type Component, createSignal, onCleanup, Show } from "solid-js";
import type { SplitNode } from "../stores/workspace";
import { resizeSplit, focusPane } from "../stores/workspace";
import TerminalPane from "./TerminalPane";

interface SplitLayoutProps {
  node: SplitNode;
  workspaceId: string;
  focusedPaneId: string | null;
}

const MIN_PANE_SIZE = 80; // px

const SplitLayout: Component<SplitLayoutProps> = (props) => {
  return (
    <Show
      when={props.node.type === "split" ? props.node : undefined}
      fallback={
        <LeafPane
          node={props.node as SplitNode & { type: "leaf" }}
          workspaceId={props.workspaceId}
          focusedPaneId={props.focusedPaneId}
        />
      }
    >
      {(splitNode) => (
        <SplitContainer
          node={splitNode()}
          workspaceId={props.workspaceId}
          focusedPaneId={props.focusedPaneId}
        />
      )}
    </Show>
  );
};

// ── Leaf Pane ───────────────────────────────────────────────────────────

interface LeafPaneProps {
  node: SplitNode & { type: "leaf" };
  workspaceId: string;
  focusedPaneId: string | null;
}

const LeafPane: Component<LeafPaneProps> = (props) => {
  return (
    <div
      class="split-leaf"
      onMouseDown={() => focusPane(props.workspaceId, props.node.paneId)}
    >
      <TerminalPane
        paneId={props.node.paneId}
        workspaceId={props.workspaceId}
        isFocused={props.focusedPaneId === props.node.paneId}
      />
    </div>
  );
};

// ── Split Container ─────────────────────────────────────────────────────

interface SplitContainerProps {
  node: SplitNode & { type: "split" };
  workspaceId: string;
  focusedPaneId: string | null;
}

const SplitContainer: Component<SplitContainerProps> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  const [isDragging, setIsDragging] = createSignal(false);

  /**
   * Compute flex-basis percentages from the ratio.
   * The divider occupies a fixed 4px; CSS handles that via flex-shrink: 0.
   */
  const firstBasis = () => `${props.node.ratio * 100}%`;
  const secondBasis = () => `${(1 - props.node.ratio) * 100}%`;

  const isHorizontal = () => props.node.direction === "horizontal";

  // ── Drag handling ───────────────────────────────────────────────────

  /** Find the first leaf pane ID in a subtree (used for resizeSplit). */
  function firstLeafId(node: SplitNode): string {
    if (node.type === "leaf") return node.paneId;
    return firstLeafId(node.first);
  }

  const handleMouseDown = (e: MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);

    const container = containerRef;
    if (!container) return;

    const rect = container.getBoundingClientRect();
    const totalSize = isHorizontal() ? rect.width : rect.height;
    const startOffset = isHorizontal() ? rect.left : rect.top;

    const onMouseMove = (moveEvent: MouseEvent) => {
      const cursor = isHorizontal() ? moveEvent.clientX : moveEvent.clientY;
      let newRatio = (cursor - startOffset) / totalSize;

      // Enforce minimum pane size
      const minRatio = MIN_PANE_SIZE / totalSize;
      const maxRatio = 1 - minRatio;
      newRatio = Math.max(minRatio, Math.min(maxRatio, newRatio));

      // Also clamp to 0.1-0.9 as specified
      newRatio = Math.max(0.1, Math.min(0.9, newRatio));

      // Use the first leaf of the first child to identify this split
      const targetPaneId = firstLeafId(props.node.first);
      resizeSplit(props.workspaceId, targetPaneId, newRatio);
    };

    const onMouseUp = () => {
      setIsDragging(false);
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    document.body.style.cursor = isHorizontal() ? "col-resize" : "row-resize";
    document.body.style.userSelect = "none";
  };

  onCleanup(() => {
    // Safety: remove any lingering event listeners if the component
    // unmounts during a drag.
    setIsDragging(false);
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  });

  return (
    <div
      ref={containerRef}
      class="split-container"
      classList={{
        "split-container--horizontal": isHorizontal(),
        "split-container--vertical": !isHorizontal(),
        "split-container--dragging": isDragging(),
      }}
    >
      <div class="split-child" style={{ "flex-basis": firstBasis() }}>
        <SplitLayout
          node={props.node.first}
          workspaceId={props.workspaceId}
          focusedPaneId={props.focusedPaneId}
        />
      </div>

      <div
        class="split-divider"
        classList={{
          "split-divider--horizontal": isHorizontal(),
          "split-divider--vertical": !isHorizontal(),
        }}
        onMouseDown={handleMouseDown}
      />

      <div class="split-child" style={{ "flex-basis": secondBasis() }}>
        <SplitLayout
          node={props.node.second}
          workspaceId={props.workspaceId}
          focusedPaneId={props.focusedPaneId}
        />
      </div>
    </div>
  );
};

export default SplitLayout;
