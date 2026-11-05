"""Tests for the automation block of sheet_tool.py. Run: python -m unittest test_sheet_tool (from this folder)."""
import contextlib
import io
import os
import struct
import tempfile
import unittest

import sheet_tool

BASE = '''
```pattern notes "Pad" bars=2 grid=4 octave=3 key="A minor" role=pad
A2+E3 - - - | C3+G3 - - -
```

```pattern drums "Beat" bars=1 grid=16
Kick |x...x...x...x...|
```

```arrangement tempo=120
section Intro 1-4 : Pad, Beat
section Main  5-8 : Pad, Beat
```
'''


def sheet(auto: str) -> str:
    return BASE + "\n" + auto


def run(argv):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        rc = sheet_tool.main(argv)
    return rc, out.getvalue()


class Fixture(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)

    def write(self, text: str) -> str:
        path = os.path.join(self.dir.name, "sheet.md")
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        return path

    def check(self, auto: str):
        return run(["check", self.write(sheet(auto))])

    def render(self, auto: str):
        out = os.path.join(self.dir.name, "song.mid")
        rc, text = run(["render", self.write(sheet(auto)), "--out", out])
        return rc, text, out


def midi_ccs(path):
    """[(track name, tick, channel, controller, value)] for every control change in a format-1 file."""
    with open(path, "rb") as f:
        data = f.read()
    ntracks = struct.unpack(">H", data[10:12])[0]
    pos, found = 14, []
    for _ in range(ntracks):
        size = struct.unpack(">I", data[pos + 4:pos + 8])[0]
        end, i, tick, name, status = pos + 8 + size, pos + 8, 0, "", 0
        while i < end:
            delta = 0
            while True:
                b = data[i]; i += 1
                delta = (delta << 7) | (b & 0x7F)
                if not b & 0x80:
                    break
            tick += delta
            if data[i] == 0xFF:
                n = data[i + 2]
                if data[i + 1] == 3:
                    name = data[i + 3:i + 3 + n].decode()
                i += 3 + n
                continue
            if data[i] & 0x80:
                status = data[i]; i += 1
            size_ = 1 if status >> 4 in (0xC, 0xD) else 2
            msg = data[i:i + size_]; i += size_
            if status >> 4 == 0xB:
                found.append((name, tick, status & 0x0F, msg[0], msg[1]))
        pos = end
    return found


