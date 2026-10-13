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
  changedGuardFiles,
  currentBranch,
  deferralPath,
  fingerprintOf,
  hookRoots,
  readRecord,
  reviewPath,
  verificationPath,
} from "./fingerprint.mjs";
import { ensureBaselines, sessionKey, sessionState, stateRoot } from "./session.mjs";
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
  if (decision.allow) {
    return null;
  }
  const move = session.lastMove(root);
  const moved = move?.move
    ? ` (A call of this session moved HEAD without authoring code: "${move.move}"; the baseline was moved with it, so only the changes made by the session's own calls count.)`
    : "";
  return blockReason({ root, fingerprint, files, missing: decision.missing }) + moved;
}

let input = {};
try {
  input = JSON.parse(readFileSync(0, "utf8"));
} catch {
  // No or unreadable input: no session id, nothing attributed to the session.
}

const warnings = [];
const output = {};
try {
  const key = sessionKey(input);
  if (!key) {
    // Without a session id nothing can be attributed: fail open, loudly.
    throw new Error("the Stop hook input has no valid session_id");
  }
  const state = stateRoot(input);
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
    // Calls that never got their PostToolUse (denied, interrupted, failed)
    // say nothing about who changed the code since: drop their snapshots.
    session.dropPending();
  }
  const reasons = roots.map((root) => judge(root, session)).filter(Boolean);
  if (reasons.length > 0) {
    output.decision = "block";
    output.reason = reasons.join("\n\n");
  }
  // Changing the guardrails is trusted only after a human reviews the pull
  // request: the agent that edits them can also run them. Say so.
  for (const root of here) {
    const guard = changedGuardFiles(root);
    if (guard.length > 0) {
      warnings.push(
        `The guardrails differ from develop in ${root} (${guard.slice(0, 5).join(", ")}${guard.length > 5 ? ", …" : ""}): the end-of-turn guard runs the modified version. Review these changes yourself before trusting it.`,
      );
    }
  }
} catch (error) {
  const detail = error instanceof Error ? error.message.split("\n")[0] : String(error);
  warnings.push(
    `Faulk's end-of-turn guard could not check the code (${detail}), so this turn ended without it. Run git status in the checkout and fix git (git on the PATH, a repository with a commit, safe.directory), or report the error.`,
  );
}
if (warnings.length > 0) {
  output.systemMessage = warnings.join(" ");
}
if (Object.keys(output).length > 0) {
  process.stdout.write(JSON.stringify(output));
}
