import { type Component, onMount, onCleanup, createEffect } from "solid-js";
import { Terminal } from "@xterm/xterm";
import { WebglAddon } from "@xterm/addon-webgl";
import { FitAddon } from "@xterm/addon-fit";
import { invoke, Channel } from "@tauri-apps/api/core";
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

/** Terminal events streamed from the Rust backend via Tauri Channel. */
interface TerminalEventData {
  type: "Data";
  pane_id: string;
  bytes: number[];
}

interface TerminalEventExit {
  type: "Exit";
  pane_id: string;
  code: number;
}

type TerminalEvent = TerminalEventData | TerminalEventExit;

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
      fontFamily:
        "'JetBrains Mono', 'Fira Code', 'Cascadia Code', Menlo, monospace",
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
    const term = terminal;
    term.onData((data: string) => {
      const encoder = new TextEncoder();
      const bytes = Array.from(encoder.encode(data));
      invoke("write_pty", { paneId: props.paneId, bytes }).catch((err) => {
        console.error("write_pty failed:", err);
      });
    });

    // ── ResizeObserver for auto-fit ──────────────────────────────────
    resizeObserver = new ResizeObserver(() => {
      if (!fitAddon || !term) return;
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

    // ── Spawn PTY and attach output channel ─────────────────────────
    // Create a Tauri Channel to receive PTY output from the backend.
    const onEvent = new Channel<TerminalEvent>();
    onEvent.onmessage = (event: TerminalEvent) => {
      if (event.type === "Data") {
        // Convert the byte array back to a Uint8Array and write to xterm.js
        const bytes = new Uint8Array(event.bytes);
        term.write(bytes);
      } else if (event.type === "Exit") {
        term.writeln(`\r\n[Process exited with code ${event.code}]`);
      }
    };

    // Get initial dimensions from the fit addon.
    const dims = fitAddon.proposeDimensions();
    const rows = dims?.rows ?? 24;
    const cols = dims?.cols ?? 80;

    // Spawn the PTY process on the backend, passing the channel for output.
    invoke("spawn_pane", {
      paneId: props.paneId,
      rows,
      cols,
      channel: onEvent,
    }).catch((err) => {
      console.error("spawn_pane failed:", err);
      term.writeln(`\r\n[Failed to start shell: ${err}]`);
    });

    // ── Apply initial focus state ────────────────────────────────────
    if (props.isFocused) {
      term.focus();
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
    // Tell the backend to kill the PTY process.
    invoke("close_pane", { paneId: props.paneId }).catch(() => {});
    terminal?.dispose();
  });

  return <div ref={containerRef} class="terminal-pane" />;
};

export default TerminalPane;
