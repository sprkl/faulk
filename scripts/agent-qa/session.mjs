/**
 * What the hooks know of a session, per checkout: its baseline (the
 * fingerprint of the code the first time the hooks saw that checkout) and
 * whether one of its own tool calls changed the guarded code there (a writer
 * marker). The Stop hook judges a session only in the checkouts it wrote to, so
 * a session that changes nothing is never blocked, whatever other sessions do.
 *
 * State lives in `.qa/sessions/<session id>/` of the session's project
 * checkout, one small file per fact so that parallel tool calls never race on
 * one file.
 *
 * @doc docs/agents/verification.md
 */

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { checkoutRoot, qaDir } from "./fingerprint.mjs";

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
  mkdirSync(join(path, ".."), { recursive: true });
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

/** A session's view of the checkouts: `{ baseline(root), isWriter(root), writerRoots(), ... }`. */
export function sessionState(state, key) {
  const dir = sessionDir(state, key);
  const baselinePath = (root) => join(dir, `baseline-${rootId(root)}.json`);
  const writerPath = (root) => join(dir, `wrote-${rootId(root)}.json`);
  const prePath = (toolUseId, root) => join(dir, `pre-${safeId(toolUseId)}-${rootId(root)}.json`);
  return {
    /** `{ fingerprint }`, `{}` when unreadable, `null` when the checkout was never seen. */
    baseline: (root) => read(baselinePath(root)),
    /** Written once, never overwritten (the first writer wins among parallel calls). */
    recordBaseline: (root, fingerprint, source) => write(baselinePath(root), { root, fingerprint, source }, true),
    isWriter: (root) => existsSync(writerPath(root)),
    markWriter: (root, tool) => write(writerPath(root), { root, tool }),
    writerRoots: () => {
      if (!existsSync(dir)) {
        return [];
      }
      return readdirSync(dir)
        .filter((name) => /^wrote-[0-9a-f]{16}\.json$/.test(name))
        .map((name) => read(join(dir, name))?.root)
        .filter((root) => typeof root === "string");
    },
    savePre: (toolUseId, root, fingerprint) => write(prePath(toolUseId, root), { root, fingerprint }),
    /**
     * The snapshots taken before this tool call (every pending one when
     * `toolUseId` is undefined), removed once read.
     */
    takePre: (toolUseId) => {
      if (!existsSync(dir)) {
        return [];
      }
      const prefix = toolUseId === undefined ? "pre-" : `pre-${safeId(toolUseId)}-`;
      const snapshots = [];
      for (const name of readdirSync(dir).filter((n) => n.startsWith(prefix))) {
        const snapshot = read(join(dir, name));
        rmSync(join(dir, name), { force: true });
        if (typeof snapshot?.root === "string") {
          snapshots.push(snapshot);
        }
      }
      return snapshots;
    },
  };
}

/** Fingerprint, or "unknown" when it cannot be computed. */
export function safeFingerprint(fingerprintFor, root) {
  try {
    return fingerprintFor(root);
  } catch {
    return "unknown";
  }
}

/** Records the session's baseline in each checkout where it has none. Never throws. */
export function ensureBaselines(session, roots, fingerprintFor, source) {
  for (const root of roots) {
    try {
      if (session.baseline(root) === null) {
        session.recordBaseline(root, safeFingerprint(fingerprintFor, root), source);
      }
    } catch {
      // Unwritable .qa/: never fail a hook for it.
    }
  }
}

/**
 * Whether a tool call wrote to the guarded code of a checkout: its
 * fingerprint after the call differs from the one before it. Without a
 * snapshot before (the call moved into a checkout the session had not seen),
 * the reference is the session's baseline there; without either, any change
 * there counts as the session's, the safe choice.
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
