# Faulk

Faulk is an open and portable DAW: it opens in the browser, users build their own instruments and effects, and an optional native helper brings pro latency and third-party plugins until browsers close the gap. Never name another DAW in the repository (code, docs, issues, commits): describe what Faulk does, not what it resembles. Product vision: `docs/product/01-vision.md`. Architecture: `docs/architecture/01-architecture.md`.

## Methodology

This project follows two protocols. Both are mandatory for any agent working here.

- **Matt Pocock's engineering skills** (`mattpocock-skills` plugin, declared in `.claude/settings.json`) for issues, triage, specs, domain modeling, TDD, diagnosis and code review. Their repo-specific configuration is in "Agent skills" below and in `docs/agents/`.
- **Open Knowledge Format v0.2** for everything in `docs/` (the bundle root). When you add or change a document:
  - put it in the matching sub-folder (`product/`, `architecture/`, `constraints/`, `design/`, `specs/`, `spikes/`, `agents/`);
  - give it frontmatter with `type`, `title` and `description`, plus `status: draft` until it is checked against the code;
  - update that folder's `index.md` and add a dated entry to `docs/log.md`.

The project is worked on with Claude Code only; this file is the agents' entry point.

## Stack and layout

The repository is a monorepo; keep this section in sync with the code. `modules/` does not exist yet.

| Path | What | Tech |
|---|---|---|
| `crates/engine` | Audio engine core: graph, transport, DSP. Pure, no I/O, no allocation on the audio path. | Rust |
| `crates/commands` | Every user action as a typed command: name, input and output schema, description for agents, undo. The single source for MCP tools and TypeScript types. | Rust (`serde`, `schemars`) |
| `crates/engine-wasm` | Web build of the engine, run inside an `AudioWorklet`. Plain C ABI exports (`fk_*`), no `wasm-bindgen`: its glue does not load in an `AudioWorkletGlobalScope`. | Rust → WASM |
| `crates/helper` | Native helper ("Boost" mode), the local bridge: runs the same engine with low-latency audio I/O, MIDI and third-party plugin hosting, serves MCP, launches ACP agents, and talks to the web UI over a local WebSocket. | Rust (`cpal`, `midir`, CLAP then VST3, `rmcp`, ACP) |
| `crates/protocol` | Messages exchanged between the page and the helper, versioned (`PROTOCOL_VERSION`), and the schema bundle (commands, messages, parameter ids and defaults) the web app generates its types from. | Rust + generated TypeScript types (`web/src/lib/generated/`, `pnpm gen:protocol`) |
| `web/` | One web app for both modes: the public landing page at `/` (prerendered, light theme; its only client JavaScript is the lazily loaded WebGL animation of the sound objects, `src/lib/landing/anim/`) and the DAW at `/app` (client-only, dark theme). | TypeScript, SvelteKit 3 (adapter-static), Svelte 5, Vite, Vitest, pnpm |
| `modules/` | Faulk's own instruments and effects. | Faust (first), Rust → WASM |
| `scripts/` | Agent guardrails: Judge Dredd's brief and headless run (`agent-review/`), verification records and the hooks of `.claude/settings.json` (`agent-qa/`). The helper's publication to the Vercel Blob store behind `get.faulk.app` (`helper-release/`, used by `release-helper.yml`). | Node 24, ESM, `node --test`; no dependency except `@vercel/blob` in `helper-release/` (pinned, loaded only for a real upload) |

Core rules:

- **AI first: every action is a command.** Anything a user can do goes through `crates/commands`; the UI has no private path to the engine or the project. A feature that adds a UI action without its command, schema, description and undo is not done. MCP tools are generated from the commands, never written by hand.
- **One engine, two targets.** Any engine feature must build and behave the same in `engine-wasm` and in `helper`. Audio never crosses the UI ↔ helper link: only commands and visual feedback do.

## Real-time audio rules

Code that runs on the audio thread (the `process` path of the engine, a module or a hosted plugin wrapper) must never:

- allocate or free memory, lock a mutex, block, do I/O, log, or call into the OS;
- panic on any input: clamp, saturate or skip instead.

Communicate with the audio thread through lock-free structures (`rtrb` ring buffers, atomics).

Known exception, until the shared-memory channel of the MVP plan is built (cross-origin isolation was accepted on 2026-10-14; tasks AU-0, CT-3 and MO-2 of `docs/specs/02-decoupage-mvp.md`): in the web worklet (`web/src/lib/engine/worklet.ts`) commands arrive through `postMessage` and the meter goes back through `postMessage` about 30 times per second, which allocates small JavaScript objects on the audio thread; the wasm instance is created in the worklet's message handler, once, before any audio. Moving both to a `SharedArrayBuffer` needs cross-origin isolation (COOP/COEP headers, or Document-Isolation-Policy where it exists). Debug builds wrap the audio path with an allocation detector (e.g. `assert_no_alloc`); a test that triggers it fails. `unsafe` is allowed only with a `// SAFETY:` comment that states the invariant.

## Branches

