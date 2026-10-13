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
import { changeFingerprint } from "./fingerprint.mjs";

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

/** A repository with `develop` and a task branch, code unchanged, Judge Dredd's definition committed. */
function makeRepo(branch = "feat/6-x") {
  const root = mkdtempSync(join(tmpdir(), "faulk-hooks-"));
  git(root, "init", "-q", "-b", "develop");
  git(root, "config", "core.autocrlf", "false");
  writeFileSync(join(root, ".gitignore"), ".qa/\n.scratch/\n");
  mkdirSync(join(root, "crates", "engine", "src"), { recursive: true });
  writeFileSync(join(root, "crates", "engine", "src", "lib.rs"), "pub fn gain() -> f32 { 1.0 }\n");
  mkdirSync(join(root, ".claude", "agents"), { recursive: true });
  cpSync(join(REPO, ".claude", "agents", "judge-dredd.md"), join(root, ".claude", "agents", "judge-dredd.md"));
  writeFileSync(join(root, "README.md"), "# test\n");
  git(root, "add", ".");
  git(root, "commit", "-q", "-m", "init");
  if (branch !== "develop") {
    git(root, "switch", "-q", "-c", branch);
  }
  return root;
}

const toplevel = (dir) => git(dir, "rev-parse", "--show-toplevel").trim();
/** The id `session.mjs` gives a checkout in its file names. */
const rootIdOf = (dir) => {
  const root = toplevel(dir).replaceAll("\\", "/");
  return createHash("sha256")
    .update(process.platform === "win32" ? root.toLowerCase() : root)
    .digest("hex")
    .slice(0, 16);
};

const LIB = ["crates", "engine", "src", "lib.rs"];
const writeCode = (root, content = "pub fn gain() -> f32 { 0.5 }\n") => writeFileSync(join(root, ...LIB), content);

const cleanEnv = () => {
  const env = { ...process.env, ...GIT_ENV };
  delete env.CLAUDE_PROJECT_DIR;
  return env;
};

/** Runs a hook as Claude Code does: JSON on stdin, project directory in the environment. */
function hook(script, project, input) {
  const result = spawnSync(process.execPath, [join(SCRIPTS, script)], {
    cwd: project,
    input: JSON.stringify({ cwd: project, ...input }),
    env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: project },
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim() ? JSON.parse(result.stdout) : null;
}

const stop = (root, session = "s1", extra = {}) => hook("agent-qa/qa-guard.mjs", root, { session_id: session, ...extra });
const startSession = (root, session = "s1", source = "startup") =>
  hook("agent-qa/session-start.mjs", root, { session_id: session, source });
const preToolUse = (root, tool_name, tool_input, session = "s1", tool_use_id = "t0", extra = {}) =>
  hook("agent-review/agent-guard.mjs", root, { session_id: session, tool_name, tool_input, tool_use_id, ...extra });

let calls = 0;
/** One tool call of a session, between its PreToolUse and PostToolUse hooks. */
function toolCall(root, session, action, { cwdBefore = root, cwdAfter = cwdBefore, project = root } = {}) {
  const id = `call-${++calls}`;
  const run = (script, cwd) =>
    spawnSync(process.execPath, [join(SCRIPTS, script)], {
      input: JSON.stringify({ session_id: session, cwd, tool_name: "Bash", tool_input: { command: "x" }, tool_use_id: id }),
      env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: project },
      encoding: "utf8",
    });
  assert.equal(run("agent-review/agent-guard.mjs", cwdBefore).status, 0);
  action();
  assert.equal(run("agent-qa/post-tool-use.mjs", cwdAfter).status, 0);
}

/** Runs a script as the agent does, from the checkout. */
function script(name, root, ...args) {
  return spawnSync(process.execPath, [join(SCRIPTS, name), ...args], { cwd: root, env: cleanEnv(), encoding: "utf8" });
}

const RUST = ["--rust", "fmt, clippy, test, wasm: all green", "--render", "sine render: peak -6 dBFS, no NaN; parity ok"];
const BRIEF = ".scratch/judge-dredd/6-delivery.md";

