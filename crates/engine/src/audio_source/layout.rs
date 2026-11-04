//! Files whose samples are read straight from the blob (moved from `audio-import` by AU-3, so
//! the engine's disk side reads them with the same code the import checks): WAV and AIFF holding 16, 24 or 32-bit
//! integers or 32-bit floats. Their samples are a fixed-size array at a known offset, so the
//! disk Worker (AU-3) reads any range of frames with one ranged read and converts it, and no
//! decoded copy is kept. Every other file is decoded into the PCM cache (`audio_import::cache`).
//!
//! The conversion to `f32` is symphonia's own (`i16 / 32 768`, `i24 / 8 388 608`,
//! `i32 / 2^31` through `f64`), so reading the blob through a layout gives exactly the samples
//! decoding gives; a test checks it on every direct fixture, and [`PcmLayout::matches`] keeps
//! a layout only when it agrees with the decoder on rate, channels and length.

/// How many bytes of the start of a file [`PcmLayout::parse`] needs: the sample data must
/// begin within them (metadata chunks larger than this before it send the file to the cache).
pub const HEAD_BYTES: usize = 64 * 1024;

/// How each sample is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleEncoding {
    /// Signed integer of 2, 3 or 4 bytes.
    Int { bytes: u8, big_endian: bool },
    /// IEEE 754 single precision.
    Float32 { big_endian: bool },
}

impl SampleEncoding {
    pub fn bytes(self) -> usize {
        match self {
            Self::Int { bytes, .. } => usize::from(bytes),
            Self::Float32 { .. } => 4,
        }
    }

    /// One sample as `f32`, from exactly [`Self::bytes`] bytes.
    pub fn sample(self, b: &[u8]) -> f32 {
        match self {
            Self::Int {
                bytes: 2,
                big_endian,
            } => {
                let v = if big_endian {
                    i16::from_be_bytes([b[0], b[1]])
                } else {
                    i16::from_le_bytes([b[0], b[1]])
                };
                v as f32 / 32_768.0
            }
            Self::Int {
                bytes: 3,
                big_endian,
            } => {
                let (hi, mid, lo) = if big_endian {
                    (b[0], b[1], b[2])
                } else {
                    (b[2], b[1], b[0])
                };
                // Sign-extended through the top byte of an i32.
                let v = i32::from_be_bytes([hi, mid, lo, 0]) >> 8;
                v as f32 / 8_388_608.0
            }
            Self::Int { big_endian, .. } => {
                let w = [b[0], b[1], b[2], b[3]];
                let v = if big_endian {
                    i32::from_be_bytes(w)
                } else {
                    i32::from_le_bytes(w)
                };
                (v as f64 / 2_147_483_648.0) as f32
            }
            Self::Float32 { big_endian } => {
                let w = [b[0], b[1], b[2], b[3]];
                if big_endian {
                    f32::from_be_bytes(w)
                } else {
                    f32::from_le_bytes(w)
                }
            }
        }
    }
}

/// Where the samples of a directly readable file are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmLayout {
    /// Byte offset of the first frame in the file.
    pub data_offset: u64,
    pub encoding: SampleEncoding,
    pub channels: u16,
    pub sample_rate: u32,
    /// Whole frames present in the file.
    pub frames: u64,
}

fn u16_at(b: &[u8], at: usize, big: bool) -> Option<u16> {
    let w = [*b.get(at)?, *b.get(at + 1)?];
    Some(if big {
        u16::from_be_bytes(w)
    } else {
        u16::from_le_bytes(w)
    })
}

fn u32_at(b: &[u8], at: usize, big: bool) -> Option<u32> {
    let w = [
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ];
    Some(if big {
        u32::from_be_bytes(w)
    } else {
        u32::from_le_bytes(w)
    })
}

/// An 80-bit IEEE 754 extended float (AIFF's sample rate) as a whole number of hertz.
fn extended_hz(b: &[u8]) -> Option<u32> {
    let b = b.get(..10)?;
    if b[0] & 0x80 != 0 {
        return None;
    }
    let exponent = i32::from(u16::from_be_bytes([b[0], b[1]]) & 0x7fff) - 16_383;
    let mantissa = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    if !(0..32).contains(&exponent) {
        return None;
    }
    let shift = 63 - exponent as u32;
    // A rate with a fraction is not one Faulk reads directly.
    if mantissa & ((1u64 << shift) - 1) != 0 {
        return None;
    }
    u32::try_from(mantissa >> shift).ok()
}

impl PcmLayout {
    /// Bytes per frame.
    pub fn frame_bytes(&self) -> usize {
        self.encoding.bytes() * usize::from(self.channels)
    }

    /// The layout of a file from its first bytes (`head`, up to [`HEAD_BYTES`]) and its
    /// length, or `None` when the file is not a WAV or AIFF of a directly readable encoding.
    pub fn parse(head: &[u8], file_len: u64) -> Option<Self> {
        match head.get(..4)? {
            b"RIFF" if head.get(8..12)? == b"WAVE" => Self::wav(head, file_len),
            b"FORM" if matches!(head.get(8..12)?, b"AIFF" | b"AIFC") => Self::aiff(head, file_len),
            _ => None,
        }
    }

