/**
 * Records, for the current state of the code, the verification
 * (`last-run.json`), Judge Dredd's delivery review (`review.json`) or the
 * deferral of one stop (`deferral.json`), with a history in `history.jsonl`,
 * all in the records folder of the checkout (`faulk-qa` in its git
 * directory, `fingerprint.mjs` > qaDir). The verification clears the Stop hook until the next
 * change of the code; the review, for the whole task branch.
 *
 * Usage:
 *   node scripts/agent-qa/record.mjs verified --rust "<...>" [--render "<...>"] [--commands "<...>"]
 *       [--web "<...>" --browser "<...>" --exploration "<...>"] [--scripts "<...>"] [--latency "<...>"] [--note "<...>"]
 *     (the flags required depend on the changed files: the command prints them)
 *   node scripts/agent-qa/record.mjs reviewed --brief .scratch/judge-dredd/<issue>-delivery.md --dispositions "<...>"
 *     (requires Judge Dredd's answer to that brief, written by scripts/agent-review/run.mjs)
 *   node scripts/agent-qa/record.mjs deferred --reason "<the question for the user>"
 *
 * @doc docs/agents/verification.md
 * @doc docs/agents/judge-dredd.md
 */

import { createHash } from "node:crypto";
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";
import { answerIsComplete, briefHeader } from "../agent-review/brief-core.mjs";
import { BRIEF_DIR, isBriefPath } from "../agent-review/guard-core.mjs";
import {
  changedFiles,
  currentBranch,
  deferralPath,
  fingerprintOf,
  historyPath,
  lastPromptPath,
  readRecord,
  reviewPath,
  scriptRoot,
  verificationPath,
} from "./fingerprint.mjs";
import { agentDefinition } from "../agent-review/headless.mjs";
import { EVIDENCE_FLAGS, canDefer, missingEvidence, taskBranchIssue, verifiedCommand } from "./stop-core.mjs";

const fail = (message) => {
  console.error(`[agent-qa] ${message}`);
  process.exit(1);
};

const USAGE =
  'usage: node scripts/agent-qa/record.mjs verified --<evidence> "<...>" | reviewed --brief <path> --dispositions "<...>" | deferred --reason "<...>"';

let parsed;
try {
  parsed = parseArgs({
    allowPositionals: true,
    options: {
      ...Object.fromEntries(EVIDENCE_FLAGS.map((flag) => [flag, { type: "string" }])),
      note: { type: "string" },
      reason: { type: "string" },
      brief: { type: "string" },
      dispositions: { type: "string" },
    },
  });
} catch (error) {
  fail(`${error.message}. ${USAGE}`);
}
const { values, positionals } = parsed;
const status = positionals[0];
if (!["verified", "reviewed", "deferred"].includes(status) || positionals.length !== 1) {
  fail(USAGE);
}

const root = scriptRoot();
const files = changedFiles(root);
const fingerprint = fingerprintOf(root, files);
if (fingerprint === null) {
  console.log("[agent-qa] no change in the guarded code: nothing to record.");
  process.exit(0);
}

if (status === "verified") {
  const missing = missingEvidence(files, values);
  if (missing.length > 0) {
    fail(
      `missing evidence for the changed files: ${missing.map((f) => `--${f}`).join(", ")} (docs/agents/verification.md). Run those checks, then: ${verifiedCommand(files)}`,
    );
  }
} else if (status === "reviewed") {
  if (!values.brief?.trim() || !values.dispositions?.trim()) {
    fail(
      '--brief and --dispositions are required: node scripts/agent-qa/record.mjs reviewed --brief .scratch/judge-dredd/<issue>-delivery.md --dispositions "<each finding: fixed, or rejected and why>"',
    );
  }
  checkReview(values.brief.trim());
} else if (!values.reason?.trim()) {
  fail('--reason is required: node scripts/agent-qa/record.mjs deferred --reason "<the question for the user>"');
} else {
  // An unreadable record counts as consumed: refusing the deferral is the safe choice.
  const previous = existsSync(deferralPath(root))
    ? (readRecord(deferralPath(root)) ?? { consumedAt: "9999" })
    : null;
  if (!canDefer(previous, readRecord(lastPromptPath(root))?.at)) {
    fail(
      `a deferral already let a stop through since the user's last message: run the checks and record them (${verifiedCommand(files)}), and the delivery review if the branch has none (docs/agents/judge-dredd.md).`,
    );
  }
}

