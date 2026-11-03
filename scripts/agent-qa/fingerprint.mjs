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

export const samePath = (a, b) => samePathKey(a) === samePathKey(b);

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
  // requiring a verification is safer than requiring none. The base is then
  // the empty tree, so the files of the root commit count too.
  return emptyTree(cwd);
}

/** The id of the empty tree in this repository's hash format (SHA-1 or SHA-256). */
export function emptyTree(cwd) {
  return execFileSync("git", ["hash-object", "-t", "tree", "--stdin"], {
    cwd,
    input: "",
    encoding: "utf8",
    stdio: ["pipe", "pipe", "ignore"],
  }).trim();
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

/**
 * What HEAD points to, read from the git directory without spawning git:
 * a branch name, or a commit for a detached head. `null` when unreadable.
 */
export function currentRef(cwd) {
  try {
    const head = readFileSync(join(gitDir(cwd), "HEAD"), "utf8").trim();
    return head.startsWith("ref: refs/heads/") ? head.slice("ref: refs/heads/".length) : head || null;
  } catch {
    return null;
  }
}

/** Whether `ref` names a branch of the checkout (not a commit). */
const isBranchName = (ref) => !/^[0-9a-f]{40}$/.test(ref);

/** The commit `ref` points to, or `undefined` when it no longer exists. */
export function refTip(cwd, ref) {
  try {
    return git(cwd, "rev-parse", "--verify", "--quiet", `${ref}^{commit}`).trim() || undefined;
  } catch {
    return undefined;
  }
}

/** Whether the commit `tip` is contained in `origin/develop` (the local remote-tracking ref, no fetch): an unpushed local merge is not settled. */
export function mergedIntoDevelop(cwd, tip) {
  if (!/^[0-9a-f]{40}$/.test(String(tip))) {
    return false;
  }
  return ["origin/develop"].some((develop) => {
    try {
      git(cwd, "merge-base", "--is-ancestor", tip, develop);
      return true;
    } catch {
      // Not an ancestor, or a missing ref or commit: not merged by this one.
      return false;
    }
  });
}

/** How many first-parent commits of develop, after the branch's base, the squash check reads. */
const SQUASH_SCAN = 100;
/** Beyond this many changed paths, the patch-id check is skipped (command line length): GitHub settles it. */
const SQUASH_PATHS = 200;

/**
 * Whether the commit `tip` was squash-merged into `origin/develop` (local refs, no fetch, #590): a
 * first-parent commit of develop after the branch's base has the tip's tree (the queue merges an
 * up-to-date branch), or the same patch-id as the branch's whole change (develop moved in between,
 * or the branch was updated on GitHub after the tip: same hunks, other line numbers). A branch
 * without a change, a missing commit or any git failure: not squashed.
 */
export function squashedIntoDevelop(cwd, tip) {
  if (!/^[0-9a-f]{40}$/.test(String(tip))) {
    return false;
  }
  try {
    const base = git(cwd, "merge-base", tip, "origin/develop").trim();
    const tree = git(cwd, "rev-parse", `${tip}^{tree}`).trim();
    if (tree === git(cwd, "rev-parse", `${base}^{tree}`).trim()) {
      return false; // no change of its own: nothing to recognize
    }
    const range = [`--max-count=${SQUASH_SCAN}`, "--first-parent", `${base}..origin/develop`];
    const trees = git(cwd, "log", "--format=%T", ...range).split("\n");
    if (trees.includes(tree)) {
      return true;
    }
    // Only the paths the branch changed: develop's whole patches run to 100 MB over 100 commits
    // (generated files), and a squash restricted to those paths is the branch's change.
    const paths = git(cwd, "diff", "--name-only", "--no-renames", base, tip).split("\n").filter(Boolean);
    if (paths.length === 0 || paths.length > SQUASH_PATHS) {
      return false;
    }
    const spec = ["--", ...paths.map((path) => `:(literal)${path}`)];
    const own = patchIds(cwd, git(cwd, "diff", "--no-color", "--no-ext-diff", "--no-renames", base, tip, ...spec));
    if (own.length !== 1) {
      return false;
    }
    const theirs = patchIds(cwd, git(cwd, "log", "-p", "--no-color", "--no-ext-diff", "--no-renames", "--format=commit %H", ...range, ...spec));
    return theirs.includes(own[0]);
  } catch {
    return false;
  }
}

/** The stable patch-ids of a patch text (one per commit, or one for a plain diff). */
function patchIds(cwd, patch) {
  if (patch.trim() === "") {
    return [];
  }
  const text = execFileSync("git", ["patch-id", "--stable"], {
    cwd,
    input: patch.startsWith("commit ") ? patch : `commit ${"0".repeat(40)}\n${patch}`,
    encoding: "utf8",
    stdio: ["pipe", "pipe", "ignore"],
    maxBuffer: 64 * 1024 * 1024,
  });
  return text
    .split("\n")
    .map((line) => line.split(" ")[0])
    .filter(Boolean);
}

/**
 * Whether a pull request from the branch `ref` into develop was merged on GitHub after `since`
 * (the marker's first write: an older pull request that reused the name does not count) with the
 * commit `tip` in it: its head is `tip`, or `tip` is a local ancestor of its head (the branch was
 * updated on GitHub). Commits made after that merge are not in it: not merged. One `gh` call, 15 s
 * at most; any failure (gh absent, offline, unauthenticated, bad output, head not fetched): not
 * merged. `FAULK_QA_GH`, a Node script run in place of `gh`, is for the tests only.
 */
export function prMergedOnGitHub(cwd, ref, since, tip) {
  if (typeof ref !== "string" || /^[0-9a-f]{40}$/.test(ref) || typeof since !== "string" || !/^[0-9a-f]{40}$/.test(String(tip))) {
    return false;
  }
  const args = ["pr", "list", "--head", ref, "--base", "develop", "--state", "merged", "--json", "number,mergedAt,headRefOid", "--limit", "20"];
  const stub = process.env.FAULK_QA_GH;
  try {
    const out = execFileSync(stub ? process.execPath : "gh", stub ? [stub, ...args] : args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
      timeout: 15000,
      windowsHide: true,
    });
    const after = Date.parse(since);
    const holdsTip = (head) => {
      if (head === tip) {
        return true;
      }
      try {
        git(cwd, "merge-base", "--is-ancestor", tip, head);
        return true;
      } catch {
        return false;
      }
    };
    return (
      Number.isFinite(after) &&
      JSON.parse(out).some(
        (pr) =>
          typeof pr?.mergedAt === "string" &&
          Date.parse(pr.mergedAt) > after &&
          /^[0-9a-f]{40}$/.test(String(pr.headRefOid)) &&
          holdsTip(pr.headRefOid),
      )
    );
  } catch {
    return false;
  }
}

