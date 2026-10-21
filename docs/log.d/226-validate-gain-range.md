2026-10-21
* **Update** (#226): `validate()` now refuses a track or master gain that is finite and outside `MIN_GAIN_DB`, `MAX_GAIN_DB` (-96..+6 dB), with the path of the mixer, as `GainOutOfRange`; silence (`null`) stays valid. The two bounds moved from `commands::mixer` to `project` and are re-exported there, so the model and the commands share one definition.
