//! `.faulkpack`: a zip of the project folder, for export and sharing only (§ 2.1), never
//! the working format.
//!
//! The archive holds the project's files (not `.faulk/`, not git's), and the blobs the
//! project refers to (not the ones left over by old versions). It is deterministic: the
//! entries are sorted, their dates fixed. Import is strict, since a pack comes from
//! anyone: only the paths of the format, no entry twice, every length counted as it is
//! produced (a header that lies about its size is no bomb), every blob checked against its
//! name, and the result opened and validated; on any failure nothing is left behind.

use super::MAX_TEXT_FILE_BYTES;
use super::error::FileError;
use super::fs::FileSystem;
use super::read::{OpenOptions, Opened, open, referenced_blobs, text_path};
use super::unit::{GITATTRIBUTES_FILE, GITIGNORE_FILE, PROJECT_FILE, blob_file};
use crate::blob::Hash;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::io::{self, Read, Seek, SeekFrom, Write};
use zip::read::{ArchiveOffset, Config};
use zip::write::{SimpleFileOptions, StreamWriter};
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

/// What an import accepts.
#[derive(Clone, Copy, Debug)]
pub struct PackLimits {
    pub max_entries: usize,
    /// The most bytes of a text file.
    pub max_text: u64,
    /// The most bytes of a blob.
    pub max_blob: u64,
    /// The most bytes of everything.
    pub max_total: u64,
    /// The most bytes of the project's own files (everything but the blobs), which a reader
    /// holds to open the project.
    pub max_text_total: u64,
}

impl Default for PackLimits {
    fn default() -> Self {
        Self {
            max_entries: 1_000_000,
            max_text: MAX_TEXT_FILE_BYTES,
            max_blob: 2 << 30,
            max_total: 16 << 30,
            max_text_total: 1 << 30,
        }
    }
}

/// What an export did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackReport {
    pub files: usize,
    /// Blobs the project refers to that the folder does not have: not in the pack.
    pub missing_blobs: Vec<Hash>,
}

fn bad(message: impl std::fmt::Display) -> FileError {
    FileError::BadPack {
        message: message.to_string(),
    }
}

/// Whether a path may appear in a pack.
fn pack_path(path: &str) -> bool {
    if path == PROJECT_FILE || path == GITATTRIBUTES_FILE || path == GITIGNORE_FILE {
        return true;
    }
    if let Some(hex) = path.strip_prefix("blobs/") {
        return Hash::parse(hex).is_ok();
    }
    text_path(path)
}

/// What an export will put in the pack, found before any byte is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackPlan {
    /// The files of the pack, in the order they are written (sorted).
    pub paths: Vec<String>,
    /// Blobs the project refers to that the folder does not have: not in the pack.
    pub missing_blobs: Vec<Hash>,
}

/// Opens (and so validates) the project in `fs` and lists the files its pack holds. An
/// invalid project is not exported. Reads the project's own files, never a blob's bytes: a
/// blob only has to exist (`FileSystem::exists`), so `fs` may know a blob by its name and size
/// alone.
pub fn plan_pack(fs: &dyn FileSystem) -> Result<PackPlan, FileError> {
    let opened = open(fs, &OpenOptions::default())?;
    let mut paths: BTreeSet<String> = opened.state.known_files().map(str::to_owned).collect();
    for support in [GITATTRIBUTES_FILE, GITIGNORE_FILE] {
        if fs.exists(support) {
            paths.insert(support.to_owned());
        }
    }
    let mut blobs: BTreeSet<Hash> = referenced_blobs(&opened.project);
    blobs.extend(opened.state.blob_lanes.values().copied());
    let mut missing = Vec::new();
    for hash in blobs {
        if fs.exists(&blob_file(&hash)) {
            paths.insert(blob_file(&hash));
        } else {
            missing.push(hash);
        }
    }
    Ok(PackPlan {
        paths: paths.into_iter().collect(),
        missing_blobs: missing,
    })
}