/**
 * Fingerprint of the committed content of a branch (or commit) against its
 * merge base with `develop`, in the same form as `changeFingerprint` (so a
 * branch whose tip holds exactly the verified working tree gets the same
 * value). `undefined` when the ref no longer exists.
 */
export function refFingerprint(cwd, ref) {
  let tip;
  try {
    tip = git(cwd, "rev-parse", "--verify", "--quiet", `${ref}^{commit}`).trim();
  } catch {
    return undefined;
  }
  let base;
  for (const develop of ["origin/develop", "develop"]) {
    try {
      base = git(cwd, "merge-base", tip, develop).trim();
      break;
    } catch {
      // Missing ref: try the next one.
    }
  }
  base ??= emptyTree(cwd);
  const files = lines(git(cwd, "diff", "--name-only", "--no-renames", base, tip, "--", ...CODE_PATHS)).sort();
  if (files.length === 0) {
    return null;
  }
  const hash = createHash("sha256");
  for (const file of files) {
    let content;
    try {
      content = execFileSync("git", ["show", `${tip}:${file}`], { cwd, stdio: ["ignore", "pipe", "ignore"], maxBuffer: 64 * 1024 * 1024 });
    } catch {
      content = "\0deleted";
    }
    hash.update(`${file}\0`).update(content);
  }
  return hash.digest("hex").slice(0, 16);
}

export { isBranchName };

/** Stash entries created at or after `since` (ISO time), newest first. */
export function stashesSince(cwd, since) {
  let output;
  try {
    output = git(cwd, "log", "-g", "--format=%gd %cI", "refs/stash");
  } catch {
    return [];
  }
  const limit = Date.parse(since);
  return lines(output)
    .map((line) => line.split(" "))
    .filter(([, date]) => Date.parse(date) >= limit - 1000)
    .map(([entry]) => entry);
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
/** The git directory of a checkout (cached per process). */
export function gitDir(cwd) {
  if (!gitDirs.has(cwd)) {
    gitDirs.set(cwd, git(cwd, "rev-parse", "--absolute-git-dir").trim());
  }
  return gitDirs.get(cwd);
}

/** The git directory shared by all the worktrees of a repository (absolute, forward slashes). */
export function commonDir(cwd) {
  return git(cwd, "rev-parse", "--path-format=absolute", "--git-common-dir").trim().replaceAll("\\", "/");
}

/**
 * The records of a checkout live in its own git directory (`.git/faulk-qa/`,
 * or `.git/worktrees/<name>/faulk-qa/` for a linked worktree): outside the
 * working tree, so `git clean -x`, a glob or a stray edit of the tree never
 * reaches them, and each worktree keeps its own.
 */
export function qaDir(cwd) {
  return join(gitDir(cwd), QA_DIR_NAME);
}
/**
 * The verifications and the reviews of a checkout, one entry per state (a JSON
 * array, oldest first, bounded by `stop-core.mjs` > RECORD_LIMIT): two branches
 * of one checkout keep their own records (#492).
 */
export const verificationsPath = (cwd) => join(qaDir(cwd), "verifications.json");
export const reviewsPath = (cwd) => join(qaDir(cwd), "reviews.json");
/**
 * The last verification and the last review alone, as before #492: still
 * written, for the guard of a branch that predates the lists, and read as one
 * more entry, for records made before them.
 */
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

/**
 * The entries of a list of records (`verificationsPath`, `reviewsPath`), then
 * the legacy single record (`legacy`) when the list does not hold it already.
 * An absent or unreadable list, or one that is not an array, counts as empty;
 * entries that are not objects are skipped.
 */
export function readRecords(path, legacy) {
  const list = readRecord(path);
  const records = Array.isArray(list) ? list.filter((entry) => entry !== null && typeof entry === "object") : [];
  const single = legacy === undefined ? null : readRecord(legacy);
  if (
    single !== null &&
    typeof single === "object" &&
    !records.some((entry) => entry.at === single.at && entry.fingerprint === single.fingerprint)
  ) {
    records.push(single);
  }
  return records;
}
