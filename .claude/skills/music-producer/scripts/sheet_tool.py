#!/usr/bin/env python3
"""Check a production sheet and render it to a Standard MIDI File.

A production sheet is a Markdown file that carries fenced ```pattern``` and
```arrangement``` blocks in the notation described in references/notation.md.
No third-party dependency: the MIDI writer is a few dozen lines.

Usage:
  python sheet_tool.py check  SHEET.md
  python sheet_tool.py render SHEET.md --out SONG.mid [--tempo 120]

`check` exits 1 on structural errors (wrong step counts, unknown pattern in a
section, unparsable token). Out-of-key notes and contour remarks are reported
as warnings, because they may be deliberate, but each one must be a decision.
"""
from __future__ import annotations

import argparse
import re
import struct
import sys
from dataclasses import dataclass, field

PPQ = 480
NOTE_PC = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}
ACC = {"#": 1, "♯": 1, "b": -1, "♭": -1, "": 0}
SCALES = {
    "major": [0, 2, 4, 5, 7, 9, 11], "ionian": [0, 2, 4, 5, 7, 9, 11],
    "minor": [0, 2, 3, 5, 7, 8, 10], "aeolian": [0, 2, 3, 5, 7, 8, 10],
    "dorian": [0, 2, 3, 5, 7, 9, 10], "phrygian": [0, 1, 3, 5, 7, 8, 10],
    "lydian": [0, 2, 4, 6, 7, 9, 11], "mixolydian": [0, 2, 4, 5, 7, 9, 10],
    "locrian": [0, 1, 3, 5, 6, 8, 10],
    "harmonic minor": [0, 2, 3, 5, 7, 8, 11], "melodic minor": [0, 2, 3, 5, 7, 9, 11],
    "minor pentatonic": [0, 3, 5, 7, 10], "major pentatonic": [0, 2, 4, 7, 9],
    "blues": [0, 3, 5, 6, 7, 10], "whole tone": [0, 2, 4, 6, 8, 10],
    "chromatic": list(range(12)),
}
DRUM_NOTES = {
    "kick": 36, "bd": 36, "kick2": 35, "rim": 37, "rimshot": 37, "snare": 38, "sd": 38,
    "clap": 39, "snare2": 40, "ltom": 45, "lowtom": 45, "mtom": 47, "midtom": 47,
    "htom": 50, "hightom": 50, "chat": 42, "closedhat": 42, "hh": 42, "hihat": 42,
    "phat": 44, "pedalhat": 44, "ohat": 46, "openhat": 46, "crash": 49, "ride": 51,
    "ridebell": 53, "tamb": 54, "tambourine": 54, "cowbell": 56, "shaker": 70,
    "clave": 75, "perc": 76, "perc2": 77, "conga": 63, "bongo": 60, "noise": 81,
}
VEL = {"x": 100, "X": 127, "o": 60}

TOKEN_RE = re.compile(r"^([A-G])([#♯b♭]?)(-?\d)?$")


@dataclass
class Note:
    start: float  # in beats from pattern start
    dur: float    # in beats
    pitch: int
    vel: int


@dataclass
class Pattern:
    kind: str
    name: str
    bars: int
    grid: int
    params: dict
    notes: list = field(default_factory=list)
    errors: list = field(default_factory=list)
    warnings: list = field(default_factory=list)
    line_no: int = 0

    @property
    def beats(self) -> float:
        return self.bars * 4.0


@dataclass
class Part:
    name: str
    start: float | None = None   # absolute beat offset (0 = bar 1 beat 1), inclusive
    end: float | None = None     # absolute beat offset, exclusive
    mutes: list = field(default_factory=list)  # (start_beat, end_beat) windows to silence
    raw: str = ""


@dataclass
class Section:
    name: str
    start_bar: int
    end_bar: int
    parts: list

    @property
    def patterns(self):
        return [p.name for p in self.parts]


