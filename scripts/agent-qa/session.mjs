/**
 * What the hooks know of a session, per checkout: its baseline (the
 * fingerprint of the code the first time the hooks saw that checkout) and
 * whether one of its own tool calls changed the guarded code there (a writer
 * marker). The Stop hook judges a session only in the checkouts it wrote to, so
 * a session that changes nothing is never blocked, whatever other sessions do.
 *
 * State lives in the `sessions/<session id>/` folder of the records of the
 * session's project checkout (`qaDir`), one small file per fact so that
 * parallel tool calls never race on one file.
 *
 * @doc docs/agents/verification.md
 */

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { CODE_PATHS, checkoutRoot, qaDir } from "./fingerprint.mjs";

export const validSessionId = (id) => typeof id === "string" && /^[\w-]{1,128}$/.test(id);

/** The key of the session in a hook input, or `undefined`. */
export const sessionKey = (input) => (validSessionId(input?.session_id) ? input.session_id : undefined);

/** Checkout holding the session's state: the project directory's, else the input's `cwd`'s. */
export function stateRoot(input) {
  for (const dir of [process.env.CLAUDE_PROJECT_DIR, input?.cwd]) {
    if (typeof dir === "string" && dir !== "" && existsSync(dir)) {
      const root = checkoutRoot(dir);
      if (root) {
        return root;
      }
    }
  }
  return null;
}

const normalize = (root) => {
  const forward = root.replaceAll("\\", "/");
  return process.platform === "win32" ? forward.toLowerCase() : forward;
};
const rootId = (root) => createHash("sha256").update(normalize(root)).digest("hex").slice(0, 16);
const safeId = (id) => String(id).replace(/[^\w-]/g, "_").slice(0, 128);

export const sessionDir = (state, key) => join(qaDir(state), "sessions", key);

function read(path) {
  try {
    return existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) : null;
  } catch {
    // Unreadable: present but unusable.
    return {};
  }
}

function write(path, value, exclusive = false) {
  mkdirSync(dirname(path), { recursive: true });
  try {
    writeFileSync(path, `${JSON.stringify({ ...value, at: new Date().toISOString() })}\n`, exclusive ? { flag: "wx" } : {});
    return true;
  } catch (error) {
    if (error?.code === "EEXIST") {
      return false;
    }
    throw error;
  }
}

/** A session's view of the checkouts. */
export function sessionState(state, key) {
  const dir = sessionDir(state, key);
  const baselinePath = (root) => join(dir, `baseline-${rootId(root)}.json`);
  // One marker per checkout and per branch (or commit) the session wrote on.
  const writerPath = (root, ref) => join(dir, `wrote-${rootId(root)}-${rootId(String(ref ?? "(none)"))}.json`);
  const writerPattern = (root) => new RegExp(`^wrote-${root ? rootId(root) : "[0-9a-f]{16}"}-[0-9a-f]{16}\\.json$`);
  const movePath = (root) => join(dir, `moved-${rootId(root)}.json`);
  const prePath = (toolUseId, root) => join(dir, `pre-${safeId(toolUseId)}-${rootId(root)}.json`);
  const listed = (pattern) => (existsSync(dir) ? readdirSync(dir).filter((name) => pattern.test(name)) : []);
  return {
    /** `{ fingerprint }`, `{}` when unreadable, `null` when the checkout was never seen. */
    baseline: (root) => read(baselinePath(root)),
    /** Written once (the first writer wins among parallel calls)… */
    recordBaseline: (root, fingerprint, source) => write(baselinePath(root), { root, fingerprint, source }, true),
    /** …except when a call moved HEAD without authoring code: the new state becomes the baseline. */
    rebaseBaseline: (root, fingerprint, move) => {
      write(baselinePath(root), { root, fingerprint, source: "head-moved" });
      write(movePath(root), { root, move });
    },
    lastMove: (root) => read(movePath(root)),
    /** Wrote in this checkout on `ref`, or on any ref when `ref` is undefined. */
    isWriter: (root, ref) =>
      ref === undefined ? listed(writerPattern(root)).length > 0 : existsSync(writerPath(root, ref)),
    /** `ref`: the branch (or commit, on a detached head) the session wrote on. Keeps the first time. */
    markWriter: (root, tool, ref) => write(writerPath(root, ref), { root, ref: ref ?? null, tool }, true),
    /** Every `{ root, ref, at }` the session wrote on. */
    writers: () =>
      listed(writerPattern())
        .map((name) => read(join(dir, name)))
        .filter((marker) => typeof marker?.root === "string"),
    writerRoots: () => [
      ...new Set(
        listed(writerPattern())
          .map((name) => read(join(dir, name))?.root)
          .filter((root) => typeof root === "string"),
      ),
    ],
    savePre: (toolUseId, root, snapshot) => write(prePath(toolUseId, root), { root, ...snapshot }),
    /** The snapshots taken before this tool call, removed once read. */
    takePre: (toolUseId) => {
      const snapshots = [];
      for (const name of listed(new RegExp(`^pre-${safeId(toolUseId)}-[0-9a-f]{16}\\.json$`))) {
        const snapshot = read(join(dir, name));
        rmSync(join(dir, name), { force: true });
        if (typeof snapshot?.root === "string") {
          snapshots.push(snapshot);
        }
      }
      return snapshots;
    },
    /**
     * Drops the snapshots of calls that got no PostToolUse (denied by another
     * hook or the user, interrupted, failed): they say nothing about who
     * changed the code since.
     */
    dropPending: () => {
      for (const name of listed(/^pre-/)) {
        rmSync(join(dir, name), { force: true });
      }
    },
  };
}

