//! The end padding of the Opus tracks of a Matroska or WebM file, from its `DiscardPadding`.
//!
//! An Opus encoder pads the end of a stream to a whole packet; a WebM file names the padding in
//! the `DiscardPadding` element of the last block (nanoseconds; FFmpeg writes it, a browser's
//! recorder, which stops on a whole packet, writes none). symphonia 0.6 reads the element but
//! drops it, so this module finds it and [`crate::Decoding`] cuts the frames from the last
//! block itself, as it does for an MP4's edit list (`mp4`): an imported WebM ends on its last
//! real sample, like an Ogg Opus file (whose last granule position symphonia applies).
//!
//! The walk reads the element headers of the whole file once (through a 64 KiB buffer: the
//! page's reads are calls into JavaScript), keeping, for each `A_OPUS` track, the padding of
//! its last block (0 when that block has none). It stops, giving nothing, on anything it does
//! not expect: a size past the end, an element of unknown size other than a Segment or a
//! Cluster, a second EBML header, a padding element out of its block group. Nothing found
//! means nothing cut: the file still imports, with its padding.

use std::io::{Read, Seek, SeekFrom};

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const CLUSTER: u32 = 0x1F43_B675;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const CODEC_ID: u32 = 0x86;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const DISCARD_PADDING: u32 = 0x75A2;

/// Bytes read from the file at once.
const BUFFER: usize = 1 << 16;
/// The largest element whose body is read whole (a track entry, a block group's head).
const MAX_BODY: u64 = 1 << 20;

/// The padding of each Opus track's last block, by track number, in frames at 48 kHz.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Paddings(Vec<(u64, u64)>);

impl Paddings {
    /// The frames to cut at the end of the track `number` (0 when the file says none).
    pub fn of(&self, number: u64) -> u64 {
        self.0
            .iter()
            .find(|(n, _)| *n == number)
            .map_or(0, |(_, frames)| *frames)
    }

    fn set(&mut self, number: u64, frames: u64) {
        if let Some(entry) = self.0.iter_mut().find(|(n, _)| *n == number) {
            entry.1 = frames;
        }
    }
}

/// The paddings of a file that starts with an EBML header; `None` for any other file, or one
/// whose walk stopped on something it does not expect.
pub fn paddings(r: &mut (impl Read + Seek), len: u64) -> Option<Paddings> {
    let mut r = Reader::new(r, len);
    let (id, size) = r.header()?;
    if id != EBML {
        return None;
    }
    r.skip(size?)?;
    let mut found = Paddings::default();
    while r.at < len {
        let (id, size) = r.header()?;
        match id {
            // Containers whose children follow at once (their size may be unknown: a live
            // recording's segment and clusters).
            SEGMENT | CLUSTER => {}
            TRACKS => {
                let end = r.at.checked_add(size?)?;
                while r.at < end {
                    let (id, size) = r.header()?;
                    let size = size?;
                    if id == TRACK_ENTRY {
                        if let Some(number) = opus_track(&r.body(size)?) {
                            found.0.push((number, 0));
                        }
                    } else {
                        r.skip(size)?;
                    }
                }
            }
            SIMPLE_BLOCK => {
                let size = size?;
                let body = r.body(size.min(8))?;
                r.skip(size - size.min(8))?;
                found.set(vint(&body)?.0, 0);
            }
            BLOCK_GROUP => {
                // `body` refuses a group over `MAX_BODY` (a block of audio is a few kilobytes).
                let (track, padding) = block_group(&r.body(size?)?)?;
                found.set(track, padding);
            }
            EBML => return None,
            _ => r.skip(size?)?,
        }
    }
    Some(found)
}

/// The number of a track entry whose codec is Opus.
fn opus_track(entry: &[u8]) -> Option<u64> {
    let (mut number, mut opus) = (None, false);
    let mut at = 0;
    while at < entry.len() {
        let (id, n) = element_id(entry.get(at..)?)?;
        at += n;
        let (size, n) = vint(entry.get(at..)?)?;
        at += n;
        let body = entry.get(at..at.checked_add(usize::try_from(size).ok()?)?)?;
        at += body.len();
        match id {
            TRACK_NUMBER => number = Some(uint(body)?),
            CODEC_ID => opus = body.split(|b| *b == 0).next() == Some(b"A_OPUS".as_slice()),
            _ => {}
        }
    }
    number.filter(|_| opus)
}

