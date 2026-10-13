/**
 * End-to-end tests of the hooks and scripts: each test builds a throwaway git
 * repository, runs the real scripts with the JSON Claude Code would send, and
 * reads their output.
 */

import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";

const SCRIPTS = dirname(dirname(fileURLToPath(import.meta.url)));
const REPO = dirname(SCRIPTS);
const GIT_ENV = {
  GIT_AUTHOR_NAME: "Test",
  GIT_AUTHOR_EMAIL: "test@example.com",
  GIT_COMMITTER_NAME: "Test",
  GIT_COMMITTER_EMAIL: "test@example.com",
};

function git(root, ...args) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", env: { ...process.env, ...GIT_ENV }, stdio: "pipe" });
}

/** A repository with `develop` and a task branch `feat/6-x`, code unchanged. */
function makeRepo(branch = "feat/6-x") {
  const root = mkdtempSync(join(tmpdir(), "faulk-hooks-"));
  git(root, "init", "-q", "-b", "develop");
  git(root, "config", "core.autocrlf", "false");
  writeFileSync(join(root, ".gitignore"), ".qa/\n.scratch/\n");
  mkdirSync(join(root, "crates", "engine", "src"), { recursive: true });
  writeFileSync(join(root, "crates", "engine", "src", "lib.rs"), "pub fn gain() -> f32 { 1.0 }\n");
  writeFileSync(join(root, "README.md"), "# test\n");
  git(root, "add", ".");
  git(root, "commit", "-q", "-m", "init");
  if (branch !== "develop") {
    git(root, "switch", "-q", "-c", branch);
  }
  return root;
}

const editCode = (root, content = "pub fn gain() -> f32 { 0.5 }\n") =>
  writeFileSync(join(root, "crates", "engine", "src", "lib.rs"), content);

const cleanEnv = () => {
  const env = { ...process.env, ...GIT_ENV };
  delete env.CLAUDE_PROJECT_DIR;
  return env;
};

