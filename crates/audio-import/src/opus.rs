//! Opus (RFC 6716, with the corrections of RFC 8251), decoded by `opuscule` behind symphonia's
//! Ogg and Matroska demuxers (#609): symphonia 0.6 reads the containers but has no Opus decoder.
//!
//! The stream's setup is its identification header, `OpusHead` (RFC 7845 § 5.1): the Ogg
//! demuxer gives it as the track's extra data, and so does the Matroska one (a WebM's
//! `CodecPrivate`). Only mono and stereo streams are decoded (what a recording or an encoder's
//! file holds): channel mapping family 0, or family 1 holding one plain stream; the other
//! layouts (surround, ambisonics), whose channels Faulk does not model, are refused by name.
//!
//! Opus always decodes at 48 kHz (the header's input rate is informational, RFC 7845 § 5.1).
//! The encoder's delay (the header's pre-skip) and the padding are cut by [`crate::Decoding`].

use crate::ImportError;
use opuscule::{Channels, Decoder, SampleRate};

/// The rate every Opus stream decodes at.
pub const RATE: u32 = 48_000;

/// The most frames one packet holds: 120 ms at 48 kHz (RFC 6716 § 3.2.5).
pub(crate) const MAX_FRAMES: usize = 5_760;

/// What the identification header says, as far as decoding goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    /// 1 or 2.
    pub channels: u16,
    /// Frames (at 48 kHz) to drop at the start: the encoder's delay.
    pub pre_skip: u16,
    /// The gain to apply to the output, in 1/256 dB (Q7.8).
    pub gain: i16,
}

impl Head {
    /// Reads an `OpusHead` packet.
    pub fn parse(bytes: &[u8]) -> Result<Head, ImportError> {
        let corrupt = |what: &str| ImportError::Corrupt(format!("the Opus header {what}"));
        if bytes.get(..8) != Some(b"OpusHead".as_slice()) {
            return Err(corrupt("is missing"));
        }
        let field = |at: usize| {
            bytes
                .get(at)
                .copied()
                .ok_or_else(|| corrupt("is cut short"))
        };
        let le16 = |at: usize| -> Result<[u8; 2], ImportError> { Ok([field(at)?, field(at + 1)?]) };
        let version = field(8)?;
        // RFC 7845 § 5.1: the upper four bits are the major version; only 0 is defined.
        if version >> 4 != 0 {
            return Err(ImportError::UnsupportedCodec(format!(
                "Opus (header version {version})"
            )));
        }
        let channels = field(9)?;
        let pre_skip = u16::from_le_bytes(le16(10)?);
        let gain = i16::from_le_bytes(le16(16)?);
        let family = field(18)?;
        let unsupported = || {
            ImportError::UnsupportedCodec(format!(
                "Opus of channel mapping family {family} ({channels} channels)"
            ))
        };
        match family {
            0 => {}
            // Family 1 (RFC 7845 § 5.1.1.2) with one stream of one or two channels in their
            // order is a plain mono or stereo stream, as some encoders write it.
            1 if (1..=2).contains(&channels) => {
                let (streams, coupled) = (field(19)?, field(20)?);
                let mapping = (0..channels).map(|c| field(21 + usize::from(c)));
                let in_order = mapping
                    .zip(0..)
                    .try_fold(true, |ok, (m, c)| Ok::<_, ImportError>(ok && m? == c))?;
                if streams != 1 || coupled != channels - 1 || !in_order {
                    return Err(unsupported());
                }
            }
            _ => return Err(unsupported()),
        }
        if !(1..=2).contains(&channels) {
            return Err(corrupt(&format!(
                "says {channels} channels for a mono or stereo stream"
            )));
        }
        Ok(Head {
            channels: u16::from(channels),
            pre_skip,
            gain,
        })
    }
}

/// One Opus stream being decoded.
pub struct OpusStream {
    // Boxed: the decoder's state is held by value and is large.
    decoder: Box<Decoder>,
    channels: usize,
    /// The header's output gain as a factor; exactly 1 when the header says 0 dB.
    gain: f32,
    /// The frames of the last packet decoded: what an empty packet conceals.
    last: usize,
}

impl OpusStream {
    pub fn new(head: Head) -> OpusStream {
        let layout = if head.channels == 1 {
            Channels::Mono
        } else {
            Channels::Stereo
        };
        let gain = if head.gain == 0 {
            1.0
        } else {
            // 10^(dB / 20), through `libm` like the decoder's own math: the same factor on
            // every target.
            libm::powf(10.0, f32::from(head.gain) / (20.0 * 256.0))
        };
        OpusStream {
            decoder: Box::new(Decoder::new(SampleRate::Hz48000, layout)),
            channels: usize::from(head.channels),
            gain,
            last: 0,
        }
    }

