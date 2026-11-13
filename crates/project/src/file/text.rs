//! The tabular files: notes, automation points and placements, one item per line
//! (§ 2.3), and the packed form of a dense lane.
//!
//! The first line is a header (`# faulk-<kind> 1` then the columns); the rest is data,
//! sorted by the natural key, `\t`-separated, `\n`-terminated. Reading is strict about
//! what a line holds (numbers, column count) and lenient about order and the final
//! newline, because a line merge by git can interleave lines: the writer sorts again.

use super::error::FileError;
use crate::blob::Hash;
use crate::ids::{ClipId, DeviceId, PlacementId};
use crate::limits::{MAX_NOTES_PER_CLIP, MAX_PLACEMENTS_PER_TRACK, MAX_POINTS_PER_LANE};
use crate::model::*;
use engine::time::{Frame, Tick};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::str::FromStr;

const NOTES_HEADER: &str = "# faulk-notes 1";
const ARRANGEMENT_HEADER: &str = "# faulk-arrangement 1";
const LANE_HEADER: &str = "# faulk-automation 1";
/// The first bytes of a lane blob: the format and its version.
pub const LANE_BLOB_MAGIC: &[u8; 8] = b"FKAUTO01";
/// Bytes of one point in a packed lane: tick (i64), value (f64), shape (u8), little endian.
pub const POINT_BYTES: usize = 17;

fn num<T: FromStr>(
    path: &str,
    line: usize,
    what: &str,
    field: Option<&str>,
) -> Result<T, FileError> {
    let field = field.ok_or_else(|| FileError::corrupt(path, format!("line {line}: no {what}")))?;
    field
        .parse()
        .map_err(|_| FileError::corrupt(path, format!("line {line}: bad {what} {field:?}")))
}

fn finite(path: &str, line: usize, field: Option<&str>) -> Result<f64, FileError> {
    let v: f64 = num(path, line, "value", field)?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(FileError::corrupt(
            path,
            format!("line {line}: a value that is not finite"),
        ))
    }
}

/// The data lines of a file after its header: `(line number, text)`. A blank line inside
/// the data is corruption; the final newline is not a blank line.
struct Rows<'a> {
    path: &'a str,
    lines: std::iter::Peekable<std::str::Split<'a, char>>,
    n: usize,
}

impl<'a> Iterator for Rows<'a> {
    type Item = Result<(usize, &'a str), FileError>;

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.lines.next()?;
        self.n += 1;
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return if self.lines.peek().is_none() {
                None
            } else {
                Some(Err(FileError::corrupt(
                    self.path,
                    format!("line {}: blank line", self.n),
                )))
            };
        }
        Some(Ok((self.n, line)))
    }
}

fn data_lines<'a>(path: &'a str, text: &'a str) -> Result<(&'a str, Rows<'a>), FileError> {
    let mut lines = text.split('\n').peekable();
    let header = lines
        .next()
        .map(|h| h.strip_suffix('\r').unwrap_or(h))
        .ok_or_else(|| FileError::corrupt(path, "empty file"))?;
    Ok((header, Rows { path, lines, n: 1 }))
}

// ---------------------------------------------------------------------------- notes

/// The text of a clip's notes: the base columns, then `cents`, `chance`, `repeat` only
/// when one note of the clip has a non-default value in that column.
pub fn notes_tsv(notes: &BTreeSet<Note>) -> Vec<u8> {
    let cents = notes.iter().any(|n| n.cents != 0);
    let chance = notes.iter().any(|n| n.chance != Note::DEFAULT_CHANCE);
    let repeat = notes.iter().any(|n| n.repeat != Note::DEFAULT_REPEAT);
    let mut s = String::with_capacity(64 + notes.len() * 24);
    s.push_str(NOTES_HEADER);
    s.push_str("\tstart\tlength\tpitch\tvelocity");
    if cents {
        s.push_str("\tcents");
    }
    if chance {
        s.push_str("\tchance");
    }
    if repeat {
        s.push_str("\trepeat");
    }
    s.push('\n');
    for n in notes {
        let _ = write!(
            s,
            "{}\t{}\t{}\t{}",
            n.start.get(),
            n.length.get(),
            n.pitch,
            n.velocity
        );
        if cents {
            let _ = write!(s, "\t{}", n.cents);
        }
        if chance {
            let _ = write!(s, "\t{}", n.chance);
        }
        if repeat {
            let _ = write!(s, "\t{}", n.repeat);
        }
        s.push('\n');
    }
    s.into_bytes()
}

