// Nouve's own chat tools (ohmypii, write_file) ask for permission here.
//
// Claude Code's hook approvals arrive over the relay pipe (see hooks.ts); these
// come straight from the chat loop in Rust, which is blocked waiting for the
// answer on an approval channel. Same card, same buttons — only the reply route
// differs, which `island.decide` picks from `ApprovalInfo.kind`.

import { Bridge, onEvent } from "../core/bridge";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import type { Island } from "./island";

const CLAUDE_ID = "integration_claude";

/** Clears the card if the Rust side gave up waiting before anyone clicked. */
let pendingTimeout: number | null = null;

interface ToolApprovalPayload {
  request_id?: string;
  tool?: string;
  command?: string;
}

export function registerToolApprovalHandlers(island: Island) {
  void onEvent<ToolApprovalPayload>("tool_approval", (payload) => handle(island, payload));
}

function handle(island: Island, payload: ToolApprovalPayload) {
  const requestId = payload.request_id ?? "";
  if (!requestId) return;
  // Paused, or another card already up: nobody can act on this, so refuse it
  // now rather than let Rust wait out the full timeout for nothing.
  if (State.paused || State.pendingApproval) {
    void Bridge.toolApprovalDecision(requestId, "deny");
    return;
  }

  if (pendingTimeout != null) window.clearTimeout(pendingTimeout);
  const tool = payload.tool ?? "Tool";
  State.pendingApproval = {
    requestId,
    sessionId: "",
    tool,
    command: payload.command || tool,
    kind: "tool",
  };
  State.updateTask(CLAUDE_ID, "approval");
  State.isPinned = true;
  Sound.play("approval");
  island.alert("approval");

  // Rust refuses the tool after APPROVAL_TIMEOUT (300 s); the card must go with
  // it, or it would offer buttons that no longer do anything.
  pendingTimeout = window.setTimeout(() => {
    pendingTimeout = null;
    if (State.pendingApproval?.kind !== "tool") return;
    State.pendingApproval = null;
    State.isPinned = false;
    island.dropPin();
    State.updateTask(CLAUDE_ID, "working");
    State.setPillBadge(CLAUDE_ID, null);
    if (State.view === "approval") island.setView(State.defaultView());
    State.notify();
  }, 300_000);

  State.notify();
}