def parse_position(text: str) -> float | None:
    """'65' -> bar 65 beat 1; '65.2' -> bar 65 beat 2; '64.4.3' -> bar 64 beat 4 sixteenth 3. Beats from bar 1."""
    m = re.match(r"^\s*(\d+)(?:\.(\d))?(?:\.(\d))?\s*$", text)
    if not m:
        return None
    bar = int(m.group(1)); beat = int(m.group(2) or 1); six = int(m.group(3) or 1)
    if bar < 1 or not (1 <= beat <= 4 and 1 <= six <= 4):
        return None
    return (bar - 1) * 4.0 + (beat - 1) + (six - 1) * 0.25


def span_end(text: str, pos: float) -> float:
    """A bare bar number spans the whole bar; bar.beat spans one beat; bar.beat.sixteenth one sixteenth."""
    dots = text.count(".")
    return pos + (4.0 if dots == 0 else 1.0 if dots == 1 else 0.25)


def parse_part(text: str, errors: list, where: str) -> Part:
    m = re.match(r"^\s*([^\[]+?)\s*(?:\[(.*)\])?\s*$", text)
    name, opts = m.group(1), (m.group(2) or "")
    part = Part(name=name, raw=text.strip())
    pattern = r"(from|to|mute)\s*=\s*([^\s,;\]]+(?:\s*,\s*(?!(?:from|to|mute)\s*=)[^\s,;\]]+)*)"
    for key, val in re.findall(pattern, opts):
        if key == "mute":
            for item in [x.strip() for x in val.split(",")]:
                a, _, b = item.partition("-")
                sa = parse_position(a)
                sb = span_end(b, parse_position(b)) if b else (span_end(a, sa) if sa is not None else None)
                if sa is None or sb is None:
                    errors.append(f"{where}: bad mute {item!r} (use mute=27 or mute=10-11 or mute=12.4)"); continue
                part.mutes.append((sa, sb))
        else:
            pos = parse_position(val)
            if pos is None:
                errors.append(f"{where}: bad position {val!r} for {key}= (use bar, bar.beat or bar.beat.sixteenth)"); continue
            if key == "from":
                part.start = pos
            else:
                part.end = pos
    leftover = re.sub(pattern, "", opts).strip(" ,;")
    if leftover:
        errors.append(f"{where}: unknown option {leftover!r} in [{opts}] (known: from=, to=, mute=)")
    return part


def parse_params(s: str) -> dict:
    out = {}
    for m in re.finditer(r'(\w+)=("([^"]*)"|\S+)', s):
        out[m.group(1)] = m.group(3) if m.group(3) is not None else m.group(2)
    return out


def pitch_of(token: str, default_octave: int) -> int | None:
    m = TOKEN_RE.match(token)
    if not m:
        return None
    letter, acc, octv = m.groups()
    octave = int(octv) if octv is not None else default_octave
    return 12 * (octave + 1) + NOTE_PC[letter] + ACC[acc]


def parse_key(key: str | None):
    """'E minor' -> (4, [0,2,3,...]) or None."""
    if not key:
        return None
    m = re.match(r"^\s*([A-G])([#♯b♭]?)\s+(.+?)\s*$", key)
    if not m or m.group(3).lower() not in SCALES:
        return None
    root = (NOTE_PC[m.group(1)] + ACC[m.group(2)]) % 12
    return root, SCALES[m.group(3).lower()]


def parse_drums(p: Pattern, body: list[str]):
    steps = p.bars * p.grid
    beat_len = 4.0 / p.grid
    for ln in body:
        if not ln.strip() or ln.strip().startswith("#"):
            continue
        if "|" not in ln:
            p.errors.append(f"drum line without '|': {ln.strip()!r}")
            continue
        name, rest = ln.split("|", 1)
        name = name.strip()
        midi = None
        if "=" in name:
            name, num = [x.strip() for x in name.split("=", 1)]
            if num.isdigit():
                midi = int(num)
        key = re.sub(r"[^a-z0-9]", "", name.lower())
        if midi is None:
            midi = DRUM_NOTES.get(key)
        if midi is None:
            p.warnings.append(f"unknown drum lane {name!r}: mapped to 37 (add '=NN' to set a MIDI note)")
            midi = 37
        cells = re.sub(r"[\s|]", "", rest)
        if len(cells) != steps:
            p.errors.append(f"lane {name!r}: {len(cells)} steps, expected {steps} (bars={p.bars} x grid={p.grid})")
            continue
        for i, c in enumerate(cells):
            if c in VEL:
                p.notes.append(Note(i * beat_len, beat_len * 0.5, midi, VEL[c]))
            elif c not in ".-":
                p.errors.append(f"lane {name!r}: unknown step character {c!r}")
                break


