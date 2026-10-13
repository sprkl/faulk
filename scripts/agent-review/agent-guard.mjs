/**
 * Claude Code `PreToolUse` hook: applies the decision of `guard-core.mjs` to
 * each guarded tool call (Judge Dredd call, file write, shell command), and,
 * before the tool runs, records the session's baseline in each checkout where
 * it has none (a session started before the hooks existed).
 *
 * Failure policy: an unexpected error lets the call through with a warning to
 * the user; the Stop hook still judges the result.
 *
 * @doc docs/agents/judge-dredd.md
 * @doc docs/agents/verification.md
 */

import { readFileSync } from "node:fs";
import { changeFingerprint, hookRoots } from "../agent-qa/fingerprint.mjs";
import { ensureBaselines, sessionKey } from "../agent-qa/session.mjs";
import { judgeToolCall } from "./guard-core.mjs";

let input;
try {
  input = JSON.parse(readFileSync(0, "utf8"));
} catch {
  // Unreadable input: nothing to judge.
  process.exit(0);
}

try {
  const roots = hookRoots(input);
  ensureBaselines(roots, sessionKey(input), changeFingerprint, "pre-tool-use");
  const verdict = judgeToolCall({
    toolName: input.tool_name,
    toolInput: input.tool_input,
    dirs: [input.cwd, ...roots, process.env.CLAUDE_PROJECT_DIR],
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
  }
} catch (error) {
  const detail = error instanceof Error ? error.message.split("\n")[0] : String(error);
  process.stdout.write(
    JSON.stringify({ systemMessage: `Faulk's tool guard failed (${detail}) and let this tool call through.` }),
  );
}
