2026-11-03
* **Update** (#486): the engine's CPU load is measured in both modes (the helper times each render chunk with `Instant`, the worklet each quantum with `Date.now()`), the report gains its peak (feedback layout 6, protocol 22), and the spec's MO-5 paragraph says how; CLAUDE.md lists the clock read as an allowed call on the audio thread.
