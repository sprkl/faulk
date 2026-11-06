2026-11-06
* **Update** (#219): a MIDI track's chain holds MIDI effects before its instrument (role `midi_effect`, folder format 6, `validate()` keeps the order MIDI effects, instrument, audio effects); `docs/architecture/02-modele-projet.md` says so, and the Workshop spec's note event carries `tune_cents` (20 bytes).