/// The track of a block group's block and the padding its `DiscardPadding` names, in frames.
fn block_group(group: &[u8]) -> Option<(u64, u64)> {
    let (mut track, mut padding) = (None, 0);
    let mut at = 0;
    while at < group.len() {
        let (id, n) = element_id(group.get(at..)?)?;
        at += n;
        let (size, n) = vint(group.get(at..)?)?;
        at += n;
        let size = usize::try_from(size).ok()?;
        // The block's samples may run past the part read: only its first bytes are needed.
        let body = group.get(at..at.saturating_add(size).min(group.len()))?;
        at = at.saturating_add(size);
        match id {
            BLOCK => track = Some(vint(body)?.0),
            DISCARD_PADDING => padding = frames(int(body)?),
            _ => {}
        }
    }
    Some((track?, padding))
}

/// Nanoseconds of padding as frames at 48 kHz, to the nearest; a negative padding (at the
/// start, by the specification) is not an end padding.
fn frames(ns: i64) -> u64 {
    let ns = u128::try_from(ns).unwrap_or(0);
    u64::try_from((ns * 48_000 + 500_000_000) / 1_000_000_000).unwrap_or(u64::MAX)
}

/// An element ID (its length marker kept, as the specification writes IDs) and its length.
fn element_id(b: &[u8]) -> Option<(u32, usize)> {
    let first = *b.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 4 {
        return None;
    }
    let mut id = 0u32;
    for byte in b.get(..len)? {
        id = id << 8 | u32::from(*byte);
    }
    Some((id, len))
}

/// A variable-size integer (the length marker dropped) and its length; all ones is the
/// "unknown size" of the specification, given as `u64::MAX`.
fn vint(b: &[u8]) -> Option<(u64, usize)> {
    let first = *b.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 {
        return None;
    }
    let mut value = u64::from(first) & (0xFF >> len);
    let mut ones = value == 0xFF >> len;
    for byte in b.get(1..len)? {
        value = value << 8 | u64::from(*byte);
        ones &= *byte == 0xFF;
    }
    Some((if ones { u64::MAX } else { value }, len))
}

fn uint(b: &[u8]) -> Option<u64> {
    (b.len() <= 8).then(|| b.iter().fold(0u64, |v, x| v << 8 | u64::from(*x)))
}

fn int(b: &[u8]) -> Option<i64> {
    if b.is_empty() || b.len() > 8 {
        return (b.is_empty()).then_some(0);
    }
    let shift = 64 - 8 * b.len() as u32;
    Some(((uint(b)? << shift) as i64) >> shift)
}

/// The file through a buffer, at a position `at`.
struct Reader<'a, R> {
    inner: &'a mut R,
    len: u64,
    buf: Vec<u8>,
    /// The file offset of `buf[0]`.
    start: u64,
    at: u64,
}

impl<'a, R: Read + Seek> Reader<'a, R> {
    fn new(inner: &'a mut R, len: u64) -> Self {
        Reader {
            inner,
            len,
            buf: Vec::new(),
            start: 0,
            at: 0,
        }
    }

    /// Makes the buffer hold `at..at + n` (or up to the end of the file).
    fn fill(&mut self, n: usize) -> Option<()> {
        let end = self.start + self.buf.len() as u64;
        // Near the end of the file the buffer holds all there is: no read again for it.
        if self.at >= self.start && (self.at + n as u64).min(self.len) <= end {
            return Some(());
        }
        self.inner.seek(SeekFrom::Start(self.at)).ok()?;
        self.buf.resize(BUFFER.max(n), 0);
        let mut got = 0;
        while got < self.buf.len() {
            match self.inner.read(&mut self.buf[got..]) {
                Ok(0) => break,
                Ok(k) => got += k,
                Err(_) => return None,
            }
        }
        self.buf.truncate(got);
        self.start = self.at;
        Some(())
    }