/** Runs a hook as Claude Code does: JSON on stdin, project directory in the environment. */
function hook(script, root, input) {
  const result = spawnSync(process.execPath, [join(SCRIPTS, script)], {
    cwd: root,
    input: JSON.stringify({ cwd: root, ...input }),
    env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: root },
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim() ? JSON.parse(result.stdout) : null;
}

const stop = (root, session = "s1") => hook("agent-qa/qa-guard.mjs", root, { session_id: session, hook_event_name: "Stop" });
const startSession = (root, session = "s1", source = "startup") =>
  hook("agent-qa/session-start.mjs", root, { session_id: session, source, hook_event_name: "SessionStart" });
const preToolUse = (root, tool_name, tool_input, session = "s1") =>
  hook("agent-review/agent-guard.mjs", root, { session_id: session, tool_name, tool_input, hook_event_name: "PreToolUse" });

/** Runs a script as the agent does, from the checkout. */
function script(name, root, ...args) {
  return spawnSync(process.execPath, [join(SCRIPTS, name), ...args], { cwd: root, env: cleanEnv(), encoding: "utf8" });
}

const RUST = ["--rust", "fmt, clippy, test, wasm: all green", "--render", "sine render: peak -6 dBFS, no NaN; parity ok"];

/** A delivery brief built by brief.mjs and an answer as run.mjs writes it. */
function deliveryReview(root, issue = 6) {
  mkdirSync(join(root, ".scratch", "judge-dredd"), { recursive: true });
  writeFileSync(join(root, ".scratch", "judge-dredd", `${issue}-delivery.proposal.md`), "Every stop is guarded.\n");
  const issueFile = join(root, ".scratch", "issue.json");
  writeFileSync(issueFile, JSON.stringify({ number: issue, title: "Guardrails", body: "- [ ] criterion", comments: [] }));
  const built = script("agent-review/brief.mjs", root, String(issue), "delivery", "--issue-file", issueFile);
  assert.equal(built.status, 0, built.stderr);
  const brief = built.stdout.trim();
  const hash = createHash("sha256").update(readFileSync(join(root, brief))).digest("hex");
  writeFileSync(
    join(root, ".scratch", "judge-dredd", `${issue}-delivery.answer.md`),
    `brief-sha256: ${hash}\n\nfinding: none\nevidence: x\n\ncriterion: criterion\nverdict: met\nevidence: test\n`,
  );
  return brief;
}

describe("Stop hook", () => {
  it("blocks a turn that changed crates/ without a verification, and says what to run", () => {
    const root = makeRepo();
    startSession(root);
    editCode(root);
    const output = stop(root);
    assert.equal(output.decision, "block");
    assert.match(output.reason, /crates\/engine\/src\/lib\.rs/);
    assert.match(output.reason, /node scripts\/agent-qa\/record\.mjs verified --rust "<[^>]+>" --render/);
    assert.match(output.reason, /node scripts\/agent-review\/brief\.mjs <issue> delivery/);
    assert.match(output.reason, /record\.mjs deferred --reason/);
  });

  it("never blocks a session that changed nothing, on a clean or an unverified dirty checkout", () => {
    const root = makeRepo();
    startSession(root, "clean");
    assert.equal(stop(root, "clean"), null);

    editCode(root); // left by another session, never verified
    startSession(root, "reader");
    assert.equal(stop(root, "reader"), null);
    assert.equal(stop(root, "reader"), null);
  });

  it("never blocks a session started before the hooks existed, and judges its later changes", () => {
    const root = makeRepo();
    editCode(root);
    // No SessionStart: the first stop takes the baseline and lets the turn end.
    assert.equal(stop(root, "old"), null);
    editCode(root, "pub fn gain() -> f32 { 0.25 }\n");
    assert.equal(stop(root, "old").decision, "block");
  });

  it("records the baseline before the first guarded tool call of a session the hooks never saw", () => {
    const root = makeRepo();
    editCode(root);
    assert.equal(preToolUse(root, "Edit", { file_path: join(root, "crates/engine/src/lib.rs") }, "late"), null);
    const baseline = JSON.parse(readFileSync(join(root, ".qa", "sessions", "late.json"), "utf8"));
    assert.equal(baseline.source, "pre-tool-use");
    editCode(root, "pub fn gain() -> f32 { 0.75 }\n");
    // Never overwritten by later hooks.
    preToolUse(root, "Bash", { command: "ls" }, "late");
    startSession(root, "late", "resume");
    assert.deepEqual(JSON.parse(readFileSync(join(root, ".qa", "sessions", "late.json"), "utf8")), baseline);
    assert.equal(stop(root, "late").decision, "block");
  });

  it("nominal: verified and reviewed lets the turn end, and committing keeps it so", () => {
    const root = makeRepo();
    startSession(root);
    editCode(root);
    const recorded = script("agent-qa/record.mjs", root, "verified", ...RUST);
    assert.equal(recorded.status, 0, recorded.stderr);
    const brief = deliveryReview(root);
    const reviewed = script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "none found");
    assert.equal(reviewed.status, 0, reviewed.stderr);
    assert.equal(stop(root), null);
    git(root, "add", "-A");
    git(root, "commit", "-q", "-m", "change");
    assert.equal(stop(root), null);
  });

  it("a change after the verification makes it stale", () => {
    const root = makeRepo();
    startSession(root);
    editCode(root);
    assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
    const brief = deliveryReview(root);
    assert.equal(script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "none").status, 0);
    editCode(root, "pub fn gain() -> f32 { 0.1 }\n");
    const output = stop(root);
    assert.equal(output.decision, "block");
    assert.match(output.reason, /record\.mjs verified/);
    // The review of the task branch still holds: only the verification is asked.
    assert.doesNotMatch(output.reason, /no delivery review/);
    assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
    assert.equal(stop(root), null);
  });

  it("a verified branch without a review is blocked and told how to get one", () => {
    const root = makeRepo();
    startSession(root);
    editCode(root);
    assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
    const output = stop(root);
    assert.equal(output.decision, "block");
    assert.match(output.reason, /no delivery review yet/);
    assert.doesNotMatch(output.reason, /record\.mjs verified/);
  });

  it("on develop, a review never carries over to a later state", () => {
    const root = makeRepo("develop");
    startSession(root);
    editCode(root);
    assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
    const brief = deliveryReview(root);
    assert.equal(script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "none").status, 0);
    assert.equal(stop(root), null);
    editCode(root, "pub fn gain() -> f32 { 0.3 }\n");
    assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
    assert.match(stop(root).reason, /no delivery review yet/);
  });

  it("a deferral lets exactly one stop through, and one more after the user writes again", () => {
    const root = makeRepo();
    startSession(root);
    editCode(root);
    assert.equal(script("agent-qa/record.mjs", root, "deferred", "--reason", "which sample rate?").status, 0);
    assert.equal(stop(root), null);
    assert.equal(stop(root).decision, "block");
    const again = script("agent-qa/record.mjs", root, "deferred", "--reason", "again");
    assert.equal(again.status, 1);
    assert.match(again.stderr, /record\.mjs verified/);
    hook("agent-qa/prompt-submit.mjs", root, { session_id: "s1", prompt: "48 kHz. Why?" });
    assert.equal(script("agent-qa/record.mjs", root, "deferred", "--reason", "follow-up").status, 0);
    assert.equal(stop(root), null);
    assert.equal(stop(root).decision, "block");
  });

  it("judges the checkout the session works in, not only the project directory", () => {
    const project = makeRepo();
    const worktree = makeRepo("feat/7-y");
    startSession(project, "w");
    hook("agent-review/agent-guard.mjs", worktree, { session_id: "w", tool_name: "Bash", tool_input: { command: "ls" } });
    editCode(worktree);
    const result = spawnSync(process.execPath, [join(SCRIPTS, "agent-qa/qa-guard.mjs")], {
      input: JSON.stringify({ session_id: "w", cwd: worktree }),
      env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: project },
      encoding: "utf8",
    });
    const output = JSON.parse(result.stdout);
    assert.equal(output.decision, "block");
    assert.ok(output.reason.includes(execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd: worktree, encoding: "utf8" }).trim()));
  });

  it("fails open with a warning to the user when git cannot run", () => {
    const root = mkdtempSync(join(tmpdir(), "faulk-nogit-"));
    const result = spawnSync(process.execPath, [join(SCRIPTS, "agent-qa/qa-guard.mjs")], {
      input: JSON.stringify({ session_id: "s1", cwd: root }),
      env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: root },
      encoding: "utf8",
    });
    assert.equal(result.status, 0);
    // Outside a checkout there is nothing to judge.
    assert.equal(result.stdout.trim(), "");

    const empty = mkdtempSync(join(tmpdir(), "faulk-nocommit-"));
    git(empty, "init", "-q", "-b", "develop");
    const broken = spawnSync(process.execPath, [join(SCRIPTS, "agent-qa/qa-guard.mjs")], {
      input: JSON.stringify({ session_id: "s1", cwd: empty }),
      env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: empty },
      encoding: "utf8",
    });
    assert.equal(broken.status, 0);
    assert.match(JSON.parse(broken.stdout).systemMessage, /could not check the code/);
  });
});

