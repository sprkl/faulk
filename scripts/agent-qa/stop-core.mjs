/**
 * Decisions of the end-of-turn guard, without input or output: a turn that
 * leaves the code changed ends only with, for the current state of the code, a
 * recorded verification and a delivery review of the task's branch. A deferral
 * lets exactly one stop through, the one where the agent asks the user a
 * question.
 *
 * @doc docs/agents/verification.md
 * @doc docs/agents/judge-dredd.md
 */

/**
 * Evidence `record.mjs verified` requires, by area of the code
 * (`docs/agents/verification.md`). `applies` receives a changed file path with
 * forward slashes.
 */
export const EVIDENCE = [
  {
    flag: "rust",
    what: "cargo fmt --all --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test of the crates verify.mjs selects (the touched ones and their dependents; --workspace when it says so or with --full) and the WASM builds it names, with their results (verify.mjs prints the line)",
    applies: (file) =>
      file.startsWith("crates/") ||
      file === "Cargo.toml" ||
      file === "Cargo.lock" ||
      (file.startsWith("modules/") && (file.endsWith(".rs") || file.endsWith("/Cargo.toml"))),
  },
  {
    flag: "render",
    what: "offline render tests asserting on the signal, the allocation detector on the audio path, native/WASM parity, with their results",
    applies: (file) => /^crates\/engine[^/]*\//.test(file) || file.startsWith("modules/"),
  },
  {
    flag: "commands",
    what: "schema, description and undo tests of each changed command, the command/MCP tool parity test, the command run from a real agent",
    applies: (file) => file.startsWith("crates/commands/") || file.startsWith("crates/helper/"),
  },
  {
    flag: "web",
    what: "typecheck, lint and tests of the web app, with their results",
    applies: (file) => file.startsWith("web/") || file.startsWith("site/"),
  },
  {
    flag: "browser",
    what: "the Playwright check in a real browser: audio context started, acceptance criteria and changed screens exercised, console read",
    applies: (file) => file.startsWith("web/") || file.startsWith("site/"),
  },
  {
    flag: "exploration",
    what: "what you tried beyond the scripted flow (double clicks, reload during playback, helper connected then disconnected, tab in the background) and what you saw",
    applies: (file) => file.startsWith("web/") || file.startsWith("site/"),
  },
  {
    flag: "scripts",
    what: "the result of node --test 'scripts/**/*.test.mjs', and how the changed hooks or scripts were tried",
    applies: (file) => file.startsWith("scripts/") || file.startsWith(".claude/"),
  },
  {
    flag: "latency",
    what: "the output latency (and round trip with input) measured in both modes with the method, or 'not measured: <why>'; never a number you did not measure",
    applies: (file) =>
      /^crates\/(helper|protocol|engine-wasm)\//.test(file) || (file.startsWith("web/") && /worklet/i.test(file)),
  },
];

const FLAGS = EVIDENCE.map((e) => e.flag);

/** Flags `record.mjs verified` requires for these changed files, in `EVIDENCE` order. */
export function requiredEvidence(files) {
  const normalized = files.map((file) => file.replaceAll("\\", "/"));
  return EVIDENCE.filter((e) => normalized.some((file) => e.applies(file))).map((e) => e.flag);
}

/** Required flags missing (absent or blank) from `values`. */
export function missingEvidence(files, values) {
  return requiredEvidence(files).filter((flag) => typeof values[flag] !== "string" || values[flag].trim() === "");
}

export { FLAGS as EVIDENCE_FLAGS };

/** The issue of a task branch named by the convention (`feat|fix|spike/<issue>-<slug>`), or `null`. */
export function taskBranchIssue(branch) {
  return (typeof branch === "string" && branch.match(/^(?:feat|fix|spike)\/(\d+)-/)?.[1]) || null;
}

/**
 * A task branch named by the convention, where a review names the task. On
 * any other branch (`develop`, `main`, a detached head, a free name) a review
 * holds only for the state it reviewed.
 */
export const isTaskBranch = (branch) => taskBranchIssue(branch) !== null;

/**
 * How many verifications, and how many reviews, a checkout keeps (#492): one
 * per state, so that the branches stacked in one checkout keep theirs. The
 * oldest go first; a state whose record went is asked for it again.
 */
export const RECORD_LIMIT = 64;

/**
 * `list` with `entry` added last: an entry of the same fingerprint, status and
 * branch is replaced (recorded again, it moves to the end), and only the last
 * `limit` entries are kept.
 */
export function addRecord(list, entry, limit = RECORD_LIMIT) {
  const kept = (Array.isArray(list) ? list : []).filter(
    (old) =>
      !(old?.fingerprint === entry.fingerprint && old?.status === entry.status && (old?.branch ?? null) === (entry.branch ?? null)),
  );
  return [...kept, entry].slice(-limit);
}

/**
 * Decision for one checkout. `writer`: one of this session's own tool calls
 * changed the guarded code there; a session that did not is never blocked,
 * whatever other sessions or the user changed. `baseline`: the fingerprint the
 * first time the hooks saw this checkout in this session. `verifications` and
 * `reviews`: every record of the checkout (`fingerprint.mjs` > readRecords),
 * searched for the judged state, so that each branch of the checkout is judged
 * on its own records.
 *
 * Returns `{ allow, consumeDeferral?, missing?, reviewNotCarried? }`;
 * `reviewNotCarried`: a review of an earlier state of this branch exists, but
 * the branch is not a task branch, so it holds for that state only.
 */