pub fn parse_notes(path: &str, text: &str) -> Result<BTreeSet<Note>, FileError> {
    let (header, rows) = data_lines(path, text)?;
    let columns: Vec<&str> = header
        .strip_prefix(NOTES_HEADER)
        .ok_or_else(|| FileError::corrupt(path, "not a notes file (bad header)"))?
        .split('\t')
        .skip(1)
        .collect();
    let (base, extra) = columns
        .split_at_checked(4)
        .ok_or_else(|| FileError::corrupt(path, "bad notes columns"))?;
    if base != ["start", "length", "pitch", "velocity"] {
        return Err(FileError::corrupt(path, "bad notes columns"));
    }
    // The optional columns, in this order, any subset.
    let order = ["cents", "chance", "repeat"];
    let mut last = None;
    for col in extra {
        let at = order
            .iter()
            .position(|c| c == col)
            .ok_or_else(|| FileError::corrupt(path, format!("unknown column {col:?}")))?;
        if last.is_some_and(|l| at <= l) {
            return Err(FileError::corrupt(path, "columns out of order or repeated"));
        }
        last = Some(at);
    }
    let mut notes = BTreeSet::new();
    let mut count = 0usize;
    for row in rows {
        let (n, line) = row?;
        count += 1;
        if count > MAX_NOTES_PER_CLIP {
            return Err(FileError::corrupt(path, "more notes than a clip can hold"));
        }
        let mut f = line.split('\t');
        let mut note = Note::new(Tick(num(path, n, "start", f.next())?), 0, Tick(0), 0);
        note.length = Tick(num(path, n, "length", f.next())?);
        note.pitch = num(path, n, "pitch", f.next())?;
        note.velocity = num(path, n, "velocity", f.next())?;
        for col in extra {
            match *col {
                "cents" => note.cents = num(path, n, "cents", f.next())?,
                "chance" => note.chance = num(path, n, "chance", f.next())?,
                _ => note.repeat = num(path, n, "repeat", f.next())?,
            }
        }
        if f.next().is_some() {
            return Err(FileError::corrupt(
                path,
                format!("line {n}: too many columns"),
            ));
        }
        if !notes.insert(note) {
            return Err(FileError::corrupt(
                path,
                format!("line {n}: the note is there twice"),
            ));
        }
    }
    Ok(notes)
}

// ---------------------------------------------------------------------- arrangement

pub fn arrangement_tsv(placements: &BTreeMap<crate::ids::PlacementId, Placement>) -> Vec<u8> {
    let mut rows: Vec<_> = placements.iter().collect();
    rows.sort_by(|a, b| (a.1.start, a.0).cmp(&(b.1.start, b.0)));
    let mut s = String::with_capacity(64 + rows.len() * 100);
    s.push_str(ARRANGEMENT_HEADER);
    s.push_str("\tid\tstart\tlength\tclip\toffset\n");
    for (id, p) in rows {
        let _ = write!(s, "{id}\t{}\t", p.start.get());
        match p.span {
            Span::Ticks { length, offset } => {
                let _ = write!(s, "{}\t{}\t{}", length.get(), p.clip, offset.get());
            }
            Span::Frames { length, offset } => {
                let _ = write!(s, "{}\t{}\t{}", length.get(), p.clip, offset.get());
            }
        }
        s.push('\n');
    }
    s.into_bytes()
}

