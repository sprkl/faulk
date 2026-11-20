---
type: llm
focus: trace
weight: 1
---

Every `Mix` line gives a level (a fader value in dB), and the lead blends into the track instead of sitting on top of it: its fader is 2 to 4 dB below the bass's, never level with it; it is not dry: it has a Reverb in the middle plane's room (the same size and damping as the middle plane's Reverbs) with a `mix` of about 0.15 to 0.25 and a predelay of about 0.03 to 0.05 s, and a synced Echo with a `mix` of about 0.15 to 0.25; a square or saw lead also has a lowpass Filter between about 5 and 8 kHz; and the sheet asks for the lead check on the render (the RMS of the densest section with and without the lead, more than about 4 dB meaning the lead is too loud) or writes `to measure on the render`; a lead-check figure given without any render is a fail.