/** `fn(root)`, or "unknown" when it throws. */
export function safely(fn, root) {
  try {
    return fn(root);
  } catch {
    return "unknown";
  }
}
export const safeFingerprint = safely;

/** Records the session's baseline in each checkout where it has none. Never throws. */
export function ensureBaselines(session, roots, fingerprintFor, source) {
  for (const root of roots) {
    try {
      if (session.baseline(root) === null) {
        session.recordBaseline(root, safely(fingerprintFor, root), source);
      }
    } catch {
      // Unwritable records: never fail a hook for it.
    }
  }
}

/**
 * Whether a call wrote to the guarded code of a checkout: its fingerprint
 * after the call differs from the one before it. Without a snapshot before
 * (the call moved into a checkout the session had not seen), the reference is
 * the session's baseline there; without either, any change there counts as
 * the session's, the safe choice.
 */
export function wroteDuringCall({ before, baseline, after }) {
  if (after === "unknown") {
    return false;
  }
  const reference = before ?? baseline;
  if (reference === undefined || reference === "unknown") {
    return after !== null;
  }
  return after !== reference;
}

/**
 * Reflog subjects of moves that author code: a commit (amend, or `commit
 * (merge)` after resolving conflicts by hand), a cherry-pick, a revert, and a
 * merge of any branch but `develop` or `main`. Merging `develop` (or `main`)
 * into a task branch brings code that was reviewed in its own pull request,
 * so it is not authoring; merging another task branch brings code nobody
 * verified here, so it is. A checkout, a switch, a pull, a reset or a rebase
 * moves HEAD to code someone else authored or the session already had.
 */
export const authorsCode = (moves) =>
  moves === null ||
  moves.some(
    (move) =>
      /^(commit|cherry-pick|revert)\b/.test(move) ||
      (/^merge\b/.test(move) && !/^merge (refs\/remotes\/)?(origin\/)?(develop|main)\b/.test(move)),
  );

/**
 * Decision for a call that moved HEAD (`moves`: the reflog since its
 * snapshot). An authoring move is judged on the whole fingerprint; a mere move
 * only on the uncommitted changes, which a checkout or a pull carries over
 * untouched unless the call also edited them.
 */
export function judgeHeadMove({ moves, before, after, localBefore, localAfter }) {
  if (authorsCode(moves)) {
    return { writer: wroteDuringCall({ before, after }) };
  }
  if (localAfter === "unknown" || localBefore === undefined) {
    return { writer: false, rebase: true };
  }
  return localAfter !== localBefore ? { writer: true } : { writer: false, rebase: true };
}

/**
 * The checkout and the path inside it of a file an edit tool targets, or
 * `null` outside a checkout. `dirs`: where a relative path resolves.
 */
export function editTarget(path, dirs) {
  if (typeof path !== "string" || path === "") {
    return null;
  }
  const base = dirs.find((dir) => typeof dir === "string" && dir !== "" && existsSync(dir));
  const absolute = isAbsolute(path) ? resolve(path) : base ? resolve(base, path) : null;
  if (!absolute) {
    return null;
  }
  let dir = dirname(absolute);
  while (!existsSync(dir) && dirname(dir) !== dir) {
    dir = dirname(dir);
  }
  const root = checkoutRoot(dir);
  if (!root) {
    return null;
  }
  // Real paths on both sides: Windows short names (RENAUD~1) and symlinks
  // would otherwise put the file outside its own checkout.
  const real = (path) => {
    try {
      return realpathSync.native(path);
    } catch {
      return resolve(path);
    }
  };
  const rel = relative(real(root), join(real(dir), relative(dir, absolute))).split(sep).join("/");
  return rel.startsWith("../") || rel === ".." ? null : { root, file: rel };
}

/** `true` when a path of a checkout is guarded code. */
export const isGuardedFile = (file) =>
  CODE_PATHS.some((path) => file === path || file.startsWith(`${path}/`));
