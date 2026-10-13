/**
 * Fingerprint of the code changes of the checkout: uncommitted changes and
 * commits not on `develop`, under the paths the guardrails protect. Any edit of
 * that code changes the fingerprint, which makes the verification stale.
 *
 * @doc docs/agents/verification.md
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * The code under guard. Tests are included on purpose: the verification record
 * holds the test runs, so a test edited after it makes it stale. The guardrails
 * themselves (scripts, hooks, Judge Dredd's definition) are guarded too, so
 * that weakening them needs a verification and a review. Paths that do not
 * exist yet are harmless in a git pathspec.
 */
export const CODE_PATHS = [
  "crates",
  "web",
  "modules",
  "site",
  "Cargo.toml",
  "Cargo.lock",
  "scripts",
  ".claude/settings.json",
  ".claude/agents",
];

function git(cwd, ...args) {
  return execFileSync("git", args, {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
    maxBuffer: 64 * 1024 * 1024,
  });
}

const roots = new Map();
/** Top level of the git checkout that contains `dir`, or `null` outside a checkout (cached per process). */
export function checkoutRoot(dir) {
  const key = samePathKey(dir);
  if (!roots.has(key)) {
    let root = null;
    try {
      root = git(dir, "rev-parse", "--show-toplevel").trim() || null;
    } catch {
      // Not a checkout.
    }
    roots.set(key, root);
  }
  return roots.get(key);
}

const samePathKey = (path) => {
  const forward = String(path).replaceAll("\\", "/").replace(/\/+$/, "");
  return process.platform === "win32" ? forward.toLowerCase() : forward;
};

/**
 * Root of the first checkout among `dirs` (empty entries skipped), else the
 * first directory given. Hooks pass the session's project directory first;
 * scripts run by the agent pass their own working directory first, so that an
 * agent working in a worktree records in that worktree.
 */
export function projectRoot(...dirs) {
  const candidates = dirs.filter((dir) => typeof dir === "string" && dir !== "");
  for (const dir of candidates) {
    if (existsSync(dir)) {
      const root = checkoutRoot(dir);
      if (root) {
        return root;
      }
    }
  }
  return candidates[0] ?? process.cwd();
}

/**
 * Checkouts a hook judges: the one the session works in (the input's `cwd`,
 * a worktree after the session entered one) and the session's project
 * directory, without duplicates. A session that moves to another checkout is
 * still judged on the one it started in.
 */
export function hookRoots(input) {
  const roots = [];
  for (const dir of [input?.cwd, process.env.CLAUDE_PROJECT_DIR]) {
    const root = typeof dir === "string" && dir !== "" && existsSync(dir) ? checkoutRoot(dir) : null;
    if (root && !roots.some((r) => samePath(r, root))) {
      roots.push(root);
    }
  }
  return roots;
}

const samePath = (a, b) => samePathKey(a) === samePathKey(b);

/** Root for a script run by the agent: its working directory first. */
export const scriptRoot = () => projectRoot(process.cwd(), process.env.CLAUDE_PROJECT_DIR);

/** Merge base of HEAD with `develop`; the root commit when there is no `develop`. */
export function baseRef(cwd) {
  for (const ref of ["origin/develop", "develop"]) {
    try {
      return git(cwd, "merge-base", "HEAD", ref).trim();
    } catch {
      // Missing ref: try the next one.
    }
  }
  // Without `develop` (fresh clone, CI), all the code counts as changed:
  // requiring a verification is safer than requiring none.
  return git(cwd, "rev-list", "--max-parents=0", "HEAD").split("\n")[0];
}

const lines = (output) => output.split("\n").filter(Boolean);

/** Changed code files (committed since the base, uncommitted, untracked), sorted. */
export function changedFiles(cwd) {
  const pathspec = ["--", ...CODE_PATHS];
  const changed = new Set([
    ...lines(git(cwd, "diff", "--name-only", "--no-renames", baseRef(cwd), ...pathspec)),
    ...lines(git(cwd, "ls-files", "--others", "--exclude-standard", ...pathspec)),
  ]);
  return [...changed].sort();
}

/**
 * `null` when the code has no change to verify. The fingerprint depends only
 * on the current content of the changed files: committing or staging a file
 * does not change it.
 */
export function fingerprintOf(cwd, files) {
  if (files.length === 0) {
    return null;
  }
  const hash = createHash("sha256");
  for (const file of files) {
    const path = join(cwd, file);
    hash.update(`${file}\0`).update(existsSync(path) ? readFileSync(path) : "\0deleted");
  }
  return hash.digest("hex").slice(0, 16);
}

export function changeFingerprint(cwd) {
  return fingerprintOf(cwd, changedFiles(cwd));
}

/**
 * The state of the guarded code in three git calls (a hook runs it around
 * every shell call, so each call counts): `fingerprint` of the changes since
 * the base, `local` fingerprint of the uncommitted changes only (files that
 * differ from HEAD, untracked included), and `head`.
 */
