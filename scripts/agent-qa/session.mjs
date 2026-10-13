/**
 * Session baseline: the fingerprint of the code the first time the hooks see a
 * session (SessionStart, its first guarded tool call, or its first stop). It is
 * written once and never overwritten, so that the Stop hook judges a session
 * only on what it changed.
 *
 * @doc docs/agents/verification.md
 */

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { sessionPath } from "./fingerprint.mjs";

export const validSessionId = (id) => typeof id === "string" && /^[\w-]{1,128}$/.test(id);

/** The key of the session's baseline in a hook input, or `undefined`. */
export const sessionKey = (input) => (validSessionId(input?.session_id) ? input.session_id : undefined);

/**
 * Records the baseline of the session in each checkout where it has none.
 * When the fingerprint cannot be computed, an "unknown" baseline still marks
 * the session as seen, so the Stop hook judges it on the records. Never throws.
 */
export function ensureBaselines(roots, key, fingerprintFor, source) {
  if (!key) {
    return;
  }
  for (const root of roots) {
    try {
      if (existsSync(sessionPath(root, key))) {
        continue;
      }
      let fingerprint = "unknown";
      try {
        fingerprint = fingerprintFor(root);
      } catch {
        // Keep "unknown".
      }
      recordSessionBaseline(root, key, fingerprint, source);
    } catch {
      // Unwritable .qa/: never fail a hook for it.
    }
  }
}

/**
 * Writes the baseline if the session has none. `false` when it already
 * existed. The exclusive create keeps the first writer when tool calls run in
 * parallel.
 */
export function recordSessionBaseline(root, sessionId, fingerprint, source) {
  if (!validSessionId(sessionId)) {
    return false;
  }
  const path = sessionPath(root, sessionId);
  mkdirSync(dirname(path), { recursive: true });
  try {
    writeFileSync(path, `${JSON.stringify({ fingerprint, source, at: new Date().toISOString() })}\n`, { flag: "wx" });
    return true;
  } catch (error) {
    if (error?.code === "EEXIST") {
      return false;
    }
    throw error;
  }
}
