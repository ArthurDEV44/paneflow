// Paneflow status bridge for Pi — INSTALLED AND REMOVED AUTOMATICALLY by
// paneflow-shim around each `pi` session started inside a Paneflow
// terminal. Safe to delete; do not edit (changes are overwritten).
//
// Forwards lifecycle events to the Paneflow sidebar by invoking the
// `paneflow-ai-hook` binary (on PATH inside Paneflow PTYs). Inert anywhere
// else: PANEFLOW_SOCKET_PATH is absent there, so every handler returns
// immediately and Pi behaves as if this extension did not exist.

export default function (pi) {
  const fire = (event) => {
    if (!process.env["PANEFLOW_SOCKET_PATH"]) return;
    try {
      // Fire-and-forget: never block the agent loop on status reporting.
      pi.exec("paneflow-ai-hook", [event]).catch(() => {});
    } catch {
      // Status reporting must never break the session.
    }
  };
  pi.on("agent_start", () => fire("UserPromptSubmit"));
  pi.on("agent_end", () => fire("Stop"));
  pi.on("tool_execution_start", () => fire("PreToolUse"));
  pi.on("tool_execution_end", () => fire("PostToolUse"));
}