class CheckTests(Fixture):
    def test_valid_block_passes(self):
        rc, out = self.check('```automation "Pad filter" part="Pad" param=cutoff curve=linear\n1.1 20, 5.1 100, 8.4 60\n```')
        self.assertEqual(rc, 0, out)
        self.assertIn("[automation] 'Pad filter'", out)
        self.assertIn("cc=74", out)

    def test_value_out_of_range(self):
        rc, out = self.check('```automation "A" part="Pad" cc=74\n1.1 20, 5.1 128\n```')
        self.assertEqual(rc, 1)
        self.assertIn("value 128 at 5.1 is outside 0-127", out)

    def test_cc_out_of_range(self):
        rc, out = self.check('```automation "A" part="Pad" cc=200\n1.1 20, 5.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn("cc 200 is outside 0-127", out)

    def test_points_out_of_order(self):
        rc, out = self.check('```automation "A" part="Pad" cc=74\n5.1 20, 1.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn("not after the previous point", out)

    def test_equal_positions_refused(self):
        rc, out = self.check('```automation "A" part="Pad" cc=74\n5.1 20, 5.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn("not after the previous point", out)

    def test_point_after_the_arrangement(self):
        rc, out = self.check('```automation "A" part="Pad" cc=74\n1.1 20, 9.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn("outside the arrangement (8 bars)", out)

    def test_last_beat_of_last_bar_is_inside(self):
        rc, out = self.check('```automation "A" part="Pad" cc=74\n1.1 20, 8.4.4 90\n```')
        self.assertEqual(rc, 0, out)

    def test_missing_target_part(self):
        rc, out = self.check('```automation "A" part="Ghost" cc=74\n1.1 20, 5.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn("target part 'Ghost' is not a pattern", out)

    def test_no_part_given(self):
        rc, out = self.check('```automation "A" cc=74\n1.1 20, 5.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn('needs part=', out)

    def test_no_controller_given(self):
        rc, out = self.check('```automation "A" part="Pad"\n1.1 20, 5.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn("needs cc=", out)

    def test_unknown_param_name(self):
        rc, out = self.check('```automation "A" part="Pad" param=wobble\n1.1 20, 5.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn("needs cc=", out)

    def test_unknown_curve(self):
        rc, out = self.check('```automation "A" part="Pad" cc=74 curve=zigzag\n1.1 20, 5.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn("unknown curve 'zigzag'", out)

    def test_bad_point_syntax(self):
        rc, out = self.check('```automation "A" part="Pad" cc=74\n1.1, 5.7 90, 6.1 x\n```')
        self.assertEqual(rc, 1)
        self.assertIn("cannot parse point '1.1'", out)
        self.assertIn("bad position '5.7'", out)
        self.assertIn("bad value 'x'", out)

    def test_no_points(self):
        rc, out = self.check('```automation "A" part="Pad" cc=74\n```')
        self.assertEqual(rc, 1)
        self.assertIn("no points", out)

    def test_duplicate_name(self):
        block = '```automation "A" part="Pad" cc=74\n1.1 20, 5.1 90\n```\n'
        rc, out = self.check(block + block)
        self.assertEqual(rc, 1)
        self.assertIn("duplicate automation name 'A'", out)

    def test_without_arrangement_limit_is_the_pattern(self):
        text = '```pattern notes "Pad" bars=2 grid=4\nA3 - - - | C3 - - -\n```\n' \
               '```automation "A" part="Pad" cc=74\n1.1 20, 3.1 90\n```\n'
        rc, out = run(["check", self.write(text)])
        self.assertEqual(rc, 1)
        self.assertIn("outside pattern 'Pad' (2 bars)", out)

    def test_old_sheets_without_automation_still_pass(self):
        rc, out = run(["check", self.write(BASE)])
        self.assertEqual(rc, 0, out)
        self.assertNotIn("[automation]", out)


class RenderTests(Fixture):
    def test_linear_ramp_is_written_as_cc_on_the_parts_channel(self):
        rc, text, out = self.render('```automation "A" part="Pad" cc=74\n1.1 0, 2.1 100\n```')
        self.assertEqual(rc, 0, text)
        ccs = midi_ccs(out)
        self.assertTrue(ccs)
        self.assertTrue(all(c[0] == "Pad" and c[2] == 0 and c[3] == 74 for c in ccs))
        self.assertEqual(ccs[0][1:], (0, 0, 74, 0))
        self.assertEqual(ccs[-1][1:], (4 * sheet_tool.PPQ, 0, 74, 100))
        values = [c[4] for c in ccs]
        self.assertEqual(values, sorted(values))
        self.assertGreater(len(values), 10)
        self.assertIn("1 automation lane(s)", text)

    def test_step_curve_holds_the_value_until_the_next_point(self):
        rc, text, out = self.render('```automation "A" part="Pad" param=cutoff curve=step\n1.1 10, 2.1 80, 3.1 30\n```')
        self.assertEqual(rc, 0, text)
        ccs = [c[1:] for c in midi_ccs(out)]
        P = sheet_tool.PPQ
        self.assertEqual(ccs, [(0, 0, 74, 10), (4 * P, 0, 74, 80), (8 * P, 0, 74, 30)])

    def test_curve_per_point_overrides_the_block_curve(self):
        rc, text, out = self.render('```automation "A" part="Pad" cc=71 curve=linear\n1.1 0 step, 2.1 100\n```')
        self.assertEqual(rc, 0, text)
        self.assertEqual([c[1:] for c in midi_ccs(out)],
                         [(0, 0, 71, 0), (4 * sheet_tool.PPQ, 0, 71, 100)])

    def test_ease_in_stays_below_linear_and_ease_out_above(self):
        def mid(curve):
            return sheet_tool.curve_value(0, 100, 0.5, curve)
        self.assertLess(mid("ease-in"), mid("linear"))
        self.assertGreater(mid("ease-out"), mid("linear"))
        self.assertAlmostEqual(mid("smooth"), 50)
        for c in sheet_tool.CURVES:
            self.assertEqual(sheet_tool.curve_value(10, 90, 0.0, c), 10)

    def test_descending_ramp(self):
        rc, text, out = self.render('```automation "A" part="Pad" cc=74 curve=smooth\n1.1 120, 2.1 20\n```')
        self.assertEqual(rc, 0, text)
        values = [c[4] for c in midi_ccs(out)]
        self.assertEqual((values[0], values[-1]), (120, 20))
        self.assertEqual(values, sorted(values, reverse=True))

    def test_drum_part_uses_channel_10(self):
        rc, text, out = self.render('```automation "A" part="Beat" cc=7\n1.1 50, 2.1 100\n```')
        self.assertEqual(rc, 0, text)
        self.assertTrue(all(c[2] == 9 for c in midi_ccs(out)))

    def test_render_refuses_a_broken_automation(self):
        rc, text, out = self.render('```automation "A" part="Pad" cc=74\n1.1 20, 99.1 90\n```')
        self.assertEqual(rc, 1)
        self.assertIn("not rendering", text)
        self.assertFalse(os.path.exists(out))

    def test_values_stay_in_midi_range(self):
        for curve in sheet_tool.CURVES:
            rc, text, out = self.render(f'```automation "A" part="Pad" cc=74 curve={curve}\n1.1 0, 3.1 127, 6.1 0\n```')
            self.assertEqual(rc, 0, text)
            self.assertTrue(all(0 <= c[4] <= 127 for c in midi_ccs(out)))


if __name__ == "__main__":
    unittest.main()
