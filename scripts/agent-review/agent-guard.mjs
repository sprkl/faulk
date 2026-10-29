/**
 * Claude Code `PreToolUse` hook: applies the decision of `guard-core.mjs` to
 * each guarded tool call (Judge Dredd call, file write, shell command). Before
 * a call that can change files, it also records the session's baseline where
 * it has none and a snapshot of the fingerprint, which `post-tool-use.mjs`
 * compares after the call to tell whether this session changed the code.
 *
 * Failure policy: an unexpected error lets the call through with a warning to
 * the user.
 *
 * @doc docs/agents/judge-dredd.md
 * @doc docs/agents/verification.md
 */

import { readFileSync } from "node:fs";
import { changeFingerprint, codeState, currentRef, hookRoots } from "../agent-qa/fingerprint.mjs";
import { agentKey, editTarget, ensureBaselines, safely, sessionKey, sessionState, stateRoot } from "../agent-qa/session.mjs";
import { EDIT_TOOLS, SNAPSHOT_TOOLS, judgeToolCall } from "./guard-core.mjs";

let input;
try {
  input = JSON.parse(readFileSync(0, "utf8"));
} catch {
  // Unreadable input: nothing to judge.
  process.exit(0);
}

try {
  const roots = hookRoots(input);
  const verdict = judgeToolCall({
    toolName: input.tool_name,
    toolInput: input.tool_input,
    dirs: [input.cwd, ...roots, process.env.CLAUDE_PROJECT_DIR],
    checkouts: roots,
  });
  if (!verdict.allow) {
    process.stdout.write(
      JSON.stringify({
        hookSpecificOutput: {
          hookEventName: "PreToolUse",
          permissionDecision: "deny",
          permissionDecisionReason: verdict.reason,
        },
      }),
    );
    process.exit(0);
  }
  const key = sessionKey(input);
  const state = key ? stateRoot(input) : null;
  const agent = agentKey(input);
  if (state && agent) {
    sessionState(state, key).touchAgent(agent, input.tool_use_id);
  }
  if (state && EDIT_TOOLS.has(input.tool_name)) {
    // The baseline of the checkout the edit targets, taken before the edit.
    const targets = Object.entries(input.tool_input ?? {})
      .filter(([name, value]) => /path/i.test(name) && typeof value === "string")
      .map(([, value]) => editTarget(value, [input.cwd, process.env.CLAUDE_PROJECT_DIR])?.root)
      .filter(Boolean);
    if (!agent) {
      ensureBaselines(sessionState(state, key), [...new Set([...roots, ...targets])], changeFingerprint, "pre-tool-use");
    }
  } else if (state && SNAPSHOT_TOOLS.has(input.tool_name)) {
    const session = sessionState(state, key);
    if (!agent) {
      ensureBaselines(session, roots, changeFingerprint, "pre-tool-use");
    }
    if (typeof input.tool_use_id === "string") {
      for (const root of roots) {
        // A session already known to have written there is judged anyway:
        // no snapshot needed, and no cost.
        if (!session.isWriter(root, currentRef(root), agent)) {
          const state = safely(codeState, root);
          session.savePre(
            input.tool_use_id,
            root,
            state === "unknown" ? { fingerprint: "unknown" } : state,
          );
        }
      }
    }
  }
} catch (error) {
  const detail = error instanceof Error ? error.message.split("\n")[0] : String(error);
  process.stdout.write(
    JSON.stringify({ systemMessage: `Faulk's tool guard failed (${detail}) and let this tool call through.` }),
  );
}