    /// Decodes one packet into `out` (cleared first), interleaved. An empty packet (every
    /// Opus packet has a TOC byte, RFC 6716 § 3.2) is a lost one: concealed for the length of
    /// the packet before, as libopus does, so the timeline keeps the length the container
    /// counts; before any packet it holds nothing.
    pub fn decode(&mut self, packet: &[u8], out: &mut Vec<f32>) -> Result<(), ImportError> {
        out.clear();
        let (packet, capacity) = match packet {
            [] if self.last == 0 => return Ok(()),
            [] => (None, self.last),
            packet => (Some(packet), MAX_FRAMES),
        };
        out.resize(capacity * self.channels, 0.0);
        let frames = match self.decoder.decode(packet, out, false) {
            Ok(frames) => frames,
            Err(e) => {
                out.clear();
                return Err(ImportError::Corrupt(format!("an Opus packet: {e}")));
            }
        };
        let frames = frames.min(capacity);
        self.last = frames;
        out.truncate(frames * self.channels);
        if self.gain != 1.0 {
            for sample in out.iter_mut() {
                *sample *= self.gain;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(version: u8, channels: u8, pre_skip: u16, gain: i16, family: u8) -> Vec<u8> {
        let mut b = b"OpusHead".to_vec();
        b.push(version);
        b.push(channels);
        b.extend_from_slice(&pre_skip.to_le_bytes());
        b.extend_from_slice(&48_000u32.to_le_bytes());
        b.extend_from_slice(&gain.to_le_bytes());
        b.push(family);
        b
    }

    #[test]
    fn a_mono_or_stereo_header_is_read() {
        assert_eq!(
            Head::parse(&head(1, 2, 312, -256, 0)),
            Ok(Head {
                channels: 2,
                pre_skip: 312,
                gain: -256
            })
        );
        assert_eq!(Head::parse(&head(1, 1, 0, 0, 0)).unwrap().channels, 1);
        // A minor version above 1 is still major version 0.
        assert!(Head::parse(&head(15, 1, 0, 0, 0)).is_ok());
        // Family 1 holding one plain mono or stereo stream.
        let stereo = [head(1, 2, 312, 0, 1), vec![1, 1, 0, 1]].concat();
        assert_eq!(Head::parse(&stereo).unwrap().channels, 2);
        let mono = [head(1, 1, 0, 0, 1), vec![1, 0, 0]].concat();
        assert_eq!(Head::parse(&mono).unwrap().channels, 1);
    }

    #[test]
    fn a_header_outside_mono_and_stereo_or_damaged_is_refused_by_name() {
        for family in [1, 2, 3, 255] {
            let e = Head::parse(&head(1, 6, 0, 0, family)).unwrap_err();
            assert_eq!(e.code(), "unsupported_codec", "family {family}");
        }
        for family in [2, 3, 4, 255] {
            let e = Head::parse(&head(1, 2, 0, 0, family)).unwrap_err();
            assert_eq!(e.code(), "unsupported_codec", "stereo of family {family}");
        }
        // Family 1 with two streams, channels swapped, or a silent channel.
        for table in [[2u8, 0, 0, 1], [1, 1, 1, 0], [1, 1, 0, 255]] {
            let e = Head::parse(&[head(1, 2, 0, 0, 1), table.to_vec()].concat()).unwrap_err();
            assert_eq!(e.code(), "unsupported_codec", "{table:?}");
        }
        // Family 1 whose table is cut short.
        let whole = [head(1, 2, 0, 0, 1), vec![1, 1, 0, 1]].concat();
        for cut in 19..whole.len() {
            assert_eq!(Head::parse(&whole[..cut]).unwrap_err().code(), "corrupt");
        }
        assert_eq!(
            Head::parse(&head(16, 2, 0, 0, 0)).unwrap_err().code(),
            "unsupported_codec"
        );
        for channels in [0, 3, 255] {
            assert_eq!(
                Head::parse(&head(1, channels, 0, 0, 0)).unwrap_err().code(),
                "corrupt"
            );
        }
        let whole = head(1, 2, 0, 0, 0);
        for cut in 0..whole.len() {
            assert_eq!(Head::parse(&whole[..cut]).unwrap_err().code(), "corrupt");
        }
        assert_eq!(
            Head::parse(b"OpusTags\x01\x02\0\0\0\0\0\0\0\0\0")
                .unwrap_err()
                .code(),
            "corrupt"
        );
    }

    #[test]
    fn the_output_gain_scales_the_samples() {
        // A CELT packet (TOC 0xfc: fullband, 20 ms, stereo, one frame) of arbitrary bytes: it
        // decodes to something not silent, the same with and without the gain but scaled.
        let packet = [0xfc, 0x8a, 0x3c, 0x55, 0x12, 0x9e, 0x01, 0x77, 0x42, 0x10];
        let mut plain = OpusStream::new(Head {
            channels: 2,
            pre_skip: 0,
            gain: 0,
        });
        let mut louder = OpusStream::new(Head {
            channels: 2,
            pre_skip: 0,
            gain: 6 * 256,
        });
        let (mut a, mut b) = (Vec::new(), Vec::new());
        plain.decode(&packet, &mut a).unwrap();
        louder.decode(&packet, &mut b).unwrap();
        assert_eq!(a.len(), 960 * 2);
        assert!(a.iter().any(|x| *x != 0.0));
        assert_eq!(a.len(), b.len());
        let factor = libm::powf(10.0, 6.0 / 20.0);
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(*y, *x * factor);
        }
        // An empty packet is a lost one: concealed for the length of the packet before.
        plain.decode(&[], &mut a).unwrap();
        assert_eq!(a.len(), 960 * 2);
        // Before any packet, it holds nothing.
        let mut fresh = OpusStream::new(Head {
            channels: 1,
            pre_skip: 0,
            gain: 0,
        });
        fresh.decode(&[], &mut a).unwrap();
        assert!(a.is_empty());
    }
}