def parse_notes(p: Pattern, body: list[str]):
    steps = p.bars * p.grid
    beat_len = 4.0 / p.grid
    default_octave = int(p.params.get("octave", 4))
    default_vel = int(p.params.get("vel", 100))
    tokens = []
    for ln in body:
        if ln.strip().startswith("#"):
            continue
        tokens += [t for t in ln.replace("|", " ").split() if t]
    if len(tokens) != steps:
        p.errors.append(f"{len(tokens)} tokens, expected {steps} (bars={p.bars} x grid={p.grid})")
        return
    open_notes: list[Note] = []
    for i, tok in enumerate(tokens):
        if tok == "-":
            for n in open_notes:
                n.dur += beat_len
            continue
        open_notes = []
        if tok == ".":
            continue
        vel = default_vel
        if "@" in tok:
            tok, v = tok.split("@", 1)
            if not v.isdigit():
                p.errors.append(f"bad velocity in token {tok!r}@{v}")
                return
            vel = int(v)
        for part in tok.split("+"):
            pitch = pitch_of(part, default_octave)
            if pitch is None:
                p.errors.append(f"unparsable note token {part!r} at step {i + 1}")
                return
            n = Note(i * beat_len, beat_len, pitch, vel)
            p.notes.append(n)
            open_notes.append(n)


def parse_sheet(text: str):
    patterns: dict[str, Pattern] = {}
    sections: list[Section] = []
    arr_params: dict = {}
    errors: list[str] = []
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        m = re.match(r"^\s*```(pattern|arrangement)\b(.*)$", lines[i])
        if not m:
            i += 1
            continue
        start = i
        i += 1
        body = []
        while i < len(lines) and not re.match(r"^\s*```\s*$", lines[i]):
            body.append(lines[i])
            i += 1
        i += 1
        head = m.group(2).strip()
        if m.group(1) == "pattern":
            hm = re.match(r'^(drums|notes)\s+"([^"]+)"\s*(.*)$', head)
            if not hm:
                errors.append(f"line {start + 1}: pattern header must be: pattern drums|notes \"Name\" bars=N ...")
                continue
            kind, name, rest = hm.groups()
            params = parse_params(rest)
            if "bars" not in params or not params["bars"].isdigit():
                errors.append(f"line {start + 1}: pattern {name!r} needs bars=N")
                continue
            grid = int(params.get("grid", 16))
            p = Pattern(kind, name, int(params["bars"]), grid, params, line_no=start + 1)
            (parse_drums if kind == "drums" else parse_notes)(p, body)
            if name in patterns:
                errors.append(f"line {start + 1}: duplicate pattern name {name!r}")
            patterns[name] = p
        else:
            arr_params.update(parse_params(head))
            for ln in body:
                if not ln.strip() or ln.strip().startswith("#"):
                    continue
                sm = re.match(r"^\s*section\s+(.+?)\s+(\d+)\s*-\s*(\d+)\s*:\s*(.*)$", ln)
                if not sm:
                    errors.append(f"arrangement: cannot parse {ln.strip()!r} (expected: section Name 1-8 : Pattern A, Pattern B)")
                    continue
                chunks = [x for x in re.split(r",(?![^\[]*\])", sm.group(4)) if x.strip()]
                where = f"arrangement, section {sm.group(1).strip()!r}"
                parts = [parse_part(x, errors, where) for x in chunks]
                sections.append(Section(sm.group(1).strip(), int(sm.group(2)), int(sm.group(3)), parts))
    return patterns, sections, arr_params, errors


# ---------------------------------------------------------------- checks

