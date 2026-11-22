//! Decoding a file with symphonia, packet by packet, to interleaved `f32` (Opus with
//! `opuscule`, behind symphonia's demuxers: `opus`).

use crate::ImportError;
use crate::opus::{self, OpusStream};
use project::limits::{MAX_CHANNELS, MAX_DECODED_BYTES, MAX_FRAME, SAMPLE_RATES};
use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_OPUS, CODEC_ID_PCM_MULAW, CODEC_ID_PCM_S32LE,
};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::well_known::{FORMAT_ID_MKV, FORMAT_ID_OGG};
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

/// About the most frames one block of [`Decoding::next`] holds (a packet's worth: 1 152 for
/// MP3, 4 096 for most FLAC, up to 65 535): the host sizes nothing on it.
pub const BLOCK_FRAMES: usize = 4096;

/// What the pipeline reads: the file, by ranges, with its length known.
pub trait Source: Read + Seek + Send + Sync {
    /// The length of the file in bytes.
    fn byte_len(&self) -> u64;
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
        Some(self.0.byte_len())
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
    decoder: Codec,
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
    /// Frames to cut from the last block (a WebM's `DiscardPadding`: `webm`); while it is not
    /// 0, one decoded block is held back in `held` until the next shows it was not the last.
    tail: u64,
    held: Option<Vec<f32>>,
    spare: Vec<f32>,
    /// The most bytes of `f32` samples this decoding may give ([`MAX_DECODED_BYTES`], or a
    /// host's lower cap): a block that would pass it fails instead of being handed out.
    max_bytes: u64,
}

/// Bytes of `f32` samples `frames` frames of `channels` channels hold (saturating).
fn decoded_bytes(frames: u64, channels: usize) -> u64 {
    frames
        .saturating_mul(channels.max(1) as u64)
        .saturating_mul(4)
}

/// What decodes the packets: one of symphonia's decoders, or the Opus one.
enum Codec {
    Symphonia(Box<dyn AudioDecoder>),
    Opus(OpusStream),
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
    /// Reads the headers and makes the decoder. `hint` is the file's extension, if any. The
    /// decoded size is capped at [`MAX_DECODED_BYTES`] ([`ImportError::TooLarge`]).
    pub fn open(source: Box<dyn Source>, hint: Option<&str>) -> Result<Self, ImportError> {
        Self::open_capped(source, hint, MAX_DECODED_BYTES)
    }

    /// [`Self::open`] with a lower cap on the decoded size, in bytes of `f32` samples (a cap
    /// above [`MAX_DECODED_BYTES`] is that limit: a host can only lower it).
    pub fn open_capped(
        source: Box<dyn Source>,
        hint: Option<&str>,
        max_bytes: u64,
    ) -> Result<Self, ImportError> {
        let max_bytes = max_bytes.min(MAX_DECODED_BYTES);
        guarded("the headers", || Self::open_inner(source, hint, max_bytes))
    }

