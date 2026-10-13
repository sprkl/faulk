/**
 * Claude Code `PostToolUse` hook: after a call that can change files, marks
 * the session as a writer in each checkout whose guarded code the call
 * changed (fingerprint after the call against the snapshot `agent-guard.mjs`
 * took before it). The Stop hook judges a session only where it wrote. Never
 * blocks.
 *
 * @doc docs/agents/verification.md
 */

import { readFileSync } from "node:fs";
import { CHANGING_TOOLS } from "../agent-review/guard-core.mjs";
import { changeFingerprint, hookRoots } from "./fingerprint.mjs";
import { safeFingerprint, sessionKey, sessionState, stateRoot, wroteDuringCall } from "./session.mjs";

try {
  const input = JSON.parse(readFileSync(0, "utf8"));
  const key = sessionKey(input);
  const state = key ? stateRoot(input) : null;
  if (state && CHANGING_TOOLS.has(input.tool_name)) {
    const session = sessionState(state, key);
    const before = new Map(
      typeof input.tool_use_id === "string"
        ? session.takePre(input.tool_use_id).map((snapshot) => [snapshot.root, snapshot.fingerprint])
        : [],
    );
    const roots = [...new Set([...before.keys(), ...hookRoots(input)])];
    for (const root of roots) {
      if (session.isWriter(root)) {
        continue;
      }
      const baseline = session.baseline(root);
      const after = safeFingerprint(changeFingerprint, root);
      if (wroteDuringCall({ before: before.get(root), baseline: baseline?.fingerprint, after })) {
        session.markWriter(root, input.tool_name);
      }
      if (baseline === null) {
        // First time the hooks see this checkout, after the call: its state
        // before is unknown, so the baseline cannot clear anything.
        session.recordBaseline(root, "unknown", "post-tool-use");
      }
    }
  }
} catch {
  // Never block: the Stop hook judges what is recorded.
}
