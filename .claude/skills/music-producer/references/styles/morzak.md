# morzak

Dark, warm, nocturnal instrumental electronic music, made on hardware (FM synth, small analog bass synth, analog drum machine) and often played live in one take. The colour comes from modes rather than from keys, the low end carries the track, and the music lives on short chord cycles held for minutes, over which arpeggios and singing melodies develop. The founder's own style under their artist name Morzak, distilled from eight of their tracks (2020 to 2023) through audio descriptors (tempo, chroma, spectrum, energy curves), not from transcription.

## Catalog

- **Tempo**: two families. A mid groove at 112–123 bpm, and a slow or half-time family at 70–90 bpm (read 70, 80 and 89 bpm as possibly half of 140, 160 and 178). One fast exception at about 152.
- **Mode and harmony**:
  - minor almost always, with F minor as the home key three times out of eight;
  - cycles of 2 to 4 chords, one or two bars each, each held for a section (the measured tracks hold one for the whole track; the evolving style alternates two, see Where the style is going);
  - dorian colour: a major IV in minor, or ii minor;
  - aeolian colour: a minor v, then ♭VI and ♭VII as passing chords;
  - now and then a major V in minor (harmonic minor) for a darker pull back to i.
- **Sound** (from the producer's notes, not measured): one FM voice for leads and plucks, an FM arpeggio as a role in its own right, a mono analog bass, an analog drum machine, few layers (five or six roles), little reverb on the low end, and synth sounds kept rather plain; the bass still dominates.
- **Rhythm**:
  - mostly straight 16ths;
  - some tracks swing the offbeats: they land at about 0.6 of the beat instead of 0.5;
  - onset density moderate (2.2 to 5.6 percussive onsets per second), never busy.
- **Melody** (confirmed by the producer, not measured): cascading FM arpeggios, and melodic lines that develop, sing and carry the emotion over the short chord cycle. The cycle stays short; the melody and the arpeggios do the travelling.
- **Form**: two shapes.
  - Density plateau, in the older work: everything present within the first 10 seconds, then 4 to 7 minutes of micro-variation. It is hypnotic.
  - Arc, in the recent work: a rising intro, a break around the middle, the densest moment late (70–95 %), then a very short ending.
  - Tracks run 3½ to 7 minutes. Favourite forms in [forms.md](../forms.md): F3 single long crescendo (closest to the arc, its peak late) and F6 loop with one interruption (closest to the plateau), and the two-peak shape under Where the style is going; the rotation rule there applies, so take one only when the two previous runs did not use it.
- **Mix**:
  - bass and sub dominate: 58–89 % of the energy below 250 Hz (median about 73 %);
  - presence (2–6 kHz) and air together stay under 5 %;
  - crest factor 11–16 dB: dynamic, not squashed;
  - the level varies a lot from track to track.

## Signature

- **A chord that flips its third**: the same root heard major, then minor (or the reverse) inside the cycle, for example F to Fm, F♯ to F♯m, or A to Am. Put it in every profile track at least once, on purpose.
- A dorian major IV, or a modal ♭VII, in place of a functional V.

## Moves to try (prescriptions, not measurements: they keep the character and fix its blind spots)

1. **An element that lives at 3–8 kHz all the way through** (closed hat, filtered noise, a bright harmonic in the lead). Keep it quiet, but don't let it drop out.
2. **Make the third-flip a unique event**: hold the cycle in one quality, then flip the chord once, at the densest moment.
3. **An ending with time in it**: the measured endings stop within seconds; take a dissolution of 8 to 16 bars, or a false end (F9), whatever the form.
4. **One contrasting B section** of 4 or 8 bars on ♭VI or the dorian IV, heard once, to break the cycle.
5. **Swing the offbeats** of one part only (hat or bass) while the rest stays straight.
6. **An arpeggio that evolves section by section**: in register, in density (8ths, then 16ths, then triplets) and in filter opening. The check's remark on a repeated highest note is expected on an arpeggio: keep the figure.
7. **Arpeggio and lead interlock**: a counter-melody answers the arpeggio in its gaps; optionally a polymetric arpeggio (a 3-step figure over 4/4).
8. **The cycle stays, the timbre moves**: at least two automation lanes (filter cutoff, resonance, FM index), so that builds come from the sound, not only from added layers. Write each lane as an `automation` block ([notation.md](../notation.md)): target part, parameter, points by bar and beat, curve.
9. **The melody develops like the rest**: stated, varied, extended at the peak, thinned in the exit.

## Where the style is going

The founder wants the style to evolve (decision of 2026-11-08) and asked for these moves, distilled from the analysis of two live hardware jams by another producer (counted, not named): stem separation, beat and chord recognition, transcription. They are moves of this profile like the ones above: of the two moves a profile track takes as constraints (README → How to apply it), make at least one come from this list.

1. **Two cycles that alternate by section over a constant groove.** Cycle A inside the mode (i–iv, or I–vi); cycle B with a borrowed colour: a ♭II and a ♭VI around a major tonic, or the tonic chord turning from major to minor inside the cycle (I, i, ♭VI, ♭VII). Sections of 16 to 40 bars swap the cycles while the energy stays on a plateau: the sections come from harmony, not from parts.
2. **A richer, faster grid for the last peak only.** The cycle that held the track (often two chords) opens into a longer one whose bass steps down one degree per bar (vi, V, IV, iii, I in major; i, ♭VII, ♭VI, v in minor). It is what makes the last peak bigger than the first.
3. **Two peaks with a long valley**: rise, a first peak around 20–30 %, a valley of a third of the track at 35–50 % of the peak's energy, a second peak around 55–75 % on the richer grid, then a slow descent.
4. **Peaks made by playing, not by stacking**: the filter opening, the effect sends thrown and pulled back, and the chord change do the lifting; the number of parts may stay the same.
5. **Swing on the fast tempo family**: offbeats at about 0.6 of the beat, on the drums and the bass together.

## Avoid

- A major key as home, or a functional ii–V–I.
- Bright, wide, heavily layered pads that bury the bass.
- Busy drums: fills every four bars, or a rolling hat at 32nds.
- Long chord progressions (more than four chords per cycle), except the richer grid of the last peak (Where the style is going, 2).
- Loud, squashed masters.
- A melody of a few notes repeated unchanged from start to end.
- A track whose only movement is adding and removing layers.

## Confidence

- **Measured**: the tempo family, the key and mode family, the band balance, the energy curves and the form, the swing proxy, the dynamics.
- **Estimated**: the chords and their cycle length (chroma templates can confuse relative chords) and the half-time readings.
- **Confirmed by the producer, not measured**: the melody line (developing, singing lines and cascading FM arpeggios) and the arpeggio role.
- **Not measured**: sound design details, effects.
- **Measured on the two jams behind Where the style is going** (estimates: separated stems and chord recognition, about four chords in five right): the two alternating cycles and their colours, the plateau at 80–86 % energy, the richer descending grid of the last peak, the two-peak energy curve with its valley, the offbeat at 0.61 on the fast jam. **Chosen by the lead**: the 16–40-bar section lengths, the peak and valley percentages as ranges, the minor-key version of the descending grid, "peaks made by playing" (from the jams' energy without added parts, inferred, not measured part by part).
- **Chosen, not measured**: the figures under Moves to try and Avoid (the 3–8 kHz element, the 8–16-bar ending, the 4- or 8-bar B section, the four-chord limit, the fill and hat figures, the arpeggio densities, the 3-step polymetric figure, the two automation lanes), and the defaults set for the producer: five or six roles.
