//! The project's key and scale (#695): a root pitch class and one of a fixed list of
//! scales. A project without one (`Project::scale` is `None`) is chromatic: every tool
//! behaves as it did before scales existed. What the scale drives (the piano roll's rows,
//! the MIDI effects that work "in the scale", agents writing music) reads it from here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The highest root: pitch classes are 0 (C) to 11 (B).
pub const MAX_ROOT: u8 = 11;

/// A key and a scale: `root` is the pitch class of the tonic (0 = C, 1 = C sharp, … 11 = B),
/// `mode` the scale built on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scale {
    /// The tonic's pitch class, 0 (C) to 11 (B).
    #[schemars(range(min = 0, max = 11))]
    pub root: u8,
    /// The scale built on the tonic.
    pub mode: ScaleMode,
}

/// The scales a project can be in (lead's decision on #695). Major is the Ionian mode and
/// natural minor the Aeolian one: they are not listed twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScaleMode {
    Major,
    NaturalMinor,
    HarmonicMinor,
    MelodicMinor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
}

impl ScaleMode {
    /// Every scale, in the order the page lists them.
    pub const ALL: [ScaleMode; 12] = [
        ScaleMode::Major,
        ScaleMode::NaturalMinor,
        ScaleMode::HarmonicMinor,
        ScaleMode::MelodicMinor,
        ScaleMode::Dorian,
        ScaleMode::Phrygian,
        ScaleMode::Lydian,
        ScaleMode::Mixolydian,
        ScaleMode::Locrian,
        ScaleMode::MajorPentatonic,
        ScaleMode::MinorPentatonic,
        ScaleMode::Blues,
    ];

    /// The scale's name as it is written in commands and files (`natural_minor`).
    pub fn name(self) -> &'static str {
        match self {
            ScaleMode::Major => "major",
            ScaleMode::NaturalMinor => "natural_minor",
            ScaleMode::HarmonicMinor => "harmonic_minor",
            ScaleMode::MelodicMinor => "melodic_minor",
            ScaleMode::Dorian => "dorian",
            ScaleMode::Phrygian => "phrygian",
            ScaleMode::Lydian => "lydian",
            ScaleMode::Mixolydian => "mixolydian",
            ScaleMode::Locrian => "locrian",
            ScaleMode::MajorPentatonic => "major_pentatonic",
            ScaleMode::MinorPentatonic => "minor_pentatonic",
            ScaleMode::Blues => "blues",
        }
    }

    /// The scale's degrees, in semitones above the root, from 0, rising. The melodic minor
    /// is its rising form (the jazz minor), the blues scale the six-note minor one.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            ScaleMode::Major => &[0, 2, 4, 5, 7, 9, 11],
            ScaleMode::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            ScaleMode::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            ScaleMode::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            ScaleMode::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            ScaleMode::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            ScaleMode::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            ScaleMode::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            ScaleMode::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            ScaleMode::MajorPentatonic => &[0, 2, 4, 7, 9],
            ScaleMode::MinorPentatonic => &[0, 3, 5, 7, 10],
            ScaleMode::Blues => &[0, 3, 5, 6, 7, 10],
        }
    }
}

/// What a command names as the scale (`project.set_scale`, `project.get`): one of the
/// scales, or `chromatic` for none (every note of the twelve; the project then holds no
/// scale).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScaleKind {
    Chromatic,
    Major,
    NaturalMinor,
    HarmonicMinor,
    MelodicMinor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
}

impl ScaleKind {
    /// The scale it names; `None` for `chromatic`.
    pub fn mode(self) -> Option<ScaleMode> {
        Some(match self {
            ScaleKind::Chromatic => return None,
            ScaleKind::Major => ScaleMode::Major,
            ScaleKind::NaturalMinor => ScaleMode::NaturalMinor,
            ScaleKind::HarmonicMinor => ScaleMode::HarmonicMinor,
            ScaleKind::MelodicMinor => ScaleMode::MelodicMinor,
            ScaleKind::Dorian => ScaleMode::Dorian,
            ScaleKind::Phrygian => ScaleMode::Phrygian,
            ScaleKind::Lydian => ScaleMode::Lydian,
            ScaleKind::Mixolydian => ScaleMode::Mixolydian,
            ScaleKind::Locrian => ScaleMode::Locrian,
            ScaleKind::MajorPentatonic => ScaleMode::MajorPentatonic,
            ScaleKind::MinorPentatonic => ScaleMode::MinorPentatonic,
            ScaleKind::Blues => ScaleMode::Blues,
        })
    }
}

impl From<Option<ScaleMode>> for ScaleKind {
    fn from(mode: Option<ScaleMode>) -> Self {
        match mode {
            None => ScaleKind::Chromatic,
            Some(ScaleMode::Major) => ScaleKind::Major,
            Some(ScaleMode::NaturalMinor) => ScaleKind::NaturalMinor,
            Some(ScaleMode::HarmonicMinor) => ScaleKind::HarmonicMinor,
            Some(ScaleMode::MelodicMinor) => ScaleKind::MelodicMinor,
            Some(ScaleMode::Dorian) => ScaleKind::Dorian,
            Some(ScaleMode::Phrygian) => ScaleKind::Phrygian,
            Some(ScaleMode::Lydian) => ScaleKind::Lydian,
            Some(ScaleMode::Mixolydian) => ScaleKind::Mixolydian,
            Some(ScaleMode::Locrian) => ScaleKind::Locrian,
            Some(ScaleMode::MajorPentatonic) => ScaleKind::MajorPentatonic,
            Some(ScaleMode::MinorPentatonic) => ScaleKind::MinorPentatonic,
            Some(ScaleMode::Blues) => ScaleKind::Blues,
        }
    }
}

impl Scale {
    /// The pitch classes of the scale (0 = C … 11 = B), sorted, without repeats. A root
    /// above [`MAX_ROOT`] (refused by [`crate::Project::validate`]) is read modulo 12.
    pub fn pitch_classes(self) -> Vec<u8> {
        let mut out: Vec<u8> = self
            .mode
            .intervals()
            .iter()
            .map(|i| (self.root % 12 + i) % 12)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Whether a MIDI pitch (0 to 127) is a note of the scale.
    pub fn contains(self, pitch: u8) -> bool {
        let pc = pitch % 12;
        self.mode
            .intervals()
            .iter()
            .any(|i| (self.root % 12 + i) % 12 == pc)
    }
}