pub fn parse_arrangement(
    path: &str,
    text: &str,
    kind: TrackKind,
) -> Result<BTreeMap<PlacementId, Placement>, FileError> {
    let (header, rows) = data_lines(path, text)?;
    if header != format!("{ARRANGEMENT_HEADER}\tid\tstart\tlength\tclip\toffset") {
        return Err(FileError::corrupt(
            path,
            "not an arrangement file (bad header)",
        ));
    }
    let mut out = BTreeMap::new();
    for row in rows {
        let (n, line) = row?;
        if out.len() >= MAX_PLACEMENTS_PER_TRACK {
            return Err(FileError::corrupt(
                path,
                "more placements than a track can hold",
            ));
        }
        let mut f = line.split('\t');
        let id = PlacementId::parse(f.next().unwrap_or(""))
            .map_err(|e| FileError::corrupt(path, format!("line {n}: {e}")))?;
        let start = Tick(num(path, n, "start", f.next())?);
        let length = f.next();
        let clip = ClipId::parse(f.next().unwrap_or(""))
            .map_err(|e| FileError::corrupt(path, format!("line {n}: {e}")))?;
        let offset = f.next();
        if f.next().is_some() {
            return Err(FileError::corrupt(
                path,
                format!("line {n}: too many columns"),
            ));
        }
        let span = match kind {
            TrackKind::Midi => Span::Ticks {
                length: Tick(num(path, n, "length", length)?),
                offset: Tick(num(path, n, "offset", offset)?),
            },
            TrackKind::Audio => Span::Frames {
                length: Frame(num(path, n, "length", length)?),
                offset: Frame(num(path, n, "offset", offset)?),
            },
        };
        if out.insert(id, Placement { clip, start, span }).is_some() {
            return Err(FileError::corrupt(
                path,
                format!("line {n}: the placement is there twice"),
            ));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------- automation

/// A lane as its file says it: the target and either the points, or the blob that holds
/// them (the points are then read from `blobs/`, or from the cache snapshot).
#[derive(Clone, Debug, PartialEq)]
pub struct LaneHeader {
    pub target: AutomationTarget,
    /// The blob and the number of points it holds, when the lane is packed.
    pub blob: Option<(Hash, usize)>,
}

pub fn target_text(target: &AutomationTarget) -> Result<String, String> {
    Ok(match target {
        AutomationTarget::Device { device, param } => {
            if param.contains(['\t', '\n', '\r']) {
                return Err("a parameter id with a tab or a line break".into());
            }
            format!("device:{device}/{param}")
        }
        AutomationTarget::Mixer {
            param: MixerParam::Gain,
        } => "mixer:gain".into(),
        AutomationTarget::Mixer {
            param: MixerParam::Pan,
        } => "mixer:pan".into(),
    })
}

pub fn parse_target_text(s: &str) -> Result<AutomationTarget, String> {
    parse_target("", s).map_err(|e| e.to_string())
}

fn parse_target(path: &str, s: &str) -> Result<AutomationTarget, FileError> {
    match s {
        "mixer:gain" => Ok(AutomationTarget::Mixer {
            param: MixerParam::Gain,
        }),
        "mixer:pan" => Ok(AutomationTarget::Mixer {
            param: MixerParam::Pan,
        }),
        _ => {
            let rest = s
                .strip_prefix("device:")
                .ok_or_else(|| FileError::corrupt(path, "unknown automation target"))?;
            let (device, param) = rest
                .split_once('/')
                .ok_or_else(|| FileError::corrupt(path, "an automation target has no parameter"))?;
            Ok(AutomationTarget::Device {
                device: DeviceId::parse(device)
                    .map_err(|e| FileError::corrupt(path, format!("target: {e}")))?,
                param: param.to_owned(),
            })
        }
    }
}

fn header_text(target: &AutomationTarget, blob: Option<(&Hash, usize)>) -> Result<String, String> {
    let mut s = format!("{LANE_HEADER}\ttarget={}", target_text(target)?);
    if let Some((hash, n)) = blob {
        let _ = write!(s, "\tblob={hash}\tpoints={n}");
    }
    s.push('\n');
    Ok(s)
}

/// A lane's text file: its points inline.
pub fn lane_tsv(
    target: &AutomationTarget,
    points: &BTreeMap<Tick, AutomationPoint>,
) -> Result<Vec<u8>, String> {
    let mut s = header_text(target, None)?;
    s.reserve(points.len() * 20);
    for (tick, p) in points {
        if !p.value.is_finite() {
            return Err("an automation value that is not finite".into());
        }
        let _ = write!(s, "{}\t", tick.get());
        super::canon::float(p.value, &mut s);
        s.push('\t');
        s.push_str(p.shape.keyword());
        s.push('\n');
    }
    Ok(s.into_bytes())
}

/// A lane's text file when its points are in a blob: only the header, naming the blob.
pub fn lane_blob_header(
    target: &AutomationTarget,
    hash: &Hash,
    points: usize,
) -> Result<Vec<u8>, String> {
    Ok(header_text(target, Some((hash, points)))?.into_bytes())
}

/// Reads a lane file. With a blob named in its header the points are not in the file:
/// the returned map is empty.
pub fn parse_lane(
    path: &str,
    text: &str,
) -> Result<(LaneHeader, BTreeMap<Tick, AutomationPoint>), FileError> {
    let (header, rows) = data_lines(path, text)?;
    let mut fields = header
        .strip_prefix(LANE_HEADER)
        .and_then(|h| h.strip_prefix('\t'))
        .ok_or_else(|| FileError::corrupt(path, "not an automation file (bad header)"))?
        .split('\t');
    let target = parse_target(
        path,
        fields
            .next()
            .and_then(|f| f.strip_prefix("target="))
            .ok_or_else(|| FileError::corrupt(path, "no target"))?,
    )?;
    let blob = match fields.next() {
        None => None,
        Some(f) => {
            let hex = f
                .strip_prefix("blob=")
                .ok_or_else(|| FileError::corrupt(path, "unknown header field"))?;
            let hash = Hash::parse(hex).map_err(|e| FileError::corrupt(path, e))?;
            let n: usize = fields
                .next()
                .and_then(|f| f.strip_prefix("points="))
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| FileError::corrupt(path, "no point count"))?;
            if n > MAX_POINTS_PER_LANE {
                return Err(FileError::corrupt(path, "more points than a lane can hold"));
            }
            Some((hash, n))
        }
    };
    if fields.next().is_some() {
        return Err(FileError::corrupt(path, "too many header fields"));
    }
    let mut points = BTreeMap::new();
    for row in rows {
        let (n, line) = row?;
        if blob.is_some() {
            return Err(FileError::corrupt(
                path,
                "a lane in a blob has no points of its own",
            ));
        }
        if points.len() >= MAX_POINTS_PER_LANE {
            return Err(FileError::corrupt(path, "more points than a lane can hold"));
        }
        let mut f = line.split('\t');
        let tick = Tick(num(path, n, "tick", f.next())?);
        let value = finite(path, n, f.next())?;
        let Some(shape) = f.next().and_then(Shape::from_keyword) else {
            return Err(FileError::corrupt(path, format!("line {n}: bad shape")));
        };
        if f.next().is_some() {
            return Err(FileError::corrupt(
                path,
                format!("line {n}: too many columns"),
            ));
        }
        if points
            .insert(tick, AutomationPoint { value, shape })
            .is_some()
        {
            return Err(FileError::corrupt(
                path,
                format!("line {n}: two points on tick {}", tick.get()),
            ));
        }
    }
    Ok((LaneHeader { target, blob }, points))
}

/// The packed records of a lane (17 bytes a point), without the blob's magic.
pub fn pack_points(points: &BTreeMap<Tick, AutomationPoint>, out: &mut Vec<u8>) {
    out.reserve(points.len() * POINT_BYTES);
    for (tick, p) in points {
        out.extend_from_slice(&tick.get().to_le_bytes());
        let v = if p.value == 0.0 { 0.0 } else { p.value };
        out.extend_from_slice(&v.to_le_bytes());
        out.push(p.shape.byte());
    }
}

/// The bytes of a lane's blob: magic, then the records.
pub fn lane_blob(points: &BTreeMap<Tick, AutomationPoint>) -> Vec<u8> {
    let mut v = Vec::with_capacity(LANE_BLOB_MAGIC.len() + points.len() * POINT_BYTES);
    v.extend_from_slice(LANE_BLOB_MAGIC);
    pack_points(points, &mut v);
    v
}

/// Reads packed records: whole records, finite values, a shape byte of 0 to 4 (`Shape::byte`), strictly
/// increasing ticks.
pub fn unpack_points(bytes: &[u8]) -> Result<BTreeMap<Tick, AutomationPoint>, String> {
    let (records, rest) = bytes.as_chunks::<POINT_BYTES>();
    if !rest.is_empty() {
        return Err("not a whole number of points".into());
    }
    if records.len() > MAX_POINTS_PER_LANE {
        return Err("more points than a lane can hold".into());
    }
    let mut out = BTreeMap::new();
    let mut last: Option<i64> = None;
    for r in records {
        let tick = i64::from_le_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]);
        let value = f64::from_le_bytes([r[8], r[9], r[10], r[11], r[12], r[13], r[14], r[15]]);
        if !value.is_finite() {
            return Err("a value that is not finite".into());
        }
        let Some(shape) = Shape::from_byte(r[16]) else {
            return Err("a bad shape".into());
        };
        if last.is_some_and(|l| tick <= l) {
            return Err("ticks are not increasing".into());
        }
        last = Some(tick);
        out.insert(Tick(tick), AutomationPoint { value, shape });
    }
    Ok(out)
}

