/**
 * Has Judge Dredd judge a brief outside the interactive session and writes its
 * answer next to the brief (`<brief>.answer.md`), headed by the brief's
 * SHA-256: `record.mjs reviewed` accepts only an answer to the current brief.
 * A review takes minutes: run it in the background.
 *
 * Usage: node scripts/agent-review/run.mjs .scratch/judge-dredd/<issue>-<checkpoint>.md
 *          [--model <default: the model of .claude/agents/judge-dredd.md>] [--budget <usd, default 5>] [--timeout <minutes, default 30>]
 *
 * @doc docs/agents/judge-dredd.md
 */

import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { scriptRoot } from "../agent-qa/fingerprint.mjs";
import { answerIsComplete } from "./brief-core.mjs";
import { BRIEF_DIR, isBriefPath } from "./guard-core.mjs";
import { agentDefinition, runJudgeDredd } from "./headless.mjs";

const USAGE =
  "usage: node scripts/agent-review/run.mjs .scratch/judge-dredd/<issue>-<plan|delivery|replies>.md [--model <model>] [--budget 5] [--timeout 30]";
const fail = (message) => {
  console.error(`[judge-dredd] ${message}`);
  process.exit(1);
};

let parsed;
try {
  parsed = parseArgs({
    allowPositionals: true,
    options: {
      model: { type: "string" },
      budget: { type: "string", default: "5" },
      timeout: { type: "string", default: "30" },
    },
  });
} catch (error) {
  fail(`${error.message}. ${USAGE}`);
}
const { values, positionals } = parsed;
const given = (positionals[0] ?? "").replaceAll("\\", "/");
const match = given.match(/(?:^|\/)(\d+)-(plan|delivery|replies)\.md$/);
if (positionals.length !== 1 || !isBriefPath(given) || !match) {
  fail(USAGE);
}
const [, issue, checkpoint] = match;
const root = scriptRoot();
const brief = `${BRIEF_DIR}/${issue}-${checkpoint}.md`;
const briefFile = join(root, brief);
if (!existsSync(briefFile) || !lstatSync(briefFile).isFile() || lstatSync(briefFile).isSymbolicLink()) {
  fail(`brief not found: ${brief}. Run node scripts/agent-review/brief.mjs ${issue} ${checkpoint} first.`);
}
if (!/^\d+(\.\d+)?$/.test(values.budget) || !/^\d+$/.test(values.timeout)) {
  fail(USAGE);
}
// This script writes the answer and its run files in the brief folder: none
// of the folders on the way, nor the files it writes, may be a link, which it
// would follow to write outside the repository.
for (const path of [
  join(root, ".scratch"),
  join(root, BRIEF_DIR),
  join(root, BRIEF_DIR, `${issue}-${checkpoint}.answer.md`),
  join(root, BRIEF_DIR, "headless.settings.json"),
  join(root, BRIEF_DIR, "headless.agents.json"),
]) {
  let stat = null;
  try {
    stat = lstatSync(path);
  } catch {
    // Absent: the script creates a regular file.
  }
  if (stat?.isSymbolicLink()) {
    fail(`${path} is a symbolic link: remove it, then run this command again.`);
  }
}

// The answer names its model: `record.mjs reviewed` accepts only a review by
// the model of Judge Dredd's definition.
const model = values.model ?? agentDefinition(root)["judge-dredd"].model ?? "opus";
// Hashed before the run: an answer always names the brief Judge Dredd read.
const briefHash = createHash("sha256").update(readFileSync(briefFile)).digest("hex");
let result;
try {
  result = runJudgeDredd({
    root,
    brief,
    model,
    budget: values.budget,
    timeoutMinutes: Number(values.timeout),
    workDir: join(root, BRIEF_DIR),
  });
} catch (error) {
  fail(`Judge Dredd did not answer (${error instanceof Error ? error.message.split("\n")[0] : error}): run the same command again.`);
}
const { text, cost } = result;
if (!answerIsComplete(text, checkpoint)) {
  fail(
    `incomplete answer (${cost.toFixed(2)} USD), not recorded: run node scripts/agent-review/run.mjs ${brief} --budget ${Number(values.budget) * 2} again.`,
  );
}
const answer = join(root, BRIEF_DIR, `${issue}-${checkpoint}.answer.md`);
writeFileSync(answer, `brief-sha256: ${briefHash}\nmodel: ${model}\n\n${text}\n`);
console.log(`${BRIEF_DIR}/${issue}-${checkpoint}.answer.md (${cost.toFixed(2)} USD)`);
