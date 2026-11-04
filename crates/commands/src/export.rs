//! What `project.export_audio` renders (issue #297): the range of the project, the format
//! of the file, and the engine stream of the offline render.
//!
//! The render itself is `engine::offline`: the same engine that plays the project, fed with
//! the same snapshot, run faster than real time. This module turns a project and the
//! command's choices into its inputs, once, for every host (the page through
//! `crates/project-wasm`, the helper, the tests).
//!
//! - **What is rendered.** By default the song: bar 1 to the end of the last placement of
//!   the arrangement, then the tail (effects ringing out) up to [`engine::offline::MAX_TAIL_SECS`].
//!   With [`ExportRegion::Loop`], the loop region once. The loop being on or off does not
//!   matter: the region is what the user marked, and a render is never looped.
//! - **What is not played.** The metronome (a click is not the song) and the loop. The
//!   session is kept: a solo left on is heard, as it is when playing.
//! - **Not yet**: audio clips play in the live engine since AU-4, but the offline render's
//!   engine has no disk host to read their files (#647): they do not sound in the render,
//!   though a placement of one counts for the song's length.

use engine::msg::{ControlMsg, MsgError};
use engine::offline::{self, MAX_BODY_SECS, SampleFormat};

const _: () = assert!(MAX_BODY_SECS as u64 == MAX_EXPORT_SECS);
use project::{ClipContent, Project, SampleRate, SessionState, Span, Tick};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::contract::CommandError;
use crate::engine_msgs::{EngineStream, snapshot};

/// The sample rates a file can have.
pub const SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 96_000];
/// The default sample rate.
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;
/// The epoch of the render's engine: a fresh engine takes any newer than 0.
const EPOCH: u16 = 1;

/// Which part of the project is rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportRegion {
    /// From the start to the end of the last placement of the arrangement, then the tail.
    #[default]
    Song,
    /// The loop region, once, then the tail.
    Loop,
    /// From the start of bar `from_bar` to the end of bar `to_bar` (both counted from 1, the
    /// second included), then the tail.
    Bars,
}

/// The longest export, in seconds of content (the tail comes on top).
pub const MAX_EXPORT_SECS: u64 = 1800;

/// How the samples are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportBitDepth {
    /// 16-bit integers.
    Pcm16,
    /// 24-bit integers.
    #[default]
    Pcm24,
    /// 32-bit floats. The output protection (#402) holds every format, floats included, under
    /// −0.3 dBFS, so no format clips; the choice is only size and precision, not headroom.
    Float32,
}

impl ExportBitDepth {
    /// The engine's format.
    pub fn format(self) -> SampleFormat {
        match self {
            ExportBitDepth::Pcm16 => SampleFormat::Pcm16,
            ExportBitDepth::Pcm24 => SampleFormat::Pcm24,
            ExportBitDepth::Float32 => SampleFormat::Float32,
        }
    }
}

/// The choices of `project.export_audio`, with their defaults filled in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportSettings {
    pub region: ExportRegion,
    pub bit_depth: ExportBitDepth,
    pub sample_rate: u32,
    /// First bar of `ExportRegion::Bars`, from 1.
    #[serde(default)]
    pub from_bar: Option<u32>,
    /// Last bar of `ExportRegion::Bars`, included.
    #[serde(default)]
    pub to_bar: Option<u32>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            region: ExportRegion::default(),
            bit_depth: ExportBitDepth::default(),
            sample_rate: DEFAULT_SAMPLE_RATE,
            from_bar: None,
            to_bar: None,
        }
    }
}

impl ExportSettings {
    /// The settings of a command's optional fields; a sample rate that a file cannot have,
    /// bars without the `bars` region (or the region without them), or bars out of order are
    /// refused.
    pub fn of(
        region: Option<ExportRegion>,
        bit_depth: Option<ExportBitDepth>,
        sample_rate: Option<u32>,
        from_bar: Option<u32>,
        to_bar: Option<u32>,
    ) -> Result<Self, CommandError> {
        let invalid = |field: &str, message: &str| CommandError::Invalid {
            field: field.into(),
            message: message.into(),
        };
        let region = region.unwrap_or_default();
        match (region, from_bar, to_bar) {
            (ExportRegion::Bars, Some(from), Some(to)) => {
                if from < 1 {
                    return Err(invalid("from_bar", "bars are counted from 1"));
                }
                if to < from {
                    return Err(invalid("to_bar", "the last bar comes after the first one"));
                }
            }
            (ExportRegion::Bars, ..) => {
                return Err(invalid(
                    "from_bar",
                    "the bars region needs from_bar and to_bar",
                ));
            }
            (_, None, None) => {}
            _ => {
                return Err(invalid(
                    "region",
                    "from_bar and to_bar go with the bars region only",
                ));
            }
        }
        let sample_rate = sample_rate.unwrap_or(DEFAULT_SAMPLE_RATE);
        if !SAMPLE_RATES.contains(&sample_rate) {
            return Err(CommandError::Invalid {
                field: "sample_rate".into(),
                message: format!("a sample rate is one of {SAMPLE_RATES:?}, not {sample_rate}"),
            });
        }
        Ok(Self {
            region,
            bit_depth: bit_depth.unwrap_or_default(),
            sample_rate,
            from_bar,
            to_bar,
        })
    }
}