export function stopDecision({ fingerprint, writer, baseline, branch, verifications = [], reviews = [], deferral }) {
  if (fingerprint === null || !writer) {
    return { allow: true };
  }
  if (baseline !== undefined && baseline !== "unknown" && baseline === fingerprint) {
    // The session undid its own changes.
    return { allow: true };
  }
  const verified = verifications.some((v) => v?.status === "verified" && v.fingerprint === fingerprint);
  // Judge Dredd reviews a task once, at the end: the review stays valid for
  // later fixes on the same task branch, which only need a new verification.
  const ofBranch = (r) => r?.status === "reviewed" && typeof branch === "string" && r.branch === branch;
  const reviewed = reviews.some(
    (r) => r?.status === "reviewed" && (r.fingerprint === fingerprint || (isTaskBranch(branch) && ofBranch(r))),
  );
  if (verified && reviewed) {
    return { allow: true };
  }
  if (deferral?.status === "deferred" && deferral.fingerprint === fingerprint && !deferral.consumedAt) {
    return { allow: true, consumeDeferral: true };
  }
  const missing = [];
  if (!verified) {
    missing.push("verification");
  }
  if (!reviewed) {
    missing.push("review");
  }
  const decision = { allow: false, missing };
  if (!reviewed && !isTaskBranch(branch) && reviews.some(ofBranch)) {
    decision.reviewNotCarried = true;
  }
  return decision;
}

/**
 * `<anything>/.claude/worktrees/agent-<id>`: the folder Claude Code gives a
 * subagent that runs isolated. `root` is a checkout root.
 */
export function isAgentWorktreePath(root) {
  const parts = String(root).replaceAll("\\", "/").replace(/\/+$/, "").split("/");
  const n = parts.length;
  return n >= 3 && parts[n - 3] === ".claude" && parts[n - 2] === "worktrees" && /^agent-[\w-]+$/.test(parts[n - 1]);
}

/**
 * Whether the lead's Stop leaves a subagent's writer marker unjudged: only a
 * subagent that is still running, in its own isolated worktree of this
 * repository, that is not a checkout the lead works in. Everything else (a
 * marker with no agent, an agent that went quiet, the lead's checkouts, a
 * folder that merely looks like an agent's) is judged as the session's own.
 */
export function exemptAgentMarker({ marker, running, agentWorktree, sameRepository, leadRoot }) {
  return Boolean(marker?.agent) && running && agentWorktree && sameRepository && !leadRoot;
}

/**
 * One deferral per user prompt: once a deferral has let a stop through, the
 * next one needs the user to have written since. Without this rule, a blocked
 * agent would defer at each stop (after touching the code, if the rule were per
 * state) and never verify.
 */
export function canDefer(existing, lastPromptAt) {
  if (!existing?.consumedAt) {
    return true;
  }
  return typeof lastPromptAt === "string" && existing.consumedAt < lastPromptAt;
}

const quote = (flag) => `--${flag} "<${EVIDENCE.find((e) => e.flag === flag).what}>"`;

/** The exact `record.mjs verified` command for these changed files. */
export function verifiedCommand(files) {
  const flags = requiredEvidence(files);
  return ["node scripts/agent-qa/record.mjs verified", ...flags.map(quote)].join(" ");
}

/** The message the Stop hook gives the agent when it blocks. */
export function blockReason({ root, fingerprint, files, missing, branch, reviewNotCarried = false }) {
  const steps = [];
  if (missing.includes("verification")) {
    steps.push(
      `run the checks of docs/agents/verification.md for the changed areas, then record them: ${verifiedCommand(files)}`,
    );
  }
  if (missing.includes("review")) {
    const why = reviewNotCarried
      ? `a delivery review of an earlier state of ${branch} exists, but it is not carried over: ${branch} is not a task branch (feat|fix|spike/<issue>-<slug>, CLAUDE.md > Branches), and on any other branch a review holds only for the state it reviewed, so this state needs its own (name the next task branch by the convention; renaming this one does not carry the old review)`
      : "this branch has no delivery review yet";
    steps.push(
      `${why}: once the task is done and before opening its pull request (docs/agents/judge-dredd.md), write your report draft to .scratch/judge-dredd/<issue>-delivery.proposal.md, run node scripts/agent-review/brief.mjs <issue> delivery, then node scripts/agent-review/run.mjs .scratch/judge-dredd/<issue>-delivery.md, show the user every finding with your opinion, then: node scripts/agent-qa/record.mjs reviewed --brief .scratch/judge-dredd/<issue>-delivery.md --dispositions "<each finding: fixed, or rejected and why>". One review per task branch (feat|fix|spike/<issue>-<slug>): later fixes on the same task branch only need a new verification`,
    );
  }
  const shown = files.slice(0, 10).join(", ") + (files.length > 10 ? `, and ${files.length - 10} more` : "");
  return [
    `The code changed by this session in ${root ?? "this checkout"} (fingerprint ${fingerprint}: ${shown}) is not cleared to end the turn.`,
    `Before ending, from that directory: ${steps.join("; and ")}.`,
    'Only if you are stopping to ask the user a question mid-task: node scripts/agent-qa/record.mjs deferred --reason "<the question>"; a deferral lets exactly one stop through. Never defer at the end of a task.',
  ].join(" ");
}
