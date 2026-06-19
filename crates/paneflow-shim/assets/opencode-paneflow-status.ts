// Paneflow status bridge for OpenCode - INSTALLED AND REMOVED AUTOMATICALLY
// by paneflow-shim around each `opencode` session started inside a Paneflow
// terminal. Safe to delete; do not edit (changes are overwritten).
//
// Reports lifecycle to the Paneflow sidebar by connecting to Paneflow's IPC
// endpoint (PANEFLOW_SOCKET_PATH - a Unix socket on Linux/macOS, a named pipe
// on Windows) and writing a single JSON-RPC frame, then closing. We do NOT
// spawn `paneflow-ai-hook` per event: on Windows, firing a fresh
// `paneflow-ai-hook.exe` rapidly from OpenCode's shell fails to start
// (0xC0000142 / desktop-heap exhaustion) and pops error dialogs. A direct
// socket write has no subprocess, so it is both reliable and cheaper.
//
// Inert anywhere else: PANEFLOW_SOCKET_PATH / PANEFLOW_WORKSPACE_ID are absent
// outside a Paneflow PTY, so every handler returns immediately.

import net from "node:net";

export const PaneflowStatus = async () => {
  const send = (method, params) => {
    const sock = process.env["PANEFLOW_SOCKET_PATH"];
    const wsId = process.env["PANEFLOW_WORKSPACE_ID"];
    if (!sock || !wsId) return;
    try {
      const p = {
        workspace_id: Number(wsId),
        tool: "opencode",
        pid: Number(process.env["PANEFLOW_AI_PID"] || process.pid),
        ...(params ?? {}),
      };
      const sid = process.env["PANEFLOW_SURFACE_ID"];
      if (sid) p.surface_id = Number(sid);
      const frame =
        JSON.stringify({ jsonrpc: "2.0", method, params: p, id: 1 }) + "\n";
      // Fire-and-forget: connect, write one frame, close. Never throws into
      // the agent loop (the 'error' handler swallows a missing/closed pipe).
      const conn = net.connect(sock);
      conn.on("error", () => {});
      conn.on("connect", () => {
        conn.end(frame);
      });
    } catch {
      // Status reporting must never break the session.
    }
  };
  return {
    "chat.message": async () => send("ai.prompt_submit", { hook_payload: {} }),
    "tool.execute.before": async (input) =>
      send("ai.tool_use", {
        tool_name: input?.tool,
        hook_payload: { tool_name: input?.tool },
      }),
    "tool.execute.after": async (input) =>
      send("ai.tool_use", {
        tool_name: input?.tool,
        hook_payload: { tool_name: input?.tool },
      }),
    event: async ({ event }) => {
      if (event?.type === "session.idle") {
        send("ai.stop", { hook_payload: {} });
      } else if (event?.type === "permission.asked") {
        send("ai.notification", {
          notification_type: "permission_prompt",
          hook_payload: {
            notification_type: "permission_prompt",
            message: "OpenCode needs permission",
          },
        });
      }
    },
  };
};
