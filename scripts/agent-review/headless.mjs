/**
 * Runs Judge Dredd outside an interactive session (`claude -p`), isolated:
 * project settings only, no MCP server, no CLAUDE.md, no hooks, no automatic
 * memory, the agent definition passed with `--agents` from its file.
 *
 * @doc docs/agents/judge-dredd.md
 */

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

/** The `--agents` JSON for Judge Dredd, built from `.claude/agents/judge-dredd.md`. */
export function agentDefinition(root) {
  const source = readFileSync(join(root, ".claude", "agents", "judge-dredd.md"), "utf8").replaceAll("\r\n", "\n");
  const match = source.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/);
  if (!match) {
    throw new Error(".claude/agents/judge-dredd.md has no frontmatter");
  }
  const [, frontmatter, prompt] = match;
  const field = (name) => frontmatter.match(new RegExp(`^${name}: (.*)$`, "m"))?.[1].trim();
  return {
    "judge-dredd": {
      description: field("description"),
      tools: (field("tools") ?? "Read, Grep, Glob").split(",").map((t) => t.trim()),
      model: field("model"),
      prompt: prompt.trim(),
    },
  };
}

/** Quotes an argument for cmd.exe, for the `claude.cmd` fallback on Windows. */
const cmdQuote = (arg) => (/^[\w./:\\=-]+$/.test(arg) ? arg : `"${arg.replaceAll('"', '""')}"`);

function claude(args, options) {
  try {
    return execFileSync("claude", args, options);
  } catch (error) {
    // npm installs `claude.cmd`, which Windows only runs through a shell.
    if (process.platform === "win32" && ["ENOENT", "EINVAL"].includes(error?.code)) {
      return execFileSync(["claude", ...args.map(cmdQuote)].join(" "), { ...options, shell: true });
    }
    throw error;
  }
}

/**
 * `root`: repository holding the agent definition and the brief; `brief`: path
 * relative to `root`; `workDir`: folder for the run's settings and agent files.
 * Returns `{ text, cost }`.
 */
export function runJudgeDredd({ root, brief, model, budget, timeoutMinutes, workDir }) {
  const settingsPath = join(workDir, "headless.settings.json");
  const agentsPath = join(workDir, "headless.agents.json");
  // Judge Dredd reads the project rules itself: nothing is loaded for it, and
  // the repository's hooks must not judge its session.
  writeFileSync(
    settingsPath,
    JSON.stringify({
      disableAllHooks: true,
      claudeMdExcludes: [join(root, "CLAUDE.md"), join(homedir(), ".claude", "CLAUDE.md")],
    }),
  );
  writeFileSync(agentsPath, JSON.stringify(agentDefinition(root)));
  const raw = claude(
    [
      "-p",
      "--setting-sources",
      "project,local",
      "--strict-mcp-config",
      "--settings",
      settingsPath,
      "--agents",
      agentsPath,
      "--agent",
      "judge-dredd",
      "--model",
      model,
      "--max-budget-usd",
      budget,
      "--output-format",
      "json",
      "--no-session-persistence",
      brief,
    ],
    {
      cwd: root,
      env: { ...process.env, CLAUDE_CODE_DISABLE_AUTO_MEMORY: "1" },
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
      timeout: timeoutMinutes * 60 * 1000,
    },
  );
  const output = JSON.parse(raw);
  if (output.is_error) {
    throw new Error(`Judge Dredd failed: ${output.result}`);
  }
  return { text: String(output.result ?? ""), cost: output.total_cost_usd ?? 0 };
}