/**
 * A delivery brief as brief.mjs writes it from GitHub (the header is what
 * record.mjs reads) and an answer as run.mjs writes it.
 */
function deliveryReview(root, { issue = 6, model = "opus", answerBody } = {}) {
  const dir = join(root, ".scratch", "judge-dredd");
  mkdirSync(dir, { recursive: true });
  const brief = `# Judge Dredd brief — #${issue}\n\ncheckpoint: delivery\nissue-source: github\nfingerprint: ${changeFingerprint(root)}\n\nbody\n`;
  writeFileSync(join(dir, `${issue}-delivery.md`), brief);
  const hash = createHash("sha256").update(brief).digest("hex");
  const body = answerBody ?? "finding: none\nevidence: x\n\ncriterion: criterion\nverdict: met\nevidence: test\n";
  writeFileSync(join(dir, `${issue}-delivery.answer.md`), `brief-sha256: ${hash}\nmodel: ${model}\n\n${body}`);
  return `.scratch/judge-dredd/${issue}-delivery.md`;
}

const review = (root, brief = BRIEF) => script("agent-qa/record.mjs", root, "reviewed", "--brief", brief, "--dispositions", "none found");

describe("Stop hook: what blocks", () => {
  it("blocks a turn whose tool call changed crates/ without a verification, and says what to run", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => writeCode(root));
    const output = stop(root);
    assert.equal(output.decision, "block");
    assert.match(output.reason, /crates\/engine\/src\/lib\.rs/);
    assert.match(output.reason, /node scripts\/agent-qa\/record\.mjs verified --rust "<[^"]+>" --render "<[^"]+>"/);
    assert.match(output.reason, /node scripts\/agent-review\/brief\.mjs <issue> delivery/);
    assert.match(output.reason, /record\.mjs deferred --reason/);
  });

  it("nominal: verified and reviewed lets the turn end, and committing keeps it so", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => writeCode(root));
    const verified = script("agent-qa/record.mjs", root, "verified", ...RUST);
    assert.equal(verified.status, 0, verified.stderr);
    deliveryReview(root);
    const reviewed = review(root);
    assert.equal(reviewed.status, 0, reviewed.stderr);
    assert.equal(stop(root), null);
    toolCall(root, "s1", () => {
      git(root, "add", "-A");
      git(root, "commit", "-q", "-m", "change");
    });
    assert.equal(stop(root), null);
  });

  it("a change after the verification makes it stale; the branch's review still holds", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => writeCode(root));
    assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
    deliveryReview(root);
    assert.equal(review(root).status, 0);
    toolCall(root, "s1", () => writeCode(root, "pub fn gain() -> f32 { 0.1 }\n"));
    const output = stop(root);
    assert.equal(output.decision, "block");
    assert.match(output.reason, /record\.mjs verified/);
    assert.doesNotMatch(output.reason, /no delivery review/);
    assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
    assert.equal(stop(root), null);
  });

  it("a verified branch without a review is blocked and told how to get one", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => writeCode(root));
    assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
    const output = stop(root);
    assert.equal(output.decision, "block");
    assert.match(output.reason, /no delivery review yet/);
    assert.doesNotMatch(output.reason, /record\.mjs verified/);
  });

  for (const branch of ["develop", "wip"]) {
    it(`on ${branch}, a review never carries over to a later state`, () => {
      const root = makeRepo(branch);
      startSession(root);
      toolCall(root, "s1", () => writeCode(root));
      assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
      deliveryReview(root);
      assert.equal(review(root).status, 0);
      assert.equal(stop(root), null);
      toolCall(root, "s1", () => writeCode(root, "pub fn gain() -> f32 { 0.3 }\n"));
      assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
      assert.match(stop(root).reason, /no delivery review yet/);
    });
  }

  it("a session that undoes its own change is let through", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => writeCode(root));
    toolCall(root, "s1", () => writeCode(root, "pub fn gain() -> f32 { 1.0 }\n"));
    assert.equal(stop(root), null);
  });

  it("a change left by a call that got no PostToolUse is still attributed at the stop", () => {
    const root = makeRepo();
    startSession(root);
    preToolUse(root, "Bash", { command: "cargo fmt" }, "s1", "lost");
    writeCode(root);
    assert.equal(stop(root).decision, "block");
  });

  it("judges a worktree the session wrote to, even when it stops elsewhere", () => {
    const project = makeRepo();
    const worktree = makeRepo("feat/7-y");
    startSession(project, "w");
    toolCall(worktree, "w", () => writeCode(worktree), { project });
    const output = stop(project, "w");
    assert.equal(output.decision, "block");
    const top = git(worktree, "rev-parse", "--show-toplevel").trim();
    assert.ok(output.reason.includes(top), output.reason);
  });

  it("attributes a change to a checkout entered and edited within one call", () => {
    const project = makeRepo();
    const worktree = makeRepo("feat/7-y");
    startSession(project, "cd");
    toolCall(worktree, "cd", () => writeCode(worktree), { cwdBefore: project, cwdAfter: worktree, project });
    assert.equal(stop(project, "cd", { cwd: worktree }).decision, "block");
  });
});