/// A file of this size or more is written with the 64-bit zip fields.
const LARGE_FILE_BYTES: u64 = 1 << 30;

/// Writes a pack one file at a time, in whatever pieces they come in, to a writer that need not
/// seek: the memory it holds is the compressor's buffers and the central directory, whatever the
/// size of the files. The same files cut in the same pieces give the same bytes (the compressor may
/// cut its blocks where the pieces end, so other pieces can give other bytes, never other files).
pub struct PackWriter<W: Write> {
    zip: ZipWriter<StreamWriter<W>>,
}

impl<W: Write> PackWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            zip: ZipWriter::new_stream(out),
        }
    }

    /// Starts the file `path`, whose `size` the caller knows (the next [`write`](Self::write)s
    /// add up to it). The previous file is closed.
    pub fn start(&mut self, path: &str, size: u64) -> Result<(), FileError> {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(DateTime::default())
            .unix_permissions(0o644)
            .large_file(size >= LARGE_FILE_BYTES);
        self.zip.start_file(path, options).map_err(bad)
    }

    /// Adds `bytes` to the file started last.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), FileError> {
        self.zip.write_all(bytes).map_err(bad)
    }

    /// Closes the last file, writes the directory and gives the writer back.
    pub fn finish(self) -> Result<W, FileError> {
        let out = self.zip.finish().map_err(bad)?;
        Ok(out.into_inner())
    }
}

/// Writes the pack of the project in `fs` into `out` (see [`plan_pack`]), reading one file at
/// a time.
pub fn export_pack<W: Write>(fs: &dyn FileSystem, out: W) -> Result<PackReport, FileError> {
    let plan = plan_pack(fs)?;
    let mut pack = PackWriter::new(out);
    for path in &plan.paths {
        let bytes = fs.read(path).map_err(|e| FileError::io(path, e))?;
        pack.start(path, bytes.len() as u64)?;
        pack.write(&bytes)?;
    }
    pack.finish()?;
    Ok(PackReport {
        files: plan.paths.len(),
        missing_blobs: plan.missing_blobs,
    })
}

/// Reads a pack into the empty folder `dest` and opens it, through a [`PackReader`] (the one
/// rule for a pack, whoever reads it). Each file is written to `dest` whole. On any failure the
/// files written are removed again.
pub fn import_pack<R: Read + Seek>(
    mut input: R,
    dest: &mut dyn FileSystem,
    limits: &PackLimits,
) -> Result<Opened, FileError> {
    if !dest.list("").map_err(|e| FileError::io("", e))?.is_empty() {
        return Err(FileError::DestinationNotEmpty);
    }
    let len = input.seek(SeekFrom::End(0)).map_err(bad)?;
    let mut reader = PackReader::new(len, *limits);
    let mut written: Vec<String> = Vec::new();
    let result = extract(&mut input, &mut reader, dest, &mut written)
        .and_then(|()| open(dest, &OpenOptions::default()));
    if result.is_err() {
        for path in &written {
            let _ = dest.remove(path);
        }
    }
    result
}

fn extract<R: Read + Seek>(
    input: &mut R,
    reader: &mut PackReader,
    dest: &mut dyn FileSystem,
    written: &mut Vec<String>,
) -> Result<(), FileError> {
    let mut given = Vec::new();
    let mut file: Option<(String, Vec<u8>)> = None;
    if let Some((offset, len)) = reader.wants() {
        input.seek(SeekFrom::Start(offset)).map_err(bad)?;
        given.resize(len as usize, 0);
        input.read_exact(&mut given).map_err(bad)?;
    }
    loop {
        let step = reader.step(&given)?;
        given.clear();
        match step {
            PackStep::Need { offset, len } => {
                input.seek(SeekFrom::Start(offset)).map_err(bad)?;
                given.resize(len as usize, 0);
                input.read_exact(&mut given).map_err(bad)?;
            }
            PackStep::Start { path, .. } => file = Some((path, Vec::new())),
            PackStep::Bytes(bytes) => {
                if let Some((_, body)) = file.as_mut() {
                    body.extend_from_slice(&bytes);
                }
            }
            PackStep::End => {
                if let Some((path, body)) = file.take() {
                    dest.write(&path, &body)
                        .map_err(|e| FileError::io(&path, e))?;
                    written.push(path);
                }
            }
            PackStep::Done => return Ok(()),
        }
    }
}

