//! Why an import failed, in terms the page shows and an agent reads.

use std::fmt;

/// Why a file cannot be imported. Every failure of the pipeline is one of these; none is a
/// panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// No reader recognises the file: not audio, or a container Faulk does not read.
    UnsupportedFormat,
    /// A container Faulk reads, holding audio in a codec it does not decode (AC-3 in MP4, or
    /// Opus of a channel layout other than mono or stereo, for two).
    UnsupportedCodec(String),
    /// The file is damaged: a header or a packet does not decode.
    Corrupt(String),
    /// The file holds no audio track, or no frame of audio.
    Empty,
    /// The sample rate is outside what a project takes (`project::limits::SAMPLE_RATES`).
    BadSampleRate(u32),
    /// No channel, or more than a project takes (`project::limits::MAX_CHANNELS`).
    BadChannels(u32),
    /// Longer than a project takes (`project::limits::MAX_FRAME` frames).
    TooLong,
    /// The host could not read the file (it changed or went away while it was read).
    Io(String),
}

impl ImportError {
    /// The stable code the page maps to a text (and an agent reads).
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedFormat => "unsupported_format",
            Self::UnsupportedCodec(_) => "unsupported_codec",
            Self::Corrupt(_) => "corrupt",
            Self::Empty => "empty",
            Self::BadSampleRate(_) => "bad_sample_rate",
            Self::BadChannels(_) => "bad_channels",
            Self::TooLong => "too_long",
            Self::Io(_) => "io",
        }
    }

    pub(crate) fn io(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat => write!(f, "not an audio file Faulk can read"),
            Self::UnsupportedCodec(codec) => {
                write!(f, "audio coded as {codec}, which Faulk does not decode")
            }
            Self::Corrupt(what) => write!(f, "the file is damaged: {what}"),
            Self::Empty => write!(f, "the file holds no audio"),
            Self::BadSampleRate(hz) => {
                write!(f, "a sample rate of {hz} Hz is outside 8 to 768 kHz")
            }
            Self::BadChannels(n) => write!(f, "{n} channels: a file has 1 to 64"),
            Self::TooLong => write!(f, "the file is longer than a project can hold"),
            Self::Io(what) => write!(f, "the file could not be read: {what}"),
        }
    }
}

impl std::error::Error for ImportError {}