    fn wav(head: &[u8], file_len: u64) -> Option<Self> {
        let mut at = 12;
        let mut format: Option<(SampleEncoding, u16, u32)> = None;
        while at + 8 <= head.len() {
            let id = &head[at..at + 4];
            let size = u32_at(head, at + 4, false)? as usize;
            let body = at + 8;
            match id {
                b"fmt " => {
                    let tag = u16_at(head, body, false)?;
                    let channels = u16_at(head, body + 2, false)?;
                    let rate = u32_at(head, body + 4, false)?;
                    let align = u16_at(head, body + 12, false)?;
                    let bits = u16_at(head, body + 14, false)?;
                    let tag = if tag == 0xfffe {
                        // WAVE_FORMAT_EXTENSIBLE: the valid bits must fill the container, and the
                        // sub-format's first two bytes are the real tag.
                        if size < 40 || u16_at(head, body + 18, false)? != bits {
                            return None;
                        }
                        u16_at(head, body + 24, false)?
                    } else {
                        tag
                    };
                    let encoding = match (tag, bits) {
                        (1, 16) => SampleEncoding::Int {
                            bytes: 2,
                            big_endian: false,
                        },
                        (1, 24) => SampleEncoding::Int {
                            bytes: 3,
                            big_endian: false,
                        },
                        (1, 32) => SampleEncoding::Int {
                            bytes: 4,
                            big_endian: false,
                        },
                        (3, 32) => SampleEncoding::Float32 { big_endian: false },
                        _ => return None,
                    };
                    if channels == 0
                        || usize::from(align) != encoding.bytes() * usize::from(channels)
                    {
                        return None;
                    }
                    format = Some((encoding, channels, rate));
                }
                b"data" => {
                    let (encoding, channels, sample_rate) = format?;
                    let offset = body as u64;
                    let present = file_len.checked_sub(offset)?.min(size as u64);
                    let frame = (encoding.bytes() * usize::from(channels)) as u64;
                    return Some(Self {
                        data_offset: offset,
                        encoding,
                        channels,
                        sample_rate,
                        frames: present / frame,
                    });
                }
                _ => {}
            }
            at = body.checked_add(size)?.checked_add(size & 1)?;
        }
        None
    }

    fn aiff(head: &[u8], file_len: u64) -> Option<Self> {
        let compressed = &head[8..12] == b"AIFC";
        let mut at = 12;
        let mut format: Option<(SampleEncoding, u16, u32, u32)> = None;
        while at + 8 <= head.len() {
            let id = &head[at..at + 4];
            let size = u32_at(head, at + 4, true)? as usize;
            let body = at + 8;
            match id {
                b"COMM" => {
                    let channels = u16_at(head, body, true)?;
                    let frames = u32_at(head, body + 2, true)?;
                    let bits = u16_at(head, body + 6, true)?;
                    let rate = extended_hz(head.get(body + 8..)?)?;
                    let kind = if compressed {
                        head.get(body + 18..body + 22)?
                    } else {
                        b"NONE"
                    };
                    let encoding = match (kind, bits) {
                        (b"NONE", 16) => SampleEncoding::Int {
                            bytes: 2,
                            big_endian: true,
                        },
                        (b"NONE", 24) => SampleEncoding::Int {
                            bytes: 3,
                            big_endian: true,
                        },
                        (b"NONE", 32) => SampleEncoding::Int {
                            bytes: 4,
                            big_endian: true,
                        },
                        (b"sowt", 16) => SampleEncoding::Int {
                            bytes: 2,
                            big_endian: false,
                        },
                        (b"sowt", 24) => SampleEncoding::Int {
                            bytes: 3,
                            big_endian: false,
                        },
                        (b"sowt", 32) => SampleEncoding::Int {
                            bytes: 4,
                            big_endian: false,
                        },
                        (b"fl32" | b"FL32", 32) => SampleEncoding::Float32 { big_endian: true },
                        _ => return None,
                    };
                    if channels == 0 {
                        return None;
                    }
                    format = Some((encoding, channels, rate, frames));
                }
                b"SSND" => {
                    let (encoding, channels, sample_rate, declared) = format?;
                    let skip = u32_at(head, body, true)? as u64;
                    let offset = (body as u64).checked_add(8)?.checked_add(skip)?;
                    let chunk_end = (body as u64).checked_add(size as u64)?;
                    let present = file_len.min(chunk_end).checked_sub(offset)?;
                    let frame = (encoding.bytes() * usize::from(channels)) as u64;
                    let frames = (present / frame).min(u64::from(declared));
                    return Some(Self {
                        data_offset: offset,
                        encoding,
                        channels,
                        sample_rate,
                        frames,
                    });
                }
                _ => {}
            }
            at = body.checked_add(size)?.checked_add(size & 1)?;
        }
        None
    }

    /// Whether the decoder read the same audio: same rate, channels and length. A layout that
    /// disagrees is not used (the file goes to the PCM cache).
    pub fn matches(&self, sample_rate: u32, channels: u16, frames: u64) -> bool {
        self.sample_rate == sample_rate && self.channels == channels && self.frames == frames
    }

    /// Converts whole frames of raw sample bytes (as read from `data_offset + k × frame_bytes`)
    /// to interleaved `f32`, appended to `out`. A trailing partial frame is ignored.
    pub fn convert(&self, bytes: &[u8], out: &mut Vec<f32>) {
        let size = self.encoding.bytes();
        let whole = bytes.len() / self.frame_bytes() * self.frame_bytes();
        out.extend(
            bytes[..whole]
                .chunks_exact(size)
                .map(|s| self.encoding.sample(s)),
        );
    }
}
