# Mix placement

Where each role sits: left to right (pan), front to back (depth), narrow to wide (width) and low to high (frequency slot). Decide it in phase 5, write it on the `Mix` line of the finishing notes, and build it with what Faulk has today. The values below are Faulk's own parameter names and ranges; the pan is written as the track's pan control shows it (`C`, `L30`, `R100`).

## What Faulk has, and what it does not

- **Track pan**, −1 to +1 (shown `L100` … `C` … `R100`), one per track, after the track's devices. It is a balance: the centre is unity on both sides, and panning lowers the far side only (−3 dB at `L50`/`R50`, silence at `L100`/`R100`). A hard pan therefore also removes the far side of the track's reverb and echo tail. Automated with `param=pan` (CC 10, 64 is `C`).
- **Reverb** (`size` 0–1: about 0.3 s to 7.5 s; `damping` 0–1: bright to dark tail; `predelay` 0–0.2 s; `mix` 0–1). It sums its input to mono, then returns a wide tail.
- **Echo** (`mode` free or synced, `division` 1/32 to a whole note with dotted and triplet values, `feedback` 0–0.95, `mix` 0–1). Each side repeats itself: the echoes sit where the dry sound sits.
- **Filter** (`mode` lowpass, highpass, bandpass, notch; `cutoff` 20 Hz–20 kHz; `resonance`; `mix`). One band per instance; two Filters in a chain cut both ends.
- **Bloom** (`mode` chorus, ensemble, vibrato; `stereo_phase` 0–180: 0 adds no width, 180 is the widest; `depth` 0–100 %; `tone` 20 Hz–2 kHz, a high-pass on the moving copies; `mix` 0–1). The only width tool.
- The **Drums** instrument and the **Oscillator** are mono (the same signal on both sides), and one Drums track plays all its pieces at one pan: to offset the hats or the percussion, give each its own Drums track.

Not in Faulk yet, so never worked around: sends and returns (left out of the MVP: decision 5 of `docs/specs/02-decoupage-mvp.md`; groups and buses are #193), so each Reverb or Echo is inserted on the track that needs it, its `mix` acting as the send amount; Contour, the equaliser (#751, in progress), so frequency slots are made by register and Filter only; Balance, the stereo utility (#162), so there is no mono switch, no width narrowing and no phase flip; signal analysis (#201), so no spectrum, correlation or loudness reading: slots and width are judged by ear.

## Pan

| Role | Pan |
|---|---|
| kick, bass, snare or clap, lead | `C` |
| chords, pad | `C`, widened by Bloom; or a doubled pair |
| doubled part (two tracks) | mirrored, `L40`/`R40` to `L70`/`R70` |
| hats, shaker | `L15`–`L35` (own Drums track) |
| percussion, second hat | the opposite side of the hats, `R20`–`R40` (own Drums track too) |
| counter-melody, arpeggio | `L30`–`L60`, on the side away from the busiest percussion |
| texture, noise bed | wide (Bloom, or a mirrored pair `L80`/`R80`) |

A mirrored pair must carry two different signals (another octave, another waveform, a Stack voice, Bloom on one side only): the same mono signal at `L50` and `R50` is one centred part. Keep the sides in balance across the track: what goes left in one section is answered right, not stacked left.

## Depth

Three planes, chosen per role; parts on one plane share a room (the same `size` and `damping` on each of their Reverbs, only `mix` and `predelay` differ).

| Plane | Roles | Reverb | Echo | Filter |
|---|---|---|---|---|
| front | kick, bass, lead, snare | none, or `mix` ≤ 0.1, `predelay` 0.02–0.04 | none, or `mix` ≤ 0.15 | open |
| middle | chords, counter-melody, percussion | `mix` 0.15–0.3, `size` 0.3–0.5 | synced `eighth_dotted` or `quarter`, `feedback` 0.2–0.35, `mix` 0.1–0.25 | lowpass above 8 kHz or none |
| back | pad, texture, a far echo of the lead | `mix` 0.35–0.6, `size` 0.6–0.9, `damping` 0.6–0.8 | `feedback` 0.4–0.6, `mix` 0.2–0.35 | lowpass 2–5 kHz |

Farther is wetter, darker and quieter: move a part back with all three, not with the reverb alone. A `predelay` of 20–40 ms keeps a front part's attack dry while it still has a room. Never put the bass or the kick on the back plane.

## Width

- No Bloom on the kick or the bass, nor on anything that carries energy under 150 Hz, unless its `tone` is at 150 Hz or more (the low end stays the same on both sides).
- Chords and pad: `ensemble`, `stereo_phase` 120–180, `depth` 30–50 %, `tone` 200–400 Hz, `mix` 0.4–0.5.
- Lead: none, or `chorus` with `stereo_phase` 90 and `mix` 0.2–0.3, so it stays a point in the centre.
- Hats, texture: `chorus`, `stereo_phase` 180, `mix` 0.3–0.5.
- One width source per role. Width is a contrast: keep one section narrower than the next (Bloom `mix` automated, or a mirrored pair entering) so the wide one opens.

## Frequency slots

Until Contour lands, the register is the equaliser: write each role in its band, and cut what spills with a Filter.

| Role | Slot | How, today |
|---|---|---|
| kick | 40–100 Hz body, click above 2 kHz | Drums `kick_tune` on the key's root or fifth; short `kick_decay` when the bass owns the sub |
| bass | 40–250 Hz | notes from E1 to E3; Filter lowpass 800 Hz–2 kHz to keep it out of the chords |
| chords, pad | 200 Hz–4 kHz | voicings above C3; Filter highpass 150–300 Hz |
| lead | 500 Hz–5 kHz | C4 to C6, above the chords' top note; nothing else in its octave while it plays |
| counter-melody | 300 Hz–3 kHz | an octave away from the lead, or in its rests |
| hats, percussion | above 5 kHz | Filter highpass 4–8 kHz on their own track |
| texture | above 300 Hz, or one narrow band | Filter highpass, or bandpass on a quiet layer |

Kick and bass agree on who owns the sub: either the kick is short (`kick_decay` low) and the bass holds the sub, or the bass goes through a Filter highpass at 60–80 Hz and the kick holds it.

## Mono check

Faulk cannot sum to mono yet (Balance, #162), so check on paper, then once by ear:

1. kick, bass and lead are at `C`, and nothing under 150 Hz has Bloom on it (or its `tone` is 150 Hz or more);
2. each mirrored pair carries two different signals, and the music still holds with either side muted (mute each in turn);
3. where Bloom's `stereo_phase` is 120 or more, its `mix` is 0.5 or less so the dry sound carries the part; switch each Bloom off once and the part must still hold;
4. no part is hard-panned with its only reverb on it (its far tail would vanish).

The sheet says which points were checked by reading and which are left to the ear.

## The sheet line

In the finishing notes, one `Mix` line per role, in this order: role, pan, depth plane with its Reverb/Echo/Filter values, width, frequency slot. Then one line for the mono check.

```
Mix: Bass: C, front (no Reverb, Filter lowpass 1.2 kHz), no width, 40–200 Hz
Mix: Pad: C, back (Reverb size 0.8 damping 0.7 mix 0.45, Filter lowpass 3 kHz), Bloom ensemble stereo_phase 150 mix 0.45 tone 300 Hz, 250 Hz–4 kHz
Mix: Hats: L25 (own Drums track), middle (Reverb size 0.4 mix 0.2), Bloom chorus stereo_phase 180 mix 0.3, above 6 kHz (Filter highpass 6 kHz)
Mono check: 1, 3 and 4 by reading; 2 left to the ear
```
