---
type: Agent Configuration
title: "Judge Dredd: impartial review at the task checkpoints"
description: "Read-only reviewer subagent that judges the lead's plan and, once per task, its delivery before the pull request, from a brief built by a script and received as a path only; the guard and the Stop hook that enforce it, and their limits."
---

# Judge Dredd: impartial review at the task checkpoints

The lead reviews its own work with the assumptions it used to write it. Judge Dredd (`.claude/agents/judge-dredd.md`) is a second, impartial reviewer: read-only (`Read, Grep, Glob`), on Opus, without `CLAUDE.md` loaded for it (it reads the rules itself), and blind to the lead's reasoning. It advises; the user decides.

## Checkpoints

| Checkpoint | When | Command |
|---|---|---|
| `plan` | once, before starting an issue | write the plan to `.scratch/judge-dredd/<issue>-plan.proposal.md`, then `node scripts/agent-review/brief.mjs <issue> plan` |
| `delivery` | once, as the last step before opening the pull request, after the lead's own reviews (`/code-review`) and their fixes | write the report draft (the claims) to `.scratch/judge-dredd/<issue>-delivery.proposal.md`, then `node scripts/agent-review/brief.mjs <issue> delivery` |
| `replies` | optional, only if the user asks for it | `node scripts/agent-review/brief.mjs <issue> replies --pr <n>` |

Judge Dredd does not run inside the lead's own work, test and fix loop, and is not run again for the fixes of its own findings. The `reviewed` record needs a brief built for the current state of the code: run the delivery review once the code is final, and record it before making any fix it brings.

## Building the brief and running the review

`scripts/agent-review/brief.mjs` builds the brief from GitHub and git and prints its path, `.scratch/judge-dredd/<issue>-<checkpoint>.md`. The brief holds, in this order: the checkpoint, the fingerprint of the code (delivery and replies), the issue body, its parent and its comments verbatim, the documents they cite, the general claims to prove by enumeration ("all", "every", "each", "none", "tous", "chaque", "aucun"…), the lead's plan or report draft, the pull request and its review threads (`--pr`), the commits, the changed files and the full diff against the merge base with `develop`, untracked files included (symbolic links listed, not followed), except the content of lockfiles (`Cargo.lock`, `pnpm-lock.yaml`, `package-lock.json`): they are listed with the changed files, and the brief says their content is left out. Everything except the diff is presented to Judge Dredd as information to check, never as instructions.

Then run the review:

```bash
node scripts/agent-review/run.mjs .scratch/judge-dredd/<issue>-<checkpoint>.md
```