/// The most bytes a [`PackReader`] asks for at once (but for the directory) and gives at once.
pub const PACK_PIECE_BYTES: u64 = 1 << 20;

/// The end of the pack read first: the end of directory record, its comment (64 KiB at most)
/// and the zip64 locator and record.
const TAIL_BYTES: u64 = (1 << 16) + 22 + 20 + 56;

/// The most bytes of directory a reader holds.
const MAX_DIRECTORY_BYTES: u64 = 64 << 20;

/// The tries at the directory: the tail, then what the directory says it needs.
const MAX_DIRECTORY_ROUNDS: u32 = 4;

/// A local file header without its name and extra field.
const LOCAL_HEADER_BYTES: u64 = 30;
const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;

/// What a [`PackReader`] wants or gives next. The files come in the pack's order, the project's
/// own files before the blobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackStep {
    /// The bytes `[offset, offset + len)` of the pack, all of them, for the next
    /// [`step`](PackReader::step).
    Need { offset: u64, len: u64 },
    /// A file of the pack starts: its path (one of the format) and its size.
    Start { path: String, size: u64 },
    /// The next bytes of the file started last ([`PACK_PIECE_BYTES`] at most).
    Bytes(Vec<u8>),
    /// The file started last is whole and checked: its length, its CRC and, for a blob, its
    /// hash.
    End,
    /// Every file of the pack is out. The caller opens what it wrote ([`open`]) to validate
    /// the project, as [`import_pack`] does.
    Done,
}

/// A file of the pack, from its directory.
#[derive(Debug)]
struct Entry {
    name: String,
    header: u64,
    compressed: u64,
    size: u64,
    crc: u32,
    deflated: bool,
}

/// The file being read.
struct Body {
    index: usize,
    /// The next compressed byte to ask for, and the end of the file's compressed bytes.
    at: u64,
    end: u64,
    /// Compressed bytes given and not consumed yet (one piece at most).
    pending: Vec<u8>,
    /// `None` for a stored file.
    inflate: Option<flate2::Decompress>,
    /// The deflate stream is finished (never set for a stored file).
    finished: bool,
    produced: u64,
    crc: crc32fast::Hasher,
    hash: Option<Sha256>,
}

enum Phase {
    /// Reading the directory: the window holds the bytes `[base, len)` given so far.
    Directory {
        base: u64,
        window: Vec<u8>,
        rounds: u32,
    },
    /// Between two files.
    Next,
    /// The local header of the file `index` was asked for.
    Header(usize),
    Body(Box<Body>),
    Done,
    Failed,
}

/// Reads a pack in the pieces it asks for, so that the memory it holds is a piece and the
/// directory, whatever the size of the pack: the caller (the page's Worker, which reads a file
/// asynchronously, or [`import_pack`]) brings the bytes of each [`PackStep::Need`] and takes the
/// files it gives. The checks are the strict import's: only the paths of the format, no entry
/// twice, only stored or deflated files, the limits on the sizes the directory declares, every
/// length counted as it is produced (a header that lies about its size is no bomb), every file's
/// CRC and every blob's hash against its name, the local header's name against the directory's.
/// A failure ends the reader: any later step fails.
pub struct PackReader {
    len: u64,
    limits: PackLimits,
    entries: Vec<Entry>,
    /// The end of the files' data (the directory's start).
    data_end: u64,
    next: usize,
    /// The range asked for last, which the next step must bring.
    asked: Option<(u64, u64)>,
    phase: Phase,
}

