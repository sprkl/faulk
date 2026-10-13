---
type: Agent Configuration
title: "Judge Dredd: impartial review at the task checkpoints"
description: "Read-only reviewer subagent that judges the lead's plan and, once per task, its delivery before the pull request, from a brief it receives as a path only."
---

# Judge Dredd: impartial review at the task checkpoints

The lead reviews its own work with the assumptions it used to write it. Judge Dredd (`.claude/agents/judge-dredd.md`) is a second, impartial reviewer: read-only (`Read, Grep, Glob`), on Opus, without `CLAUDE.md` loaded for it (it reads the rules itself), and blind to the lead's reasoning. It advises; the user decides.

## Checkpoints

| Checkpoint | When | Brief |
|---|---|---|
| `plan` | once, before starting an issue | `.scratch/judge-dredd/<issue>-plan.md` |
| `delivery` | once, as the last step before opening the pull request, after the lead's own reviews (`/code-review`) and their fixes | `.scratch/judge-dredd/<issue>-delivery.md` |
| `replies` | optional, only if the user asks for it | `.scratch/judge-dredd/<issue>-replies.md` |

Judge Dredd does not run inside the lead's own work, test and fix loop, and is not run again for the fixes of its own findings.

## Writing the brief

Until a script builds briefs (see Limits), the lead writes the brief by hand. It holds, in this order:

1. `checkpoint: plan|delivery|replies` and the issue number.
2. The issue body and comments (`gh issue view <n> --comments`), verbatim.
3. The paths of the documents the issue cites.
4. **General claims to prove by enumeration**: every "all", "every", "each", "none" of the plan or the report draft.
5. The plan (plan checkpoint) or the report draft (delivery).
6. Delivery and replies only: `git log --oneline develop..HEAD` and the full `git diff develop...HEAD`, pasted from the command output, never summarised.

Then call the `judge-dredd` subagent with the brief's path as the **whole** prompt.

## The lead's obligations

1. **Every finding is shown to the user**, with the lead's opinion: fixed, or rejected and why. No finding is dropped or rejected silently. The lead acts on its own disposition without waiting, unless a finding raises a decision that is the user's.
2. **General claims are proven** by an enumeration (a command, a Grep), never by memory.
3. **Every mechanism the lead invents gets an attack pass**: how it fails or is bypassed, and a test of each state transition it meets.

## Limits

Nothing enforces these checkpoints yet: they rest on the lead's good faith. Once the codebase exists, add the enforcement used in `sprkl/constat-ai`: a script that builds the brief from GitHub and git, a `PreToolUse` guard that only lets `judge-dredd` receive a brief path, and a `Stop` hook that requires the delivery review and the verification record for the current state of the code. Judge Dredd's agreement is weak evidence, since it shares the lead's model family.
