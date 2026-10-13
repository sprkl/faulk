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

/** Top level of the git checkout that contains `dir`, or `null` outside a checkout. */
export function checkoutRoot(dir) {
  try {
    return git(dir, "rev-parse", "--show-toplevel").trim() || null;
  } catch {
    return null;
  }
}

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

const samePath = (a, b) =>
  process.platform === "win32" ? a.replaceAll("\\", "/").toLowerCase() === b.replaceAll("\\", "/").toLowerCase() : a === b;

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

/** Current branch, or `null` without a readable branch (detached head included). */
export function currentBranch(cwd) {
  try {
    const branch = git(cwd, "rev-parse", "--abbrev-ref", "HEAD").trim();
    return branch && branch !== "HEAD" ? branch : null;
  } catch {
    return null;
  }
}

export const qaDir = (cwd) => join(cwd, ".qa");
export const verificationPath = (cwd) => join(qaDir(cwd), "last-run.json");
export const reviewPath = (cwd) => join(qaDir(cwd), "review.json");
/** A deferral is kept apart, so it never erases a verification. */
export const deferralPath = (cwd) => join(qaDir(cwd), "deferral.json");
export const historyPath = (cwd) => join(qaDir(cwd), "history.jsonl");
export const sessionPath = (cwd, sessionId) => join(qaDir(cwd), "sessions", `${sessionId}.json`);
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