    /// The bytes from the position, at most `n` (fewer at the end of the file).
    fn peek(&mut self, n: usize) -> Option<&[u8]> {
        self.fill(n)?;
        let from = usize::try_from(self.at - self.start).ok()?;
        Some(&self.buf[from..(from + n).min(self.buf.len())])
    }

    /// The next element's ID and size (`None` for an unknown size); refused past the end.
    fn header(&mut self) -> Option<(u32, Option<u64>)> {
        let head = self.peek(12)?;
        let (id, a) = element_id(head)?;
        let (size, b) = vint(head.get(a..)?)?;
        self.at += (a + b) as u64;
        let size = (size != u64::MAX).then_some(size);
        if size.is_some_and(|s| s > self.len.saturating_sub(self.at)) {
            return None;
        }
        Some((id, size))
    }

    fn skip(&mut self, n: u64) -> Option<()> {
        self.at = self.at.checked_add(n).filter(|end| *end <= self.len)?;
        Some(())
    }

    /// The next `n` bytes, which the position passes.
    fn body(&mut self, n: u64) -> Option<Vec<u8>> {
        if n > MAX_BODY {
            return None;
        }
        let n = usize::try_from(n).ok()?;
        let bytes = self.peek(n)?.to_vec();
        if bytes.len() < n {
            return None;
        }
        self.at += n as u64;
        Some(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// An element: its ID bytes, an 8-byte size (or unknown), its body.
    fn el(id: u32, body: &[u8]) -> Vec<u8> {
        let mut b = id_bytes(id);
        b.push(0x01);
        b.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
        b.extend_from_slice(body);
        b
    }

    fn unknown(id: u32) -> Vec<u8> {
        let mut b = id_bytes(id);
        b.extend_from_slice(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        b
    }

    fn id_bytes(id: u32) -> Vec<u8> {
        id.to_be_bytes()
            .into_iter()
            .skip_while(|b| *b == 0)
            .collect()
    }

    fn track(number: u8, codec: &[u8]) -> Vec<u8> {
        el(
            TRACK_ENTRY,
            &[el(TRACK_NUMBER, &[number]), el(CODEC_ID, codec)].concat(),
        )
    }

    fn simple(track: u8) -> Vec<u8> {
        el(SIMPLE_BLOCK, &[0x80 | track, 0, 0, 0x80, 0xfc, 1, 2])
    }

    fn group(track: u8, padding_ns: Option<i64>) -> Vec<u8> {
        let mut body = el(BLOCK, &[0x80 | track, 0, 0, 0, 0xfc, 1, 2]);
        if let Some(ns) = padding_ns {
            body.extend(el(DISCARD_PADDING, &ns.to_be_bytes()));
        }
        el(BLOCK_GROUP, &body)
    }

    fn file(segment: Vec<u8>, known: bool) -> Vec<u8> {
        let mut b = el(EBML, &el(0x4282, b"webm"));
        if known {
            b.extend(el(SEGMENT, &segment));
        } else {
            b.extend(unknown(SEGMENT));
            b.extend(segment);
        }
        b
    }

    fn walk(bytes: &[u8]) -> Option<Paddings> {
        paddings(&mut Cursor::new(bytes), bytes.len() as u64)
    }

    #[test]
    fn the_last_block_of_an_opus_track_gives_its_padding() {
        let tracks = el(TRACKS, &[track(1, b"A_OPUS"), track(2, b"V_VP9")].concat());
        let cluster = el(
            CLUSTER,
            &[simple(1), simple(2), group(1, Some(13_500_000)), simple(2)].concat(),
        );
        for known in [true, false] {
            let found = walk(&file([tracks.clone(), cluster.clone()].concat(), known)).unwrap();
            assert_eq!(found.of(1), 648, "known size {known}");
            assert_eq!(found.of(2), 0);
        }
        // A later block without a padding: the padding was not the end's.
        let later = el(CLUSTER, &[group(1, Some(13_500_000)), simple(1)].concat());
        assert_eq!(
            walk(&file([tracks.clone(), later].concat(), true))
                .unwrap()
                .of(1),
            0
        );
        // Clusters of unknown size, one after the other.
        let mut segment = tracks.clone();
        segment.extend(unknown(CLUSTER));
        segment.extend(simple(1));
        segment.extend(unknown(CLUSTER));
        segment.extend(group(1, Some(1_000_000)));
        assert_eq!(walk(&file(segment, false)).unwrap().of(1), 48);
        // A negative padding is not an end padding.
        let negative = el(CLUSTER, &group(1, Some(-1_000_000)));
        assert_eq!(
            walk(&file([tracks, negative].concat(), true))
                .unwrap()
                .of(1),
            0
        );
    }

    #[test]
    fn anything_unexpected_gives_nothing_and_never_panics() {
        assert_eq!(walk(b""), None);
        assert_eq!(walk(b"RIFF\0\0\0\0WAVE"), None);
        let tracks = el(TRACKS, &track(1, b"A_OPUS"));
        let whole = file(
            [tracks.clone(), el(CLUSTER, &group(1, Some(13_500_000)))].concat(),
            true,
        );
        // Cut anywhere: nothing, or the paddings of what is whole.
        for cut in 0..whole.len() {
            let _ = walk(&whole[..cut]);
        }
        // A size past the end.
        let mut past = whole.clone();
        past.extend(el(0xEC, &[]));
        let n = past.len();
        past[n - 2] = 0x40;
        assert_eq!(walk(&past), None);
        // A second EBML header (two files one after the other).
        let twice = [whole.clone(), whole.clone()].concat();
        assert_eq!(walk(&twice), None);
        // An unknown size on an element that is not a segment or a cluster.
        let mut odd = whole.clone();
        odd.extend(unknown(0xEC));
        assert_eq!(walk(&odd), None);
        // Every byte flipped in turn.
        for at in 0..whole.len() {
            for bit in 0..8 {
                let mut flipped = whole.clone();
                flipped[at] ^= 1 << bit;
                let _ = walk(&flipped);
            }
        }
    }

    /// The cost of the walk on a long file: `FAULK_WEBM=<file> cargo test -p audio-import
    /// --release --lib webm::tests::cost -- --ignored --nocapture` (#609: a one-hour stereo
    /// WebM of 62 MB made by FFmpeg: 17.5 ms in release on 2026-11-05, 944 reads of 64 KiB).
    #[test]
    #[ignore = "needs a long WebM file (FAULK_WEBM)"]
    fn cost() {
        struct Counting(std::fs::File, u32);
        impl Read for Counting {
            fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
                self.1 += 1;
                self.0.read(b)
            }
        }
        impl Seek for Counting {
            fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
                self.0.seek(p)
            }
        }
        let path = std::env::var("FAULK_WEBM").expect("FAULK_WEBM names a WebM file");
        let mut file = Counting(std::fs::File::open(path).unwrap(), 0);
        let len = file.0.metadata().unwrap().len();
        let t = std::time::Instant::now();
        let found = paddings(&mut file, len);
        println!("{found:?} in {:?}, {} reads", t.elapsed(), file.1);
    }

    #[test]
    fn integers_read_as_the_specification_writes_them() {
        assert_eq!(vint(&[0x81]), Some((1, 1)));
        assert_eq!(vint(&[0x40, 0x02]), Some((2, 2)));
        assert_eq!(vint(&[0xFF]), Some((u64::MAX, 1)));
        assert_eq!(
            vint(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
            Some((u64::MAX, 8))
        );
        assert_eq!(vint(&[0x00, 1]), None);
        assert_eq!(vint(&[0x40]), None);
        assert_eq!(element_id(&[0x1A, 0x45, 0xDF, 0xA3]), Some((EBML, 4)));
        assert_eq!(element_id(&[0x08, 0, 0, 0, 0]), None);
        assert_eq!(int(&[0xFF]), Some(-1));
        assert_eq!(int(&[]), Some(0));
        assert_eq!(int(&[0x00, 0xcd, 0xfe, 0x60]), Some(13_500_000));
        assert_eq!(frames(13_500_000), 648);
        assert_eq!(frames(10_000), 0);
        assert_eq!(frames(20_834), 1);
        assert_eq!(frames(i64::MAX), 442_721_857_769_029);
    }
}
