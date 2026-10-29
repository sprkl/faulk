/**
 * Claude Code `SessionStart` hook: records the fingerprint of the code when a
 * session starts (any source: startup, resume, clear, compact, fork), unless
 * the session already has one. With it, the Stop hook lets a session through
 * once it has undone its own changes.
 *
 * Never fails the session start.
 *
 * @doc docs/agents/verification.md
 */

import { readFileSync } from "node:fs";
import { changeFingerprint, hookRoots } from "./fingerprint.mjs";
import { agentKey, ensureBaselines, sessionKey, sessionState, stateRoot } from "./session.mjs";

try {
  const input = JSON.parse(readFileSync(0, "utf8"));
  const key = sessionKey(input);
  const state = key ? stateRoot(input) : null;
  // A subagent never writes the session's baselines (#392), whatever fires this hook.
  if (state && !agentKey(input)) {
    ensureBaselines(sessionState(state, key), hookRoots(input), changeFingerprint, `session-start:${input.source ?? "unknown"}`);
  }
} catch {
  // See above: never block a session start.
}