impl PackReader {
    /// A reader of a pack of `len` bytes. It wants the end of the pack first ([`wants`](Self::wants)).
    pub fn new(len: u64, limits: PackLimits) -> Self {
        let base = len.saturating_sub(TAIL_BYTES);
        Self {
            len,
            limits,
            entries: Vec::new(),
            data_end: 0,
            next: 0,
            asked: Some((base, len - base)),
            phase: Phase::Directory {
                base: len,
                window: Vec::new(),
                rounds: 0,
            },
        }
    }

    /// The range the next step must bring, if any.
    pub fn wants(&self) -> Option<(u64, u64)> {
        self.asked
    }

    /// The next step. `input` is the bytes of the range [`wants`](Self::wants) named, or empty
    /// when it names none.
    pub fn step(&mut self, input: &[u8]) -> Result<PackStep, FileError> {
        if matches!(self.phase, Phase::Failed) {
            return Err(bad("the pack could not be read"));
        }
        let result = self.advance(input);
        if result.is_err() {
            self.phase = Phase::Failed;
            self.asked = None;
        }
        result
    }

    fn advance(&mut self, input: &[u8]) -> Result<PackStep, FileError> {
        match self.asked.take() {
            Some((offset, len)) => {
                if input.len() as u64 != len {
                    return Err(bad(format!(
                        "{} bytes given at {offset}, {len} asked for (the pack changed or ended early)",
                        input.len()
                    )));
                }
                self.absorb(offset, input)
            }
            None if !input.is_empty() => Err(bad("bytes given that were not asked for")),
            None => self.produce(),
        }
    }

    fn need(&mut self, offset: u64, len: u64) -> PackStep {
        self.asked = Some((offset, len));
        PackStep::Need { offset, len }
    }

    /// Takes the bytes asked for at `offset`.
    fn absorb(&mut self, offset: u64, input: &[u8]) -> Result<PackStep, FileError> {
        match &mut self.phase {
            Phase::Directory {
                base,
                window,
                rounds,
            } => {
                // The new bytes come right before the ones held.
                let mut joined = Vec::with_capacity(input.len() + window.len());
                joined.extend_from_slice(input);
                joined.extend_from_slice(window);
                *window = joined;
                *base = offset;
                *rounds += 1;
                self.directory()
            }
            Phase::Header(index) => {
                let index = *index;
                self.header(index, input)
            }
            Phase::Body(body) => {
                body.at += input.len() as u64;
                body.pending.extend_from_slice(input);
                self.produce()
            }
            Phase::Next | Phase::Done | Phase::Failed => {
                Err(bad("bytes given that were not asked for"))
            }
        }
    }

