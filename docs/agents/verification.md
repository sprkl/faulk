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
cargo build -p project-wasm --target wasm32-unknown-unknown --release
```

## Engine and modules

- **Offline render tests**: render a known input through the engine or module without an audio device, and assert on the signal: peak and RMS level, expected spectrum (FFT bins), silence where silence is expected, no NaN, no infinity, no denormals.
- **Render harness** (`crates/test-support`, QA-1): build the project through commands, render it, assert on the signal. A `Session::single_synth()` (or `Session::from_state(state, ctx)`, which starts with a `Reset` and the snapshot of that project) runs each command (`note_on`, `note_off`, `set_param`, or any `Command` with `run`, or `exec` for its output) on a real project state, so a refused command fails the test, and records the engine messages of each outcome (`commands::engine_msgs::compile`, PR-3) plus the blocks to render (`render_secs`, `render_blocks`) as a `Scenario`; `resync` sends a new snapshot. Raw engine messages go in with `messages`. `EngineView::of_stream(scenario, &ctx.engine)` folds a scenario's messages into the engine's control state named by ids, and `EngineView::of_project` builds the same view from a project without the compiler: live commands and a snapshot must give the project's view (`crates/test-support/tests/compiler.rs`). Then:
  - `render_native(&scenario)` renders through `engine::host`; both flush the prepare side before each block and panic on a rejected, skipped or still pending message; `render_wasm` through the `wasm32-unknown-unknown` build of `engine-wasm` in an interpreter (built into `target/parity`; not the shipped `wasm32-wasip1-threads` module, which the web tests run); `parity(&scenario)` does both, asserts they agree within `PARITY_TOLERANCE` (1e-5) and returns the native render.
  - On the `Render`: `peak`, `rms` (whole render or `_in(start..end)` seconds), `assert_finite`, `assert_no_denormals`, `assert_clean` (adds full scale), `assert_silent(window)` (exact zeros), `assert_audible`, `onset(threshold, from)` and `assert_onset_at`, `spectrum(window)` (`dominant_hz`, `peak_in(band)`), `assert_close`, and `write_wav(name)` for `.qa/renders/<name>.wav`.
  - Reference projects: `test_support::reference::ALL`, built by functions in `reference.rs` and recorded in `crates/test-support/tests/fixtures/<name>.stream`; `reference::load` fails when a fixture is stale. After a change of the command layer's mapping, `FAULK_RECORD=1 cargo test -p test-support` rewrites them. A new reference project (audio, automation: AU-8, AM-6) is a function added to `ALL` plus its recorded fixture.
  - A crate uses it with `test-support = { path = "../test-support" }` in `[dev-dependencies]`; examples: `crates/engine/tests/project_render.rs`, `crates/engine-wasm/tests/project_parity.rs`.
- **Real-time safety**: the audio path runs under the allocation detector in debug; a test renders several seconds of audio with parameter changes and MIDI events and must not trigger it.
- **Parity**: the same render through the native build and the WASM build gives the same output within a stated tolerance.
- **Listening**: when a change affects the sound, write the render to a WAV file under `.qa/renders/` and tell the user where it is. Never claim something "sounds right": only the user judges by ear.

## Commands and agents (any change under `crates/commands` or to the MCP and ACP bridges)

- Each new or changed command: schema, description and undo covered by tests. Add one example of the command, with the state it runs from, to `examples()` in `crates/commands/tests/contract.rs` (the catalog test there proves its description and both schemas; the harness `commands::testing::check` proves the metadata rules and the undo: apply, undo restores the state, redo), and one `check_refusal` case per refusal of the command (the state stays intact). A declared command without an example fails the tests.
- The parity test passes: every command is exposed as an MCP tool, and no MCP tool exists without its command.
- Drive the change from a real agent: connect Claude Code to the helper's MCP server and run the command, then check the result in the page and in the history (author shown, undo works).

## Web UI (any change under `web/`)

1. In `web/`: `pnpm gen:check` (regenerates `src/lib/generated/`, fails on a diff; `dev`, `build` and `test` regenerate it too), `pnpm typecheck`, `pnpm lint`, `pnpm test` (builds the wasm engine and runs it under Node), `pnpm build`, then `pnpm test:e2e` (Playwright, `web/tests/e2e/`: starts its own dev server on 5180 and a fresh build under the preview on 4180, never reuses one; cross-origin isolation, helper sockets against a stand-in helper on 47800, which must be free (`FAULK_E2E_HELPER_PORT=<port>` moves it, page included, when an installed helper holds it); `FAULK_E2E_DEV_PORT` and `FAULK_E2E_PREVIEW_PORT` move the 5180 and 4180 servers when another checkout holds them; the e2e build sets `VITE_FAULK_DEV_ROUTES=1`, which makes the test surfaces under `/dev` (the time grid's, AR-3) show their content, empty in a production build), simulated Web MIDI, the landing page). `FAULK_E2E_CHANNEL=chrome` or `msedge` runs it in the installed Chrome or Edge; the first run needs `pnpm exec playwright install chromium`.
2. Start the dev server (`pnpm dev`, port 5173): the landing page is at `/` (`/en`, `/fr`) and the DAW at `/app`. Check the landing page with Playwright at 1440 and 390 px in both languages (no horizontal scroll, keyboard focus, no request to another host, no JavaScript loaded besides the lazily loaded animation chunk `landing/anim.*.js`, no wasm). The sound objects' animation (`web/src/lib/landing/anim/`): six `.is-live` stages, no console error, a still frame under `prefers-reduced-motion: reduce`, no rendering while off screen or in a hidden tab, axe 0 violation, and frame times measured while hovering the hero and the library (`?fk-anim-t=<s>&fk-anim-p=<x>,<y>` gives a reproducible frame for screenshots). Open the DAW (`/app`) with Playwright. Without a real MIDI device, inject a fake `navigator.requestMIDIAccess` with `page.addInitScript` to exercise detection, hot-plug and notes; start the audio context (a user gesture is required), exercise every acceptance criterion and the screens the change touches, read the console (no error) and the screenshots.
3. Explore beyond the scripted flow: double clicks, reload during playback, helper connected then disconnected, tab in the background.

### Timing budgets of the e2e specs

`engine-mo2.spec.ts` (the shipped worklet) measures the cost of the engine channel on the render thread of a real AudioWorklet: 2 µs per pumped message and 500 µs for the longest pump (the swap of a table at its limit), budgets of CT-3 and MO-2. `engine-channel.spec.ts` is the CT-3 prototype, kept as the evidence of the chosen way (and of way (a), which was not shipped); it holds ceilings only, in every mode (10 times 2 µs and 500 µs, and 500 µs per message for a burst of way (a), whose own bound is 50 µs). Every number is printed, with the mode (`timing: ceiling…` or `timing: STRICT…`); in strict mode each of the five runs is printed and checked, then the asserted medians.

- **CI and default**: the assertions hold a ceiling ten times the budget (20 µs per message, 5 000 µs for the longest pump). A shared runner is slower and noisier than a workstation (in the prototype spec (way (b), which moves 8 192 slots and a million notes in a pump) the longest pump measured 512 µs in CI run 38705342790 of `develop`, 1 049 µs in the report of #220, and up to 1 201 µs in 20 local runs on a Windows workstation that was also running other work; the shipped worklet (`engine-mo2`) measured 15 µs median and 169 µs at most over 40 such runs, and once 2.40 µs per message against a budget of 2: one preempted quantum is enough), so the ceiling only catches a gross regression (about 65 times the measured per-message cost of the shipped worklet, 280 times its longest pump), not an overrun of the budget: that is the strict run's job. The 500 µs swap of 8 192 slots and a million notes (prototype, way (b)) is held by the ceiling only; the strict run measures the shipped worklet's 2 048-entry swap.
- **Strict, on a reference machine**: `FAULK_E2E_STRICT_TIMING=1` runs the shipped measurement five times and holds the budgets themselves on the **median**. A reference machine is a workstation with nothing else running (no build, no other browser), named with its CPU and OS in the pull request; the strict run is never done in CI. Any value other than `1` or `0` is an error.

  ```bash
  cd web && FAULK_E2E_STRICT_TIMING=1 pnpm exec playwright test engine-mo2 --project=dev
  ```

  PowerShell (the variable stays set in the session, remove it afterwards): `cd web; $env:FAULK_E2E_STRICT_TIMING='1'; pnpm exec playwright test engine-mo2 --project=dev; Remove-Item Env:FAULK_E2E_STRICT_TIMING`.

Run it before a release and after any change to `crates/engine`, `crates/engine-wasm` or `web/src/lib/engine/worklet.ts`; report the printed numbers and the machine in the pull request.

`timegrid.spec.ts` (AR-3) scrolls and zooms the shared time grid over 200 bars for 360 frames and prints the frame interval (mean, p95, worst, share over 20 ms) and the cost of each canvas draw (mean, p95, worst). Its default ceilings (mean frame under 25 ms, draw p95 under 12 ms) only catch a gross regression; `FAULK_E2E_STRICT_TIMING=1` holds 60 images per second (mean under 17.5 ms, under 5 % of frames over 20 ms, draw p95 under 4 ms) on one sweep, not on a median of five. A headless frame interval is locked to the display's refresh, so the draw cost is the figure that shows the headroom. Run it strict, on a reference machine, after any change to `web/src/lib/components/timegrid/`, and name the machine in the pull request.

The e2e build sets `VITE_FAULK_DEV_ROUTES=1`: the `preview` project is the production build plus that flag, and the `dist` folder an e2e run leaves behind has the test surfaces switched on.

## Helper packaging and updates (changes to `crates/helper/src/updater.rs`, `release-helper.yml`, `web/vercel.json`, `scripts/helper-release/`, the tray's update entries)

- `cargo test -p helper updater` covers channels, feed filtering, URL building, error classification against a loopback server, the update thread on a fake backend, the `get.faulk.app` redirects read from `web/vercel.json` (one public Blob origin, each helper URL to its store path, the download address cited by the page texts), and `cargo test -p helper tray_text` the tray's update entries for every `UpdateStatus`.
- `node --test "scripts/helper-release/*.test.mjs"` covers the Blob publication without network or token: write order (packages, then feed last), immutable packages, the channel guard, the store origin check, the delta base, and the helper URLs resolved through `web/vercel.json` to the paths the script writes.
- Dry run on a real Velopack output: `node scripts/helper-release/publish.mjs publish --dir <vpk output> --os win --version <X.Y.Z> --dry-run` (every trial build of the workflow runs it).
- After changing `web/vercel.json`, compile it with Vercel's route compiler (`getTransformedRoutes` of `@vercel/routing-utils`, in a scratch folder) and resolve the helper URLs through the compiled routes: the tests above translate the source syntax themselves.
- Lint the workflow with `actionlint` (with `shellcheck` available, so the `run:` scripts are checked too).
- A real upload is tested only on request, with a pre-release dispatched from `develop` (`gh workflow run release-helper.yml --ref develop -f version=<X.Y.Z-beta.N>`): it reaches the beta folder of the store only.
- End to end on Windows (procedure in `docs/architecture/04-distribution-helper.md` > Éprouver la chaîne en local): pack two versions with `vpk` (same version as the `velopack` crate), install the older one, serve the feed on loopback, run the installed helper with `FAULK_UPDATE_URL=http://127.0.0.1:<port>`, check the update is downloaded, applied and the helper restarted, and not applied while another helper runs; uninstall afterwards (folder, shortcuts and a "start with Windows" value pointing at the installed helper removed).

