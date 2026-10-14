/**
 * Builds Judge Dredd's brief from GitHub and git, then prints its path: that
 * path is all Judge Dredd receives.
 *
 * Usage:
 *   node scripts/agent-review/brief.mjs <issue> plan       (plan in .scratch/judge-dredd/<issue>-plan.proposal.md)
 *   node scripts/agent-review/brief.mjs <issue> delivery [--pr <n>]
 *                                                           (report draft in .scratch/judge-dredd/<issue>-delivery.proposal.md)
 *   node scripts/agent-review/brief.mjs <issue> replies --pr <n>
 * Option: --issue-file <json> (the issue as `gh issue view --json number,title,body,comments` prints it,
 * instead of calling gh, for tests and offline work). Such a brief says `issue-source: file`, and
 * `record.mjs reviewed` refuses it: a review that counts judges the ticket as GitHub holds it.
 *
 * @doc docs/agents/judge-dredd.md
 */

import { execFileSync } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";
import { changeFingerprint, scriptRoot } from "../agent-qa/fingerprint.mjs";
import { CHECKPOINTS, renderBrief } from "./brief-core.mjs";
import { BRIEF_DIR } from "./guard-core.mjs";

const fail = (message) => {
  console.error(`[judge-dredd] ${message}`);
  process.exit(1);
};

let parsed;
try {
  parsed = parseArgs({
    allowPositionals: true,
    options: {
      pr: { type: "string" },
      "issue-file": { type: "string" },
    },
  });
} catch (error) {
  fail(`${error.message}. Usage: node scripts/agent-review/brief.mjs <issue> <plan|delivery|replies> [--pr <n>]`);
}
const { values, positionals } = parsed;
const [issueNumber, checkpoint] = positionals;
if (!/^\d+$/.test(issueNumber ?? "") || !CHECKPOINTS.includes(checkpoint) || positionals.length !== 2) {
  fail("usage: node scripts/agent-review/brief.mjs <issue> <plan|delivery|replies> [--pr <n>]");
}
if (checkpoint === "replies" && !values.pr) {
  fail(`--pr is required at the replies checkpoint: node scripts/agent-review/brief.mjs ${issueNumber} replies --pr <n>`);
}

const root = scriptRoot();
const run = (cmd, args) => execFileSync(cmd, args, { cwd: root, encoding: "utf8", maxBuffer: 128 * 1024 * 1024 });
const gh = (...args) => JSON.parse(run("gh", args));

function loadIssue() {
  if (values["issue-file"]) {
    return JSON.parse(readFileSync(values["issue-file"], "utf8"));
  }
  try {
    return gh("issue", "view", issueNumber, "--json", "number,title,body,comments,parent");
  } catch {
    // Older gh without the `parent` field.
    return gh("issue", "view", issueNumber, "--json", "number,title,body,comments");
  }
}

const raw = loadIssue();
const issue = {
  number: raw.number,
  title: raw.title,
  body: raw.body,
  parent: raw.parent ? { number: raw.parent.number, title: raw.parent.title } : null,
  comments: (raw.comments ?? []).map((c) => ({ author: c.author?.login ?? "unknown", createdAt: c.createdAt, body: c.body })),
};

const proposalPath = join(root, BRIEF_DIR, `${issueNumber}-${checkpoint}.proposal.md`);
const proposal = existsSync(proposalPath) ? readFileSync(proposalPath, "utf8") : undefined;
if (checkpoint !== "replies" && !proposal?.trim()) {
  const what = checkpoint === "plan" ? "plan" : "report draft (the claims Judge Dredd checks)";
  fail(`write your ${what} to ${BRIEF_DIR}/${issueNumber}-${checkpoint}.proposal.md first, then run this command again.`);
}

const brief = { checkpoint, issue, issueSource: values["issue-file"] ? "file" : "github", proposal };
if (checkpoint !== "plan") {
  let base;
  for (const ref of ["origin/develop", "develop"]) {
    try {
      base = run("git", ["merge-base", "HEAD", ref]).trim();
      break;
    } catch {
      // Missing ref: try the next one.
    }
  }
  if (!base) {
    fail("no develop branch to compare with: run git fetch origin develop, then this command again.");
  }
  // Lockfiles are listed with the changed files but their content is left
  // out of the diff (thousands of generated lines); the brief says so, and the
  // manifests that drive them (Cargo.toml, package.json) are in the diff.
  const lockfiles = [":(exclude)Cargo.lock", ":(exclude,glob)**/pnpm-lock.yaml", ":(exclude,glob)**/package-lock.json"];
  brief.base = base.slice(0, 12);
  brief.head = "working tree";
  brief.fingerprint = changeFingerprint(root) ?? "none";
  brief.commits = run("git", ["log", "--format=%h %s%n%n%b", `${base}..HEAD`]);
  brief.files = run("git", ["diff", "--name-status", base, "--", "."]);
  brief.diff = run("git", ["diff", "--no-ext-diff", base, "--", ".", ...lockfiles]);
  const changedLockfiles = run("git", ["diff", "--name-only", base, "--", ".", ":(glob)**/Cargo.lock", ":(glob)**/pnpm-lock.yaml", ":(glob)**/package-lock.json"])
    .split("\n")
    .filter(Boolean);
  if (changedLockfiles.length > 0) {
    brief.diffNote = `The content of these lockfiles is left out of the diff: ${changedLockfiles.join(", ")}. Check the dependency changes in the manifests and run git diff ${base.slice(0, 12)} -- <lockfile> if needed.`;
  }
  const untracked = run("git", ["ls-files", "--others", "--exclude-standard"]).split("\n").filter(Boolean);
  for (const file of untracked) {
    const path = join(root, file);
    // Never follow a link: its target may be outside the repository.
    const stat = lstatSync(path);
    if (stat.isSymbolicLink()) {
      brief.files += `A\t${file} (untracked symbolic link, not followed)\n`;
      continue;
    }
    brief.files += `A\t${file} (untracked)\n`;
    const content = !stat.isFile() || stat.size > 1024 * 1024 ? null : readFileSync(path);
    const text = content && !content.includes(0) ? content.toString("utf8") : "(binary or larger than 1 MB: not shown)";
    brief.diff += `\n--- /dev/null\n+++ b/${file} (untracked)\n${text}`;
  }
}

if (values.pr) {
  brief.pullRequest = gh("pr", "view", values.pr, "--json", "number,title,body");
  if (checkpoint === "replies") {
    const [owner, repo] = gh("repo", "view", "--json", "nameWithOwner").nameWithOwner.split("/");
    const comments = gh("api", "--paginate", "--slurp", `repos/${owner}/${repo}/pulls/${values.pr}/comments`).flat();
    const threads = new Map();
    for (const c of comments) {
      const rootId = c.in_reply_to_id ?? c.id;
      if (!threads.has(rootId)) {
        threads.set(rootId, { path: c.path, line: c.line ?? c.original_line, messages: [] });
      }
      threads.get(rootId).messages.push({ author: c.user.login, body: c.body });
    }
    brief.reviewThreads = [...threads.values()];
  }
}

const out = join(root, BRIEF_DIR, `${issueNumber}-${checkpoint}.md`);
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, renderBrief(brief));
console.log(`${BRIEF_DIR}/${issueNumber}-${checkpoint}.md`);
