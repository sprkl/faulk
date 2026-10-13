/**
 * Claude Code `UserPromptSubmit` hook: records when the user last wrote, so
 * that `record.mjs deferred` allows one new question per user prompt on the
 * same state of the code (the user answered with a question back). Never
 * blocks a prompt.
 *
 * @doc docs/agents/verification.md
 */

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { hookRoots, lastPromptPath } from "./fingerprint.mjs";

try {
  const input = JSON.parse(readFileSync(0, "utf8"));
  for (const root of hookRoots(input)) {
    const path = lastPromptPath(root);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, `${JSON.stringify({ at: new Date().toISOString() })}\n`);
  }
} catch {
  // Never block a prompt.
}
