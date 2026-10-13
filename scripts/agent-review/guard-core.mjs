/**
 * Decision of Judge Dredd's guard, without input or output: Judge Dredd
 * receives only the path of a brief, never a text the lead could turn to its
 * advantage, and neither briefs (`brief.mjs`), Judge Dredd's answers
 * (`run.mjs`) nor the records of `record.mjs` are written by hand.
 *
 * Paths are matched on their last components, so a brief is protected in any
 * checkout or worktree, with `/` or `\` separators.
 *
 * @doc docs/agents/judge-dredd.md
 */

import { lstatSync } from "node:fs";
import { isAbsolute, resolve } from "node:path";

export const AGENT_NAMES = new Set(["judge-dredd"]);
export const SUBAGENT_TOOLS = new Set(["Agent", "Task"]);
export const BRIEF_DIR = ".scratch/judge-dredd";
const BRIEF_NAME = /^\d+-(plan|delivery|replies)\.md$/;
// Case-insensitive: Windows and macOS file systems are, so `.Scratch/Judge-Dredd/6-Plan.md` is the same file.
const PROTECTED_BRIEF_NAME = /^\d+-(plan|delivery|replies)(\.answer)?\.md$/i;
const QA_RECORD = /^(last-run\.json|review\.json|deferral\.json|history\.jsonl|last-prompt\.json)$/i;

