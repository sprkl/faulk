# Spikes

* [S1 — Moteur hybride web et natif](S1-moteur-hybride.md) - First spike: prove that one Rust engine runs both in an AudioWorklet and in a native helper with ASIO, that one command layer drives it from the UI, an MCP agent and an ACP agent, and measure the latency of each mode.
* [QA-7 — Banc de charge du format de projet](QA-7-banc-de-charge-format-projet.md) - Load bench of the project folder format: a deterministic synthetic project of 100 tracks, 200 000 notes and 6 million automation points, measured on open, save, memory, engine structure swaps and git, natively and in the browser, with the proposed format thresholds and CT-3's safety limits confronted with the numbers.
