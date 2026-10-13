/**
 * Decision of Judge Dredd's guard, without input or output: Judge Dredd
 * receives only the path of a brief, never a text the lead could turn to its
 * advantage, and neither briefs (`brief.mjs`), Judge Dredd's answers
 * (`run.mjs`) nor the records of `record.mjs` are written by hand.
 *
 * Briefs and answers live in `.scratch/judge-dredd/`; the records live in the
 * git directory, in a folder named `faulk-qa` (`fingerprint.mjs` > qaDir).
 * Paths are normalized (`.`, `..`, separators, case) before they are matched.
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
const QA_DIR_NAME = "faulk-qa";

// Judge Dredd run outside `run.mjs`, where the prompt is not checked.
const DIRECT_JUDGE_RUN = /(^|[\s;&|(\\/"'])claude(\.exe|\.cmd)?["']?[ \t][^\n]*\bjudge-dredd\b/i;
const POSIX_WRITE = String.raw`>|\btee\b|\bsed\s+(-\w+\s+)*-i|\bcp\b|\bmv\b|\bln\b|\brm\b|\brmdir\b|\bunlink\b|\btruncate\b|\bdd\b|\bperl\s+-\w*i|\bawk\b.*\s-i\b|\binstall\b|\btouch\b|\brsync\b|\btar\b|\bunzip\b|\bfind\b[^\n]*\s-(delete|exec|execdir|ok|fprint)\b|\bgit\s+(checkout|restore|clean|stash|rm|mv)\b`;
const POWERSHELL_WRITE = String.raw`\b(set-content|add-content|out-file|copy-item|move-item|new-item|remove-item|rename-item|clear-content|set-item|sc|ac|ni|ri|rd|del|erase|cpi|mi|rni|copy|move|ren|expand-archive)\b|\[(system\.)?io\.(file|directory)\]::\s*(write|append|copy|move|delete|create|replace)`;
const SHELL_WRITE = new RegExp(`${POSIX_WRITE}|${POWERSHELL_WRITE}`, "i");
// One call of an official script, on one line, with no chaining, redirection
// or substitution: anything more is no longer that script alone.
const OFFICIAL_SCRIPT =
  /^[ \t]*node[ \t]+(["']?\$\{?CLAUDE_PROJECT_DIR\}?[\\/])?["']?scripts[\\/](agent-review[\\/](brief|run)|agent-qa[\\/]record)\.mjs["']?(?![\w.])[^;&|<>\n\r$`()]*$/;
const MAX_PROMPT = 300;
const GLOB = /[*?[\]{}]/;

export const EDIT_TOOLS = new Set(["Edit", "Write", "MultiEdit", "NotebookEdit"]);
/** Tools whose effect on the code the hooks measure with a fingerprint before and after the call. */
export const SNAPSHOT_TOOLS = new Set(["Bash", "PowerShell", "Agent", "Task"]);
/** Tools whose calls can change files. */
export const CHANGING_TOOLS = new Set([...EDIT_TOOLS, ...SNAPSHOT_TOOLS]);

const deny = (reason) => ({ allow: false, reason });
const parts = (path) => path.split(/[\\/]+/).filter((p) => p !== "" && p !== ".");

/** The components of a path with `.` and `..` resolved (as far as the path itself goes), lower-cased. */
function normalizedParts(path) {
  const out = [];
  for (const part of parts(path)) {
    if (part === "..") {
      if (out.length > 0 && out.at(-1) !== "..") {
        out.pop();
      } else {
        out.push("..");
      }
    } else {
      out.push(part.toLowerCase());
    }
  }
  return out;
}

/**
 * `true` when `path` names a brief, an answer, or anything in a records
 * folder, once `.` and `..` are resolved. A relative path is also resolved
 * against each of `dirs`, so that `../review.json` from inside the records is
 * caught.
 */