## Latency (changes to audio I/O, the worklet, the helper or the protocol)

Measure the output latency (and the round trip when input is involved) in both modes, on the user's machine, and report the numbers with how they were measured. Never state a latency you did not measure.

## Agent scripts and hooks (any change under `scripts/` or `.claude/`)

Run `node --test "scripts/**/*.test.mjs"` (CI runs it too), then try the changed hook or script by hand.

## CI (GitHub Actions, `.github/workflows/`)

On a pull request, each job of `rust.yml` (`rust`, `helper-windows`), `ci.yml` (`web`) and `agent-scripts.yml` always starts, so its check name always reports, but its heavy steps run only when a changed file can affect it. A first step, `Changed files`, lists `git diff --name-only --no-renames HEAD^1 HEAD` (the merge commit of the pull request against its base) and matches it with the job's `PATTERN`. Pushes to `develop` and `main` run everything. A pattern that misses a path the job reads lets a broken change through green: add the path to the `PATTERN` of the job in the same change that makes the job read it.

| Job | Heavy steps run when the pull request changes |
|---|---|
| `rust` | `crates/`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/`, `web/src/lib/engine/proto/crate/` (the prototype crate it lints), `web/vercel.json` and `web/src/lib/i18n/helper.ts` (read by the helper's updater tests, which `cargo test --workspace` runs), `rust.yml` |
| `helper-windows` | `crates/helper/` and the crates it depends on by path (`engine`, `commands`, `protocol`, `project`), `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/`, `web/vercel.json` and `web/src/lib/i18n/helper.ts` (read by its tests), `rust.yml` |
| `web` | `web/`, `crates/` (the page embeds the engine and project WASM and the generated types), `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/`, `ci.yml` |
| `agent-scripts` | `scripts/`, `.claude/`, `web/vercel.json` (read by the release script tests), `agent-scripts.yml` |

`release-helper.yml` (Windows, macOS and Linux trial build) runs on a pull request only for the helper's updater, wiring and manifest, `scripts/helper-release/`, `web/vercel.json` and itself (not `Cargo.lock`: it changes for any dependency); it runs in full on a push to `develop` or `main` that touches `crates/helper/`, `Cargo.lock`, `scripts/helper-release/`, `web/vercel.json` or the workflow, on `helper-v*` tags and on dispatch.

After changing a workflow, lint it: `actionlint` with `shellcheck` available. To see the filter work, a pull request that changes only `docs/` shows `Changed files` ending with `heavy steps run: false` in each job, and one that touches `crates/` shows `true`.

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
  - Bash, PowerShell, Agent, Task: the `PreToolUse` hook takes the state of each checkout the session works in (its current directory and the project directory) and the `PostToolUse` hook compares after the call. A call that changed the fingerprint makes the session a writer. Agent and Task calls make the lead a writer for what its subagents change during the call.
  - A checkout first seen at the end of a call (the call moved into another worktree) has no state before the call: it gets its baseline from its state after the call, and the call makes nobody a writer there. Visiting a worktree that carries another agent's work never blocks.
  - A call that moved HEAD without authoring code (reflog: checkout, switch, pull, reset, rebase, a merge of `develop` or `main`; `gh pr checkout` is a checkout) is judged only on the uncommitted changes, which such a move carries over untouched unless the call also edited them; the session's baseline then moves to the new state, and a later block message says so. Merging `develop` into a task branch brings code reviewed in its own pull request. A call that commits (`commit (merge)` after resolving conflicts included), cherry-picks, reverts or merges any other branch is judged on the whole fingerprint: that code was not verified here.
  - A call that never gets its `PostToolUse` (denied by another hook or by the user, interrupted, failed) is dropped at the next stop: it attributes nothing.
  - A session is a writer on the branch it wrote on (or the commit, on a detached head). Once it is a writer on the current branch of a checkout, its calls there are no longer measured.
- **Leaving the branch**: at the stop, each branch the session wrote on is judged where it is, not where HEAD is: with the working tree when HEAD is still on it, with its committed content otherwise. Switching to `develop` after committing leaves the branch to verify, and the message says to switch back. A stash made since the session first wrote in a checkout holds code out of the working tree: the stop is blocked until it is popped (or dropped).
- **Session baseline**: the fingerprint the first time the hooks see the session in a checkout (`SessionStart`, a tool call, or a stop). A writer whose checkout is back to its baseline (it undid its changes) is let through.
- **Checkouts**: the hook judges every checkout the session wrote to, wherever it stops; each has its own records.
- **Deferral**: to stop mid-task only to ask the user a question, run `node scripts/agent-qa/record.mjs deferred --reason "<the question>"`. It lets exactly one stop through for the current state of the code. Once used, another one is accepted only after the user writes again (the `UserPromptSubmit` hook records when), whatever the code does meanwhile. It is kept apart (`deferral.json`) and never erases a verification. Never defer at the end of a task.
- **Failure**: fail open, loudly. If the hook cannot compute a fingerprint (git missing, a repository without a commit) or its input has no session id, it lets the turn end and shows the user a warning. Claude Code also stops honouring a Stop hook after eight blocks in a row without a tool call (hooks reference, https://code.claude.com/docs/en/hooks; not measured here).
- **Guard changes**: when `scripts/agent-qa/`, `scripts/agent-review/`, `.claude/settings.json` or `.claude/agents/` differ from the merge base with `develop`, every stop shows the user a warning: the guard that runs is the modified one.

### Cost

Measured on 2026-10-14 on the lead's Windows machine, in this repository's worktree on `feat/6-agent-guardrails`, with PowerShell: ten runs of each hook fed a fixed input (`Get-Content in.json | node scripts/...`, timed with `Measure-Command`). Bash input: `{"tool_name":"Bash","tool_input":{"command":"ls"},"tool_use_id":"t1",…}`, for a session that had not written yet, its state deleted after each round, so each `PreToolUse` run also records the session's baseline (the first call of a session). Edit input: an `Edit` of `scripts/agent-qa/stop-core.mjs` (a guarded file), ten runs in a row.

| Command | Runs (ms) |
|---|---|
| `node -e "0"` (Node start alone) | 89 79 80 82 79 87 94 91 89 90 |
| `agent-guard.mjs` (PreToolUse, Bash, first call) | 377 435 392 499 388 432 432 441 468 499 |
| `post-tool-use.mjs` (PostToolUse, Bash) | 326 301 301 291 286 294 280 359 310 318 |
| `agent-guard.mjs` (PreToolUse, Edit) | 418 235 214 213 269 218 216 206 202 197 |
| `post-tool-use.mjs` (PostToolUse, Edit) | 241 221 213 207 293 222 216 193 225 207 |
| `qa-guard.mjs` (Stop, no writer) | 284 275 276 273 288 281 334 329 311 351 |

An earlier round without the baseline (later calls of a session) gave about 300 ms for `PreToolUse` on Bash. So about 0.6 to 0.75 s per Bash, PowerShell, Agent or Task call until the session writes on the current branch, about 0.4 s per edit-tool call, and about 0.3 s per stop. Linux is not measured.

### Threat model

The guardrails exist to stop an honest agent that skips or forgets a step: ending a task without the checks, without the delivery review, or with records of an older state. They do not stop a deliberately evasive agent: one that hides a write from the hooks on purpose (an interpreter, an encoded command, a variable that expands to a path, a background process, deleting state through a path the guard does not recognise) is out of scope. Those ways around are listed below so that nobody mistakes them for protection.

### Limits

- The record holds what the lead declares: the hook checks that the fields are there, not that the checks ran.
- A change made in the very call that enters a checkout (`cd ../other && sed -i …`), or a shell write to an absolute path in another checkout, is not attributed: the hooks measure only the checkouts of the session's current directory and project directory, and that checkout has no state before the call. The next measured call there, or an edit tool on one of its files, is.
- Symbolic links are not resolved by the shell guard: a link created to point at a record or a brief (`ln -s .git/faulk-qa/review.json alias`) and then written through passes. That is deliberate evasion. `brief.mjs` never follows an untracked link, and `run.mjs` refuses to write through one.
- The records are shared by every session of a checkout: one session's deferral can be consumed by another's stop, and the last verification or review recorded wins.
- A change made by someone else during one of the session's own Bash, PowerShell, Agent or Task calls (between its two measures) is counted as the session's. Modification times cannot settle it: the change and the call overlap in time.
- Background work is not attributed: a command started with `run_in_background`, `&`, `nohup` or `Start-Process`, or a subagent run in the background, writes after its call has returned. A subagent in its own worktree writes to a checkout the lead's calls do not measure: the lead's later checkout or merge of its branch is a move, not an authoring.
- A tool outside the eight above (an MCP server that writes files) is not measured.
- Subagents' own stops are not checked (`SubagentStop` is not hooked); their changes count only through the lead's Agent or Task call, or through their own tool calls if Claude Code gives them the lead's session id (not documented).
- The guard reads command lines only. It removes quotes and escapes the way bash and PowerShell do (`faulk-q""a`, `faulk-\qa`, ``faulk-q`a``) before matching, but deliberate evasion passes: an interpreter (`node -e`, `python -c`), an encoded command (`powershell -EncodedCommand`), a variable or a substitution that expands to a path, or a file writer it does not know.
- A branch the session wrote on and then deleted cannot be judged: the stop lets it through with a warning to the user.
- Changing the guard is trusted only after a human reviews the pull request: the agent that edits `record.mjs` or a hook can also run the edited version. The Stop hook warns the user when the guard differs from `develop`; it cannot do more.
- The hooks run through Git Bash on Windows (`node "$CLAUDE_PROJECT_DIR/..."`). Without Git Bash, Claude Code runs hooks in PowerShell, where `$CLAUDE_PROJECT_DIR` is not expanded: the hooks then fail without blocking anything.