    /// Reads the directory from the window, or asks for more of it.
    fn directory(&mut self) -> Result<PackStep, FileError> {
        let Phase::Directory {
            base,
            window,
            rounds,
        } = &self.phase
        else {
            return Err(bad("not reading the directory"));
        };
        let (base, rounds) = (*base, *rounds);
        let missed = Cell::new(None);
        let reader = Window {
            len: self.len,
            base,
            bytes: window,
            pos: 0,
            missed: &missed,
        };
        let config = Config {
            archive_offset: ArchiveOffset::Known(0),
        };
        let parsed = ZipArchive::with_config(config, reader);
        let missed = missed.get();
        let zip = match parsed {
            Ok(zip) => zip,
            Err(error) => {
                // A directory bigger than the tail: ask for the bytes before the window.
                let Some(missed) = missed else {
                    return Err(bad(error));
                };
                if rounds >= MAX_DIRECTORY_ROUNDS || self.len - missed > MAX_DIRECTORY_BYTES {
                    return Err(bad("the directory of the pack is too big"));
                }
                return Ok(self.need(missed, base - missed));
            }
        };
        if zip.len() > self.limits.max_entries {
            return Err(bad("too many entries"));
        }
        let data_end = zip.central_directory_start();
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        let mut total = 0u64;
        let mut text_total = 0u64;
        for i in 0..zip.len() {
            let file = zip.by_index_data(i).map_err(bad)?;
            let name = file.name().map_err(bad)?.into_owned();
            if file.is_dir() {
                continue;
            }
            if !pack_path(&name) {
                return Err(bad(format!("{name:?} is not a file of a Faulk project")));
            }
            if !seen.insert(name.clone()) {
                return Err(bad(format!("{name:?} is there twice")));
            }
            let max = if name.starts_with("blobs/") {
                self.limits.max_blob
            } else {
                self.limits.max_text
            };
            if file.size() > max {
                return Err(bad(format!("{name:?} is bigger than {max} bytes")));
            }
            total = total.saturating_add(file.size());
            if total > self.limits.max_total {
                return Err(bad("the pack is too big"));
            }
            if !name.starts_with("blobs/") {
                text_total = text_total.saturating_add(file.size());
                if text_total > self.limits.max_text_total {
                    return Err(bad("the project's own files are too big"));
                }
            }
            if file.encrypted() {
                return Err(bad(format!("{name:?} is encrypted")));
            }
            let deflated = match file.compression() {
                CompressionMethod::Stored => false,
                CompressionMethod::Deflated => true,
                other => return Err(bad(format!("{name:?} is compressed with {other}"))),
            };
            if !deflated && file.compressed_size() != file.size() {
                return Err(bad(format!("{name:?} is stored with two sizes")));
            }
            let header = file.header_start();
            if header.saturating_add(LOCAL_HEADER_BYTES) > data_end {
                return Err(bad(format!("{name:?} starts past the data of the pack")));
            }
            entries.push(Entry {
                name,
                header,
                compressed: file.compressed_size(),
                size: file.size(),
                crc: file.crc32(),
                deflated,
            });
        }
        drop(zip);
        // The project's own files first, then the blobs: a reader learns the project (its
        // identity, what it refers to) before the media.
        entries.sort_by_key(|e| (e.name.starts_with("blobs/"), e.header));
        self.entries = entries;
        self.data_end = data_end;
        self.phase = Phase::Next;
        self.produce()
    }

    /// Checks the local header of the file `index` and starts its body.
    fn header(&mut self, index: usize, input: &[u8]) -> Result<PackStep, FileError> {
        let entry = &self.entries[index];
        let u16_at = |at: usize| u64::from(u16::from_le_bytes([input[at], input[at + 1]]));
        let signature = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
        let name_len = u16_at(26);
        let extra_len = u16_at(28);
        let name = &input[LOCAL_HEADER_BYTES as usize..];
        if signature != LOCAL_HEADER_SIGNATURE
            || name_len != entry.name.len() as u64
            || name != entry.name.as_bytes()
        {
            return Err(bad(format!(
                "{:?}: its local header is not the directory's",
                entry.name
            )));
        }
        let start = entry.header + LOCAL_HEADER_BYTES + name_len + extra_len;
        let end = start
            .checked_add(entry.compressed)
            .filter(|end| *end <= self.data_end)
            .ok_or_else(|| bad(format!("{:?} runs past the data of the pack", entry.name)))?;
        let path = entry.name.clone();
        let size = entry.size;
        self.phase = Phase::Body(Box::new(Body {
            index,
            at: start,
            end,
            pending: Vec::new(),
            inflate: entry.deflated.then(|| flate2::Decompress::new(false)),
            finished: false,
            produced: 0,
            crc: crc32fast::Hasher::new(),
            hash: path.starts_with("blobs/").then(Sha256::new),
        }));
        Ok(PackStep::Start { path, size })
    }

