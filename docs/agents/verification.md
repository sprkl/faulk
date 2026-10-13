---
type: Agent Configuration
title: "Verification before ending a task"
description: "Checks required before a task is done, by area (Rust, engine and modules, commands and agents, web UI, latency, agent scripts), how to record them, and the Stop hook that enforces them."
status: draft
---

# Verification before ending a task

Tests passing is not enough to call a task done. Run the checks of every area the task touches. The commands below are the target; they become real as the first spike creates the workspace. Update this file in the task that changes them.

## Rust (any change under `crates/`)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p engine-wasm --target wasm32-unknown-unknown --release
```

## Engine and modules

- **Offline render tests**: render a known input through the engine or module without an audio device, and assert on the signal: peak and RMS level, expected spectrum (FFT bins), silence where silence is expected, no NaN, no infinity, no denormals.
- **Real-time safety**: the audio path runs under the allocation detector in debug; a test renders several seconds of audio with parameter changes and MIDI events and must not trigger it.
- **Parity**: the same render through the native build and the WASM build gives the same output within a stated tolerance.
- **Listening**: when a change affects the sound, write the render to a WAV file under `.qa/renders/` and tell the user where it is. Never claim something "sounds right": only the user judges by ear.

## Commands and agents (any change under `crates/commands` or to the MCP and ACP bridges)

- Each new or changed command: schema, description and undo covered by tests.
- The parity test passes: every command is exposed as an MCP tool, and no MCP tool exists without its command.
- Drive the change from a real agent: connect Claude Code to the helper's MCP server and run the command, then check the result in the page and in the history (author shown, undo works).

## Web UI (any change under `web/`)

1. Typecheck, lint and tests of `web/`.
2. Start the dev server and open the app with Playwright: start the audio context (a user gesture is required), exercise every acceptance criterion and the screens the change touches, read the console (no error) and the screenshots.
3. Explore beyond the scripted flow: double clicks, reload during playback, helper connected then disconnected, tab in the background.

## Latency (changes to audio I/O, the worklet, the helper or the protocol)

Measure the output latency (and the round trip when input is involved) in both modes, on the user's machine, and report the numbers with how they were measured. Never state a latency you did not measure.

## Agent scripts and hooks (any change under `scripts/` or `.claude/`)

Run `node --test "scripts/**/*.test.mjs"` (CI runs it too), then try the changed hook or script by hand.

## Recording the verification

Once the checks pass, record them for the current state of the code:

```bash
node scripts/agent-qa/record.mjs verified --rust "<...>" --render "<...>" ...
```

Each flag holds what was run and what came out. The flags required depend on the changed files; `record.mjs` refuses a record that lacks one and prints the full command:

| Flag | Required when the change touches | Holds |
|---|---|---|
| `--rust` | `crates/`, `Cargo.toml`, `Cargo.lock`, Rust code under `modules/` | fmt, clippy, test, WASM build |
| `--render` | `crates/engine*`, `modules/` | render tests, allocation detector, native/WASM parity |
| `--commands` | `crates/commands`, `crates/helper` | schema, description, undo, command/MCP parity, run from a real agent |
| `--web`, `--browser`, `--exploration` | `web/`, `site/` | typecheck, lint, tests; the Playwright check; the exploration beyond the script |
| `--scripts` | `scripts/`, `.claude/settings.json`, `.claude/agents/` | `node --test` and the manual try |
| `--latency` | `crates/helper`, `crates/protocol`, `crates/engine-wasm`, a `web/` path containing `worklet` | measured numbers and method, or `not measured: <why>` |

`--note` is optional. The records live outside the working tree, in the checkout's git directory: `.git/faulk-qa/` (`.git/worktrees/<name>/faulk-qa/` for a linked worktree), so `git clean -x`, a glob in the tree or a stray edit never reaches them, and each worktree keeps its own. `last-run.json` holds the verification, `history.jsonl` every record. They are written only by `record.mjs` and the hooks; the guard denies the edit tools and the shell write commands on any path through a `faulk-qa` folder.

### The Stop hook

A `Stop` hook (`scripts/agent-qa/qa-guard.mjs`) refuses to end a turn when the session's own tool calls changed the guarded code of a checkout, unless that checkout's current state has a verification record **and** its task branch has a delivery review record (`judge-dredd.md`). Its message lists the exact commands to run.

- **Guarded code**: `crates/`, `web/`, `modules/`, `site/`, `Cargo.toml`, `Cargo.lock`, `scripts/`, `.claude/settings.json`, `.claude/agents/`, tests included. Files ignored by git (`target/`, `node_modules/`, `test-results/`…) never count.
- **Fingerprint**: a hash of the content of the guarded files that differ from the merge base with `develop` (committed or not, untracked included). Any edit changes it and makes the record stale; committing does not. Without a `develop` reference, all the guarded code counts as changed.
- **Who changed the code** (a session is judged only in the checkouts where it is a *writer*):
  - Edit, Write, MultiEdit, NotebookEdit: the session becomes a writer in the checkout of the file the tool targets, when that file is guarded code. Only that file counts, so another writer's concurrent edit never does.
  - Bash, PowerShell, Agent, Task: the `PreToolUse` hook takes the state of each checkout the session works in (its current directory and the project directory) and the `PostToolUse` hook compares after the call. A call that changed the fingerprint makes the session a writer. A call that moved into a checkout the session had not seen counts any change there as the session's. Agent and Task calls make the lead a writer for what its subagents change during the call.
  - A call that moved HEAD without authoring code (reflog: checkout, switch, pull, merge, reset, rebase; `gh pr checkout` is a checkout) is judged only on the uncommitted changes, which such a move carries over untouched unless the call also edited them; the session's baseline then moves to the new state, and a later block message says so. A call that commits, cherry-picks or reverts is judged on the whole fingerprint.
  - A call that never gets its `PostToolUse` (denied by another hook or by the user, interrupted, failed) is dropped at the next stop: it attributes nothing.
  - Once a session is a writer in a checkout, its calls there are no longer measured.
- **Session baseline**: the fingerprint the first time the hooks see the session in a checkout (`SessionStart`, a tool call, or a stop). A writer whose checkout is back to its baseline (it undid its changes) is let through.
- **Checkouts**: the hook judges every checkout the session wrote to, wherever it stops; each has its own records.
- **Deferral**: to stop mid-task only to ask the user a question, run `node scripts/agent-qa/record.mjs deferred --reason "<the question>"`. It lets exactly one stop through for the current state of the code. Once used, another one is accepted only after the user writes again (the `UserPromptSubmit` hook records when), whatever the code does meanwhile. It is kept apart (`deferral.json`) and never erases a verification. Never defer at the end of a task.
- **Failure**: fail open, loudly. If the hook cannot compute a fingerprint (git missing, a repository without a commit) or its input has no session id, it lets the turn end and shows the user a warning. Claude Code also stops honouring a Stop hook after eight blocks in a row without a tool call (hooks reference, https://code.claude.com/docs/en/hooks; not measured here).
- **Guard changes**: when `scripts/agent-qa/`, `scripts/agent-review/`, `.claude/settings.json` or `.claude/agents/` differ from the merge base with `develop`, every stop shows the user a warning: the guard that runs is the modified one.

### Cost

Measured on 2026-10-14 on the lead's Windows machine, in this repository's worktree on `feat/6-agent-guardrails`, with PowerShell: ten runs of each hook fed the same Bash `PreToolUse` input (`Get-Content in.json | node scripts/...`, timed with `Measure-Command`), for a session that had not written yet.

| Command | Runs (ms) |
|---|---|
| `node -e "0"` (Node start alone) | 85 84 85 90 78 91 89 84 81 92 |
| `agent-guard.mjs` (PreToolUse, Bash) | 282 311 297 293 326 293 327 278 336 315 |
| `post-tool-use.mjs` (PostToolUse, Bash) | 317 270 280 298 286 303 304 298 307 315 |
| `qa-guard.mjs` (Stop) | 286 292 298 282 268 286 287 303 319 305 |

So about 0.6 s per Bash, PowerShell, Agent or Task call until the session writes, and about 0.3 s per stop. Edit tools are not measured with a fingerprint and cost about one Node start each. Linux is not measured.

### Limits

- The record holds what the lead declares: the hook checks that the fields are there, not that the checks ran.
- The records are shared by every session of a checkout: one session's deferral can be consumed by another's stop, and the last verification or review recorded wins.
- A change made by someone else during one of the session's own Bash, PowerShell, Agent or Task calls (between its two measures) is counted as the session's. Modification times cannot settle it: the change and the call overlap in time.
- Background work is not attributed: a command started with `run_in_background`, `&`, `nohup` or `Start-Process`, or a subagent run in the background, writes after its call has returned. A subagent in its own worktree writes to a checkout the lead's calls do not measure: the lead's later checkout or merge of its branch is a move, not an authoring.
- A tool outside the eight above (an MCP server that writes files) is not measured.
- Subagents' own stops are not checked (`SubagentStop` is not hooked); their changes count only through the lead's Agent or Task call, or through their own tool calls if Claude Code gives them the lead's session id (not documented).
- The guard reads command lines only: an interpreter (`node -e`, `python -c`) can still write or delete a record.
- Changing the guard is trusted only after a human reviews the pull request: the agent that edits `record.mjs` or a hook can also run the edited version. The Stop hook warns the user when the guard differs from `develop`; it cannot do more.
- The hooks run through Git Bash on Windows (`node "$CLAUDE_PROJECT_DIR/..."`). Without Git Bash, Claude Code runs hooks in PowerShell, where `$CLAUDE_PROJECT_DIR` is not expanded: the hooks then fail without blocking anything.

