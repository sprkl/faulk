/**
 * Claude Code `SessionStart` hook: records the fingerprint of the code when a
 * session starts (any source: startup, resume, clear, compact, fork), unless
 * the session already has one. The Stop hook then blocks only a session that
 * changed the code.
 *
 * Never fails the session start: on an error the next guarded tool call or
 * stop records the baseline.
 *
 * @doc docs/agents/verification.md
 */

import { readFileSync } from "node:fs";
import { changeFingerprint, hookRoots } from "./fingerprint.mjs";
import { ensureBaselines, sessionKey } from "./session.mjs";

try {
  const input = JSON.parse(readFileSync(0, "utf8"));
  ensureBaselines(hookRoots(input), sessionKey(input), changeFingerprint, `session-start:${input.source ?? "unknown"}`);
} catch {
  // See above: never block a session start.
}
