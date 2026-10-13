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

`--note` is optional. The record lives in `.qa/last-run.json` (ignored by git), with a history in `.qa/history.jsonl`; the files are written only by `record.mjs`, and the guard denies editing them.

### The Stop hook

A `Stop` hook (`scripts/agent-qa/qa-guard.mjs`) refuses to end a turn while the guarded code differs from what the session found, unless the current state has a verification record **and** the task's branch has a delivery review record (`judge-dredd.md`). Its message lists the exact commands to run.

- **Guarded code**: `crates/`, `web/`, `modules/`, `site/`, `Cargo.toml`, `Cargo.lock`, `scripts/`, `.claude/settings.json`, `.claude/agents/`, tests included. Files ignored by git (`target/`, `node_modules/`, `test-results/`…) never count.
- **Fingerprint**: a hash of the content of the guarded files that differ from the merge base with `develop` (committed or not, untracked included). Any edit changes it and makes the record stale; committing does not. Without a `develop` reference, all the guarded code counts as changed.
- **Session baseline**: the fingerprint the first time the hooks see a session (`SessionStart`, its first guarded tool call, or its first stop), stored once in `.qa/sessions/<session id>.json` and never overwritten. A session that leaves the code as it found it is never blocked, even on a checkout with unverified changes left by someone else. A session the hooks never saw before its first stop (it started before the hooks existed and called no guarded tool) is let through and gets its baseline then.
- **Checkouts**: the hook judges the checkout of the session's current directory (a worktree the session entered) and the project directory; each has its own `.qa/`.
- **Deferral**: to stop mid-task only to ask the user a question, run `node scripts/agent-qa/record.mjs deferred --reason "<the question>"`. It lets exactly one stop through for the current state of the code; another one is accepted only after the user writes again. It is kept apart (`.qa/deferral.json`) and never erases a verification. Never defer at the end of a task.
- **Failure**: if the hook cannot compute the fingerprint (git missing, a repository without a commit), it lets the turn end and shows the user a warning. Claude Code also stops honouring a Stop hook after eight blocks in a row without a tool call.

### Limits

- The record holds what the lead declares: the hook checks that the fields are there, not that the checks ran.
- `.qa/` is shared by every session of a checkout: one session's deferral can be consumed by another's stop, and the last verification or review recorded wins.
- A session that stops in another directory than the checkout it changed (it moved back from a worktree) is judged only on the checkouts it stops in.
- After `/clear` or a resume that starts a new session id, the code as found then is the new session's baseline: changes left unverified before (after a deferral) are not blocked by the new session.
- Subagents are not checked by the Stop hook (`SubagentStop` is not hooked, and a subagent shares its lead's session id, so it could not be told apart from the lead's baseline).
- The hooks run through Git Bash on Windows (`node "$CLAUDE_PROJECT_DIR/..."`). Without Git Bash, Claude Code runs hooks in PowerShell, where `$CLAUDE_PROJECT_DIR` is not expanded: the hooks then fail without blocking anything.
