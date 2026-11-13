# Production sheet notation

The production sheet is a Markdown document. Prose goes anywhere; musical material lives in fenced blocks that `scripts/sheet_tool.py` can check and render to a Standard MIDI File. Write the blocks exactly as below: the tool is strict on purpose, because a pattern whose step count is off by one is a pattern nobody can play.

Time is always 4/4. One bar = 4 beats = 16 sixteenths. Bars and beats are numbered from 1 in prose; the tool reports them the same way.

## Drum pattern

````
```pattern drums "Beat A" bars=1 grid=16
Kick  |x...x...x...x...|
Clap  |....x.......x...|
CHat  |..x...x...x...x.|
OHat  |......x.......x.|
Perc=75 |x..x..x..x..x..x|
```
````

- Header: `pattern drums "<Name>" bars=<N> [grid=<steps per bar, default 16>]`. Names are unique in a sheet and are what the arrangement refers to.
- One lane per line: `<lane> | <steps> |`. Pipes and spaces inside the steps are ignored, so group by beat for readability. Each lane must have exactly `bars × grid` step characters.
- Step characters: `.` rest, `x` hit (velocity 100), `X` accent (127), `o` ghost (60).
- Lane names map to General MIDI drum notes: `Kick`, `Snare`, `Clap`, `Rim`, `CHat` (closed hat), `PHat` (pedal hat), `OHat` (open hat), `LTom`, `MTom`, `HTom`, `Crash`, `Ride`, `RideBell`, `Tamb`, `Cowbell`, `Shaker`, `Clave`, `Conga`, `Bongo`, `Perc`, `Perc2`, `Noise`. Any other sound: `<Name>=<midi note>`.

## Pitched pattern

````
```pattern notes "Bass A" bars=2 grid=8 octave=2 key="A minor" role=bass
A . A . C3 . E3 . | A . G . E3 . C3 . |
```
````

- Header: `pattern notes "<Name>" bars=<N> [grid=16] [octave=4] [vel=100] [key="<Root> <scale>"] [role=bass|chords|melody|pad|lead|...] [program=<GM program>]`.
- Body: one token per grid step, whitespace-separated; `|` is decoration. Exactly `bars × grid` tokens.
- Tokens: `.` rest; `-` sustains whatever sounded on the previous step; a note `C`, `F#`, `Bb` (sharps `#`/`♯`, flats `b`/`♭`), with an optional octave (`C3`) that otherwise defaults to `octave=`; a chord joins notes with `+` (`A2+E3`, `C4+E4+G4`); `@<vel>` sets the velocity of a token (`A2@80`).
- Octaves follow the MIDI convention: C4 = middle C = note 60; a bass sits around octave 1–2, chords around 3–4, melodies around 4–6.
- `key=` turns on the in-key check. Known scales: major/ionian, minor/aeolian, dorian, phrygian, lydian, mixolydian, locrian, harmonic minor, melodic minor, minor pentatonic, major pentatonic, blues, whole tone, chromatic. Leave `key=` out for deliberately non-diatonic material (parallel planing, chromatic passing chords) and say so in prose.
- `role=melody|lead|hook` adds the single-peak contour remark; other monophonic lines only get the contour statistics.

## Arrangement

````
```arrangement tempo=124
section Intro      1-16   : Hats A, Pad
section Groove    17-48   : Beat A, Bass A [from=15.3], Pad, Chords [mute=32]
section Break     49-64   : Pad, Chords, Lead [from=49.2]
section Peak      65-96   : Beat A [from=65.2], Bass A, Pad, Chords, Lead [to=95.3]
section Outro     97-112  : Beat A [mute=105-112], Pad
```
````