    fn open_inner(
        mut source: Box<dyn Source>,
        hint: Option<&str>,
        max_bytes: u64,
    ) -> Result<Self, ImportError> {
        let len = source.byte_len();
        if len == 0 {
            return Err(ImportError::Empty);
        }
        let edit = crate::mp4::edit(&mut source, len);
        source.seek(SeekFrom::Start(0)).map_err(ImportError::io)?;
        let paddings = crate::webm::paddings(&mut source, len);
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
        let declared = track.num_frames;
        let container = format.format_info().format;
        // Matroska and WebM are read for Opus only (#609): another codec there is refused by
        // name (no fixture checks its encoder delay and padding in that container).
        if container == FORMAT_ID_MKV && params.codec != CODEC_ID_OPUS {
            return Err(ImportError::UnsupportedCodec(format!(
                "codec {} in Matroska",
                params.codec
            )));
        }
        let (decoder, sample_rate, channels, skip, tail) = if params.codec == CODEC_ID_OPUS {
            if container != FORMAT_ID_OGG && container != FORMAT_ID_MKV {
                return Err(ImportError::UnsupportedCodec(format!(
                    "Opus in {}",
                    format.format_info().short_name
                )));
            }
            let head = opus::Head::parse(params.extra_data.as_deref().unwrap_or_default())?;
            // The pre-skip, in both containers: symphonia marks it on no packet. The end: the
            // last packet's trim (Ogg's last granule position, which symphonia applies), or the
            // last block's `DiscardPadding` (WebM, which symphonia drops).
            // Never more than one packet holds (120 ms): the cut is from the last block only, and a
            // forged `DiscardPadding` must not hide the frames handed out from the limits (#780).
            let tail = if container == FORMAT_ID_MKV {
                paddings
                    .map_or(0, |p| p.of(u64::from(id)))
                    .min(opus::MAX_FRAMES as u64)
            } else {
                0
            };
            (
                Codec::Opus(OpusStream::new(head)),
                opus::RATE,
                usize::from(head.channels),
                u64::from(head.pre_skip),
                tail,
            )
        } else {
            let decoder = symphonia::default::get_codecs()
                .make_audio_decoder(&params, &AudioDecoderOptions::default())
                .map_err(|e| match e {
                    Error::Unsupported(_) => {
                        ImportError::UnsupportedCodec(format!("codec {}", params.codec))
                    }
                    e => decode_error(e, "the codec's setup"),
                })?;
            (
                Codec::Symphonia(decoder),
                params.sample_rate.unwrap_or(0),
                params.channels.as_ref().map(|c| c.count()).unwrap_or(0),
                0,
                0,
            )
        };
        if sample_rate != 0 && !SAMPLE_RATES.contains(&sample_rate) {
            return Err(ImportError::BadSampleRate(sample_rate));
        }
        if channels > usize::from(MAX_CHANNELS) {
            return Err(ImportError::BadChannels(channels as u32));
        }
        // The length the headers declare (FLAC's `STREAMINFO`, an Ogg's last granule, MP4, CAF,
        // a WAV's `fact`, an ADPCM data size), known before a packet is decoded: beyond the cap,
        // refused now. Plain PCM is left to the decoding's own count: it decodes only the bytes
        // the file holds (at most 4 bytes of `f32` per byte), so a header that overstates its
        // length (a WAV or AIFF written in a stream) costs nothing, and such a file is read
        // directly without a cache.
        // symphonia 0.6 numbers its PCM codecs 0x100 (`CODEC_ID_PCM_S32LE`) to 0x125
        // (`CODEC_ID_PCM_MULAW`), every integer, float, A-law and µ-law layout; ADPCM starts at 0x200
        // (`pcm_codecs_are_one_range` pins it).
        let pcm = (CODEC_ID_PCM_S32LE..=CODEC_ID_PCM_MULAW).contains(&params.codec);
        if let Some(frames) = declared.filter(|_| !pcm)
            && decoded_bytes(frames, channels) > max_bytes
        {
            return Err(ImportError::TooLarge(max_bytes));
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
            skip,
            keep: None,
            tail,
            held: None,
            spare: Vec::new(),
            max_bytes,
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
        let result = guarded("a packet", || self.next_held(out));
        if result.is_err() {
            self.failed = true;
            out.clear();
        }
        result
    }

    /// [`Self::next_inner`], one block late while a tail is to be cut from the last block: the
    /// last block is the one after which the stream ends.
    fn next_held(&mut self, out: &mut Vec<f32>) -> Result<bool, ImportError> {
        if self.tail == 0 && self.held.is_none() {
            return self.next_inner(out);
        }
        loop {
            // The buffers go round: `out` (the caller's), `held` and `spare`.
            let mut fresh = std::mem::take(&mut self.spare);
            fresh.clear();
            let more = self.next_inner(&mut fresh)?;
            if more && fresh.is_empty() {
                // A block of no frame (all pre-skip) is not the one to cut from.
                self.spare = fresh;
                continue;
            }
            match (more, self.held.take()) {
                (true, None) => self.held = Some(fresh),
                (true, Some(previous)) => {
                    self.spare = std::mem::replace(out, previous);
                    self.held = Some(fresh);
                    return Ok(true);
                }
                (false, Some(mut last)) => {
                    let channels = usize::from(self.channels.max(1));
                    let frames = last.len() / channels;
                    let cut = usize::try_from(self.tail).unwrap_or(usize::MAX).min(frames);
                    last.truncate((frames - cut) * channels);
                    self.frames -= cut as u64;
                    self.tail = 0;
                    self.spare = std::mem::replace(out, last);
                    return Ok(true);
                }
                (false, None) => {
                    self.spare = fresh;
                    return Ok(false);
                }
            }
        }
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
            let (rate, channels) = match &mut self.decoder {
                Codec::Symphonia(decoder) => {
                    let buffer = decoder
                        .decode(&packet)
                        .map_err(|e| decode_error(e, "a packet"))?;
                    let spec = buffer.spec();
                    let channels = spec.channels().count();
                    out.resize(buffer.frames() * channels, 0.0);
                    buffer.copy_to_slice_interleaved(&mut out[..]);
                    (spec.rate(), channels)
                }
                Codec::Opus(stream) => {
                    stream.decode(&packet.data, out)?;
                    // The padding the last granule position names (Ogg); never more than
                    // the packet holds.
                    let channels = usize::from(self.channels);
                    let frames = out.len() / channels;
                    let cut = usize::try_from(packet.trim_end.get())
                        .unwrap_or(usize::MAX)
                        .min(frames);
                    out.truncate((frames - cut) * channels);
                    (opus::RATE, channels)
                }
            };
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
            let frames = out.len() / channels.max(1);
            // The encoder's delay (an Opus pre-skip) and the padding an MP4 edit list names
            // (`mp4`).
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
            // Less the tail still to cut (one packet at most): the limit is on the frames kept.
            if self.frames.saturating_sub(self.tail) > MAX_FRAME {
                return Err(ImportError::TooLong);
            }
            // Every frame counted so far, the tail included, has been or is about to be handed
            // to the host, which writes it: the block that would pass the cap is not (#780).
            if decoded_bytes(self.frames, channels) > self.max_bytes {
                return Err(ImportError::TooLarge(self.max_bytes));
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

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::codecs::audio::well_known::*;

    /// The plain-PCM exemption of the header check is one range of codec ids: every PCM codec
    /// symphonia has is inside it, and every other codec Faulk decodes outside (#780).
    #[test]
    fn pcm_codecs_are_one_range() {
        let range = CODEC_ID_PCM_S32LE..=CODEC_ID_PCM_MULAW;
        for pcm in [
            CODEC_ID_PCM_S32LE,
            CODEC_ID_PCM_S32LE_PLANAR,
            CODEC_ID_PCM_S32BE,
            CODEC_ID_PCM_S32BE_PLANAR,
            CODEC_ID_PCM_S24LE,
            CODEC_ID_PCM_S24LE_PLANAR,
            CODEC_ID_PCM_S24BE,
            CODEC_ID_PCM_S24BE_PLANAR,
            CODEC_ID_PCM_S16LE,
            CODEC_ID_PCM_S16LE_PLANAR,
            CODEC_ID_PCM_S16BE,
            CODEC_ID_PCM_S16BE_PLANAR,
            CODEC_ID_PCM_S8,
            CODEC_ID_PCM_S8_PLANAR,
            CODEC_ID_PCM_U32LE,
            CODEC_ID_PCM_U32LE_PLANAR,
            CODEC_ID_PCM_U32BE,
            CODEC_ID_PCM_U32BE_PLANAR,
            CODEC_ID_PCM_U24LE,
            CODEC_ID_PCM_U24LE_PLANAR,
            CODEC_ID_PCM_U24BE,
            CODEC_ID_PCM_U24BE_PLANAR,
            CODEC_ID_PCM_U16LE,
            CODEC_ID_PCM_U16LE_PLANAR,
            CODEC_ID_PCM_U16BE,
            CODEC_ID_PCM_U16BE_PLANAR,
            CODEC_ID_PCM_U8,
            CODEC_ID_PCM_U8_PLANAR,
            CODEC_ID_PCM_F32LE,
            CODEC_ID_PCM_F32LE_PLANAR,
            CODEC_ID_PCM_F32BE,
            CODEC_ID_PCM_F32BE_PLANAR,
            CODEC_ID_PCM_F64LE,
            CODEC_ID_PCM_F64LE_PLANAR,
            CODEC_ID_PCM_F64BE,
            CODEC_ID_PCM_F64BE_PLANAR,
            CODEC_ID_PCM_ALAW,
            CODEC_ID_PCM_MULAW,
        ] {
            assert!(range.contains(&pcm), "{pcm}");
        }
        for other in [
            CODEC_ID_ADPCM_MS,
            CODEC_ID_ADPCM_IMA_WAV,
            CODEC_ID_FLAC,
            CODEC_ID_MP3,
            CODEC_ID_AAC,
            CODEC_ID_ALAC,
            CODEC_ID_VORBIS,
            CODEC_ID_OPUS,
            CODEC_ID_MP1,
            CODEC_ID_MP2,
        ] {
            assert!(!range.contains(&other), "{other}");
        }
    }
}
