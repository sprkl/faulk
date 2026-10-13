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
    what: "cargo fmt --all --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test --workspace and the WASM build of the engine, with their results",
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

/** A task branch, not `develop`, `main` or a detached head, where a review would not name the task. */
export function isTaskBranch(branch) {
  return typeof branch === "string" && branch !== "" && !["develop", "main", "HEAD"].includes(branch);
}

/**
 * `session` is the record the hooks wrote the first time they saw this
 * session (`{ fingerprint }`), `null` when they never saw it, `undefined` when
 * the session id is unknown. A session judged only on what it changed: a
 * session that leaves the code as the hooks first found it is never blocked.
 *
 * Returns `{ allow, recordBaseline?, consumeDeferral?, missing? }`.
 */
export function stopDecision({ fingerprint, session, branch, verification, review, deferral }) {
  if (fingerprint === null) {
    return { allow: true };
  }
  if (session === null) {
    // No tool call of this session went through the PreToolUse hook, and
    // SessionStart did not run (session started before the hooks existed):
    // this session changed nothing under the hooks. Its baseline is now.
    return { allow: true, recordBaseline: true };
  }
  if (session && session.fingerprint === fingerprint) {
    return { allow: true };
  }
  const verified = verification?.fingerprint === fingerprint && verification.status === "verified";
  // Judge Dredd reviews a task once, at the end: the review stays valid for
  // later fixes on the same task branch, which only need a new verification.
  const reviewed =
    review?.status === "reviewed" &&
    (review.fingerprint === fingerprint || (isTaskBranch(branch) && review.branch === branch));
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
  return { allow: false, missing };
}

/**
 * One deferral per state of the code and per user prompt: without this rule, a
 * blocked agent would defer again at each stop and never verify. A new user
 * prompt (the user answered) allows a new question on the same state.
 */
export function canDefer(existing, fingerprint, lastPromptAt) {
  if (!(existing?.fingerprint === fingerprint && existing.consumedAt)) {
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
export function blockReason({ root, fingerprint, files, missing }) {
  const steps = [];
  if (missing.includes("verification")) {
    steps.push(
      `run the checks of docs/agents/verification.md for the changed areas, then record them: ${verifiedCommand(files)}`,
    );
  }
  if (missing.includes("review")) {
    steps.push(
      'this branch has no delivery review yet: once the task is done and before opening its pull request (docs/agents/judge-dredd.md), write your report draft to .scratch/judge-dredd/<issue>-delivery.proposal.md, run node scripts/agent-review/brief.mjs <issue> delivery, then node scripts/agent-review/run.mjs .scratch/judge-dredd/<issue>-delivery.md, show the user every finding with your opinion, then: node scripts/agent-qa/record.mjs reviewed --brief .scratch/judge-dredd/<issue>-delivery.md --dispositions "<each finding: fixed, or rejected and why>". One review per task branch: later fixes on the same branch only need a new verification',
    );
  }
  const shown = files.slice(0, 10).join(", ") + (files.length > 10 ? `, and ${files.length - 10} more` : "");
  return [
    `The code changed by this session in ${root ?? "this checkout"} (fingerprint ${fingerprint}: ${shown}) is not cleared to end the turn.`,
    `Before ending, from that directory: ${steps.join("; and ")}.`,
    'Only if you are stopping to ask the user a question mid-task: node scripts/agent-qa/record.mjs deferred --reason "<the question>"; a deferral lets exactly one stop through. Never defer at the end of a task.',
  ].join(" ");
}