describe("Stop hook: a session that changes nothing is never blocked", () => {
  it("on a clean checkout, or one with unverified changes it found", () => {
    const root = makeRepo();
    startSession(root, "clean");
    assert.equal(stop(root, "clean"), null);
    writeCode(root); // left by someone else, never verified
    startSession(root, "reader");
    toolCall(root, "reader", () => {});
    assert.equal(stop(root, "reader"), null);
  });

  it("while another session or the user changes the code during it", () => {
    const root = makeRepo();
    startSession(root, "reader");
    startSession(root, "writer");
    toolCall(root, "reader", () => {});
    toolCall(root, "writer", () => writeCode(root));
    writeCode(root, "pub fn gain() -> f32 { 0.9 }\n"); // the user's editor
    toolCall(root, "reader", () => {});
    assert.equal(stop(root, "reader"), null);
    assert.equal(stop(root, "writer").decision, "block");
  });

  it("when it started before the hooks existed, while its later changes are judged", () => {
    const root = makeRepo();
    writeCode(root);
    assert.equal(stop(root, "old"), null);
    toolCall(root, "old", () => writeCode(root, "pub fn gain() -> f32 { 0.25 }\n"));
    assert.equal(stop(root, "old").decision, "block");
  });

  it("when its baseline could not be computed", () => {
    const root = makeRepo();
    writeCode(root);
    const dir = join(root, ".qa", "sessions", "u");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, `baseline-${rootIdOf(root)}.json`), JSON.stringify({ root: toplevel(root), fingerprint: "unknown" }));
    toolCall(root, "u", () => {});
    assert.equal(stop(root, "u"), null);
  });

  it("when git cannot run: the hook fails open with a warning to the user", () => {
    const outside = mkdtempSync(join(tmpdir(), "faulk-nogit-"));
    assert.equal(stop(outside), null);
    const empty = mkdtempSync(join(tmpdir(), "faulk-nocommit-"));
    git(empty, "init", "-q", "-b", "develop");
    mkdirSync(join(empty, ".qa", "sessions", "s1"), { recursive: true });
    // A writer marker for a checkout whose fingerprint cannot be computed.
    writeFileSync(join(empty, ".qa", "sessions", "s1", `wrote-${rootIdOf(empty)}.json`), JSON.stringify({ root: toplevel(empty) }));
    assert.match(stop(empty).systemMessage, /could not check the code/);
  });
});

describe("deferral", () => {
  it("lets exactly one stop through, and one more only after the user writes again", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => writeCode(root));
    assert.equal(script("agent-qa/record.mjs", root, "deferred", "--reason", "which sample rate?").status, 0);
    assert.equal(stop(root), null);
    assert.equal(stop(root).decision, "block");
    const again = script("agent-qa/record.mjs", root, "deferred", "--reason", "again");
    assert.equal(again.status, 1);
    assert.match(again.stderr, /record\.mjs verified --rust/);
    // Touching the code does not buy a new deferral.
    toolCall(root, "s1", () => writeCode(root, "pub fn gain() -> f32 { 0.6 }\n"));
    assert.equal(script("agent-qa/record.mjs", root, "deferred", "--reason", "after a change").status, 1);
    hook("agent-qa/prompt-submit.mjs", root, { session_id: "s1", prompt: "48 kHz. Why?" });
    assert.equal(script("agent-qa/record.mjs", root, "deferred", "--reason", "follow-up").status, 0);
    assert.equal(stop(root), null);
    assert.equal(stop(root).decision, "block");
  });
});