/// Reads a lane blob.
pub fn parse_lane_blob(bytes: &[u8]) -> Result<BTreeMap<Tick, AutomationPoint>, String> {
    let records = bytes
        .strip_prefix(LANE_BLOB_MAGIC.as_slice())
        .ok_or("not a lane blob (bad magic)")?;
    unpack_points(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: i64) -> Note {
        Note::new(Tick(start), 60, Tick(10), 100)
    }

    #[test]
    fn notes_write_only_the_columns_in_use() {
        let mut notes = BTreeSet::from([note(0), note(480)]);
        assert_eq!(
            String::from_utf8(notes_tsv(&notes)).unwrap(),
            "# faulk-notes 1\tstart\tlength\tpitch\tvelocity\n0\t10\t60\t100\n480\t10\t60\t100\n"
        );
        notes.insert(Note {
            chance: 50,
            ..note(960)
        });
        let text = String::from_utf8(notes_tsv(&notes)).unwrap();
        assert!(text.starts_with("# faulk-notes 1\tstart\tlength\tpitch\tvelocity\tchance\n"));
        assert!(text.contains("\n0\t10\t60\t100\t100\n"));
        assert_eq!(parse_notes("n", &text).unwrap(), notes);
    }

    #[test]
    fn notes_refuse_what_is_not_a_note() {
        let h = "# faulk-notes 1\tstart\tlength\tpitch\tvelocity\n";
        for bad in [
            "".to_owned(),
            "# faulk-notes 2\tstart\tlength\tpitch\tvelocity\n".to_owned(),
            "# faulk-notes 1\tstart\tlength\tpitch\n".to_owned(),
            "# faulk-notes 1\tstart\tlength\tpitch\tvelocity\trepeat\tcents\n".to_owned(),
            format!("{h}0\t10\t60\n"),
            format!("{h}0\t10\t60\t100\t1\n"),
            format!("{h}0\t10\t300\t100\n"),
            format!("{h}x\t10\t60\t100\n"),
            format!("{h}0\t10\t60\t100\n0\t10\t60\t100\n"),
            format!("{h}0\t10\t60\t100\n\n1\t10\t60\t100\n"),
        ] {
            assert!(
                matches!(parse_notes("n", &bad), Err(FileError::Corrupt { .. })),
                "{bad:?}"
            );
        }
        // Lenient: no final newline, CRLF, unsorted lines.
        let ok = format!("{h}5\t10\t60\t100\r\n0\t10\t60\t100");
        assert_eq!(parse_notes("n", &ok).unwrap().len(), 2);
    }

    #[test]
    fn lane_targets_round_trip() {
        let mut ids = crate::IdGenerator::for_tests();
        for target in [
            AutomationTarget::Mixer {
                param: MixerParam::Gain,
            },
            AutomationTarget::Mixer {
                param: MixerParam::Pan,
            },
            AutomationTarget::Device {
                device: DeviceId::generate(&mut ids),
                param: "cutoff.hz".into(),
            },
        ] {
            let points: BTreeMap<_, _> = Shape::ALL
                .into_iter()
                .enumerate()
                .map(|(i, shape)| {
                    (
                        Tick(i as i64 * 10),
                        AutomationPoint {
                            value: 0.1 * i as f64,
                            shape,
                        },
                    )
                })
                .collect();
            let bytes = lane_tsv(&target, &points).unwrap();
            let text = std::str::from_utf8(&bytes).unwrap();
            for word in [
                "\tlinear\n",
                "\tstep\n",
                "\tease_in\n",
                "\tease_out\n",
                "\tsmooth\n",
            ] {
                assert!(text.contains(word), "{word:?} in {text}");
            }
            assert_eq!(parse_lane_blob(&lane_blob(&points)).unwrap(), points);
            let (header, back) = parse_lane("l", std::str::from_utf8(&bytes).unwrap()).unwrap();
            assert_eq!(header.target, target);
            assert_eq!(header.blob, None);
            assert_eq!(back, points);
        }
        let bad = AutomationTarget::Device {
            device: DeviceId::generate(&mut ids),
            param: "a\tb".into(),
        };
        assert!(lane_tsv(&bad, &BTreeMap::new()).is_err());
    }

    #[test]
    fn packed_points_refuse_garbage() {
        let points = BTreeMap::from([
            (
                Tick(1),
                AutomationPoint {
                    value: -0.0,
                    shape: Shape::Linear,
                },
            ),
            (
                Tick(5),
                AutomationPoint {
                    value: 2.5,
                    shape: Shape::Step,
                },
            ),
        ]);
        let blob = lane_blob(&points);
        let back = parse_lane_blob(&blob).unwrap();
        assert_eq!(back.len(), 2);
        assert!(parse_lane_blob(&blob[..blob.len() - 1]).is_err());
        assert!(parse_lane_blob(b"nope").is_err());
        let mut nan = blob.clone();
        nan[8 + 8..8 + 16].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(parse_lane_blob(&nan).is_err());
        let mut shape = blob.clone();
        shape[8 + 16] = 5;
        assert!(parse_lane_blob(&shape).is_err());
        let h = format!("{LANE_HEADER}\ttarget=mixer:gain\n");
        for bad in ["ease-in", "Linear", "curve", ""] {
            let text = format!("{h}0\t0.5\t{bad}\n");
            assert!(
                matches!(parse_lane("l", &text), Err(FileError::Corrupt { .. })),
                "{bad:?}"
            );
        }
        let mut order = blob;
        order[8..16].copy_from_slice(&9i64.to_le_bytes());
        assert!(parse_lane_blob(&order).is_err());
    }
}
