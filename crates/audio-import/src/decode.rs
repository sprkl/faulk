//! Decoding a file with symphonia, packet by packet, to interleaved `f32`.

use crate::ImportError;
use project::limits::{MAX_CHANNELS, MAX_FRAME, SAMPLE_RATES};
use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

/// About the most frames one block of [`Decoding::next`] holds (a packet's worth: 1 152 for
/// MP3, 4 096 for most FLAC, up to 65 535): the host sizes nothing on it.
pub const BLOCK_FRAMES: usize = 4096;

/// What the pipeline reads: the file, by ranges, with its length known.
pub trait Source: Read + Seek + Send + Sync {
    fn len(&self) -> u64;
}

struct Media(Box<dyn Source>);

impl Read for Media {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for Media {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl MediaSource for Media {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.0.len())
    }
}

/// The metadata of a decoded file, as a clip writes it in clear (`project::AudioMedia`, but
/// the name and the hash, which are not the decoder's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaInfo {
    pub sample_rate: u32,
    pub channels: u16,
    /// Frames decoded: the playable length, after the encoder's delay and padding are cut
    /// where the format says them (gapless).
    pub frames: u64,
}

/// A file being decoded. [`Decoding::open`] reads the headers; [`Decoding::next`] decodes one
/// packet at a time; [`Decoding::finish`] checks what was decoded and gives the metadata.
pub struct Decoding {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    sample_rate: u32,
    channels: u16,
    frames: u64,
    /// Set once a step failed: the decoder's state is not to be trusted after it.
    failed: bool,
    /// An MP4's edit list, until the first block gives the rate to read it at.
    edit: Option<crate::mp4::Edit>,
    /// Frames still to drop at the start (the encoder's delay).
    skip: u64,
    /// Frames still to keep, when the file says where the audio ends (before the padding).
    keep: Option<u64>,
}

/// Runs `f`, turning a panic of a dependency into [`ImportError::Corrupt`] (natively; in
/// WebAssembly a panic aborts the instance and the host reports the trap).
fn guarded<T>(what: &str, f: impl FnOnce() -> Result<T, ImportError>) -> Result<T, ImportError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        Err(ImportError::Corrupt(format!(
            "the decoder failed on {what}"
        )))
    })
}

fn decode_error(error: Error, what: &str) -> ImportError {
    match error {
        Error::IoError(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            ImportError::Corrupt(format!("the file ends inside {what}"))
        }
        Error::IoError(e) => ImportError::Io(e.to_string()),
        Error::DecodeError(m) => ImportError::Corrupt(format!("{what}: {m}")),
        Error::LimitError(m) => ImportError::Corrupt(format!("{what}: {m}")),
        Error::Unsupported(m) => ImportError::Corrupt(format!("{what}: unsupported {m}")),
        Error::SeekError(_) => ImportError::Corrupt(format!("{what}: a seek failed")),
        Error::ResetRequired => ImportError::Corrupt("the stream changes its format midway".into()),
        _ => ImportError::Corrupt(format!("{what}: {error}")),
    }
}

impl Decoding {
    /// Reads the headers and makes the decoder. `hint` is the file's extension, if any.
    pub fn open(source: Box<dyn Source>, hint: Option<&str>) -> Result<Self, ImportError> {
        guarded("the headers", || Self::open_inner(source, hint))
    }

    fn open_inner(mut source: Box<dyn Source>, hint: Option<&str>) -> Result<Self, ImportError> {
        let len = source.len();
        if len == 0 {
            return Err(ImportError::Empty);
        }
        let edit = crate::mp4::edit(&mut source, len);
        source.seek(SeekFrom::Start(0)).map_err(ImportError::io)?;
        let stream = MediaSourceStream::new(
            Box::new(Media(source)),
            MediaSourceStreamOptions {
                buffer_len: 1 << 20,
            },
        );
        let mut probe_hint = Hint::new();
        if let Some(ext) = hint {
            probe_hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(
                &probe_hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| match e {
                Error::Unsupported(_) => ImportError::UnsupportedFormat,
                e => decode_error(e, "the headers"),
            })?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or(ImportError::Empty)?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or(ImportError::Empty)?
            .clone();
        let id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| match e {
                Error::Unsupported(_) if params.codec == CODEC_ID_OPUS => {
                    ImportError::UnsupportedCodec("Opus".into())
                }
                Error::Unsupported(_) => {
                    ImportError::UnsupportedCodec(format!("codec {}", params.codec))
                }
                e => decode_error(e, "the codec's setup"),
            })?;
        let sample_rate = params.sample_rate.unwrap_or(0);
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(0);
        if sample_rate != 0 && !SAMPLE_RATES.contains(&sample_rate) {
            return Err(ImportError::BadSampleRate(sample_rate));
        }
        if channels > usize::from(MAX_CHANNELS) {
            return Err(ImportError::BadChannels(channels as u32));
        }
        Ok(Self {
            format,
            decoder,
            track: id,
            sample_rate,
            channels: channels as u16,
            frames: 0,
            failed: false,
            edit,
            skip: 0,
            keep: None,
        })
    }

