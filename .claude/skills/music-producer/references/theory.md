# Theory for producers

The minimum needed to make decisions that hold up: pulse and grid, scales and keys, chords and progressions, and the roles of bass and melody. Terms used here appear in the production sheet and in the tool's reports.

## Pulse, tempo, grid

- The **beat** is the regular pulse; a **bar** groups four of them. The sheet's grid divides a beat into 4 sixteenths (grid=16 per bar) or 2 eighths (grid=8).
- **Strong and weak**: beats 1 and 3 are strong, 2 and 4 weak; within a beat, the odd subdivisions are stronger than the even ones. Accents on weak positions are **syncopation** and **offbeats** (the "and" of each beat).
- A steady **kick on every beat** ("four on the floor") makes the pulse impossible to lose, whatever the other parts do. A **backbeat** is clap or snare on 2 and 4. **Offbeat chords** on every "and" (the reggae and dub skank) push against the pulse.
- **Tempo** sets the genre as much as the sounds do. Typical ranges (bpm): dub 60–90; hip-hop 60–100 (often felt in half time, with a backbeat on 3); house 115–130; techno and trance 120–140; dubstep 135–145 (half-time snare on 3); drum and bass 160–180 (snares on 2 and 4 at speed). The same pattern at 90 and at 170 is two different musics.
- Sections are built in **multiples of 4 bars**: 4, 8, 16, 32. Listeners count in fours without knowing it; an odd length is heard as an event.

## Pitch, scales, keys

- Twelve pitch classes per octave; sharps and flats name the same black keys (`C#` = `Db`). MIDI numbering: C4 = 60, each octave adds 12.
- A **scale** is a palette: an ordered subset of the twelve notes chosen for how they relate. The **tonic** is the home note; the **key** is the tonic plus the scale. Music using all twelve notes without a tonic sounds "wrong" or "floating" to ears raised on seven-note subsets; that is a usable effect, not a mistake.
- **Major**: whole-whole-half-whole-whole-whole-half (C D E F G A B). Bright. **Natural minor**: whole-half-whole-whole-half-whole-whole (C D Eb F G Ab Bb). Dark. Spelling rule: each letter once, in order, which is why some keys need `E#` or `Cb`.
- **Modes** rotate the same seven notes to a new tonic, and each rotation has a colour:

| Mode | From C major | Steps (W/H) | Colour |
|---|---|---|---|
| Ionian (major) | C | W W H W W W H | bright, settled |
| Dorian | D | W H W W W H W | minor with a raised 6th; the house and funk minor |
| Phrygian | E | H W W W H W W | minor with a flat 2nd; dark, Spanish, tense |
| Lydian | F | W W W H W W H | major with a raised 4th; floating, filmic |
| Mixolydian | G | W W H W W H W | major with a flat 7th; blues, rock, groove |
| Aeolian (minor) | A | W H W W H W W | the standard minor |
| Locrian | B | H W W H W W W | unstable; rarely a home |

