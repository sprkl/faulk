---
type: Agent Configuration
title: "Verification before ending a task"
description: "Checks required before a task is done, by area: Rust, engine and modules, commands and agents, web UI, latency."
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

1. In `web/`: `pnpm gen:protocol` (no diff), `pnpm typecheck`, `pnpm lint`, `pnpm test` (builds the wasm engine and runs it under Node), `pnpm build`.
2. Start the dev server (`pnpm dev`, port 5173) and open the app with Playwright. Without a real MIDI device, inject a fake `navigator.requestMIDIAccess` with `page.addInitScript` to exercise detection, hot-plug and notes; start the audio context (a user gesture is required), exercise every acceptance criterion and the screens the change touches, read the console (no error) and the screenshots.
3. Explore beyond the scripted flow: double clicks, reload during playback, helper connected then disconnected, tab in the background.

## Latency (changes to audio I/O, the worklet, the helper or the protocol)

Measure the output latency (and the round trip when input is involved) in both modes, on the user's machine, and report the numbers with how they were measured. Never state a latency you did not measure.