def contour_report(p: Pattern) -> list[str]:
    """Remarks for a monophonic line: range, peak uniqueness, steps vs leaps, direction balance."""
    by_start: dict[float, list[Note]] = {}
    for n in p.notes:
        by_start.setdefault(n.start, []).append(n)
    if any(len(v) > 1 for v in by_start.values()):
        return []  # chords: contour rules do not apply
    seq = [v[0].pitch for _, v in sorted(by_start.items())]
    if len(seq) < 3:
        return []
    out = []
    rng = max(seq) - min(seq)
    peaks = seq.count(max(seq))
    ints = [b - a for a, b in zip(seq, seq[1:]) if b != a]
    steps = sum(1 for d in ints if abs(d) <= 2)
    leaps = sum(1 for d in ints if abs(d) > 2)
    up = sum(d for d in ints if d > 0)
    down = -sum(d for d in ints if d < 0)
    out.append(f"contour: range {rng} semitones, highest note occurs {peaks}x, steps {steps} / leaps {leaps}, up {up} / down {down} semitones, {len(seq) / p.bars:.1f} notes per bar")
    if peaks > 1 and p.kind == "notes" and p.params.get("role", "").lower() in ("melody", "lead", "hook"):
        out.append("contour: the highest note repeats; a memorable line usually has a single peak")
    if ints and (steps == 0 or leaps == 0):
        out.append("contour: all steps or all leaps; strong lines balance the two")
    return out


def key_report(p: Pattern) -> list[str]:
    key = p.params.get("key")
    if not key:
        return []
    parsed = parse_key(key)
    if not parsed:
        return [f"key {key!r} not understood (use e.g. key=\"E minor\"; known scales: {', '.join(SCALES)})"]
    root, scale = parsed
    allowed = {(root + d) % 12 for d in scale}
    bad = [(n.start, n.pitch) for n in p.notes if n.pitch % 12 not in allowed]
    if not bad:
        return [f"all {len(p.notes)} notes are in {key}"]
    names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
    where = ", ".join(f"{names[pc % 12]}{pc // 12 - 1} at bar {int(st // 4) + 1} beat {st % 4 + 1:g}" for st, pc in bad[:8])
    more = "" if len(bad) <= 8 else f" (+{len(bad) - 8} more)"
    return [f"{len(bad)} note(s) outside {key}: {where}{more}. Keep each one only if it is a decision."]


def check(patterns, sections, arr_params, errors) -> int:
    rc = 0
    for e in errors:
        print(f"ERROR  {e}")
        rc = 1
    for p in patterns.values():
        print(f"\n[{p.kind}] {p.name!r}  bars={p.bars} grid={p.grid} notes={len(p.notes)}")
        for e in p.errors:
            print(f"  ERROR  {e}")
            rc = 1
        for w in p.warnings:
            print(f"  WARN   {w}")
        if p.kind == "drums" and not p.errors:
            pulse = [n for n in p.notes if n.pitch in (35, 36)]
            if not pulse:
                print("  NOTE   no kick in this pattern: fine if another part carries the pulse")
        if p.kind == "notes" and not p.errors:
            for r in key_report(p):
                print(f"  {'OK    ' if r.startswith('all ') else 'WARN  '} {r}")
            for r in contour_report(p):
                print(f"  INFO   {r}")
    if sections:
        print("\n[arrangement]", " ".join(f"{k}={v}" for k, v in arr_params.items()))
        last_end = 0
        for s in sections:
            length = s.end_bar - s.start_bar + 1
            flags = []
            if length % 4:
                flags.append("length not a multiple of 4 bars")
            if s.start_bar != last_end + 1:
                flags.append(f"starts at bar {s.start_bar}, previous section ended at {last_end}")
            missing = [n for n in s.patterns if n not in patterns]
            if missing:
                flags.append("unknown pattern(s): " + ", ".join(repr(n) for n in missing))
                rc = 1
            last_end = s.end_bar
            edits = [pt.raw for pt in s.parts if pt.start is not None or pt.end is not None or pt.mutes]
            print(f"  {s.name:<14} bars {s.start_bar:>3}-{s.end_bar:<3} ({length:>2}) {len(s.parts)} parts"
                  + (f"  <- {'; '.join(flags)}" if flags else "")
                  + (f"\n{'':17}edits: " + "; ".join(edits) if edits else ""))
        print(f"  total {last_end} bars")
    else:
        print("\n[arrangement] none: render will lay every pattern once from bar 1")
    print("\nRESULT", "errors" if rc else "ok")
    return rc


