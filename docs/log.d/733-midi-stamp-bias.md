2026-11-05
* **Update** (#733): `specs/03-enregistrement-midi.md` (input stamps) states the absolute bias of recorded MIDI notes measured in Chromium (worst 0.52 ms over 15 runs, no median removed) thanks to the report's new `host_frame` word (Feedback version 9, protocol 41), and the limit of that measure: it is circular on `getOutputTimestamp()`.