describe("record.mjs", () => {
  it("refuses a verification without the evidence the changed areas need, naming the command", () => {
    const root = makeRepo();
    writeCode(root);
    const result = script("agent-qa/record.mjs", root, "verified", "--rust", "green");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /--render/);
    assert.match(result.stderr, /node scripts\/agent-qa\/record\.mjs verified --rust/);
  });

  it("refuses unknown flags and statuses", () => {
    const root = makeRepo();
    writeCode(root);
    assert.equal(script("agent-qa/record.mjs", root, "verified", "--browsr", "x").status, 1);
    assert.equal(script("agent-qa/record.mjs", root, "approved").status, 1);
  });

  it("refuses a review whose brief is missing", () => {
    const root = makeRepo();
    writeCode(root);
    const result = review(root);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /node scripts\/agent-review\/brief\.mjs 6 delivery/);
  });

  it("refuses a review of another state of the code", () => {
    const root = makeRepo();
    writeCode(root);
    deliveryReview(root);
    writeCode(root, "pub fn gain() -> f32 { 2.0 }\n");
    const result = review(root);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /another state of the code/);
  });

  it("refuses a review without an answer to that exact brief", () => {
    const root = makeRepo();
    writeCode(root);
    deliveryReview(root);
    const answer = join(root, ".scratch", "judge-dredd", "6-delivery.answer.md");
    writeFileSync(answer, "brief-sha256: 0000\nmodel: opus\n\ncriterion: c\nverdict: met\n");
    let result = review(root);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /node scripts\/agent-review\/run\.mjs \.scratch\/judge-dredd\/6-delivery\.md/);
    rmSync(answer);
    result = review(root);
    assert.equal(result.status, 1);
  });

  it("refuses an answer that judges no criterion, or by another model than Judge Dredd's", () => {
    const root = makeRepo();
    writeCode(root);
    deliveryReview(root, { answerBody: "finding: none\nverdict: x\n" });
    assert.match(review(root).stderr, /judges no acceptance criterion/);
    deliveryReview(root, { model: "haiku" });
    assert.match(review(root).stderr, /expected model: opus/);
  });

  it("refuses a brief not built from the GitHub issue", () => {
    const root = makeRepo();
    writeCode(root);
    mkdirSync(join(root, ".scratch", "judge-dredd"), { recursive: true });
    writeFileSync(join(root, ".scratch", "judge-dredd", "6-delivery.proposal.md"), "Claims.\n");
    const issueFile = join(root, ".scratch", "issue.json");
    writeFileSync(issueFile, JSON.stringify({ number: 6, title: "t", body: "- [ ] easy", comments: [] }));
    assert.equal(script("agent-review/brief.mjs", root, "6", "delivery", "--issue-file", issueFile).status, 0);
    const brief = readFileSync(join(root, BRIEF));
    const hash = createHash("sha256").update(brief).digest("hex");
    writeFileSync(
      join(root, ".scratch", "judge-dredd", "6-delivery.answer.md"),
      `brief-sha256: ${hash}\nmodel: opus\n\ncriterion: easy\nverdict: met\n`,
    );
    const result = review(root);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /without --issue-file/);
  });

  it("refuses a review of another issue than the task branch's", () => {
    const root = makeRepo("feat/7-other");
    writeCode(root);
    deliveryReview(root, { issue: 6 });
    const result = review(root);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /issue #7/);
  });

  it("refuses a plan brief or a brief outside .scratch/judge-dredd", () => {
    const root = makeRepo();
    writeCode(root);
    for (const brief of [".scratch/judge-dredd/6-plan.md", "docs/6-delivery.md"]) {
      assert.equal(review(root, brief).status, 1);
    }
  });

  it("records nothing when the code has no change", () => {
    const root = makeRepo();
    assert.equal(script("agent-qa/record.mjs", root, "verified").status, 0);
    assert.equal(existsSync(join(root, ".qa", "last-run.json")), false);
  });
});