/// What a render covers, before its tail.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportPlan {
    pub settings: ExportSettings,
    /// Where the render starts, in ticks.
    pub start_tick: Tick,
    /// Where the content ends, in ticks; the tail follows.
    pub end_tick: Tick,
    /// Frames of the content, at the sample rate.
    pub body_frames: u64,
    /// Seconds of the content.
    pub duration_secs: f64,
}

/// The last tick any placement of the arrangement reaches (0 when it is empty).
pub fn arrangement_end(project: &Project) -> Tick {
    let tempo = project.tempo();
    let mut end = 0i64;
    for track in project.tracks.values() {
        for placement in track.placements.values() {
            let length = match placement.span {
                Span::Ticks { length, .. } => length.0,
                Span::Frames { length, .. } => {
                    // An audio placement has its length in source frames: at the project's
                    // tempo it covers this many ticks.
                    let rate = match track.clips.get(&placement.clip).map(|c| &c.content) {
                        Some(ClipContent::Audio { media, .. }) => media.sample_rate,
                        _ => SampleRate::HZ_48000,
                    };
                    let seconds = length.0 as f64 / f64::from(rate.hz());
                    tempo.seconds_to_tick(seconds).0
                }
            };
            end = end.max(placement.start.0.saturating_add(length));
        }
    }
    Tick(end)
}

/// What `settings` renders of the project, or why nothing can be.
pub fn plan(project: &Project, settings: ExportSettings) -> Result<ExportPlan, CommandError> {
    let (start, end) = match settings.region {
        ExportRegion::Song => (Tick(0), arrangement_end(project)),
        ExportRegion::Bars => {
            let bar = project.time_signature().ticks_per_bar().0;
            let (from, to) = (
                i64::from(settings.from_bar.unwrap_or(1)),
                i64::from(settings.to_bar.unwrap_or(1)),
            );
            let (Some(start), Some(end)) = ((from - 1).checked_mul(bar), to.checked_mul(bar))
            else {
                return Err(CommandError::Invalid {
                    field: "to_bar".into(),
                    message: "that bar is beyond the end of time".into(),
                });
            };
            let ends = arrangement_end(project).0;
            if start >= ends {
                let bars = (ends + bar - 1) / bar;
                return Err(CommandError::Invalid {
                    field: "from_bar".into(),
                    message: if ends == 0 {
                        "the arrangement is empty: there is nothing to export".into()
                    } else {
                        format!(
                            "the arrangement ends in bar {bars}: there is nothing to export from bar {from}"
                        )
                    },
                });
            }
            (Tick(start), Tick(end))
        }
        ExportRegion::Loop => {
            let Some(region) = project.transport.loop_region else {
                return Err(CommandError::Invalid {
                    field: "region".into(),
                    message: "there is no loop region to export: set one, or export the song"
                        .into(),
                });
            };
            (region.start(), region.end())
        }
    };
    if end.0 <= start.0 {
        return Err(CommandError::Invalid {
            field: "region".into(),
            message: "the arrangement is empty: there is nothing to export".into(),
        });
    }
    let tempo = project.tempo();
    let rate = SampleRate::new(settings.sample_rate).unwrap_or(SampleRate::HZ_48000);
    let frames = (tempo.tick_to_sample(end, rate) - tempo.tick_to_sample(start, rate)).ceil();
    let duration_secs = frames / f64::from(rate.hz());
    if duration_secs > MAX_BODY_SECS {
        // The project is as it was: only this request is too long.
        return Err(CommandError::Limit {
            entity: "export seconds".into(),
            max: MAX_EXPORT_SECS,
        });
    }
    Ok(ExportPlan {
        settings,
        start_tick: start,
        end_tick: end,
        body_frames: frames as u64,
        duration_secs,
    })
}

/// A render ready to run: its plan and the two streams `engine::offline::Offline` takes.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportJob {
    pub plan: ExportPlan,
    /// A `Reset`, the snapshot of the project (no metronome, no loop) and the `Play` from
    /// the start of the range.
    pub setup: Vec<u8>,
    /// The `Stop` sent when the content has been rendered (it follows `setup` in the same
    /// stream).
    pub stop: Vec<u8>,
}

/// The project's render: see the module doc.
pub fn job(
    project: &Project,
    session: &SessionState,
    settings: ExportSettings,
) -> Result<ExportJob, ExportError> {
    let plan = plan(project, settings).map_err(ExportError::Refused)?;
    let mut project = project.clone();
    project.transport.metronome.enabled = false;
    project.transport.loop_enabled = false;
    let (snapshot, _) = snapshot(&project, session);
    let mut msgs = Vec::with_capacity(snapshot.len() + 2);
    msgs.push(ControlMsg::Reset { epoch: EPOCH });
    msgs.extend(snapshot);
    msgs.push(ControlMsg::Play {
        from: plan.start_tick,
    });
    let mut stream = EngineStream::new(EPOCH);
    let setup = stream.encode(&msgs).map_err(ExportError::Stream)?;
    let stop = stream
        .encode(&[ControlMsg::Stop])
        .map_err(ExportError::Stream)?;
    Ok(ExportJob { plan, setup, stop })
}

/// Why no render could be prepared.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportError {
    Refused(CommandError),
    /// The project's messages did not encode.
    Stream(MsgError),
}

/// The engine's offline plan of a project's job.
pub fn offline_plan(plan: &ExportPlan) -> offline::Plan {
    offline::Plan {
        body_frames: plan.body_frames,
        format: plan.settings.bit_depth.format(),
    }
}
