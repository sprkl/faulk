/**
 * End-to-end tests of the hooks and scripts: each test builds a throwaway git
 * repository, runs the real scripts with the JSON Claude Code would send, and
 * reads their output.
 */

import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { changeFingerprint, codeState } from "./fingerprint.mjs";
import { sessionState } from "./session.mjs";

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
/** The records folder of a checkout (in its git directory). */
const qa = (dir, ...rest) => join(git(dir, "rev-parse", "--absolute-git-dir").trim(), "faulk-qa", ...rest);
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
/**
 * One tool call of a session, between its PreToolUse and PostToolUse hooks
 * (`post: false`: the call never gets its PostToolUse, as when it is denied).
 */
function toolCall(
  root,
  session,
  action,
  {
    cwdBefore = root,
    cwdAfter = cwdBefore,
    project = root,
    tool = "Bash",
    toolInput = { command: "x" },
    post = true,
    pre = true,
    id = `call-${++calls}`,
    agent,
  } = {},
) {
  const run = (script, cwd) =>
    spawnSync(process.execPath, [join(SCRIPTS, script)], {
      input: JSON.stringify({
        session_id: session,
        cwd,
        tool_name: tool,
        tool_input: toolInput,
        tool_use_id: id,
        ...(agent === undefined ? {} : { agent_id: agent, agent_type: "general-purpose" }),
      }),
      env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: project },
      encoding: "utf8",
    });
  if (pre) {
    assert.equal(run("agent-review/agent-guard.mjs", cwdBefore).status, 0);
  }
  action();
  if (post) {
    assert.equal(run("agent-qa/post-tool-use.mjs", cwdAfter).status, 0);
  }
}

