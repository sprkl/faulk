# Pattern library

Starting points in the sheet notation ([notation.md](notation.md)), all in 4/4. Every block here passes `scripts/sheet_tool.py check`, so copying one into a sheet gives a valid, playable pattern to mutate. Pitched examples are in A minor (or say otherwise); transpose by changing the note names. Treat these as archetypes to mutate, never as the deliverable: a sheet made only of library patterns has not been produced yet.

## Drums

Pulse carriers. The kick on every beat makes the pulse unlosable; everything else can then be free.

```pattern drums "Four on the floor" bars=1 grid=16
Kick |x...x...x...x...|
```

```pattern drums "Classic house" bars=1 grid=16
Kick |x...x...x...x...|
Clap |....x.......x...|
CHat |..x...x...x...x.|
```

```pattern drums "Basic rock" bars=1 grid=16
Kick  |x.......x.......|
Snare |....x.......x...|
CHat  |x.x.x.x.x.x.x.x.|
```

The rock and house beats are almost the same pattern: tempo and the hats on or off the beat make the genre.

```pattern drums "Stomp stomp clap" bars=1 grid=16
Kick |x.x.....x.x.....|
Clap |....x.......x...|
```

```pattern drums "Head nod 90" bars=1 grid=16
Kick |x......x..x.....|
Clap |....x.......x...|
CHat |x.x.x.o.x.x.x.o.|
OHat |..............x.|
```

A late kick (the "a" of 2) and ghost hats give the lean-back feel at 85–95 bpm. The open hat replaces the closed hat on its step: hats choke each other.

```pattern drums "Two bars with a turn" bars=2 grid=16
Kick |x...x...x...x...|x...x...x...x...|
Clap |....x.......x...|....x.......x.x.|
CHat |..x...x...x...x.|..x...x...x.....|
OHat |................|..............x.|
```

Bar 2 ends differently from bar 1: the smallest form there is. Loop two bars, not one, and the ear already hears a phrase.

### Linear drumming

No two hits on one step; groups of three against the sixteenth grid, resolved by the clap at the end of the bar.

```pattern drums "Linear groove" bars=1 grid=16
Kick |x.....x.....x...|
Clap |...x.....x.....x|
CHat |.xx.xx.xx.xx.xx.|
```

Every sixteenth filled with exactly one drum; velocity alone shapes the line.

```pattern drums "Dense linear" bars=1 grid=16
Kick |x.......x.......|
Clap |....X.......X...|
CHat |.oxo.oxo.oxo.oxo|
```

### Polyrhythm

A 4-step loop against a 5-step loop: the composite only repeats after 20 steps, which is 5 bars when written out. In an engine that loops clips of different lengths, write the two one-step patterns instead and let them drift.

```pattern drums "Four against five" bars=5 grid=16
Perc  |x...x...x...x...|x...x...x...x...|x...x...x...x...|x...x...x...x...|x...x...x...x...|
Perc2 |x....x....x....x|....x....x....x.|...x....x....x..|..x....x....x...|.x....x....x....|
```

### Fills that blur the bar line

Anticipation: the roll peaks on the last sixteenth of bar 1, beat 1 of bar 2 is empty, the crash lands on beat 2.

```pattern drums "Fill anticipation" bars=2 grid=16
Kick  |x...x...x...x...|....x...x...x...|
Clap  |....x.......x...|....x.......x...|
CHat  |..x...x...x...x.|..x...x...x...x.|
Snare |..........xxxxxX|................|
Crash |................|....x...........|
```

Hesitation: the roll crosses the bar line and ends on beat 2 of bar 2, where the new groove takes over.

```pattern drums "Fill hesitation" bars=2 grid=16
Kick  |x...x...x...x...|....x...x...x...|
Clap  |....x.......x...|............x...|
CHat  |..x...x...x...x.|..x...x...x...x.|
Snare |............xxxx|xxxxX...........|
```

## Chords

Offbeat chords (the skank): every "and", nothing on the beat.

```pattern notes "Skank chords" bars=1 grid=8 octave=4 key="C minor" role=chords
. C+Eb+G . C+Eb+G . C+Eb+G . C+Eb+G
```