const SEP = String.raw`[\\/]+`;
// A brief, an answer or a record named in a shell command, with either separator.
const PROTECTED_IN_COMMAND = new RegExp(
  String.raw`\.scratch${SEP}judge-dredd${SEP}\d+-(plan|delivery|replies)(\.answer)?\.md(?![\w.])` +
    String.raw`|\.qa${SEP}(last-run\.json|review\.json|deferral\.json|history\.jsonl|last-prompt\.json|sessions)(?![\w.-])` +
    String.raw`|(^|[\s'"=])\.qa[\\/]*(?=[\s'"]|$)`,
  "i",
);
// Judge Dredd run outside `run.mjs`, where the prompt is not checked.
const DIRECT_JUDGE_RUN = /(^|[\s;&|(\\/"'])claude(\.exe|\.cmd)?["']?[ \t][^\n]*\bjudge-dredd\b/i;
const POSIX_WRITE = String.raw`>|\btee\b|\bsed\s+(-\w+\s+)*-i|\bcp\b|\bmv\b|\bln\b|\brm\b|\brmdir\b|\btruncate\b|\bdd\b|\bperl\s+-\w*i|\bawk\b.*\s-i\b|\binstall\b|\btouch\b|\bgit\s+(checkout|restore|clean|stash)\b`;
const POWERSHELL_WRITE = String.raw`\b(set-content|add-content|out-file|copy-item|move-item|new-item|remove-item|rename-item|clear-content|set-item|sc|ac|ni|ri|del|erase|cpi|mi|rni|copy|move|ren)\b|\[(system\.)?io\.file\]::\s*(write|append|copy|move|delete|create|replace)`;
const SHELL_WRITE = new RegExp(`${POSIX_WRITE}|${POWERSHELL_WRITE}`, "i");
// One call of an official script, on one line, with no chaining, redirection
// or substitution: anything more is no longer that script alone.
const OFFICIAL_SCRIPT =
  /^[ \t]*node[ \t]+(["']?\$\{?CLAUDE_PROJECT_DIR\}?[\\/])?["']?scripts[\\/](agent-review[\\/](brief|run)|agent-qa[\\/]record)\.mjs["']?(?![\w.])[^;&|<>\n\r$`()]*$/;
const MAX_PROMPT = 300;

const WRITE_TOOLS = new Set(["Edit", "Write", "MultiEdit", "NotebookEdit"]);
/** Tools whose calls can change files: the hooks fingerprint the code around them. */
export const CHANGING_TOOLS = new Set([...WRITE_TOOLS, "Bash", "PowerShell"]);

const deny = (reason) => ({ allow: false, reason });
const parts = (path) => path.split(/[\\/]+/).filter((p) => p !== "" && p !== ".");
/** The last three components of a path, right-aligned: `[grandparent, parent, name]`. */
const last3 = (path) => {
  const p = parts(path);
  return [p.at(-3), p.at(-2), p.at(-1)];
};
const lower = (items) => items.map((item) => item?.toLowerCase());

/**
 * The command with the content of its quoted arguments removed, so that
 * `--dispositions "a > b"` is not taken for a redirection. A double-quoted
 * argument holding `$` or a backtick (substitution) is kept as is.
 */
export function withoutQuotedText(command) {
  return command.replace(/'[^'\n]*'/g, "''").replace(/"[^"$`\n]*"/g, '""');
}

/** `true` when `path` names a brief, an answer, or a record of `record.mjs`. */
export function isProtectedPath(path) {
  if (typeof path !== "string") {
    return false;
  }
  const [dir2, dir1, name] = lower(last3(path));
  if (dir2 === ".scratch" && dir1 === "judge-dredd" && PROTECTED_BRIEF_NAME.test(name ?? "")) {
    return true;
  }
  if (dir1 === ".qa" && QA_RECORD.test(name ?? "")) {
    return true;
  }
  // The record folder itself, and anything in the sessions' state.
  const p = lower(parts(path));
  return name === ".qa" || p.some((part, i) => part === ".qa" && p[i + 1] === "sessions");
}

/** `true` when `path` has the shape of a brief: `.scratch/judge-dredd/<issue>-<checkpoint>.md`, without `..`. */
export function isBriefPath(path) {
  if (typeof path !== "string" || parts(path).includes("..")) {
    return false;
  }
  const [dir2, dir1, name] = last3(path);
  return dir2 === ".scratch" && dir1 === "judge-dredd" && BRIEF_NAME.test(name ?? "");
}

function judgeAgentCall(prompt, dirs) {
  if (typeof prompt !== "string") {
    return deny("Judge Dredd receives only the path of a brief written by node scripts/agent-review/brief.mjs.");
  }
  const path = prompt.trim();
  if (path === "" || path.length > MAX_PROMPT || /[\n\r'"`]/.test(path)) {
    return deny(
      "Judge Dredd receives only the path of a brief, nothing else: run node scripts/agent-review/brief.mjs <issue> <plan|delivery|replies> and pass the path it prints as the whole prompt.",
    );
  }
  if (!isBriefPath(path)) {
    return deny(
      `Not a brief path: ${path}. Expected ${BRIEF_DIR}/<issue>-<plan|delivery|replies>.md, built by node scripts/agent-review/brief.mjs <issue> <checkpoint>.`,
    );
  }
  const candidates = isAbsolute(path) ? [path] : dirs.filter(Boolean).map((dir) => resolve(dir, path));
  for (const candidate of candidates) {
    let stat;
    try {
      stat = lstatSync(candidate);
    } catch {
      continue;
    }
    if (stat.isSymbolicLink() || !stat.isFile()) {
      return deny(`Not a regular file: ${path}. Rebuild it with node scripts/agent-review/brief.mjs.`);
    }
    return { allow: true };
  }
  return deny(`Brief not found: ${path}. Run node scripts/agent-review/brief.mjs <issue> <checkpoint> first.`);
}

function pathValues(input) {
  const values = [];
  for (const [key, value] of Object.entries(input ?? {})) {
    if (!/path|file/i.test(key)) {
      continue;
    }
    for (const item of Array.isArray(value) ? value : [value]) {
      if (typeof item === "string") {
        values.push(item);
      }
    }
  }
  return values;
}

/**
 * `{ allow: true }` or `{ allow: false, reason }` (reason addressed to the
 * agent). `dirs`: directories a relative brief path resolves against, in order
 * (the input's `cwd`, then the project directory).
 */
export function judgeToolCall({ toolName, toolInput, dirs }) {
  if (SUBAGENT_TOOLS.has(toolName) && AGENT_NAMES.has(toolInput?.subagent_type)) {
    return judgeAgentCall(toolInput.prompt, dirs ?? []);
  }
  if (WRITE_TOOLS.has(toolName)) {
    for (const path of pathValues(toolInput)) {
      if (isProtectedPath(path)) {
        return deny(
          `${path} cannot be edited: briefs are written only by node scripts/agent-review/brief.mjs, answers only by node scripts/agent-review/run.mjs, and the records in .qa/ only by node scripts/agent-qa/record.mjs. Write your plan or report draft to the .proposal.md file next to the brief.`,
        );
      }
    }
  }
  if ((toolName === "Bash" || toolName === "PowerShell") && typeof toolInput?.command === "string") {
    const command = toolInput.command;
    const official = OFFICIAL_SCRIPT.test(withoutQuotedText(command));
    if (!official && DIRECT_JUDGE_RUN.test(command)) {
      return deny(
        "Judge Dredd runs only through node scripts/agent-review/run.mjs .scratch/judge-dredd/<issue>-<checkpoint>.md, or the judge-dredd subagent with that path as the whole prompt.",
      );
    }
    if (!official && PROTECTED_IN_COMMAND.test(command) && SHELL_WRITE.test(command)) {
      return deny(
        "This command could write a Judge Dredd brief or answer, or a record in .qa/. Briefs come from node scripts/agent-review/brief.mjs, answers from node scripts/agent-review/run.mjs, records from node scripts/agent-qa/record.mjs: run each alone on its line. Read files with cat, and write your plan or report draft to the .proposal.md file.",
      );
    }
  }
  return { allow: true };
}