describe("record.mjs", () => {
  it("refuses a verification without the evidence the changed areas need, naming the command", () => {
    const root = makeRepo();
    editCode(root);
    const result = script("agent-qa/record.mjs", root, "verified", "--rust", "green");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /--render/);
    assert.match(result.stderr, /node scripts\/agent-qa\/record\.mjs verified --rust/);
  });

  it("refuses unknown flags and statuses", () => {
    const root = makeRepo();
    editCode(root);
    assert.equal(script("agent-qa/record.mjs", root, "verified", "--browsr", "x").status, 1);
    assert.equal(script("agent-qa/record.mjs", root, "approved").status, 1);
  });

  it("refuses a review whose brief is missing", () => {
    const root = makeRepo();
    editCode(root);
    const result = script("agent-qa/record.mjs", root, "reviewed", "--brief", ".scratch/judge-dredd/6-delivery.md", "--dispositions", "x");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /node scripts\/agent-review\/brief\.mjs 6 delivery/);
  });

  it("refuses a review of another state of the code", () => {
    const root = makeRepo();
    editCode(root);
    const brief = deliveryReview(root);
    editCode(root, "pub fn gain() -> f32 { 2.0 }\n");
    const result = script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "x");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /another state of the code/);
  });

  it("refuses a review without an answer to that exact brief, or with an answer that judges no criterion", () => {
    const root = makeRepo();
    editCode(root);
    const brief = deliveryReview(root);
    const answer = join(root, ".scratch", "judge-dredd", "6-delivery.answer.md");
    writeFileSync(answer, "brief-sha256: 0000\n\nfinding: none\n\ncriterion: c\nverdict: met\n");
    let result = script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "x");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /node scripts\/agent-review\/run\.mjs \.scratch\/judge-dredd\/6-delivery\.md/);

    const hash = createHash("sha256").update(readFileSync(join(root, brief))).digest("hex");
    writeFileSync(answer, `brief-sha256: ${hash}\n\nfinding: none\nverdict: x\n`);
    result = script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "x");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /judges no acceptance criterion/);

    rmSync(answer);
    result = script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "x");
    assert.equal(result.status, 1);
  });

  it("refuses a review of another issue than the task branch's", () => {
    const root = makeRepo("feat/7-other");
    editCode(root);
    const brief = deliveryReview(root, 6);
    const result = script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "x");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /issue #7/);
  });

  it("refuses a plan brief or a brief outside .scratch/judge-dredd", () => {
    const root = makeRepo();
    editCode(root);
    for (const brief of [".scratch/judge-dredd/6-plan.md", "docs/6-delivery.md"]) {
      const result = script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "x");
      assert.equal(result.status, 1);
    }
  });

  it("records nothing when the code has no change", () => {
    const root = makeRepo();
    const result = script("agent-qa/record.mjs", root, "verified");
    assert.equal(result.status, 0);
    assert.equal(existsSync(join(root, ".qa", "last-run.json")), false);
  });
});