- **Pentatonic**: five notes. Minor pentatonic = minor without degrees 2 and 6 (A C D E G); nearly every combination of its notes sounds acceptable, which makes it the safe palette for melodies and the black keys of the piano. Major pentatonic = major without 4 and 7.
- **Whole tone** (six notes, all whole steps) has no tonic and sounds blurred; **octatonic / diminished** (alternating half and whole steps) sounds tense and jazzy. Both are for passages, rarely for a whole track.
- Other tunings exist (seven unequal notes per octave in some gamelan traditions, 19 equal divisions, by-ear tunings between the piano's keys). An engine that accepts any frequency can use them; the only rule is that the parts share the tuning.

## Chords

- A **triad** stacks the 3rd and 5th of the scale above a **root**. Major triad: root, major 3rd (4 semitones), 5th (7). Minor triad: root, minor 3rd (3), 5th (7). Diminished: 3 + 6.
- **Diatonic triads** use only the key's notes; roman numerals name them by degree, upper case for major, lower for minor, ° for diminished.

| Degree | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|
| Major key (C) | I C | ii Dm | iii Em | IV F | V G | vi Am | vii° Bdim |
| Natural minor (C) | i Cm | ii° Ddim | III Eb | iv Fm | v Gm | VI Ab | VII Bb |

- **Seventh chords** add a 3rd on top: Cmaj7 = C E G B; Cm7 = C Eb G Bb; G7 (dominant) = G B D F. Diatonic sevenths in major: Imaj7, ii7, iii7, IVmaj7, V7, vi7, viiø7. Sevenths are the default colour of house, soul and jazz-leaning harmony.
- **Inversions** put a chord note other than the root at the bottom: first inversion (3rd in the bass, written C/E), second (5th, C/G), third for sevenths (7th, Cm7/Bb). **Voicing** is the full arrangement of the notes: which octave, what is doubled, how wide. Same chord, different voicing, different weight.
- **Voice leading** keeps notes shared by adjacent chords in place and moves the others by step; it sounds smooth and conventional. **Parallel motion** (planing) moves every note by the same interval; it sounds floating, modern, often non-diatonic. Both are valid; the choice is the sound.
- **Non-diatonic** chords contain notes outside the key. Used sparingly they are colour (a borrowed iv in a major key, a chromatic passing chord); used systematically (parallel minor sevenths) they suspend the key altogether.
- **Sus chords** (sus2, sus4) put the 2nd or the 4th in place of the 3rd and sound open, neither major nor minor. Held unresolved for a phrase, they build tension without another part; resolve them at the peak, or never.
- **Key change** (modulation) moves the whole cycle to a new tonic. Three practical kinds: a **lift** up a whole step or a minor third, around 65–75 % of the track; a **pivot**, through a chord both keys share (smooth); a **direct cut**, with no pivot: one beat of silence or a riser, and a note common to both keys held across the join. Thin the texture a bar or two before it. At most one per track under six minutes.

## Progressions

- **One chord**: a vamp sustained or repeated for the whole track, interest coming from rhythm and sound. Legitimate and common.
- **I and V** (1 and 5): the elementary tension and release; V sounds unfinished, I sounds home. Alternating them already makes a song.
- **I V vi IV** (1 5 6 4): the modern pop staple in D major: D, A, Bm, G. Reordering (vi IV I V) changes the balance of tension without changing the notes.
- **I iii V IV**, or any progression a bass line **implies**: a bass that lands on the chord tones of a cycle suggests the chords even when nothing plays them.
- **Parallel m7 planing**: the same minor seventh voicing copied to several roots (for instance roots a 4th and a 5th apart, or a 3rd). No single key contains the result; the sound is vague and warm.
- **Harmonising a melody**: collect the notes the melody emphasises (long, accented, repeated) and pick chords that contain them; notes between two chord tones are **passing tones** and need no support. Degrees I and V are the first candidates; IV, vi and ii follow.
- Tension and release come from leaving the tonic and coming back. Changing chords every bar or every two bars at 4-bar phrase lengths keeps the cycle legible; faster changes are for pre-chorus lifts and turnarounds.
- **A cycle belongs to a section, not to the track.** Plan a chord picture per section: A, B, break and peak each get their own cycle, or at least their own order, bass or voicing. Sections change every 16 or 32 bars; the chords inside change every 1 or 2.
- **The same chords, another weight**: start the cycle on another chord (rotation), put the 3rd or 5th in the bass for a whole pass (inversion), or hold the tonic in the bass under the moving chords (**pedal**) and move the bass only with the next section. Each gives a new passage from the same notes.
- **Two cycles that alternate**: one inside the key, one with a borrowed colour (a ♭II, a ♭VI, or the tonic chord turning from major to minor inside the cycle), swapped by section over a constant groove. The sections then come from harmony, not from adding parts.
- **Acceleration before the peak**: chords every two bars, then every bar, then every half bar in the last two bars before the peak; or a bass that steps down one degree per bar through a longer cycle (i, ♭VII, ♭VI, v… in minor) for the last peak only.
- **A mode change on one tonic**: the same root with the parallel scale (minor to major, aeolian to dorian), strongest in a stripped passage of 4 to 8 bars before the full band returns.

## Bass

- Low notes that **anchor the chords** (root on the change, 5th and octave as motion, arpeggios through the chord) and **talk to the drums**: locked to the kick (reinforcing) or filling the kick's gaps (contrasting).
- Archetypes: the **arpeggio** (chord notes in a rhythm, two bars repeated, then transposed with the next chord); **three repeats then an answer** (a short figure three times, a new figure on the fourth); the **sparse dub line** (very few notes, mostly between the beats, space as the main material); the **tight sixteenth line** (short staccato notes in an intricate rhythm against a four-on-the-floor kick); the **octave bounce** (root alternating with its octave on offbeats).
- Range: roughly octave 1 to 2 (MIDI 28–52). Above that it becomes a chord part; below, it becomes a feeling.

## Melody

- The **identity** of a song: what someone sings back. Higher than the bass, more rhythmically active, wider range.
- Built from **phrases** of two or four bars that relate to each other: a phrase repeated with a changed ending (**question** ending away from the tonic, **answer** ending on it); a pair transposed up a step for the second pair; **three repeats then change**.
- **Contour** rules in [creative-strategies.md](creative-strategies.md): balance up and down, alternate steps and leaps, one peak. Starting a melody on a non-tonic note (the 2nd or the 5th) against the implied key creates tension that can last the whole track.
- A line with a very **wide range** can split in the ear into two voices, a high melody and a low bass line, and the listener decides which it is. A repeating line with no clear first note invites different hearings of where the pattern starts; in minimal music that ambiguity is the content.
- Melodies may use notes **between** the twelve, tuned by ear; everything else in the track then has to agree.