/** A clone of `root` whose `develop` is ahead with a commit that changes the guarded code. */
function remoteAhead(root) {
  const remote = mkdtempSync(join(tmpdir(), "faulk-remote-"));
  git(remote, "clone", "-q", "--bare", root, ".");
  const other = mkdtempSync(join(tmpdir(), "faulk-other-"));
  git(other, "clone", "-q", remote, ".");
  git(other, "switch", "-q", "feat/6-x");
  writeFileSync(join(other, ...LIB), "pub fn gain() -> f32 { 0.8 }\n");
  git(other, "commit", "-q", "-am", "someone else's change");
  git(other, "push", "-q", "origin", "feat/6-x");
  git(root, "remote", "add", "origin", remote);
  git(root, "fetch", "-q", "origin");
  git(root, "branch", "-q", "--set-upstream-to=origin/feat/6-x");
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

describe("codeState", () => {
  it("gives the same fingerprint as changeFingerprint, and the uncommitted part apart", () => {
    const root = makeRepo();
    const check = () => assert.equal(codeState(root).fingerprint, changeFingerprint(root));
    check();
    assert.equal(codeState(root).local, null);
    writeCode(root);
    writeFileSync(join(root, "crates", "engine", "src", "new file.rs"), "pub const N: u8 = 1;\n");
    check();
    const dirty = codeState(root);
    git(root, "add", "-A");
    git(root, "commit", "-q", "-m", "commit");
    check();
    const committed = codeState(root);
    assert.equal(committed.fingerprint, dirty.fingerprint);
    assert.equal(committed.local, null);
    assert.notEqual(committed.head, dirty.head);
    rmSync(join(root, "crates", "engine", "src", "new file.rs"));
    check();
    assert.notEqual(codeState(root).local, null);
  });
});

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

  it("an Edit of a guarded file makes the session a writer, wherever the file is", () => {
    const project = makeRepo();
    const worktree = makeRepo("feat/7-y");
    startSession(project, "e");
    const file = join(worktree, ...LIB);
    toolCall(project, "e", () => writeCode(worktree), { tool: "Edit", toolInput: { file_path: file } });
    const output = stop(project, "e");
    assert.equal(output.decision, "block");
    assert.ok(output.reason.includes(toplevel(worktree)));
  });

  it("a subagent's change during an Agent call makes the lead a writer", () => {
    const root = makeRepo();
    startSession(root, "lead");
    toolCall(root, "lead", () => writeCode(root), { tool: "Agent", toolInput: { subagent_type: "general-purpose", prompt: "fix" } });
    assert.equal(stop(root, "lead").decision, "block");
  });

  it("a commit made in the same call as an edit is attributed", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => {
      writeCode(root);
      git(root, "commit", "-q", "-am", "edit and commit");
    });
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

  it("limit: a change made in the very call that enters a checkout is not attributed; the next measured call is", () => {
    const project = makeRepo();
    const worktree = makeRepo("feat/7-y");
    startSession(project, "cd");
    toolCall(worktree, "cd", () => writeCode(worktree), { cwdBefore: project, cwdAfter: worktree, project });
    assert.equal(stop(project, "cd", { cwd: worktree }), null);
    toolCall(worktree, "cd", () => writeCode(worktree, "pub fn gain() -> f32 { 0.15 }\n"), { project });
    assert.equal(stop(project, "cd", { cwd: worktree }).decision, "block");
  });

  it("a writer that commits then leaves its branch for develop is still judged on that branch", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => writeCode(root));
    toolCall(root, "s1", () => git(root, "commit", "-q", "-am", "work"));
    toolCall(root, "s1", () => git(root, "switch", "-q", "develop"));
    const output = stop(root);
    assert.equal(output.decision, "block");
    assert.match(output.reason, /wrote this code on feat\/6-x, and HEAD is now on develop: switch back \(git switch feat\/6-x\)/);
  });

  it("a writer that verified and reviewed its branch can leave it", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => writeCode(root));
    toolCall(root, "s1", () => git(root, "commit", "-q", "-am", "work"));
    assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
    deliveryReview(root);
    assert.equal(review(root).status, 0);
    toolCall(root, "s1", () => git(root, "switch", "-q", "develop"));
    assert.equal(stop(root), null);
  });

  describe("a task branch deleted after the session wrote on it", () => {
    const WARNING = /wrote code on feat\/6-x in .*, which no longer exists/;
    /** The session writes and commits on feat/6-x, verified and reviewed; one stop records the branch tip. */
    function wroteAndLeft() {
      const root = makeRepo();
      startSession(root);
      toolCall(root, "s1", () => writeCode(root));
      toolCall(root, "s1", () => git(root, "commit", "-q", "-am", "work"));
      assert.equal(script("agent-qa/record.mjs", root, "verified", ...RUST).status, 0);
      deliveryReview(root);
      assert.equal(review(root).status, 0);
      toolCall(root, "s1", () => git(root, "switch", "-q", "develop"));
      assert.equal(stop(root), null);
      return root;
    }
    const sessionOf = (project, key) => sessionState(toplevel(project), key);
    const markerFiles = (root) =>
      readdirSync(qa(root, "sessions", "s1")).filter((name) => name.startsWith("wrote-"));

    it("still warns when its last commit is only in the local develop (an unpushed merge)", () => {
      const root = wroteAndLeft();
      assert.equal(markerFiles(root).length, 1);
      git(root, "merge", "-q", "--no-ff", "-m", "merge", "feat/6-x");
      git(root, "branch", "-d", "feat/6-x");
      assert.match(stop(root).systemMessage, WARNING);
      assert.equal(markerFiles(root).length, 1);
    });

    it("warns again when a warned branch comes back and is lost again", () => {
      const root = wroteAndLeft();
      const tip = git(root, "rev-parse", "feat/6-x").trim();
      git(root, "branch", "-D", "feat/6-x");
      assert.match(stop(root).systemMessage, WARNING);
      assert.equal(stop(root), null); // warned once
      git(root, "branch", "feat/6-x", tip);
      assert.equal(stop(root), null); // the ref exists: the flag is cleared
      git(root, "branch", "-D", "feat/6-x");
      assert.match(stop(root).systemMessage, WARNING);
    });

    it("is settled silently when origin/develop contains its last commit", () => {
      const root = wroteAndLeft();
      git(root, "merge", "-q", "--no-ff", "-m", "merge", "feat/6-x");
      git(root, "update-ref", "refs/remotes/origin/develop", "HEAD");
      git(root, "reset", "-q", "--hard", "HEAD~1");
      git(root, "branch", "-D", "feat/6-x");
      assert.equal(stop(root), null);
      assert.equal(markerFiles(root).length, 0);
    });

    it("still warns, once, when its last commit is not in develop", () => {
      const root = wroteAndLeft();
      // An agent's marker on the same branch says the same: one message.
      sessionOf(root, "s1").markWriter(toplevel(root), "Edit", "feat/6-x", "a1");
      sessionOf(root, "s1").noteTip(toplevel(root), "feat/6-x", "a1", git(root, "rev-parse", "feat/6-x").trim());
      sessionOf(root, "s1").endAgent("a1");
      git(root, "branch", "-D", "feat/6-x");
      const output = stop(root);
      assert.match(output.systemMessage, WARNING);
      assert.equal(output.systemMessage.match(/which no longer exists/g).length, 1);
      assert.equal(markerFiles(root).length, 2);
      assert.equal(stop(root), null); // not repeated at the next turn
    });

    it("warns once for a marker written without a commit", () => {
      const root = wroteAndLeft();
      const [name] = markerFiles(root);
      const path = qa(root, "sessions", "s1", name);
      const marker = JSON.parse(readFileSync(path, "utf8"));
      assert.match(marker.tip, /^[0-9a-f]{40}$/);
      delete marker.tip;
      writeFileSync(path, JSON.stringify(marker));
      git(root, "merge", "-q", "--no-ff", "-m", "merge", "feat/6-x");
      git(root, "branch", "-d", "feat/6-x");
      assert.match(stop(root).systemMessage, WARNING);
      assert.equal(markerFiles(root).length, 1);
    });

    it("follows the branch: a commit made after the last stop is the known tip", () => {
      const root = wroteAndLeft();
      toolCall(root, "s1", () => git(root, "switch", "-q", "feat/6-x"));
      toolCall(root, "s1", () => {
        writeCode(root, "pub fn gain() -> f32 { 0.6 }\n");
        git(root, "commit", "-q", "-am", "more");
      });
      toolCall(root, "s1", () => git(root, "switch", "-q", "develop"));
      git(root, "branch", "-D", "feat/6-x"); // the new commit never reached develop
      assert.match(stop(root).systemMessage, WARNING);
    });
  });

  it("merging develop into the task branch is not authoring; merging another task branch is", () => {
    const root = makeRepo();
    git(root, "switch", "-q", "develop");
    writeCode(root, "pub fn gain() -> f32 { 0.65 }\n");
    git(root, "commit", "-q", "-am", "reviewed work on develop");
    git(root, "switch", "-q", "-c", "feat/10-other", "HEAD~1");
    writeFileSync(join(root, "crates", "engine", "src", "other.rs"), "pub const OTHER: u8 = 1;\n");
    git(root, "add", "-A");
    git(root, "commit", "-q", "-m", "unreviewed work");
    git(root, "switch", "-q", "feat/6-x");
    startSession(root, "m");
    toolCall(root, "m", () => git(root, "merge", "-q", "--no-ff", "--no-edit", "develop"));
    assert.equal(stop(root, "m"), null);
    toolCall(root, "m", () => git(root, "merge", "-q", "--no-ff", "--no-edit", "feat/10-other"));
    assert.equal(stop(root, "m").decision, "block");
  });

  it("a session working in a linked worktree while the project directory is the main checkout records and stops there", () => {
    const main = makeRepo("develop");
    const linked = join(mkdtempSync(join(tmpdir(), "faulk-linked-")), "w");
    git(main, "worktree", "add", "-q", "-b", "feat/6-x", linked);
    startSession(main, "lw");
    toolCall(linked, "lw", () => writeCode(linked), { project: main });
    const blocked = stop(main, "lw", { cwd: linked });
    assert.equal(blocked.decision, "block");
    assert.ok(blocked.reason.includes(toplevel(linked)));
    assert.equal(script("agent-qa/record.mjs", linked, "verified", ...RUST).status, 0);
    deliveryReview(linked);
    assert.equal(review(linked).status, 0);
    assert.equal(stop(main, "lw", { cwd: linked }), null);
    assert.equal(stop(main, "lw", { cwd: main }), null);
  });

  it("a writer that stashes its changes is still judged", () => {
    const root = makeRepo();
    startSession(root);
    toolCall(root, "s1", () => writeCode(root));
    toolCall(root, "s1", () => git(root, "stash", "-q", "-u"));
    const output = stop(root);
    assert.equal(output.decision, "block");
    assert.match(output.reason, /stash@\{0\} was stashed since: stashed code is not verified\. Restore it \(git stash pop\)/);
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

  it("while another session or the user changes the code between its calls", () => {
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

  it("while someone else edits the code during one of its edit calls on another file", () => {
    const root = makeRepo();
    startSession(root, "reader");
    toolCall(root, "reader", () => {
      writeFileSync(join(root, "README.md"), "# notes\n"); // the session's own edit, not guarded
      writeCode(root, "pub fn gain() -> f32 { 0.4 }\n"); // someone else, at the same time
    }, { tool: "Edit", toolInput: { file_path: join(root, "README.md") } });
    assert.equal(stop(root, "reader"), null);
  });

  it("limit: someone else's edit during one of its shell calls is counted as the session's", () => {
    const root = makeRepo();
    startSession(root, "reader");
    toolCall(root, "reader", () => writeCode(root, "pub fn gain() -> f32 { 0.4 }\n"));
    assert.equal(stop(root, "reader").decision, "block");
  });

  it("when it enters a worktree that carries another agent's commits and uncommitted changes, read-only", () => {
    const project = makeRepo();
    const worktree = makeRepo("feat/9-other");
    writeCode(worktree);
    git(worktree, "commit", "-q", "-am", "another agent's commit");
    writeFileSync(join(worktree, "crates", "engine", "src", "wip.rs"), "pub const WIP: u8 = 1;\n");
    startSession(project, "visitor");
    toolCall(worktree, "visitor", () => {}, { cwdBefore: project, cwdAfter: worktree, project });
    toolCall(worktree, "visitor", () => git(worktree, "log", "-1"), { project });
    assert.equal(stop(project, "visitor", { cwd: worktree }), null);
  });

  it("after a call that was denied or interrupted, followed by someone else's edit", () => {
    const root = makeRepo();
    startSession(root, "d");
    toolCall(root, "d", () => {}, { post: false });
    writeCode(root); // someone else
    assert.equal(stop(root, "d"), null);
    assert.equal(stop(root, "d"), null);
  });

  it("when it switches branch, checks out a pull request or pulls", () => {
    const root = makeRepo();
    git(root, "switch", "-q", "-c", "feat/8-other");
    writeCode(root, "pub fn gain() -> f32 { 0.7 }\n");
    git(root, "commit", "-q", "-am", "other branch work");
    git(root, "switch", "-q", "feat/6-x");
    startSession(root, "sw");
    toolCall(root, "sw", () => git(root, "switch", "-q", "feat/8-other"));
    assert.equal(stop(root, "sw"), null);
    toolCall(root, "sw", () => git(root, "switch", "-q", "feat/6-x"));
    assert.equal(stop(root, "sw"), null);

    const pulled = makeRepo();
    remoteAhead(pulled);
    startSession(pulled, "pl");
    toolCall(pulled, "pl", () => git(pulled, "pull", "-q", "--ff-only"));
    assert.equal(changeFingerprint(pulled) !== null, true);
    assert.equal(stop(pulled, "pl"), null);
  });

  it("a switch that also edits the uncommitted code is attributed, and the message says HEAD moved", () => {
    const root = makeRepo();
    git(root, "switch", "-q", "-c", "feat/8-other");
    writeFileSync(join(root, "README.md"), "# other\n");
    git(root, "commit", "-q", "-am", "other branch, no guarded change");
    git(root, "switch", "-q", "feat/6-x");
    startSession(root, "swe");
    toolCall(root, "swe", () => git(root, "switch", "-q", "feat/8-other"));
    toolCall(root, "swe", () => writeCode(root, "pub fn gain() -> f32 { 0.2 }\n"));
    const output = stop(root, "swe");
    assert.equal(output.decision, "block");
    assert.match(output.reason, /moved HEAD without authoring code/);
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
    const dir = qa(root, "sessions", "u");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, `baseline-${rootIdOf(root)}.json`), JSON.stringify({ root: toplevel(root), fingerprint: "unknown" }));
    toolCall(root, "u", () => {});
    assert.equal(stop(root, "u"), null);
  });

  it("when git cannot run: the hook fails open with a warning to the user", () => {
    const outside = mkdtempSync(join(tmpdir(), "faulk-nogit-"));
    assert.equal(stop(outside), null);
    const project = makeRepo();
    // A writer marker for a directory where git cannot compute anything (no longer a checkout).
    const broken = mkdtempSync(join(tmpdir(), "faulk-broken-"));
    const id = (text) => createHash("sha256").update(text.replaceAll("\\", "/").toLowerCase()).digest("hex").slice(0, 16);
    const brokenId = process.platform === "win32" ? id(broken) : createHash("sha256").update(broken).digest("hex").slice(0, 16);
    mkdirSync(qa(project, "sessions", "s1"), { recursive: true });
    writeFileSync(qa(project, "sessions", "s1", `wrote-${brokenId}-${id("(none)")}.json`), JSON.stringify({ root: broken, ref: null }));
    assert.match(stop(project).systemMessage, /could not check the code/);
  });

  it("without develop, the files of the root commit count as changed", () => {
    const root = makeRepo("develop");
    git(root, "branch", "-q", "-m", "develop", "trunk");
    assert.notEqual(changeFingerprint(root), null);
    assert.notEqual(codeState(root).fingerprint, null);
    assert.equal(codeState(root).fingerprint, changeFingerprint(root));
  });

  it("when the Stop input has no session id: the hook fails open with a warning to the user", () => {
    const root = makeRepo();
    const result = spawnSync(process.execPath, [join(SCRIPTS, "agent-qa/qa-guard.mjs")], {
      input: JSON.stringify({ cwd: root }),
      env: { ...cleanEnv(), CLAUDE_PROJECT_DIR: root },
      encoding: "utf8",
    });
    assert.equal(result.status, 0);
    const output = JSON.parse(result.stdout);
    assert.equal(output.decision, undefined);
    assert.match(output.systemMessage, /no valid session_id/);
  });

  it("warns the user, without blocking, when the guardrails themselves differ from develop", () => {
    const root = makeRepo();
    mkdirSync(join(root, "scripts", "agent-qa"), { recursive: true });
    writeFileSync(join(root, "scripts", "agent-qa", "record.mjs"), "// weakened\n");
    startSession(root, "g");
    const output = stop(root, "g");
    assert.equal(output.decision, undefined);
    assert.match(output.systemMessage, /guardrails differ from develop.*scripts\/agent-qa\/record\.mjs/);
  });
});