describe("brief.mjs and run.mjs", () => {
  it("builds a delivery brief with the fingerprint, the claims and the full diff, untracked files included", () => {
    const root = makeRepo();
    writeCode(root);
    writeFileSync(join(root, "crates", "engine", "src", "new.rs"), "pub const NEW: u8 = 1;\n");
    mkdirSync(join(root, ".scratch", "judge-dredd"), { recursive: true });
    writeFileSync(join(root, ".scratch", "judge-dredd", "6-delivery.proposal.md"), "Every stop is guarded.\n");
    const issueFile = join(root, ".scratch", "issue.json");
    writeFileSync(issueFile, JSON.stringify({ number: 6, title: "t", body: "b", comments: [] }));
    const built = script("agent-review/brief.mjs", root, "6", "delivery", "--issue-file", issueFile);
    assert.equal(built.status, 0, built.stderr);
    assert.equal(built.stdout.trim(), BRIEF);
    const text = readFileSync(join(root, BRIEF), "utf8");
    assert.match(text, /^checkpoint: delivery$/m);
    assert.match(text, /^issue-source: file$/m);
    assert.match(text, new RegExp(`^fingerprint: ${changeFingerprint(root)}$`, "m"));
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
    let result = script("agent-review/run.mjs", root, BRIEF);
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
    writeCode(root);
    deliveryReview(root);
    assert.equal(preToolUse(root, "Agent", { subagent_type: "judge-dredd", prompt: BRIEF }), null);
  });

  it("denies writing a record or a session's state by hand", () => {
    const root = makeRepo();
    for (const path of [join(root, ".qa", "review.json"), join(root, ".qa", "sessions", "s1", "wrote-0.json")]) {
      const output = preToolUse(root, "Write", { file_path: path, content: "{}" });
      assert.equal(output.hookSpecificOutput.permissionDecision, "deny");
    }
  });
});

describe("settings.json", () => {
  const settings = JSON.parse(readFileSync(join(REPO, ".claude", "settings.json"), "utf8"));

  it("keeps the existing keys and declares the hooks", () => {
    for (const key of ["extraKnownMarketplaces", "enabledPlugins", "permissions", "hooks"]) {
      assert.ok(key in settings, key);
    }
    assert.deepEqual(Object.keys(settings.hooks).sort(), ["PostToolUse", "PreToolUse", "SessionStart", "Stop", "UserPromptSubmit"]);
  });

  it("runs every hook command through bash as Claude Code does, and each script exists", (t) => {
    const commands = Object.values(settings.hooks).flatMap((groups) => groups.flatMap((g) => g.hooks.map((h) => h.command)));
    if (spawnSync("bash", ["-c", "true"]).error) {
      t.skip("bash is not installed");
      return;
    }
    // A copy of the scripts in a throwaway project, so the hooks write nothing in this checkout.
    const root = makeRepo();
    cpSync(SCRIPTS, join(root, "scripts"), { recursive: true });
    for (const command of commands) {
      assert.match(command, /^node "\$CLAUDE_PROJECT_DIR\/scripts\/[\w/-]+\.mjs"$/);
      const result = spawnSync("bash", ["-c", command], {
        input: JSON.stringify({ session_id: "bash", cwd: root, tool_name: "Bash", tool_input: { command: "ls" }, tool_use_id: "b" }),
        env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: root },
        encoding: "utf8",
      });
      assert.equal(result.status, 0, `${command}: ${result.stderr}`);
      assert.doesNotMatch(result.stdout, /could not check|failed|block/, command);
    }
    assert.ok(existsSync(join(root, ".qa", "sessions", "bash")), "SessionStart ran");
    assert.ok(existsSync(join(root, ".qa", "last-prompt.json")), "UserPromptSubmit ran");
  });
});