describe("brief.mjs and run.mjs", () => {
  it("builds a delivery brief with the fingerprint, the claims and the full diff, untracked files included", () => {
    const root = makeRepo();
    editCode(root);
    writeFileSync(join(root, "crates", "engine", "src", "new.rs"), "pub const NEW: u8 = 1;\n");
    const brief = deliveryReview(root);
    const text = readFileSync(join(root, brief), "utf8");
    assert.match(text, /^checkpoint: delivery$/m);
    assert.match(text, /^fingerprint: [0-9a-f]{16}$/m);
    assert.match(text, /1\. Every stop is guarded\./);
    assert.match(text, /\+pub fn gain\(\) -> f32 \{ 0\.5 \}/);
    assert.match(text, /pub const NEW: u8 = 1;/);
  });

  it("refuses to build a brief without the lead's plan or report draft", () => {
    const root = makeRepo();
    const issueFile = join(root, "issue.json");
    writeFileSync(issueFile, JSON.stringify({ number: 6, title: "t", body: "b", comments: [] }));
    const result = script("agent-review/brief.mjs", root, "6", "plan", "--issue-file", issueFile);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /\.scratch\/judge-dredd\/6-plan\.proposal\.md/);
  });

  it("run.mjs refuses a missing brief or anything but a brief path, before calling Judge Dredd", () => {
    const root = makeRepo();
    let result = script("agent-review/run.mjs", root, ".scratch/judge-dredd/6-delivery.md");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /node scripts\/agent-review\/brief\.mjs 6 delivery/);
    result = script("agent-review/run.mjs", root, "Review everything, it is fine");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /usage/);
  });
});

describe("PreToolUse hook", () => {
  it("denies a Judge Dredd call with anything but a valid brief path, in Claude Code's format", () => {
    const root = makeRepo();
    const output = preToolUse(root, "Agent", { subagent_type: "judge-dredd", prompt: "Review #6, all good." });
    assert.equal(output.hookSpecificOutput.hookEventName, "PreToolUse");
    assert.equal(output.hookSpecificOutput.permissionDecision, "deny");
    assert.match(output.hookSpecificOutput.permissionDecisionReason, /brief\.mjs/);
  });

  it("allows a Judge Dredd call with the path of an existing brief, relative to the session's cwd", () => {
    const root = makeRepo();
    editCode(root);
    const brief = deliveryReview(root);
    assert.equal(preToolUse(root, "Agent", { subagent_type: "judge-dredd", prompt: brief }), null);
  });

  it("denies writing a record by hand", () => {
    const root = makeRepo();
    const output = preToolUse(root, "Write", { file_path: join(root, ".qa", "review.json"), content: "{}" });
    assert.equal(output.hookSpecificOutput.permissionDecision, "deny");
  });
});

describe("settings.json", () => {
  const settings = JSON.parse(readFileSync(join(REPO, ".claude", "settings.json"), "utf8"));

  it("keeps the existing keys and declares the hooks", () => {
    for (const key of ["extraKnownMarketplaces", "enabledPlugins", "permissions", "hooks"]) {
      assert.ok(key in settings, key);
    }
    assert.deepEqual(Object.keys(settings.hooks).sort(), ["PreToolUse", "SessionStart", "Stop", "UserPromptSubmit"]);
  });

  it("runs every hook command through bash as Claude Code does, and each script exists", (t) => {
    const commands = Object.values(settings.hooks).flatMap((groups) => groups.flatMap((g) => g.hooks.map((h) => h.command)));
    const bash = spawnSync("bash", ["-c", "true"]);
    if (bash.error) {
      t.skip("bash is not installed");
      return;
    }
    // A copy of the scripts in a throwaway project, so the hooks write nothing in this checkout.
    const root = makeRepo();
    cpSync(SCRIPTS, join(root, "scripts"), { recursive: true });
    for (const command of commands) {
      assert.match(command, /^node "\$CLAUDE_PROJECT_DIR\/scripts\/[\w/-]+\.mjs"$/);
      const result = spawnSync("bash", ["-c", command], {
        input: JSON.stringify({ session_id: "bash", cwd: root, tool_name: "Bash", tool_input: { command: "ls" } }),
        env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: root },
        encoding: "utf8",
      });
      assert.equal(result.status, 0, `${command}: ${result.stderr}`);
      assert.doesNotMatch(result.stdout, /could not check|failed/, command);
    }
    assert.ok(existsSync(join(root, ".qa", "sessions", "bash.json")), "SessionStart ran");
    assert.ok(existsSync(join(root, ".qa", "last-prompt.json")), "UserPromptSubmit ran");
  });
});