- `develop`: default branch. Task branches start from it and come back through a pull request.
- `main`: releases only, reached through a pull request from `develop` with green CI, merged as a **merge commit**.
- Hotfix: branch from `main`, **squash** PR into `main`, then merge `main` into `develop`.
- Task branches: `feat/<issue>-<slug>`, `fix/<issue>-<slug>`, `spike/<issue>-<slug>`.

## Language

- **French** for what people read: GitHub issues, pull requests (title, description, replies to reviews), the agent's messages to the user, `README.md` and the documentation in `docs/`.
- **English** for everything else: identifiers, file names, code comments and doc comments, test names, log messages, commits, developer-facing errors, technical and internal documentation (`CLAUDE.md`, `docs/agents/`), and the frontmatter values, `index.md` and `log.md` files of `docs/`.
- The product UI is in English and French from the start (user's decision of 2026-10-13): every UI string goes through `web/src/lib/i18n/` in both languages (`app.ts` for the DAW, `landing.ts` for the landing page).

When the audience or the language of a text is unclear, ask before writing it; never deduce.

## Links in documentation

- Links are relative and must stay valid: when a file moves or is renamed, update every link to it.
- From outside `docs/` (`README.md`, root files), link to the `docs/` folder, never to a file inside it.
- Inside `docs/`, the `index.md` files and the links between documents follow the Open Knowledge Format.

## Source of truth

The code is always right; documentation, `docs/` included, may be stale. When a doc disagrees with the code, follow the code and point out the stale doc.

## Working on an issue

An issue is carried through autonomously, from plan to a reviewed pull request, without waiting for the user's go-ahead between steps. The detailed steps are in `docs/agents/issue-tracker.md` > Working on a task.

1. **Plan**: read the issue, its spec and the code, write the plan, have Judge Dredd review it, and settle its findings.
2. **Implement** on the task's own branch from `develop`, with the checks of the Definition of done below, then Judge Dredd's delivery review.
3. **Pull request**: commit, push the branch and open the pull request against `develop`. Pushing the task branch and opening its pull request need no approval.
4. **Copilot review**, when Copilot code review is enabled on the repository: request it, settle each finding (fix with a test, or reply with the reason it is rejected), until a round brings nothing to fix (three rounds at most).
5. **Merge** when the pull request is ready (see below), with a merge commit.
6. **Close the loop**: update the issue, then send the final message.

Stop for the user only on a decision that is theirs: product behaviour, sound or scope that the issue, the spec and the code do not settle, or a destructive or irreversible action. The lead merges the pull request into `develop` itself once it is ready (founder's decision of 2026-10-13): CI green, Judge Dredd and Copilot findings settled, and nothing left that is the founder's to decide or validate (product, sound or scope choices, user-facing text). A pull request that carries such a decision waits for the founder. Releases to `main` stay with the founder.

## Definition of done

Checks depend on what the task touches (`docs/agents/verification.md` gives the commands):

- **Rust**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and the WASM build of the engine.
- **Engine and modules**: offline render tests that assert on the signal (level, spectrum, silence, no NaN/denormals), the allocation detector on the audio path, and parity between the native and WASM builds on the same render.
- **Web**: typecheck, lint, tests, then a check in a real browser with Playwright (audio context started, no console error, the changed screens exercised).
- **Commands**: a new or changed command has its schema, description and undo tested, and the test that compares commands with MCP tools passes.
- **Latency-sensitive changes**: measure and report the latency in both modes; never claim a latency you did not measure.

Judge Dredd, an impartial read-only reviewer subagent, judges the work twice per task: the plan before starting, and the delivery once, as the last step before opening the pull request (`docs/agents/judge-dredd.md`). Briefs are built by `scripts/agent-review/brief.mjs` and the delivery review runs through `scripts/agent-review/run.mjs`. Every one of its findings is shown to the user with the lead's opinion; none is dropped silently.

A `Stop` hook enforces this: a turn in which the session's own tool calls changed the guarded code (`crates/`, `web/`, `modules/`, `scripts/`…) ends only once the checks are recorded for its current state (`node scripts/agent-qa/record.mjs verified …`, `docs/agents/verification.md`) and the branch's delivery review is recorded (`node scripts/agent-qa/record.mjs reviewed …`). To stop mid-task only to ask the user a question, `node scripts/agent-qa/record.mjs deferred --reason "<question>"` lets one stop through per user message. A session whose own tool calls change no guarded file is not blocked, except in the cases listed in `docs/agents/verification.md` > Limits (another writer's edit during one of its shell calls is counted as its own). The guardrails stop an honest agent that forgets a step, not a deliberately evasive one.

Before reporting a task as done:

- every general claim ("all", "every", "each", "none") in the report, the pull request or a commit is proven by an enumeration, never stated from memory;
- every mechanism introduced by the task (guard, fallback, cache, retry, lock-free queue, reconnection) gets an attack pass: how it fails or is bypassed, and a test of each state transition it meets.

The final message to the user says what was verified and how, what Judge Dredd found and what became of it, and which out-of-scope defects were found.

## Agent skills

### Issue tracker

Issues live in GitHub Issues (`sprkl/faulk`), written in French, structured as epics, tasks (sub-issues) and phases (milestones). See `docs/agents/issue-tracker.md`.

### Triage labels

The five default labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root, created lazily. See `docs/agents/domain.md`.