describe("Stop hook: the worktree of a running subagent (#392)", () => {
  // Claude Code gives the hooks of a subagent's tool calls the lead's session id
  // and an `agent_id` (hooks reference, "common input fields").
  const idHash = (text) =>
    createHash("sha256")
      .update(process.platform === "win32" ? text.toLowerCase() : text)
      .digest("hex")
      .slice(0, 16);

  function agentWorktree(project, name = "agent-a1", branch = "feat/6-agent") {
    const dir = join(project, ".claude", "worktrees", name);
    git(project, "worktree", "add", "-q", "-b", branch, dir);
    return dir;
  }

  /** An edit of the guarded code by the subagent, in `dir`; its hooks run with the lead's session. */
  const agentEdit = (project, dir, session, agent, content) =>
    toolCall(dir, session, () => writeCode(dir, content), {
      project,
      agent,
      tool: "Edit",
      toolInput: { file_path: join(dir, ...LIB) },
    });

  const backdate = (project, session, agent, minutes) => {
    const path = qa(project, "sessions", session, `agent-${idHash(agent)}.json`);
    const record = JSON.parse(readFileSync(path, "utf8"));
    record.at = new Date(Date.now() - minutes * 60_000).toISOString();
    writeFileSync(path, JSON.stringify(record));
  };

  const sessionOf = (project, key) => sessionState(toplevel(project), key);

  /** The input of a subagent's own stop, as the hooks reference describes it. */
  const subagentStop = (agent, cwd) => ({
    hook_event_name: "SubagentStop",
    agent_id: agent,
    agent_type: "general-purpose",
    agent_transcript_path: join(cwd, "agent.jsonl"),
    last_assistant_message: "done",
    stop_hook_active: false,
    cwd,
  });

  /** The lead's Agent call: `during` runs between its two hooks, as the subagent's own calls would. */
  const leadAgentCall = (project, during) =>
    toolCall(project, "lead", during, { tool: "Agent", toolInput: { subagent_type: "general-purpose", prompt: "work" } });

  it("ends the exemption when the lead's Agent call returns: a finished foreground agent is judged at once", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    leadAgentCall(project, () => agentEdit(project, worktree, "lead", "a1"));
    assert.equal(sessionOf(project, "lead").agentRunning("a1"), false);
    const output = stop(project, "lead");
    assert.equal(output.decision, "block");
    assert.ok(output.reason.includes(toplevel(worktree)), output.reason);
  });

  it("leaves an agent that started before the lead's Agent call running, and one that resumes", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "a1"); // a background agent, launched earlier
    leadAgentCall(project, () => {}); // for instance a Judge Dredd plan review
    assert.equal(stop(project, "lead"), null);
    leadAgentCall(project, () => agentEdit(project, worktree, "lead", "a2", "pub fn gain() -> f32 { 0.7 }\n"));
    assert.equal(sessionOf(project, "lead").agentRunning("a1"), true);
    agentEdit(project, worktree, "lead", "a2");
    assert.equal(sessionOf(project, "lead").agentRunning("a2"), true, "an event after the end runs it again");
  });

  it("a call that never got its Post stops keeping a finished agent running", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    leadAgentCall(project, () => {
      agentEdit(project, worktree, "lead", "a1");
      toolCall(worktree, "lead", () => {}, { project, agent: "a1", post: false }); // a failed cargo test
    });
    assert.equal(stop(project, "lead").decision, "block");
    const session = sessionOf(project, "lead");
    assert.equal(session.agentRunning("a1", Date.now() + 30 * 60_000), false);
  });

  it("the lead's stop during a subagent's shell call keeps that call's snapshot, so its change is attributed", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    toolCall(worktree, "lead", () => {}, { project, agent: "a1", id: "bg-1", post: false }); // sed -i is about to run
    assert.equal(stop(project, "lead"), null);
    toolCall(worktree, "lead", () => writeCode(worktree), { project, agent: "a1", id: "bg-1", pre: false });
    assert.equal(sessionOf(project, "lead").isWriter(toplevel(worktree), undefined, "a1"), true);
  });

  it("a Stop that carries an agent_id but is not a SubagentStop is the lead's own", () => {
    const project = makeRepo();
    startSession(project, "lead");
    toolCall(project, "lead", () => writeCode(project));
    assert.equal(stop(project, "lead", { agent_id: "a1" }).decision, "block");
  });

  it("lets the lead stop while a subagent edits its own worktree, with the shell and the edit tools", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "a1");
    toolCall(worktree, "lead", () => writeCode(worktree, "pub fn gain() -> f32 { 0.2 }\n"), { project, agent: "a1" });
    assert.equal(stop(project, "lead"), null);
  });

  it("judges that worktree again once the subagent has gone quiet, and names it", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "a1");
    backdate(project, "lead", "a1", 30);
    const output = stop(project, "lead");
    assert.equal(output.decision, "block");
    assert.ok(output.reason.includes(toplevel(worktree)), output.reason);
  });

  it("keeps a subagent running while one of its calls is in progress, however quiet", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "a1");
    toolCall(worktree, "lead", () => {}, { project, agent: "a1", post: false }); // a long cargo run
    backdate(project, "lead", "a1", 90);
    assert.equal(stop(project, "lead"), null);
    const session = sessionOf(project, "lead");
    assert.equal(session.agentRunning("a1", Date.now()), true);
    assert.equal(session.agentRunning("a1", Date.now() + 3 * 60 * 60_000), false); // a call that never ends stops counting
  });

  it("still blocks the lead on an edit it made itself in the worktree of a running subagent", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "a1");
    toolCall(worktree, "lead", () => writeCode(worktree, "pub fn gain() -> f32 { 0.3 }\n"), {
      project,
      tool: "Edit",
      toolInput: { file_path: join(worktree, ...LIB) },
    });
    const output = stop(project, "lead");
    assert.equal(output.decision, "block");
    assert.ok(output.reason.includes(toplevel(worktree)), output.reason);
  });

  it("does not exempt a subagent that edits the lead's own checkout", () => {
    const project = makeRepo();
    startSession(project, "lead");
    agentEdit(project, project, "lead", "a1");
    assert.equal(stop(project, "lead").decision, "block");
  });

  it("does not exempt the worktree the lead stops in, nor one that is not an agent's", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    const other = agentWorktree(project, "wt-other", "feat/6-other");
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "a1");
    agentEdit(project, other, "lead", "a2");
    const output = stop(project, "lead", { cwd: worktree });
    assert.equal(output.decision, "block");
    assert.ok(output.reason.includes(toplevel(worktree)), output.reason);
    assert.ok(output.reason.includes(toplevel(other)), output.reason);
    const away = stop(project, "lead");
    assert.ok(away.reason.includes(toplevel(other)), away.reason);
    assert.ok(!away.reason.includes(toplevel(worktree)), away.reason);
  });

  it("does not exempt a folder named like an agent's that belongs to another repository", () => {
    const project = makeRepo();
    const stranger = makeRepo("feat/7-y");
    const fake = join(stranger, ".claude", "worktrees", "agent-fake");
    git(stranger, "worktree", "add", "-q", "-b", "feat/7-fake", fake);
    startSession(project, "lead");
    agentEdit(project, fake, "lead", "a1");
    const output = stop(project, "lead");
    assert.equal(output.decision, "block");
    assert.ok(output.reason.includes(toplevel(fake)), output.reason);
  });

  it("treats a malformed agent id as the lead's own call", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "bad id!");
    assert.equal(stop(project, "lead").decision, "block");
  });

  it("holds the subagent to its own check: its stop is blocked until verified and reviewed, then it has ended", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "a1");
    const blocked = stop(project, "lead", subagentStop("a1", worktree));
    assert.equal(blocked.decision, "block");
    assert.match(blocked.reason, /record\.mjs verified/);
    assert.equal(sessionOf(project, "lead").agentRunning("a1"), true);
    assert.equal(stop(project, "lead"), null); // the lead is still not held for it
    assert.equal(script("agent-qa/record.mjs", worktree, "verified", ...RUST).status, 0);
    deliveryReview(worktree);
    assert.equal(review(worktree).status, 0);
    assert.equal(stop(project, "lead", subagentStop("a1", worktree)), null);
    assert.equal(sessionOf(project, "lead").agentRunning("a1"), false);
    assert.equal(stop(project, "lead"), null); // verified and reviewed: nothing left to judge either
  });

  it("does not hold a subagent's stop to the lead's checkout: the lead's stop judges that work with its own", () => {
    const project = makeRepo();
    startSession(project, "lead");
    agentEdit(project, project, "lead", "a1");
    assert.equal(stop(project, "lead", subagentStop("a1", project)), null);
    assert.equal(stop(project, "lead").decision, "block");
  });

  it("judges nothing on a SubagentStop without a valid agent_id, and leaves the lead's work to the lead's stop", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    // The lead's call in progress (it has its snapshot: the lead has not written there yet), then its edit.
    toolCall(project, "lead", () => {}, { post: false });
    toolCall(project, "lead", () => writeCode(project, "pub fn gain() -> f32 { 0.25 }\n"), {
      tool: "Edit",
      toolInput: { file_path: join(project, ...LIB) },
    });
    const pending = () => readdirSync(qa(project, "sessions", "lead")).filter((name) => name.startsWith("pre-"));
    assert.equal(pending().length, 1);
    for (const agent of [undefined, "bad id!"]) {
      const result = stop(project, "lead", { ...subagentStop(agent, worktree), agent_id: agent });
      assert.equal(result?.decision, undefined);
      assert.match(result.systemMessage, /without a valid agent_id/);
    }
    // No side effect on the lead's session: its call in progress keeps its snapshot, no baseline for the worktree.
    assert.equal(pending().length, 1);
    assert.equal(sessionOf(project, "lead").baseline(toplevel(worktree)), null);
    assert.equal(stop(project, "lead").decision, "block");
  });

  it("a subagent's stop judges nothing in an agent worktree that is the project's own checkout", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(worktree, "lead");
    agentEdit(worktree, worktree, "lead", "a1");
    // Run with CLAUDE_PROJECT_DIR set to the worktree itself: it is the lead's checkout, not the agent's own.
    assert.equal(stop(worktree, "lead", subagentStop("a1", worktree)), null);
    assert.equal(stop(worktree, "lead").decision, "block");
  });

  it("a subagent's stop judges nothing in a lookalike agent folder of another repository", () => {
    const project = makeRepo();
    const other = makeRepo();
    const lookalike = agentWorktree(other);
    startSession(project, "lead");
    agentEdit(project, lookalike, "lead", "a1");
    assert.equal(stop(project, "lead", subagentStop("a1", lookalike)), null);
    assert.equal(stop(project, "lead").decision, "block");
  });

  it("an ended subagent that wrote again is running again, and an unverified end is judged by the lead", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "a1");
    assert.equal(script("agent-qa/record.mjs", worktree, "verified", ...RUST).status, 0);
    deliveryReview(worktree);
    assert.equal(review(worktree).status, 0);
    assert.equal(stop(project, "lead", subagentStop("a1", worktree)), null);
    agentEdit(project, worktree, "lead", "a1", "pub fn gain() -> f32 { 0.9 }\n");
    assert.equal(sessionOf(project, "lead").agentRunning("a1"), true);
    sessionOf(project, "lead").endAgent("a1"); // as a SubagentStop that let it through would
    assert.equal(stop(project, "lead").decision, "block"); // the new edit is unverified
  });

  it("a subagent's events never write the session's baseline, and its marker never stands for the lead's", () => {
    const project = makeRepo();
    const worktree = agentWorktree(project);
    startSession(project, "lead");
    agentEdit(project, worktree, "lead", "a1");
    toolCall(worktree, "lead", () => {}, { project, agent: "a1" });
    hook("agent-qa/session-start.mjs", project, { session_id: "lead", source: "startup", agent_id: "a1", cwd: worktree });
    assert.equal(existsSync(qa(project, "sessions", "lead", `baseline-${rootIdOf(worktree)}.json`)), false);
    const session = sessionOf(project, "lead");
    const root = toplevel(worktree);
    assert.equal(session.isWriter(root, "feat/6-agent"), false);
    assert.equal(session.isWriter(root, undefined), false);
    assert.equal(session.isWriter(root, "feat/6-agent", "a1"), true);
    assert.equal(session.isWriter(root, "feat/6-agent", "a2"), false);
  });

  it("limits: an agent never seen, with an unreadable record or past the idle time, is not running", () => {
    const project = makeRepo();
    const session = sessionOf(project, "lead");
    assert.equal(session.agentRunning("never-seen"), false);
    session.touchAgent("a1");
    assert.equal(session.agentRunning("a1"), true);
    assert.equal(session.agentRunning("a1", Date.now() + 21 * 60_000), false);
    writeFileSync(qa(project, "sessions", "lead", `agent-${idHash("a1")}.json`), "{not json");
    assert.equal(session.agentRunning("a1"), false);
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
    assert.match(review(root).stderr, /expected model: sonnet or opus/);
  });

  it("accepts an answer by Judge Dredd's model or by Opus", () => {
    const root = makeRepo();
    writeCode(root);
    deliveryReview(root, { model: "sonnet" });
    assert.equal(review(root).status, 0);
    deliveryReview(root, { model: "opus" });
    assert.equal(review(root).status, 0);
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
    assert.equal(existsSync(qa(root, "last-run.json")), false);
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

  it("lists a changed lockfile and says its content is left out of the diff", () => {
    const root = makeRepo();
    writeFileSync(join(root, "Cargo.lock"), "# lock v2\n");
    writeCode(root);
    mkdirSync(join(root, ".scratch", "judge-dredd"), { recursive: true });
    writeFileSync(join(root, ".scratch", "judge-dredd", "6-delivery.proposal.md"), "Claims.\n");
    const issueFile = join(root, ".scratch", "issue.json");
    writeFileSync(issueFile, JSON.stringify({ number: 6, title: "t", body: "b", comments: [] }));
    git(root, "add", "Cargo.lock");
    assert.equal(script("agent-review/brief.mjs", root, "6", "delivery", "--issue-file", issueFile).status, 0);
    const text = readFileSync(join(root, BRIEF), "utf8");
    assert.match(text, /A\tCargo\.lock/);
    assert.match(text, /content of these lockfiles is left out of the diff: Cargo\.lock/);
    assert.doesNotMatch(text, /# lock v2/);
  });

  it("lists only the lockfiles in its note, and summarises heavy generated files instead of copying them (#566)", () => {
    const root = makeRepo();
    const gen = join(root, "web", "src", "lib", "generated");
    mkdirSync(gen, { recursive: true });
    const lines = (n, tag) => Array.from({ length: n }, (_, i) => `row ${i} ${tag}`).join("\n") + "\n";
    writeFileSync(join(gen, "schema.json"), lines(40_000, "old"));
    git(root, "add", "-A");
    git(root, "commit", "-q", "-m", "generated");
    writeFileSync(join(root, "Cargo.lock"), "# lock v2\n");
    writeFileSync(join(gen, "schema.json"), lines(40_000, "new"));
    writeFileSync(join(gen, "extra.json"), lines(40_000, "fresh"));
    writeCode(root);
    mkdirSync(join(root, ".scratch", "judge-dredd"), { recursive: true });
    writeFileSync(join(root, ".scratch", "judge-dredd", "6-delivery.proposal.md"), "Claims.\n");
    const issueFile = join(root, ".scratch", "issue.json");
    writeFileSync(issueFile, JSON.stringify({ number: 6, title: "t", body: "b", comments: [] }));
    git(root, "add", "Cargo.lock");
    assert.equal(script("agent-review/brief.mjs", root, "6", "delivery", "--issue-file", issueFile).status, 0);
    const text = readFileSync(join(root, BRIEF), "utf8");
    assert.match(text, /content of these lockfiles is left out of the diff: Cargo\.lock\./);
    assert.match(text, /web\/src\/lib\/generated\/schema\.json \(added\)/);
    assert.match(text, /web\/src\/lib\/generated\/extra\.json \(added, untracked\)/);
    assert.match(text, /\+pub fn gain\(\) -> f32 \{ 0\.5 \}/, "the ordinary diff is still copied");
    assert.doesNotMatch(text, /row 5 new/);
    assert.doesNotMatch(text, /row 5 fresh/);
    assert.ok(text.length < 100_000, `the brief stays small (${text.length} bytes)`);
  });

  it("does not follow an untracked symbolic link", (t) => {
    const root = makeRepo();
    const secret = join(mkdtempSync(join(tmpdir(), "faulk-secret-")), "secret.txt");
    writeFileSync(secret, "TOP SECRET\n");
    try {
      symlinkSync(secret, join(root, "crates", "engine", "src", "link.rs"));
    } catch {
      t.skip("symbolic links are not allowed for this user on this system");
      return;
    }
    mkdirSync(join(root, ".scratch", "judge-dredd"), { recursive: true });
    writeFileSync(join(root, ".scratch", "judge-dredd", "6-delivery.proposal.md"), "Claims.\n");
    const issueFile = join(root, ".scratch", "issue.json");
    writeFileSync(issueFile, JSON.stringify({ number: 6, title: "t", body: "b", comments: [] }));
    assert.equal(script("agent-review/brief.mjs", root, "6", "delivery", "--issue-file", issueFile).status, 0);
    const text = readFileSync(join(root, BRIEF), "utf8");
    assert.match(text, /link\.rs \(untracked symbolic link, not followed\)/);
    assert.doesNotMatch(text, /TOP SECRET/);
  });

  it("run.mjs refuses to write through a symbolic link", (t) => {
    const root = makeRepo();
    writeCode(root);
    deliveryReview(root);
    const answer = join(root, ".scratch", "judge-dredd", "6-delivery.answer.md");
    rmSync(answer);
    try {
      symlinkSync(join(root, "README.md"), answer);
    } catch {
      t.skip("symbolic links are not allowed for this user on this system");
      return;
    }
    const result = script("agent-review/run.mjs", root, BRIEF);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /is a symbolic link/);
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
    for (const path of [qa(root, "review.json"), qa(root, "sessions", "s1", "wrote-0.json"), qa(root, "x", "..", "review.json")]) {
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
    assert.deepEqual(Object.keys(settings.hooks).sort(), ["PostToolUse", "PreToolUse", "SessionStart", "Stop", "SubagentStop", "UserPromptSubmit"]);
    // A subagent is checked by its own stop with the same guard as the main thread.
    assert.deepEqual(settings.hooks.SubagentStop, settings.hooks.Stop);
  });

  it("runs every hook command through bash as Claude Code does, and each script exists", (t) => {
    const commands = Object.values(settings.hooks).flatMap((groups) => groups.flatMap((g) => g.hooks.map((h) => h.command)));
    const probe = spawnSync("bash", ["-c", "true"]);
    if (probe.error || probe.status !== 0) {
      // On Windows `bash` can be WSL's launcher, which exits non-zero when no distribution is installed (#539).
      t.skip("bash is not installed, or does not run (WSL without a distribution)");
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
    assert.ok(existsSync(qa(root, "sessions", "bash")), "SessionStart ran");
    assert.ok(existsSync(qa(root, "last-prompt.json")), "UserPromptSubmit ran");
  });
});
