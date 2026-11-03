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
  commonDir,
  currentBranch,
  currentRef,
  deferralPath,
  fingerprintOf,
  hookRoots,
  isBranchName,
  mergedIntoDevelop,
  prMergedOnGitHub,
  readRecord,
  readRecords,
  refFingerprint,
  refTip,
  reviewPath,
  reviewsPath,
  samePath,
  squashedIntoDevelop,
  stashesSince,
  verificationPath,
  verificationsPath,
} from "./fingerprint.mjs";
import { agentKey, ensureBaselines, sessionKey, sessionState, stateRoot } from "./session.mjs";
import { blockReason, exemptAgentMarker, isAgentWorktreePath, stopDecision } from "./stop-core.mjs";

/** The local checks failed: GitHub is asked at most once per stop, and once per marker every `GH_RECHECK_MS`. */
const GH_RECHECK_MS = 10 * 60 * 1000;
let ghCallsLeft = 1;

/** Whether GitHub says the deleted branch of `marker` was merged (see `prMergedOnGitHub`), within the budget above. */
function askedGitHub(root, session, marker) {
  if (marker.ref === null || ghCallsLeft <= 0) {
    return false;
  }
  const last = Date.parse(marker.prCheckedAt ?? "");
  if (Number.isFinite(last) && Date.now() - last < GH_RECHECK_MS) {
    return false;
  }
  ghCallsLeft -= 1;
  session.notePrCheck(root, marker.ref, marker.agent, new Date().toISOString());
  return prMergedOnGitHub(root, marker.ref, marker.at);
}

/**
 * Judges one branch (or commit) the session wrote on in a checkout: the
 * working tree when HEAD is still on it, its committed content otherwise, so
 * that leaving the branch (`git switch develop`) does not leave its code
 * unverified. Returns `{ reason }` to block, `{ warning }`, or `{}`.
 */
function judgeRef(root, session, marker, current, ctx) {
  const onIt = marker.ref === null || marker.ref === current;
  if (marker.ref !== null) {
    // Remember the branch's last known commit while it exists (see below).
    session.noteTip(root, marker.ref, marker.agent, refTip(root, marker.ref));
  }
  let files;
  let fingerprint;
  if (onIt) {
    files = changedFiles(root);
    fingerprint = fingerprintOf(root, files);
  } else {
    fingerprint = refFingerprint(root, marker.ref);
    if (fingerprint === undefined) {
      // A branch deleted after its pull request was merged: its last known
      // commit is in develop (a merge), or squashed into it, or GitHub says
      // its pull request was merged since the session wrote on it (#590);
      // nothing is left to check. Markers written without a commit, or whose
      // branch is not merged, still warn.
      if (mergedIntoDevelop(root, marker.tip) || squashedIntoDevelop(root, marker.tip) || askedGitHub(root, session, marker)) {
        session.dropWriter(root, marker.ref, marker.agent);
        return {};
      }
      if (marker.warned === true) {
        return {};
      }
      session.markWarned(root, marker.ref, marker.agent);
      return {
        warning: `This session wrote code on ${marker.ref} in ${root}, which no longer exists: the end-of-turn guard cannot check it.`,
      };
    }
    files = [`the committed content of ${marker.ref}`];
  }
  const branch = onIt ? (currentBranch(root) ?? undefined) : isBranchName(marker.ref) ? marker.ref : undefined;
  const decision = stopDecision({
    fingerprint,
    writer: true,
    // A subagent has no baseline: its events never write the session's, and one
    // taken by the lead's own stop would hold the agent's work as already there.
    baseline: onIt && !marker.agent ? session.baseline(root)?.fingerprint : undefined,
    branch,
    // Every record of the checkout: each branch stacked in it is judged on its own (#492).
    verifications: readRecords(verificationsPath(root), verificationPath(root)),
    reviews: readRecords(reviewsPath(root), reviewPath(root)),
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
  return { reason: [blockReason({ root, fingerprint, files, missing: decision.missing, branch, reviewNotCarried: decision.reviewNotCarried }), ...notes].join(" ") };
}

/**
 * The markers of a checkout this stop answers for. A subagent's stop (an
 * input with `agent_id`) answers for that agent's markers in an agent worktree
 * of this repository only, none exempt; its markers in any other checkout (the
 * lead's) are judged by the lead's stop with the lead's own work, which the
 * subagent could neither verify nor review.
 * The lead's stop answers for its own and for the subagents' markers, except
 * those of a subagent still running in its own worktree (`exemptAgentMarker`):
 * that work is not finished, and the agent is judged on its own stop.
 */
function markersFor(root, session, ctx) {
  const markers = session.writers().filter((marker) => marker.root === root);
  if (ctx.agent) {
    const own = isAgentWorktreePath(root) && sameRepository(root, ctx.state) && !ctx.leadRoots.some((dir) => samePath(dir, root));
    return own ? markers.filter((marker) => marker.agent === ctx.agent) : [];
  }
  return markers.filter(
    (marker) =>
      !exemptAgentMarker({
        marker,
        running: Boolean(marker.agent) && session.agentRunning(marker.agent),
        agentWorktree: isAgentWorktreePath(root),
        sameRepository: Boolean(marker.agent) && sameRepository(root, ctx.state),
        leadRoot: ctx.here.some((dir) => samePath(dir, root)),
      }),
  );
}

function sameRepository(a, b) {
  try {
    return samePath(commonDir(a), commonDir(b));
  } catch {
    return false;
  }
}

/** Reasons to block and warnings for one checkout. */
function judge(root, session, ctx) {
  // A checkout removed since (a worktree cleaned up) has nothing left to verify.
  if (!session || !existsSync(root)) {
    return [];
  }
  const markers = markersFor(root, session, ctx);
  if (markers.length === 0) {
    return [];
  }
  const current = currentRef(root);
  const results = markers.map((marker) => judgeRef(root, session, marker, current, ctx));
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
  // `agent`: this is a subagent's own stop. Only a `SubagentStop` counts (Claude Code
  // turns a subagent's `Stop` into one): a `Stop` that carried an `agent_id` anyway is the main thread's.
  const subagentStop = input.hook_event_name === "SubagentStop";
  // `leadRoots`: the project's own checkout, never a subagent's to answer for.
  const ctx = { agent: subagentStop ? agentKey(input) : undefined, here, state, leadRoots: hookRoots({}) };
  if (subagentStop && !ctx.agent) {
    // A subagent's stop that names no valid agent cannot be told apart from the
    // others: judging it as the main thread's would block it on the lead's work and
    // touch the lead's baselines and snapshots. Judge nothing; the lead's stop will.
    warnings.push("Faulk's end-of-turn guard got a SubagentStop without a valid agent_id: this subagent's stop was not checked, the lead's stop judges its work.");
  } else {
  if (session && !ctx.agent) {
    ensureBaselines(session, here, changeFingerprint, "stop");
    // Calls that never got their PostToolUse (denied, interrupted, failed)
    // say nothing about who changed the code since: drop their snapshots.
    session.dropPending();
  }
  const results = roots.flatMap((root) => judge(root, session, ctx));
  // One warning per distinct text: several markers of a branch (lead and agents) say the same.
  warnings.push(...new Set(results.map((result) => result.warning).filter(Boolean)));
  const reasons = results.map((result) => result.reason).filter(Boolean);
  if (session && ctx.agent && reasons.length === 0) {
    // Its own stop let the subagent through: it has ended.
    session.endAgent(ctx.agent);
  }
  if (reasons.length > 0) {
    output.decision = "block";
    output.reason = reasons.join("\n\n");
  }
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
