/**
 * Claude Code `Stop` hook: refuses to end a turn while a checkout where this
 * session's own tool calls changed the guarded code has no recorded
 * verification for its current state, or its task branch has no delivery
 * review (records of `record.mjs`). The reason it returns is addressed to the
 * agent.
 *
 * Failure policy: fail open, loudly. When the hook itself cannot decide (git
 * unavailable, unexpected error), it lets the turn end and shows the user a
 * warning, so that a broken environment never blocks a session.
 *
 * @doc docs/agents/verification.md
 * @doc docs/agents/judge-dredd.md
 */

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import {
  changeFingerprint,
  changedFiles,
  currentBranch,
  deferralPath,
  fingerprintOf,
  hookRoots,
  readRecord,
  reviewPath,
  verificationPath,
} from "./fingerprint.mjs";
import { ensureBaselines, safeFingerprint, sessionKey, sessionState, stateRoot, wroteDuringCall } from "./session.mjs";
import { blockReason, stopDecision } from "./stop-core.mjs";

/** The reason to block for one checkout, or `null` to let the turn end. */
function judge(root, session) {
  const writer = session ? session.isWriter(root) : false;
  // A checkout removed since (a worktree cleaned up) has nothing left to verify.
  if (!writer || !existsSync(root)) {
    return null;
  }
  const files = changedFiles(root);
  const fingerprint = fingerprintOf(root, files);
  const decision = stopDecision({
    fingerprint,
    writer,
    baseline: session.baseline(root)?.fingerprint,
    branch: currentBranch(root) ?? undefined,
    verification: readRecord(verificationPath(root)),
    review: readRecord(reviewPath(root)),
    deferral: readRecord(deferralPath(root)),
  });
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
  // No or unreadable input: no session id, nothing attributed to the session.
}

try {
  const key = sessionKey(input);
  const state = key ? stateRoot(input) : null;
  const session = state ? sessionState(state, key) : null;
  const here = hookRoots(input);
  const roots = [...here];
  for (const root of session?.writerRoots() ?? []) {
    if (!roots.includes(root)) {
      roots.push(root);
    }
  }
  if (session) {
    ensureBaselines(session, here, changeFingerprint, "stop");
    // Snapshots left by calls that got no PostToolUse (a failed call can still
    // have written): compare them with the code now.
    for (const { root, fingerprint } of session.takePre(undefined)) {
      if (!session.isWriter(root) && existsSync(root)) {
        const now = safeFingerprint(changeFingerprint, root);
        if (wroteDuringCall({ before: fingerprint, after: now })) {
          session.markWriter(root, "unfinished call");
        }
      }
      if (!roots.includes(root)) {
        roots.push(root);
      }
    }
  }
  const reasons = roots.map((root) => judge(root, session)).filter(Boolean);
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