export function codeState(cwd) {
  const status = git(
    cwd,
    "status",
    "--porcelain=v2",
    "--branch",
    "--untracked-files=all",
    "--no-renames",
    "-z",
    "--",
    ...CODE_PATHS,
  ).split("\0");
  let head = null;
  const local = new Set();
  const untracked = [];
  for (const entry of status) {
    if (entry.startsWith("# branch.oid ")) {
      const oid = entry.slice("# branch.oid ".length);
      head = oid === "(initial)" ? null : oid;
    } else if (entry.startsWith("? ")) {
      untracked.push(entry.slice(2));
      local.add(entry.slice(2));
    } else if (/^[12u] /.test(entry)) {
      // Ordinary (1), renamed (2, not produced with --no-renames) and unmerged (u) entries end with the path.
      const fields = entry.split(" ");
      local.add(fields.slice(entry.startsWith("1 ") ? 8 : entry.startsWith("u ") ? 10 : 9).join(" "));
    }
  }
  const committed = lines(git(cwd, "diff", "--name-only", "--no-renames", baseRef(cwd), "--", ...CODE_PATHS));
  return {
    head,
    fingerprint: fingerprintOf(cwd, [...new Set([...committed, ...untracked])].sort()),
    local: fingerprintOf(cwd, [...local].sort()),
  };
}

/** Fingerprint of the uncommitted changes only (guarded files that differ from HEAD, untracked included). */
export function localFingerprint(cwd) {
  return codeState(cwd).local;
}

/** The commit HEAD points to, or `null` (no commit yet). */
export function headOf(cwd) {
  try {
    return git(cwd, "rev-parse", "HEAD").trim() || null;
  } catch {
    return null;
  }
}

/**
 * The reflog subjects of the moves of HEAD since it pointed to `from`, newest
 * first (`commit: …`, `checkout: moving from …`, `pull: Fast-forward`…), or
 * `null` when `from` is not in the last 100 entries.
 */
export function headMovesSince(cwd, from) {
  let output;
  try {
    output = git(cwd, "log", "-g", "--format=%H %gs", "-n", "100", "HEAD");
  } catch {
    return null;
  }
  const moves = [];
  for (const line of lines(output)) {
    const space = line.indexOf(" ");
    if (line.slice(0, space) === from) {
      return moves;
    }
    moves.push(line.slice(space + 1));
  }
  return null;
}

/** Files of the guardrails themselves that differ from the merge base with `develop`. */
export const GUARD_PATHS = ["scripts/agent-qa", "scripts/agent-review", ".claude/settings.json", ".claude/agents"];

export function changedGuardFiles(cwd) {
  const pathspec = ["--", ...GUARD_PATHS];
  return [
    ...new Set([
      ...lines(git(cwd, "diff", "--name-only", "--no-renames", baseRef(cwd), ...pathspec)),
      ...lines(git(cwd, "ls-files", "--others", "--exclude-standard", ...pathspec)),
    ]),
  ].sort();
}

/** Current branch, or `null` without a readable branch (detached head included). */
export function currentBranch(cwd) {
  try {
    const branch = git(cwd, "rev-parse", "--abbrev-ref", "HEAD").trim();
    return branch && branch !== "HEAD" ? branch : null;
  } catch {
    return null;
  }
}

/** Name of the records folder, protected by the guard wherever it appears in a path. */
export const QA_DIR_NAME = "faulk-qa";

const gitDirs = new Map();
/**
 * The records of a checkout live in its own git directory (`.git/faulk-qa/`,
 * or `.git/worktrees/<name>/faulk-qa/` for a linked worktree): outside the
 * working tree, so `git clean -x`, a glob or a stray edit of the tree never
 * reaches them, and each worktree keeps its own.
 */
export function qaDir(cwd) {
  if (!gitDirs.has(cwd)) {
    gitDirs.set(cwd, git(cwd, "rev-parse", "--absolute-git-dir").trim());
  }
  return join(gitDirs.get(cwd), QA_DIR_NAME);
}
export const verificationPath = (cwd) => join(qaDir(cwd), "last-run.json");
export const reviewPath = (cwd) => join(qaDir(cwd), "review.json");
/** A deferral is kept apart, so it never erases a verification. */
export const deferralPath = (cwd) => join(qaDir(cwd), "deferral.json");
export const historyPath = (cwd) => join(qaDir(cwd), "history.jsonl");
/** Time of the last user prompt in this checkout (`prompt-submit.mjs`): one deferral per prompt. */
export const lastPromptPath = (cwd) => join(qaDir(cwd), "last-prompt.json");

/** A record, or `null` when absent or unreadable (an unreadable record counts as absent). */
export function readRecord(path) {
  try {
    return existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) : null;
  } catch {
    return null;
  }
}