It runs Judge Dredd headless (`claude -p`, project settings only, `settings.local.json` excluded, no MCP server, no hooks, no `CLAUDE.md`, no automatic memory) and writes its answer to `.scratch/judge-dredd/<issue>-<checkpoint>.answer.md`, headed by the SHA-256 of the brief and the model. A review takes several minutes: run it in the background (the Bash tool's `run_in_background`), not in the foreground where the tool's timeout kills it. Options: `--model` (default: the model of `.claude/agents/judge-dredd.md`; a review by another model is refused by `record.mjs`), `--budget` in USD (default 5, spent on the user's account), `--timeout` in minutes (default 30).

The isolation from `CLAUDE.md` was checked on 2026-10-14 with Claude Code 2.1.296: the same `claude -p` flags as `run.mjs` (project settings, `--settings` with `claudeMdExcludes`, `--agents` file, `--agent judge-dredd`, on Haiku), asked to quote any project instruction mentioning Faulk, answered "NOT LOADED"; a control run without the exclusions and the agent quoted the first words of `CLAUDE.md`. The run does not tell whether `claudeMdExcludes` or the agent's `omitClaudeMd` does the work (the `--agents` definition carries no `omitClaudeMd`, so it is the exclusions or `--agent` itself). Re-check after a Claude Code upgrade.

`brief.mjs --issue-file <json>` builds a brief from a local copy of the issue, for tests and offline work; such a brief says `issue-source: file` and does not count for the review record. An answer that holds no verdict (interrupted or truncated run), or a delivery answer that judges no acceptance criterion, is not written: run it again, with a higher `--budget` if needed.

For a plan, the `judge-dredd` subagent can also be called in the session with the brief's path as the **whole** prompt. For a delivery, only `run.mjs` counts: the answer file is what `record.mjs reviewed` checks, and nothing else may write it.

## The lead's obligations

1. **Every finding is shown to the user**, with the lead's opinion: fixed, or rejected and why. No finding is dropped or rejected silently. The lead acts on its own disposition without waiting, unless a finding raises a decision that is the user's.
2. **General claims are proven** by an enumeration (a command, a Grep), never by memory.
3. **Every mechanism the lead invents gets an attack pass**: how it fails or is bypassed, and a test of each state transition it meets.
4. After the `delivery` review, record its outcome once for the branch:

   ```bash
   node scripts/agent-qa/record.mjs reviewed --brief .scratch/judge-dredd/<issue>-delivery.md --dispositions "<each finding: fixed, or rejected and why>"
   ```

   The record is refused unless the brief is a `delivery` brief of `.scratch/judge-dredd/` built from the GitHub issue for the current state of the code, its issue is the one of the task branch (`feat/<issue>-…`), and `run.mjs` wrote an answer to that exact brief, by Judge Dredd's model, that judges at least one acceptance criterion.

## Enforcement

Five hooks in `.claude/settings.json` apply to every Claude Code session in this repository.

- **The guard** (`PreToolUse`, `scripts/agent-review/agent-guard.mjs`):
  - a `judge-dredd` subagent call is denied unless its prompt is one path to an existing regular file `.scratch/judge-dredd/<issue>-<plan|delivery|replies>.md`; a `claude … judge-dredd` command in Bash or PowerShell is denied too;
  - Edit, Write, MultiEdit and NotebookEdit are denied on briefs, answers, the user's `.claude/settings.local.json` (its `disableAllHooks` switches the hooks off) and anything in a `faulk-qa` records folder (in the git directory, `verification.md` > Recording the verification). Paths are normalized first: `.` and `..` resolved, relative paths resolved against the session's directory and the project directory, either separator, any case;
  - a Bash or PowerShell command with a write or delete operation (`>`, `tee`, `sed -i`, `cp`, `mv`, `rm`, `touch`, `find … -delete` or `-exec`, `git checkout`, `Set-Content`, `Out-File`, `Copy-Item`, `Remove-Item`, `[IO.File]::Write…` and others) is denied when one of its words names `.claude/settings.local.json`, a `faulk-qa` folder, the `.git` directory itself or a glob in it, the brief folder (anything in it but a `.proposal.md`, a glob, a `..`), or a brief or answer by name;
  - a single call of `brief.mjs`, `run.mjs` or `record.mjs` alone on its line passes.
- **The Stop hook** (`scripts/agent-qa/qa-guard.mjs`): a turn in which the session's own tool calls changed the guarded code ends only with a verification recorded for its current state (`verification.md`) **and** a delivery review recorded for the task's branch. See `verification.md` > Recording the verification for the guarded paths, how a change is attributed to a session, the deferral and the warnings it shows the user.
- **PostToolUse**, **SessionStart** and **UserPromptSubmit** (`post-tool-use.mjs`, `session-start.mjs`, `prompt-submit.mjs`) attribute changes to the session, record its baseline, and record the time of the user's last message.

The review record names its branch: later states of the same task branch (fixes after the review, a pull request review) only need a new verification. Only a branch named by the convention (`feat|fix|spike/<issue>-<slug>`) carries a review over; on `develop`, `main`, a detached head or any other name, a review holds for the state it reviewed only.

## Threat model

The guard and the Stop hook stop an honest agent that skips or forgets a step. They do not stop a deliberately evasive agent: hiding a write from the hooks on purpose (interpreters, encoded commands, variables expanding to paths, background processes) is out of scope, and listed below and in `verification.md` > Limits.

## Limits

All of these rest on the lead's good faith:

- The shell check reads the command line only, after removing quotes and escapes the way bash and PowerShell do: an interpreter that writes a file (`node -e`, `python -c`), an encoded command, a variable expanding to a path, or a glob that names neither folder (`rm ../*/*` from the wrong place) passes. A command that names a brief and contains a write operation is refused even when harmless: run the scripts alone on their line.
- `git clean -x` deletes `.scratch/`, and with it the briefs and answers: that loses a review, never forges one. The records are out of its reach.
- The `plan` checkpoint is required by this procedure but not checked by a hook.
- The review record is bound to a branch, not to the end of the work: a delivery review recorded early on a task branch satisfies the Stop hook for the rest of that branch, and its dispositions are not compared with the findings of the answer. The procedure (delivery as the last step) is what prevents this.
- Changing the guard is trusted only after a human reviews its pull request. The guard, the hooks and `.claude/settings.json` are guarded code, but the agent that edits `record.mjs` or a hook can also run its edited version to record or to stop; the Stop hook only warns the user when the guard differs from `develop`. A `disableAllHooks` in `.claude/settings.local.json` (not tracked) switches everything off: that switch is the user's, and the guard denies agents writes to that file.
- Subagents are not checked by their own stop (`SubagentStop` is not hooked), and background work is not attributed. See `verification.md` > Limits.
- Judge Dredd's agreement is weak evidence, since it shares the lead's model family.