# ---------------------------------------------------------------- MIDI

def vlq(n: int) -> bytes:
    out = [n & 0x7F]
    n >>= 7
    while n:
        out.append(0x80 | (n & 0x7F))
        n >>= 7
    return bytes(reversed(out))


def track_bytes(events: list[tuple[int, bytes]]) -> bytes:
    events.sort(key=lambda e: e[0])
    data = bytearray()
    t = 0
    for tick, msg in events:
        data += vlq(tick - t) + msg
        t = tick
    data += vlq(0) + b"\xff\x2f\x00"
    return b"MTrk" + struct.pack(">I", len(data)) + bytes(data)


def place(p: Pattern, from_beat: float, until_beat: float, channel: int, events: list, part: "Part | None" = None):
    """Loop p over [from_beat, until_beat), aligned to from_beat; keep only onsets inside the part's window and outside its mutes."""
    lo = from_beat if part is None or part.start is None else max(from_beat, part.start)
    hi = until_beat if part is None or part.end is None else min(until_beat, part.end)
    mutes = part.mutes if part else []
    pos = from_beat
    while pos < until_beat - 1e-9:
        for n in p.notes:
            s = pos + n.start
            if s < lo - 1e-9 or s >= hi - 1e-9:
                continue
            if any(a - 1e-9 <= s < b - 1e-9 for a, b in mutes):
                continue
            e = min(s + n.dur, hi)
            on = int(round(s * PPQ))
            off = max(on + 1, int(round(e * PPQ)))
            events.append((on, bytes([0x90 | channel, n.pitch & 0x7F, n.vel & 0x7F])))
            events.append((off, bytes([0x80 | channel, n.pitch & 0x7F, 0])))
        pos += p.beats


def render(patterns, sections, arr_params, out_path: str, tempo: float | None):
    bpm = tempo or float(arr_params.get("tempo", 120))
    tracks = []
    meta = [(0, b"\xff\x51\x03" + struct.pack(">I", int(60_000_000 / bpm))[1:]),
            (0, b"\xff\x58\x04\x04\x02\x18\x08")]
    tracks.append(track_bytes(meta))
    melodic_channel = 0
    for p in patterns.values():
        channel = 9 if p.kind == "drums" else melodic_channel
        if p.kind == "notes":
            melodic_channel = (melodic_channel + 1) % 16
            if melodic_channel == 9:
                melodic_channel = 10
        events: list = [(0, b"\xff\x03" + vlq(len(p.name.encode())) + p.name.encode())]
        prog = p.params.get("program")
        if prog and prog.isdigit() and p.kind == "notes":
            events.append((0, bytes([0xC0 | channel, int(prog) & 0x7F])))
        if sections:
            for s in sections:
                for part in s.parts:
                    if part.name == p.name:
                        place(p, (s.start_bar - 1) * 4.0, s.end_bar * 4.0, channel, events, part)
        else:
            place(p, 0.0, p.beats, channel, events)
        tracks.append(track_bytes(events))
    header = b"MThd" + struct.pack(">IHHH", 6, 1, len(tracks), PPQ)
    with open(out_path, "wb") as f:
        f.write(header + b"".join(tracks))
    total = max([s.end_bar for s in sections], default=max([p.bars for p in patterns.values()], default=0))
    print(f"wrote {out_path}: {len(tracks) - 1} part tracks, {total} bars at {bpm:g} bpm")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check"); c.add_argument("sheet")
    r = sub.add_parser("render"); r.add_argument("sheet"); r.add_argument("--out", required=True); r.add_argument("--tempo", type=float)
    a = ap.parse_args(argv)
    with open(a.sheet, encoding="utf-8") as f:
        text = f.read()
    patterns, sections, arr_params, errors = parse_sheet(text)
    if not patterns:
        print("no ```pattern``` block found")
        return 1
    rc = check(patterns, sections, arr_params, errors)
    if a.cmd == "render":
        if rc:
            print("not rendering: fix the errors above")
            return rc
        render(patterns, sections, arr_params, a.out, a.tempo)
    return rc


if __name__ == "__main__":
    sys.exit(main())
