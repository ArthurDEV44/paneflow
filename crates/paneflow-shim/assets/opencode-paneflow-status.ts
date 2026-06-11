// Paneflow status bridge for OpenCode — INSTALLED AND REMOVED AUTOMATICALLY
// by paneflow-shim around each `opencode` session started inside a Paneflow
// terminal. Safe to delete; do not edit (changes are overwritten).
//
// Forwards lifecycle events to the Paneflow sidebar by invoking the
// `paneflow-ai-hook` binary (on PATH inside Paneflow PTYs). Inert anywhere
// else: PANEFLOW_SOCKET_PATH is absent there, so every handler returns
// immediately and OpenCode behaves as if this plugin did not exist.

export const PaneflowStatus = async ({ $ }) => {
  const fire = (event, payload) => {
    if (!process.env["PANEFLOW_SOCKET_PATH"]) return;
    try {
      const json = JSON.stringify(payload ?? {});
      // Fire-and-forget: never block the agent loop on status reporting.
      void $`echo ${json} | paneflow-ai-hook ${event}`.quiet().nothrow();
    } catch {
      // Status reporting must never break the session.
    }
  };
  return {
    "chat.message": async () => {
      fire("UserPromptSubmit");
    },
    "tool.execute.before": async (input) => {
      fire("PreToolUse", { tool_name: input?.tool });
    },
    "tool.execute.after": async (input) => {
      fire("PostToolUse", { tool_name: input?.tool });
    },
    event: async ({ event }) => {
      if (event?.type === "session.idle") {
        fire("Stop");
      } else if (event?.type === "permission.asked") {
        fire("Notification", {
          notification_type: "permission_prompt",
          message: "OpenCode needs permission",
        });
      }
    },
  };
};
