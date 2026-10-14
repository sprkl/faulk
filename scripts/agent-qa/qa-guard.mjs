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
  currentRef,
  deferralPath,
  fingerprintOf,
  hookRoots,
  isBranchName,
  readRecord,
  refFingerprint,
  reviewPath,
  stashesSince,
  verificationPath,
} from "./fingerprint.mjs";
import { ensureBaselines, sessionKey, sessionState, stateRoot } from "./session.mjs";
import { blockReason, stopDecision } from "./stop-core.mjs";

/**
 * Judges one branch (or commit) the session wrote on in a checkout: the
 * working tree when HEAD is still on it, its committed content otherwise, so
 * that leaving the branch (`git switch develop`) does not leave its code
 * unverified. Returns `{ reason }` to block, `{ warning }`, or `{}`.
 */
function judgeRef(root, session, marker, current) {
  const onIt = marker.ref === null || marker.ref === current;
  let files;
  let fingerprint;
  if (onIt) {
    files = changedFiles(root);
    fingerprint = fingerprintOf(root, files);
  } else {
    fingerprint = refFingerprint(root, marker.ref);
    if (fingerprint === undefined) {
      return {
        warning: `This session wrote code on ${marker.ref} in ${root}, which no longer exists: the end-of-turn guard cannot check it.`,
      };
    }
    files = [`the committed content of ${marker.ref}`];
  }
  const decision = stopDecision({
    fingerprint,
    writer: true,
    baseline: onIt ? session.baseline(root)?.fingerprint : undefined,
    branch: onIt ? (currentBranch(root) ?? undefined) : isBranchName(marker.ref) ? marker.ref : undefined,
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
    return {};
  }
  const notes = [];
  if (!onIt) {
    notes.push(
      `This session wrote this code on ${marker.ref}, and HEAD is now on ${current ?? "another commit"}: switch back (git switch ${marker.ref}) to verify and record it.`,
    );
  }
  const move = session.lastMove(root);
  if (move?.move) {
    notes.push(
      `A call of this session moved HEAD without authoring code ("${move.move}"); the baseline moved with it, so only the changes made by the session's own calls count.`,
    );
  }
  return { reason: [blockReason({ root, fingerprint, files, missing: decision.missing }), ...notes].join(" ") };
}

/** Reasons to block and warnings for one checkout. */
function judge(root, session) {
  // A checkout removed since (a worktree cleaned up) has nothing left to verify.
  if (!session || !existsSync(root)) {
    return [];
  }
  const markers = session.writers().filter((marker) => marker.root === root);
  if (markers.length === 0) {
    return [];
  }
  const current = currentRef(root);
  const results = markers.map((marker) => judgeRef(root, session, marker, current));
  // A stash made since the session first wrote here holds code it may have
  // changed, out of the working tree: it is still pending.
  const since = markers.map((marker) => marker.at).sort()[0];
  const stashes = stashesSince(root, since);
  if (stashes.length > 0) {
    results.push({
      reason: `This session wrote code in ${root} and ${stashes.join(", ")} was stashed since: stashed code is not verified. Restore it (git stash pop), then verify and record it as the guard asks; drop the stash (git stash drop) only if it holds nothing of yours.`,
    });
  }
  return results;
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
  const results = roots.flatMap((root) => judge(root, session));
  warnings.push(...results.map((result) => result.warning).filter(Boolean));
  const reasons = results.map((result) => result.reason).filter(Boolean);
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
