import { type Component, onMount, onCleanup, createEffect } from "solid-js";
import { Terminal } from "@xterm/xterm";
import { WebglAddon } from "@xterm/addon-webgl";
import { FitAddon } from "@xterm/addon-fit";
import { invoke } from "@tauri-apps/api/core";
import "@xterm/xterm/css/xterm.css";

interface TerminalPaneProps {
  paneId: string;
  isFocused: boolean;
}

/** Dark theme matching the app's Catppuccin-inspired color scheme. */
const DARK_THEME = {
  background: "#1e1e2e",
  foreground: "#cdd6f4",
  cursor: "#f5e0dc",
  cursorAccent: "#1e1e2e",
  selectionBackground: "#45475a",
  selectionForeground: "#cdd6f4",
  black: "#45475a",
  red: "#f38ba8",
  green: "#a6e3a1",
  yellow: "#f9e2af",
  blue: "#89b4fa",
  magenta: "#f5c2e7",
  cyan: "#94e2d5",
  white: "#bac2de",
  brightBlack: "#585b70",
  brightRed: "#f38ba8",
  brightGreen: "#a6e3a1",
  brightYellow: "#f9e2af",
  brightBlue: "#89b4fa",
  brightMagenta: "#f5c2e7",
  brightCyan: "#94e2d5",
  brightWhite: "#a6adc8",
} as const;

/**
 * TerminalPane renders a single xterm.js terminal with WebGL acceleration.
 *
 * Input captured by the terminal is forwarded to the Tauri backend via
 * `invoke("write_pty")`. Container resize is observed so the terminal
 * stays fitted and the backend is notified via `invoke("resize_pty")`.
 *
 * PTY output will be wired through `terminal.write(data)` by a future
 * story that connects the Tauri Channel-based attach_pty flow.
 */
const TerminalPane: Component<TerminalPaneProps> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  let terminal: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let resizeObserver: ResizeObserver | undefined;

  onMount(() => {
    if (!containerRef) return;

    // ── Create Terminal ──────────────────────────────────────────────
    terminal = new Terminal({
      theme: DARK_THEME,
      fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', Menlo, monospace",
      fontSize: 14,
      lineHeight: 1.2,
      cursorBlink: true,
      cursorStyle: "block",
      allowProposedApi: true,
      scrollback: 10_000,
    });

    // ── Fit Addon ────────────────────────────────────────────────────
    fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);

    // ── Open in DOM ──────────────────────────────────────────────────
    terminal.open(containerRef);

    // ── WebGL Addon (with canvas fallback) ───────────────────────────
    try {
      const webglAddon = new WebglAddon();
      webglAddon.onContextLoss(() => {
        webglAddon.dispose();
      });
      terminal.loadAddon(webglAddon);
    } catch (err) {
      console.warn(
        "WebGL addon failed to load — falling back to canvas renderer.",
        err,
      );
    }

    // Initial fit after the terminal is mounted and rendered.
    fitAddon.fit();

    // ── Forward keyboard input to the PTY backend ────────────────────
    terminal.onData((data: string) => {
      const encoder = new TextEncoder();
      const bytes = Array.from(encoder.encode(data));
      invoke("write_pty", { paneId: props.paneId, bytes }).catch((err) => {
        console.error("write_pty failed:", err);
      });
    });

    // ── ResizeObserver for auto-fit ──────────────────────────────────
    resizeObserver = new ResizeObserver(() => {
      if (!fitAddon || !terminal) return;

      // requestAnimationFrame avoids layout thrashing when the browser
      // fires multiple resize observations in quick succession.
      requestAnimationFrame(() => {
        fitAddon!.fit();
        const dims = fitAddon!.proposeDimensions();
        if (dims) {
          invoke("resize_pty", {
            paneId: props.paneId,
            rows: dims.rows,
            cols: dims.cols,
          }).catch((err) => {
            console.error("resize_pty failed:", err);
          });
        }
      });
    });
    resizeObserver.observe(containerRef);

    // ── Apply initial focus state ────────────────────────────────────
    if (props.isFocused) {
      terminal.focus();
    }
  });

  // ── Reactive focus tracking ──────────────────────────────────────
  createEffect(() => {
    if (!terminal) return;
    if (props.isFocused) {
      terminal.focus();
    } else {
      terminal.blur();
    }
  });

  onCleanup(() => {
    resizeObserver?.disconnect();
    terminal?.dispose();
  });

  return <div ref={containerRef} class="terminal-pane" />;
};

export default TerminalPane;

/**
 * Utility handle exposed for parent components that need to push PTY
 * output into the terminal. Usage:
 *
 *   const ref = getTerminalHandle(paneId);
 *   ref?.write(data);
 *
 * This will be consumed by the attach_pty channel integration in a
 * future story.
 */
export type TerminalHandle = Pick<Terminal, "write">;
