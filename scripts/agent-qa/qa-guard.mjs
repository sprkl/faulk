/**
 * Claude Code `Stop` hook: refuses to end a turn while the code this session
 * changed has no recorded verification for its current state, or the task's
 * branch has no delivery review (records of `record.mjs`). The reason it
 * returns is addressed to the agent.
 *
 * Failure policy: fail open, loudly. When the hook itself cannot decide (git
 * unavailable, unexpected error), it lets the turn end and shows the user a
 * warning, so that a session that changed nothing is never blocked by a broken
 * environment.
 *
 * @doc docs/agents/verification.md
 * @doc docs/agents/judge-dredd.md
 */

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import {
  changedFiles,
  currentBranch,
  deferralPath,
  fingerprintOf,
  hookRoots,
  readRecord,
  reviewPath,
  sessionPath,
  verificationPath,
} from "./fingerprint.mjs";
import { recordSessionBaseline, sessionKey } from "./session.mjs";
import { blockReason, stopDecision } from "./stop-core.mjs";

// An unreadable baseline counts as a session seen without a usable baseline
// (judged on the records), never as a session the hooks never saw.
const sessionRecord = (path) => (existsSync(path) ? (readRecord(path) ?? {}) : null);

/** The reason to block for one checkout, or `null` to let the turn end. */
function judge(root, key) {
  const files = changedFiles(root);
  const fingerprint = fingerprintOf(root, files);
  const decision = stopDecision({
    fingerprint,
    session: key ? sessionRecord(sessionPath(root, key)) : undefined,
    branch: currentBranch(root) ?? undefined,
    verification: readRecord(verificationPath(root)),
    review: readRecord(reviewPath(root)),
    deferral: readRecord(deferralPath(root)),
  });
  if (decision.recordBaseline) {
    recordSessionBaseline(root, key, fingerprint, "stop");
  }
  if (decision.consumeDeferral) {
    const deferral = readRecord(deferralPath(root));
    writeFileSync(
      deferralPath(root),
      `${JSON.stringify({ ...deferral, consumedAt: new Date().toISOString() }, null, 2)}\n`,
    );
  }
  return decision.allow ? null : blockReason({ root, fingerprint, files, missing: decision.missing });
}

let input = {};
try {
  input = JSON.parse(readFileSync(0, "utf8"));
} catch {
  // No or unreadable input: no session id, the hook judges without a baseline.
}

try {
  const key = sessionKey(input);
  const reasons = [];
  for (const root of hookRoots(input)) {
    const reason = judge(root, key);
    if (reason) {
      reasons.push(reason);
    }
  }
  if (reasons.length > 0) {
    process.stdout.write(JSON.stringify({ decision: "block", reason: reasons.join("\n\n") }));
  }
} catch (error) {
  const detail = error instanceof Error ? error.message.split("\n")[0] : String(error);
  process.stdout.write(
    JSON.stringify({
      systemMessage: `Faulk's end-of-turn guard could not check the code (${detail}), so this turn ended without it. Run git status in the checkout and fix git (git on the PATH, a repository with a commit, safe.directory), or report the error.`,
    }),
  );
}