export function isProtectedPath(path, dirs = []) {
  if (typeof path !== "string" || path === "") {
    return false;
  }
  const forms = [path, ...(isAbsolute(path) ? [] : dirs.filter(Boolean).map((dir) => resolve(dir, path)))];
  return forms.some((form) => {
    const p = normalizedParts(form);
    if (p.includes(QA_DIR_NAME)) {
      return true;
    }
    const [dir2, dir1, name] = [p.at(-3), p.at(-2), p.at(-1)];
    return dir2 === ".scratch" && dir1 === "judge-dredd" && PROTECTED_BRIEF_NAME.test(name ?? "");
  });
}

/**
 * Whether a word of a shell command reaches protected files: a records folder
 * named in any form; the `.git` directory itself or a glob inside it (the
 * records live there); the brief folder itself, a glob or `..` in it, or any
 * file of it but a `.proposal.md`; a brief or answer named without its folder
 * (after a `cd`).
 */
export function isProtectedWord(word) {
  const p = normalizedParts(word);
  if (p.length === 0) {
    return false;
  }
  if (p.includes(QA_DIR_NAME)) {
    return true;
  }
  const gitAt = p.indexOf(".git");
  if (gitAt !== -1 && (gitAt === p.length - 1 || GLOB.test(word))) {
    return true;
  }
  const dreddAt = p.indexOf("judge-dredd");
  if (dreddAt !== -1) {
    const rest = p.slice(dreddAt + 1);
    const proposal = rest.length === 1 && rest[0].endsWith(".proposal.md") && !GLOB.test(rest[0]);
    if (!proposal || parts(word).includes("..")) {
      return true;
    }
  }
  return PROTECTED_BRIEF_NAME.test(p.at(-1));
}

const words = (command) => command.split(/[\s'"`;|&<>()=]+/).filter(Boolean);

/** `true` when `path` has the shape of a brief: `.scratch/judge-dredd/<issue>-<checkpoint>.md`, without `..`. */
export function isBriefPath(path) {
  if (typeof path !== "string" || parts(path).includes("..")) {
    return false;
  }
  const p = parts(path);
  const [dir2, dir1, name] = [p.at(-3), p.at(-2), p.at(-1)];
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
 * The command with the content of its quoted arguments removed, so that
 * `--dispositions "a > b"` is not taken for a redirection. A double-quoted
 * argument holding `$` or a backtick (substitution) is kept as is.
 */
export function withoutQuotedText(command) {
  return command.replace(/'[^'\n]*'/g, "''").replace(/"[^"$`\n]*"/g, '""');
}

/**
 * `{ allow: true }` or `{ allow: false, reason }` (reason addressed to the
 * agent). `dirs`: directories a relative path resolves against, in order (the
 * input's `cwd`, then the project directory).
 */
export function judgeToolCall({ toolName, toolInput, dirs }) {
  if (SUBAGENT_TOOLS.has(toolName) && AGENT_NAMES.has(toolInput?.subagent_type)) {
    return judgeAgentCall(toolInput.prompt, dirs ?? []);
  }
  if (EDIT_TOOLS.has(toolName)) {
    for (const path of pathValues(toolInput)) {
      if (isProtectedPath(path, dirs ?? [])) {
        return deny(
          `${path} cannot be edited: briefs are written only by node scripts/agent-review/brief.mjs, answers only by node scripts/agent-review/run.mjs, and the records (faulk-qa in the git directory) only by node scripts/agent-qa/record.mjs. Write your plan or report draft to the .proposal.md file next to the brief.`,
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
    if (!official && SHELL_WRITE.test(command) && words(command).some(isProtectedWord)) {
      return deny(
        "This command could write or delete a Judge Dredd brief or answer, or the records (faulk-qa in the git directory). Briefs come from node scripts/agent-review/brief.mjs, answers from node scripts/agent-review/run.mjs, records from node scripts/agent-qa/record.mjs: run each alone on its line. Read files with cat, and write your plan or report draft to the .proposal.md file.",
      );
    }
  }
  return { allow: true };
}
