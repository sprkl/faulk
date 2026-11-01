# Product

<!-- impeccable:product-schema 1 -->

Faulk's durable product record for design work (Impeccable). English, agent-facing. The product vision in French lives in `docs/product/`; when the two disagree, the code wins, then `docs/product/01-vision.md`, and this file is updated.

## Platform

web

## Users

- **Primary: experienced music producers and beatmakers** (founder's decision, 2026-11-01). They already know how a DAW works: clips, arrangement, device chains, mixer, piano roll, automation. They expect density, keyboard shortcuts, speed, and the conventions their hands already know. The app is optimised for them first.
- The founder is the first of them: a tool of his own to produce his music.
- Later: a community that builds and shares its own instruments and effects (modules).
- Second operator: **AI agents** acting on the same project through the same commands (MCP today, ACP agents in the Atelier later). They are not a visual audience, but every UI decision must leave room for them to act alongside the human.

## Product Purpose

Faulk is an open, portable DAW that opens in a browser tab, where users build their own instruments and effects. It exists because today's DAWs are excellent but closed and tied to one machine. Success: a producer finishes real tracks in Faulk over long sessions, in the browser or with Boost, and an agent can do anything the producer can.

## Positioning

- **Open DAW for humans and agents** (tagline, 2026-11-01). Every user action is a typed command with a schema, a description and an undo; the UI has no private path, so an agent can do everything a human can, and MCP tools are generated from the same commands.
- **Opens in the browser, no install**; an optional native helper, **Boost**, brings low-latency audio I/O and direct MIDI with the same engine and the same interface. Going to Boost is a visible state change, not another application.
- **Build your own sounds**: instruments and effects are modules (Faust or Rust compiled to WebAssembly) that run in both modes; the Atelier (planned) creates them from a description.
- Open source under MIT; the name "Faulk" and its logo stay Sparkle's (`TRADEMARKS.md`).

## Operating Context

- Long production sessions on **large screens** (founder, 2026-11-01: professionals work on big desktop monitors, not tablets; design for 1440 px wide and up, laptop as the floor, no tablet or phone target for the app), **mouse and keyboard**; often a **MIDI controller** (keyboard, pads) in front of the screen, so the UI is read at arm's length while playing.
- Two engine modes at any time: **Web** (WebAssembly in the browser, about 20 to 60 ms of latency, estimated and shown) and **Boost** (native helper, low latency, measured). The current mode, latency and load are always visible.
- Projects live in a real folder the user picks (Chrome, Edge), otherwise in the browser with archive export; drafts autosave.
- The UI ships in **English and French**; device names (instruments, effects) are never translated, their parameter labels are.
- Agents act on the open project while the human watches: their actions arrive in the same history.

## Capabilities and Constraints

Shipped today (develop, 2026-11-01):
- MIDI tracks with built-in devices: Oscillator synth, Drums, Filter, Echo (delay), Reverb, Compressor; a master chain with an output limiter.
- Views: Arrangement (timeline with automation lanes, insert and delete bars), Grid (clip launching by scene), Mixer (strips, meters, pan), Instrument; bottom panel with Devices, Piano roll and History.
- Transport with tempo, meter, loop, metronome, follow, launch quantisation; undo and redo of every command; named versions are planned ("Version" = a named snapshot, "Variante" = a parallel exploration).
- Boost helper for Windows (download at `get.faulk.app/helper/windows`); protocol versioned between page and helper.

Not shipped yet (do not show as available): third-party plugin hosting (VST3, CLAP), the Atelier (module creation by description), audio recording, sends and returns, a command palette, the module library.

Constraints:
- Every new UI action ships with its command, schema, description and undo; no UI-only path to the engine or project.
- The audio thread never waits on the UI; visual feedback is read from the engine's report, never pushed per sample.
- No visible ceiling on tracks, devices, scenes or notes; only a very high safety limit.
- The repository never names another DAW: describe what Faulk does, not what it resembles.

## Brand Commitments

- Name **Faulk**, pronounced like "folk"; domain `faulk.app`. Tagline: "Open DAW for humans and agents."; French: "Le DAW ouvert, pour les humains et les agents."
- Boost is named as such: "Browser + Boost (native low latency)".
- Device names are one English name each, shown as is in both languages.
- The visual reference is the **Faulk DS-0.1** canvas (founder's decision, 2026-10-13; tokens copied to `web/src/lib/styles/tokens.css`), summarised in `docs/design/01-brief-design-system.md`.
- Voice: plain, precise, no hype; UI copy is checked against overclaiming (only what the code does).

## Evidence on Hand

- Tracks produced in Faulk by the product-owner agent runs (e.g. `.qa/po-run-2026-11-01/maree-basse.mp3`), with screenshots of real sessions in `.qa/po-run-*/shots/`.
- No external users, testimonials, benchmarks or press yet: never fabricate them. Latency figures are shown only when measured (Boost) or labelled as estimates (Web).

## Product Principles

1. **Every action is a command, and agent actions are visible.** What an agent does appears live, attributed to the agent (history, highlight), and undoable like a human action (founder's decision, 2026-11-01).
2. **Expert hands first.** Respect the conventions experienced producers rely on (shortcuts, direct manipulation, density); reach depth through the keyboard and progressive disclosure, never by hiding core functions.
3. **Sound before interface.** Waveforms, meters, clips and the playhead are the heroes; chrome recedes during playback.
4. **The engine's state is always visible.** Mode (Web or Boost), latency and load, in one permanent, understandable indicator.
5. **One interface, two modes.** Web and Boost run the same engine behind the same UI; nothing forks the experience.

## Accessibility & Inclusion

- WCAG AA contrast, including in the dark theme and on meters and clips.
- Fully operable from the keyboard, with visible focus; every control has an accessible name for screen readers (founder's decision, 2026-11-01).
- No information carried by colour alone: each state (play, record, solo, mute, Boost, clipping) also has a glyph or text.
- Legible at arm's length while playing a MIDI controller: minimum text size and hit targets suited to long sessions.