The pop staple, 1 5 6 4 in D major, as block chords and then voice-led (shared notes stay, the others move by step).

```pattern notes "1 5 6 4 blocks" bars=4 grid=4 octave=4 key="D major" role=chords
D+F#+A - - - | A+C#+E - - - | B+D+F# - - - | G+B+D - - - |
```

```pattern notes "1 5 6 4 voice-led" bars=4 grid=4 key="D major" role=chords
D4+F#4+A4 - - - | C#4+E4+A4 - - - | D4+F#4+B4 - - - | D4+G4+B4 - - - |
```

Parallel planing: one minor seventh voicing copied to four roots. No key fits, so no `key=`; the floating sound is the point.

```pattern notes "Parallel m7" bars=4 grid=4 octave=4 role=chords
E+G+B+D5 - - - | G+Bb+D5+F5 - - - | A+C5+E5+G5 - - - | B+D5+F#5+A5 - - - |
```

One chord for the whole track; rhythm carries it.

```pattern notes "Am7 vamp" bars=1 grid=16 octave=3 key="A minor" role=chords
A+C4+E4+G4 . . A+C4+E4+G4 . . A+C4+E4+G4 . . . A+C4+E4+G4 . . . A+C4+E4+G4 .
```

## Bass

Arpeggio through the chord, two bars repeated, then the same figure on the next chord.

```pattern notes "Arpeggio bass" bars=4 grid=8 octave=2 key="A minor" role=bass
A . C3 . E3 . C3 . | A . E3 . C3 . A . | F . A . C3 . A . | F . C3 . A . F . |
```

A figure three times, then an answer.

```pattern notes "Three then answer" bars=2 grid=16 octave=2 key="A minor" role=bass
A . . A . C3 . . | A . . A . C3 . . | A . . A . C3 . . | E . G . . A . . |
```

Sparse dub bass: four notes in two bars, all between the beats.

```pattern notes "Sparse dub bass" bars=2 grid=16 octave=2 key="A minor" role=bass
. . . A - - . . . . E - . . . . | . . . . . G - - . . . . . A - . |
```

Short, busy sixteenths that contrast with a four-on-the-floor kick.

```pattern notes "Tight sixteenth bass" bars=1 grid=16 octave=2 key="A minor" role=bass
A . A A . A . . A . G . A . C3 .
```

Octave bounce, the pumping house line.

```pattern notes "Octave bounce" bars=1 grid=8 octave=2 key="A minor" role=bass
A A3 A A3 A A3 A A3
```

## Melody

Question and answer: the same opening, the first phrase ends away from home, the second climbs to its single peak and resolves to the tonic.

```pattern notes "Question answer" bars=4 grid=8 octave=5 key="A minor" role=melody
C . D . E . D . | C . B4 - - - . . | C . D . F . E . | D . B4 . A4 - - - |
```

Steps up to a single peak, then a leap down and steps back.

```pattern notes "Steps then leap" bars=2 grid=8 octave=5 key="A minor" role=melody
A4 B4 C D E - . . | G4 . A4 . C . A4 . |
```

A two-bar pair, then the same pair a step higher.

```pattern notes "Transposed pair" bars=4 grid=8 octave=5 key="A minor" role=melody
A4 . C . B4 . A4 . | G4 . A4 - - - . . | B4 . D . C . B4 . | A4 . B4 - - - . . |
```

One line with a wide range that the ear splits into a low voice and a high voice.

```pattern notes "Interlocking line" bars=1 grid=16 octave=4 key="A minor" role=line
A3 . E5 . A3 . C5 . G3 . E5 . A3 . D5 .
```

Hocket: one melody split across two instruments so that they never sound together. Give each voice its own sound with a fast attack.

```pattern notes "Hocket voice 1" bars=1 grid=8 octave=5 key="A minor" role=melody
A . C . E . D .
```

```pattern notes "Hocket voice 2" bars=1 grid=8 octave=5 key="A minor" role=melody
. B4 . D . C . B4
```