/** Refuses a review record that is not backed by Judge Dredd's answer to a current delivery brief. */
function checkReview(brief) {
  const name = brief.replaceAll("\\", "/");
  // Only a brief of the folder the guard protects counts: elsewhere, a brief
  // and its answer could be written by hand.
  if (!isBriefPath(name) || !/(^|\/)\.scratch\/judge-dredd\/\d+-delivery\.md$/.test(name)) {
    fail(`${brief} is not a delivery brief of ${BRIEF_DIR}: run node scripts/agent-review/brief.mjs <issue> delivery.`);
  }
  const issue = name.match(/(\d+)-delivery\.md$/)[1];
  const briefFile = join(root, BRIEF_DIR, `${issue}-delivery.md`);
  if (!existsSync(briefFile)) {
    fail(`${brief} does not exist in ${root}: run node scripts/agent-review/brief.mjs ${issue} delivery.`);
  }
  const branch = currentBranch(root);
  const branchIssue = taskBranchIssue(branch);
  if (branchIssue && branchIssue !== issue) {
    fail(`${brief} reviews issue #${issue}, but branch ${branch} is the task of issue #${branchIssue}.`);
  }
  const content = readFileSync(briefFile);
  const header = briefHeader(content.toString("utf8"));
  if (header.checkpoint !== "delivery") {
    fail(`${brief} is not a delivery brief (checkpoint: delivery): run node scripts/agent-review/brief.mjs ${issue} delivery.`);
  }
  if (header.issueSource !== "github") {
    fail(
      `${brief} was not built from the GitHub issue (issue-source: ${header.issueSource ?? "none"}): run node scripts/agent-review/brief.mjs ${issue} delivery without --issue-file.`,
    );
  }
  if (header.fingerprint !== fingerprint) {
    fail(
      `${brief} was built for another state of the code (${header.fingerprint ?? "none"}, now ${fingerprint}): run node scripts/agent-review/brief.mjs ${issue} delivery, then node scripts/agent-review/run.mjs ${BRIEF_DIR}/${issue}-delivery.md.`,
    );
  }
  const answerFile = join(root, BRIEF_DIR, `${issue}-delivery.answer.md`);
  const briefHash = createHash("sha256").update(content).digest("hex");
  const answer = existsSync(answerFile) ? readFileSync(answerFile, "utf8") : "";
  const [hashLine, modelLine] = answer.split("\n");
  if (hashLine !== `brief-sha256: ${briefHash}`) {
    fail(`no Judge Dredd answer to this brief: run node scripts/agent-review/run.mjs ${BRIEF_DIR}/${issue}-delivery.md.`);
  }
  // Judge Dredd's own model, or Opus when the lead asks for a stronger review of a delicate task.
  const expectedModel = agentDefinition(root)["judge-dredd"].model ?? "opus";
  const allowed = [...new Set([expectedModel, "opus"])];
  if (!allowed.some((model) => modelLine === `model: ${model}`)) {
    fail(
      `the answer was not written by Judge Dredd's model (${modelLine ?? "none"}, expected model: ${allowed.join(" or ")}): run node scripts/agent-review/run.mjs ${BRIEF_DIR}/${issue}-delivery.md without --model, or with --model opus.`,
    );
  }
  if (!answerIsComplete(answer, "delivery")) {
    fail(
      `Judge Dredd's answer to ${brief} judges no acceptance criterion: run node scripts/agent-review/run.mjs ${BRIEF_DIR}/${issue}-delivery.md again.`,
    );
  }
}

const entry = {
  status,
  fingerprint,
  // The review stays valid for later fixes on this branch (`stopDecision`).
  branch: currentBranch(root),
  files,
  ...Object.fromEntries(EVIDENCE_FLAGS.filter((f) => values[f] !== undefined).map((f) => [f, values[f]])),
  note: values.note,
  brief: values.brief,
  dispositions: values.dispositions,
  reason: values.reason,
  at: new Date().toISOString(),
};
const path = { verified: verificationPath, reviewed: reviewPath, deferred: deferralPath }[status](root);
mkdirSync(dirname(path), { recursive: true });
writeFileSync(path, `${JSON.stringify(entry, null, 2)}\n`);
appendFileSync(historyPath(root), `${JSON.stringify(entry)}\n`);
console.log(`[agent-qa] ${status} recorded for fingerprint ${fingerprint} in ${root}.`);