- One `section <Name> <first bar>-<last bar> : <part>, <part>, ...` per line, bars inclusive and contiguous. A section with no parts is silence. `tempo=` on the header is the tempo the render uses (`--tempo` on the command line overrides it).
- A part is a pattern name, optionally followed by a **window** in brackets. The pattern loops over the whole section, aligned to the section's first bar, and the window decides which of its onsets are kept:
  - `from=<pos>`: nothing sounds before this position, the position itself included (a part that enters late, or a beat-1 hole after a fill: `[from=65.2]` keeps beat 2 onwards);
  - `to=<pos>`: nothing sounds from this position on, the position itself excluded (`[to=64.4]` keeps beats 1 to 3 of bar 64 and cuts beat 4; `[to=65]` plays bar 64 whole and stops at the bar line);
  - `mute=<bar>` or `mute=<bar>-<bar>` or `mute=<pos>`: silence for those bars, or for that one beat or sixteenth; several items separated by commas.
  - Several options go in one bracket, separated by spaces or commas: `Bass A [from=25 mute=31.4]`.
  - A position is `bar`, `bar.beat` or `bar.beat.sixteenth`, counted from 1 (`65.2` is bar 65 beat 2; `64.4.4` is the last sixteenth of bar 64). A position may lie outside the section (a part entering two bars before its section starts in the previous section's line, `Bass A [from=15.3]` in a section that starts at bar 17, is simply clipped to the section).
- Windows are how seams are blurred without multiplying patterns: a part that carries over, enters on beat 2, drops a bar early or skips one bar is the same pattern with a window, not a new pattern. Make a new pattern only when the notes differ (`Beat A`, `Beat A fill`). A sheet with more than about twenty patterns is usually encoding arrangement edits as patterns.
- Every pattern listed must exist in the sheet. The check flags section lengths that are not multiples of 4 bars and gaps between sections, as information: an irregular length is fine when it is a decision. Processing (filter sweeps, sends, effects switched on once) is not notation: it goes in prose, by bar and beat.

## Automation

````
```automation "Acid cutoff" part="Acid" param=cutoff curve=ease-in
17.1 30, 32.4 110 step, 33.1 40 linear
49.1 40, 56.4 90 smooth, 57.1 100
```
````

- Header: `automation "<Name>" part="<pattern name>" param=<name> | cc=<0-127> [curve=<shape>]`. The name is unique in a sheet. `part=` is a pattern that exists in the sheet: the controller is written on that part's MIDI channel (channel 10 for a drum pattern; melodic patterns take channels in order and wrap after 15, so past 15 melodic patterns two parts share a channel and one lane moves both). `param=` is one of `cutoff` (CC 74), `resonance` (71), `attack` (73), `release` (72), `decay` (75), `volume` (7), `pan` (10), `expression` (11), `modulation` (1), `reverb` (91), `chorus` (93), `delay` (94); any other parameter takes its number with `cc=` (when both are given, `cc=` wins). A sound maps the number to its own knob: say in prose which one.
- Body: points `<position> <value> [curve]`, separated by commas, semicolons or line breaks. The position is `bar`, `bar.beat` or `bar.beat.sixteenth`, counted from 1 like everywhere else, in absolute bars of the arrangement. The value is an integer from 0 to 127.
- Points are in strictly increasing order of position. Nothing is written before the first point; the last value is held to the end of the track.
- The curve of a point shapes the segment that leaves it, towards the next point; without one, the block's `curve=` applies (default `linear`). Shapes: `step` holds the value until the next point (a sudden jump: the point that drops it is the next one); `linear`; `ease-in` (slow start, fast end: a filter that opens, a build); `ease-out` (fast start, slow end: a decay that settles); `smooth` (slow at both ends, an S-shaped crossfade). Choose by what the ear should feel and say it in prose.
- In Faulk, each shape is a curve of an automation point, set in the "Ramp to…" dialog, from a point's menu or with C on a point: `linear` is Linear, `step` is Step, `ease-in` is Slow then fast, `ease-out` is Fast then slow, `smooth` is S-curve, with the same formulas (`t²`, `1 − (1 − t)²`, `3t² − 2t³` of the progress `t`). Checking a sheet against a project compares the shapes point by point, not the values: the sheet's 0-127 is a controller, while a Faulk lane is in the parameter's unit (Hz for a cutoff, on a logarithmic knob), so the same curve over the same span does not move the knob the same way. A sweep that must feel slow then fast on a cutoff needs `ease-in` and a span chosen by ear.
- In the example, the cutoff rises from 30 to 110 over bars 17 to 32 with `ease-in` (the block's curve), holds 110 until bar 33 beat 1, where it falls to 40, then follows the later points.
- `check` flags a value or controller outside 0-127, a point out of order, a point at or after the end of the arrangement (without an arrangement: the pattern's length), an unknown curve, a missing `part=` or controller, and a `part` that is not a pattern of the sheet. It warns when the part sits in no section. `render` writes one control change on the part's channel at each point and, for the shaped curves, one per sixteenth where the value moves, on that part's track.
- Automation is the movement of one MIDI controller. Sends, effect chains and anything that is not a controller stay in prose, by bar and beat.

## Running the tool

```
python scripts/sheet_tool.py check  sheet.md
python scripts/sheet_tool.py render sheet.md --out song.mid [--tempo 124]
```

`check` prints, per pattern, the errors (wrong counts, bad tokens: exit code 1), the in-key verdict, and contour statistics (range, how many times the top note occurs, steps versus leaps, up versus down, notes per bar). `render` refuses to write a file while errors remain; without an arrangement block it lays every pattern once from bar 1, which is enough to audition loop material. The MIDI file has one track per pattern (its automation lanes are control changes on that track), drums on channel 10, so any sequencer or sampler can open it.

Read the warnings as questions, not verdicts: an out-of-key note or a six-bar section is often the best idea in the sheet. Each one must simply be a choice you can name.