    /// The sample rate the headers say, 0 when they say none (the first block sets it).
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The channels the headers say, 0 when they say none (the first block sets them).
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Frames decoded so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Decodes the next packet into `out` (cleared first), interleaved. `Ok(false)` at the end
    /// of the stream, with `out` empty. A block may be empty (a packet that holds only the
    /// encoder's delay).
    pub fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, ImportError> {
        out.clear();
        if self.failed {
            return Err(ImportError::Corrupt("decoding already failed".into()));
        }
        let result = guarded("a packet", || self.next_inner(out));
        if result.is_err() {
            self.failed = true;
            out.clear();
        }
        result
    }

    fn next_inner(&mut self, out: &mut Vec<f32>) -> Result<bool, ImportError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(false),
                Err(e) => return Err(decode_error(e, "a packet")),
            };
            if packet.track_id != self.track {
                continue;
            }
            let buffer = self
                .decoder
                .decode(&packet)
                .map_err(|e| decode_error(e, "a packet"))?;
            let spec = buffer.spec();
            let rate = spec.rate();
            let channels = spec.channels().count();
            if self.sample_rate == 0 {
                if !SAMPLE_RATES.contains(&rate) {
                    return Err(ImportError::BadSampleRate(rate));
                }
                self.sample_rate = rate;
            } else if rate != self.sample_rate {
                return Err(ImportError::Corrupt(
                    "the sample rate changes midway".into(),
                ));
            }
            if self.channels == 0 {
                if channels == 0 || channels > usize::from(MAX_CHANNELS) {
                    return Err(ImportError::BadChannels(channels as u32));
                }
                self.channels = channels as u16;
            } else if channels != usize::from(self.channels) {
                return Err(ImportError::Corrupt(
                    "the number of channels changes midway".into(),
                ));
            }
            if let Some(edit) = self.edit.take() {
                self.skip = edit.start(rate).unwrap_or(0);
                self.keep = edit.length(rate);
            }
            if self.keep == Some(0) {
                return Ok(false);
            }
            let frames = buffer.frames();
            out.resize(frames * channels, 0.0);
            buffer.copy_to_slice_interleaved(&mut out[..]);
            // The encoder's delay and padding an MP4 edit list names (`mp4`).
            let skip = self.skip.min(frames as u64) as usize;
            self.skip -= skip as u64;
            let mut kept = frames - skip;
            if let Some(keep) = self.keep.as_mut() {
                kept = kept.min(usize::try_from(*keep).unwrap_or(usize::MAX));
                *keep -= kept as u64;
            }
            out.drain(..skip * channels);
            out.truncate(kept * channels);
            self.frames = self.frames.saturating_add(kept as u64);
            if self.frames > MAX_FRAME {
                return Err(ImportError::TooLong);
            }
            return Ok(true);
        }
    }

    /// The metadata of everything decoded; refused when nothing was.
    pub fn finish(self) -> Result<MediaInfo, ImportError> {
        if self.failed {
            return Err(ImportError::Corrupt("decoding failed".into()));
        }
        if self.frames == 0 {
            return Err(ImportError::Empty);
        }
        if self.channels == 0 {
            return Err(ImportError::BadChannels(0));
        }
        Ok(MediaInfo {
            sample_rate: self.sample_rate,
            channels: self.channels,
            frames: self.frames,
        })
    }
}
