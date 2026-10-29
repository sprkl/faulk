/**
 * Claude Code `PostToolUse` hook: after a call that can change files, marks
 * the session as a writer in each checkout whose guarded code the call
 * changed. The Stop hook judges a session only where it wrote. Never blocks.
 *
 * - Edit, Write, MultiEdit, NotebookEdit: the session wrote to the checkout of
 *   the file the tool targets when that file is guarded code. Nothing else is
 *   attributed, so another writer's concurrent edit never is.
 * - Bash, PowerShell, Agent, Task: the fingerprint after the call is compared
 *   with the snapshot `agent-guard.mjs` took before it. A call that moved HEAD
 *   without authoring code (checkout, switch, pull, reset, rebase) is judged
 *   only on the uncommitted changes, and the session's baseline moves with it.
 *
 * @doc docs/agents/verification.md
 */

import { readFileSync } from "node:fs";
import { EDIT_TOOLS, SNAPSHOT_TOOLS, SUBAGENT_TOOLS } from "../agent-review/guard-core.mjs";
import { codeState, currentRef, headMovesSince, hookRoots } from "./fingerprint.mjs";
import {
  editTarget,
  isGuardedFile,
  judgeHeadMove,
  safely,
  agentKey,
  sessionKey,
  sessionState,
  stateRoot,
  wroteDuringCall,
} from "./session.mjs";

function afterEdit(session, input, agent) {
  for (const [key, value] of Object.entries(input.tool_input ?? {})) {
    if (!/path/i.test(key) || typeof value !== "string") {
      continue;
    }
    const target = editTarget(value, [input.cwd, process.env.CLAUDE_PROJECT_DIR]);
    if (target && isGuardedFile(target.file)) {
      session.markWriter(target.root, input.tool_name, currentRef(target.root), agent);
    }
  }
}

function afterSnapshotCall(session, input, agent) {
  const before = new Map(
    typeof input.tool_use_id === "string"
      ? session.takePre(input.tool_use_id).map((snapshot) => [snapshot.root, snapshot])
      : [],
  );
  const roots = [...new Set([...before.keys(), ...hookRoots(input)])];
  for (const root of roots) {
    const ref = currentRef(root);
    if (session.isWriter(root, ref, agent)) {
      continue;
    }
    const snapshot = before.get(root);
    const state = safely(codeState, root);
    const after = state === "unknown" ? "unknown" : state.fingerprint;
    if (!snapshot) {
      // The call moved into this checkout (or had no id): its state before
      // the call is unknown, so the call proves nothing about who changed it.
      // Its state now becomes the baseline if it has none; only an edit tool
      // targeting one of its files, or a later measured call, makes the
      // session a writer there.
      if (!agent && session.baseline(root) === null && after !== "unknown") {
        session.recordBaseline(root, after, "post-tool-use");
      }
      continue;
    }
    if (snapshot.head && state !== "unknown" && state.head !== snapshot.head) {
      const moves = headMovesSince(root, snapshot.head);
      const verdict = judgeHeadMove({
        moves,
        before: snapshot.fingerprint,
        after,
        localBefore: snapshot.local,
        localAfter: state.local,
      });
      if (verdict.writer) {
        session.markWriter(root, input.tool_name, ref, agent);
      } else if (verdict.rebase && !agent) {
        session.rebaseBaseline(root, after, moves?.[0] ?? "HEAD moved");
      }
      continue;
    }
    if (wroteDuringCall({ before: snapshot.fingerprint, after })) {
      session.markWriter(root, input.tool_name, ref, agent);
    }
  }
}

try {
  const input = JSON.parse(readFileSync(0, "utf8"));
  const key = sessionKey(input);
  const state = key ? stateRoot(input) : null;
  if (state) {
    const session = sessionState(state, key);
    const agent = agentKey(input);
    if (agent) {
      session.touchAgent(agent, input.tool_use_id, true);
    } else if (SUBAGENT_TOOLS.has(input.tool_name) && typeof input.tool_use_id === "string") {
      session.endLaunched(input.tool_use_id);
    }
    if (EDIT_TOOLS.has(input.tool_name)) {
      afterEdit(session, input, agent);
    } else if (SNAPSHOT_TOOLS.has(input.tool_name)) {
      afterSnapshotCall(session, input, agent);
    }
  }
} catch {
  // Never block: the Stop hook judges what is recorded.
}