    /// The next output, or the next range to ask for.
    fn produce(&mut self) -> Result<PackStep, FileError> {
        match &mut self.phase {
            Phase::Next => {
                let Some(entry) = self.entries.get(self.next) else {
                    self.phase = Phase::Done;
                    return Ok(PackStep::Done);
                };
                let (header, name_len) = (entry.header, entry.name.len() as u64);
                self.phase = Phase::Header(self.next);
                self.next += 1;
                Ok(self.need(header, LOCAL_HEADER_BYTES + name_len))
            }
            Phase::Body(body) => {
                let entry = &self.entries[body.index];
                let out = body.output()?;
                if !out.is_empty() {
                    body.produced += out.len() as u64;
                    if body.produced > entry.size {
                        return Err(bad(format!(
                            "{:?} is bigger than its header says",
                            entry.name
                        )));
                    }
                    body.crc.update(&out);
                    if let Some(hash) = body.hash.as_mut() {
                        hash.update(&out);
                    }
                    return Ok(PackStep::Bytes(out));
                }
                let inflating = body.inflate.is_some();
                if body.at < body.end && body.pending.is_empty() && !(inflating && body.finished) {
                    let (at, len) = (body.at, (body.end - body.at).min(PACK_PIECE_BYTES));
                    return Ok(self.need(at, len));
                }
                if inflating && !body.finished {
                    return Err(bad(format!(
                        "{:?}: its compressed data is cut short",
                        entry.name
                    )));
                }
                if !body.pending.is_empty() || body.at < body.end {
                    return Err(bad(format!(
                        "{:?}: bytes after the end of its compressed data",
                        entry.name
                    )));
                }
                let crc = std::mem::take(&mut body.crc).finalize();
                if body.produced != entry.size || crc != entry.crc {
                    return Err(bad(format!("{:?} is not what its header says", entry.name)));
                }
                if let Some(hash) = body.hash.take() {
                    let hash = Hash(hash.finalize().into());
                    if format!("blobs/{hash}") != entry.name {
                        return Err(bad(format!(
                            "{:?} is not the blob its name says",
                            entry.name
                        )));
                    }
                }
                self.phase = Phase::Next;
                Ok(PackStep::End)
            }
            Phase::Done => Ok(PackStep::Done),
            Phase::Directory { .. } | Phase::Header(_) | Phase::Failed => {
                Err(bad("the reader waits for bytes"))
            }
        }
    }
}

impl Body {
    /// Up to a piece of the file from the bytes held: stored bytes as they are, deflated ones
    /// inflated. Empty when the reader needs more input (or the file is whole).
    fn output(&mut self) -> Result<Vec<u8>, FileError> {
        let Some(inflate) = self.inflate.as_mut() else {
            return Ok(std::mem::take(&mut self.pending));
        };
        if self.finished {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(PACK_PIECE_BYTES as usize);
        loop {
            let (before_in, before_out) = (inflate.total_in(), inflate.total_out());
            let status = inflate
                .decompress_vec(&self.pending, &mut out, flate2::FlushDecompress::None)
                .map_err(|e| bad(format!("the compressed data is damaged: {e}")))?;
            let consumed = (inflate.total_in() - before_in) as usize;
            self.pending.drain(..consumed);
            if status == flate2::Status::StreamEnd {
                self.finished = true;
                break;
            }
            let moved = consumed > 0 || inflate.total_out() > before_out;
            if !moved || out.len() == out.capacity() {
                break;
            }
        }
        Ok(out)
    }
}

/// The bytes of the pack given so far, `[base, len)`, as a reader for the zip directory: a read
/// before `base` fails and says where (`missed`), so the reader can ask for those bytes.
struct Window<'a> {
    len: u64,
    base: u64,
    bytes: &'a [u8],
    pos: u64,
    missed: &'a Cell<Option<u64>>,
}

impl Read for Window<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos < self.base {
            let pos = self.pos;
            self.missed
                .set(Some(self.missed.get().map_or(pos, |m: u64| m.min(pos))));
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "outside the bytes read so far",
            ));
        }
        if self.pos >= self.len {
            return Ok(0);
        }
        let from = (self.pos - self.base) as usize;
        let n = buf.len().min(self.bytes.len() - from);
        buf[..n].copy_from_slice(&self.bytes[from..from + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Window<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos =
            pos.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek out of range"))?;
        Ok(self.pos)
    }
}
