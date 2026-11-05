//! `record.*`: MIDI recording (epic 9, RE; `docs/specs/03-enregistrement-midi.md`): the
//! settings, the start and the end of a take, and the command that writes a take into the
//! project as **one** history entry.
//!
//! How a take travels (the page owns the project, the engine stamps the notes):
//!
//! 1. `record.start` arms the capture in the engine (`SetRecording`), and plays when the
//!    transport is stopped (the engine counts in first, `count_in_bars` of the metronome).
//!    The armed MIDI tracks record (`track.arm`); a selected track that is not armed plays
//!    and records nothing.
//! 2. The engine stamps the notes played on those tracks in transport ticks, moved back by the
//!    take's shift (the latency plus the offset, `SetRecording`, #560), and returns them in its
//!    report (`engine::recorder`). Nothing is written in the project meanwhile.
//! 3. `record.stop` (or `transport.stop`, `transport.pause`) ends the capture; the notes still
//!    held are cut there. The host, which reads the report, then writes the take with
//!    `record.commit_take` once the engine has returned its last notes, or drops it with
//!    `record.discard_take` when nothing was played. The command layer never reads the
//!    report, so `record.stop` cannot carry the notes itself.
//!
//! `record.commit_take` is self-contained (the target, the shift and the notes are its
//! input), so an agent can also write a take it played. Its inverse is `record.restore`,
//! which puts every clip the take touched back as it was.

use crate::clip::{ClipSnapshot, PlacementSnapshot, check_name, invalid, midi_track, not_found};
use crate::contract::{
    ClipPart, CommandError, Context, Outcome, ProjectEvent, State, TakeState, Undo,
};
use crate::macros::domain;
use project::limits::MAX_TICK;
use project::{
    Clip, ClipContent, ClipId, InputGrid, Note, PPQ, Placement, PlacementId, Project, SceneId,
    Span, Take, Tick, TickLoop, TrackId, TrackKind,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The largest manual latency offset, in milliseconds, either way.
pub const MAX_OFFSET_MS: i32 = 500;
/// The largest output latency a host may report, in milliseconds.
pub const MAX_LATENCY_MS: f64 = 1000.0;

/// Where a take goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "at", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordTarget {
    /// The launch grid: the cell of each armed track in this scene. A cell with a clip
    /// takes the notes on top of its own (overdub); an empty cell gets a new clip.
    Scene { scene: SceneId },
    /// The arrangement: each note goes where it was played, into the clip placed there, or
    /// into a new clip where no clip is placed.
    Arrangement {},
}

/// A note of a take, as the engine stamped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TakeNote {
    /// The transport tick where it was played (before the latency shift); may be negative.
    pub start: Tick,
    /// How long it was held, in ticks, at least 1.
    pub length: Tick,
    #[schemars(range(max = 127))]
    pub pitch: u8,
    #[schemars(range(min = 1, max = 127))]
    pub velocity: u8,
}

/// The notes a take recorded on one track.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TakeTrack {
    pub track: TrackId,
    pub notes: Vec<TakeNote>,
    /// For a scene target whose cell holds a clip that played during the take: the
    /// transport tick of the clip's position 0, as the engine reported it. Without it the
    /// clip's position 0 is the bar of the first note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Tick>,
}

/// A clip a take wrote into.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TakeClip {
    pub track: TrackId,
    pub clip: ClipId,
    /// The clip was created by the take.
    pub created: bool,
    /// Notes of the take written into it (a note already in the clip counts once).
    pub notes: u32,
}

/// A clip as `record.restore` puts it back: its content and its placements.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoredClip {
    pub snapshot: ClipSnapshot,
    pub placements: Vec<PlacementSnapshot>,
}

/// One clip of `record.restore`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipRestore {
    pub track: TrackId,
    pub clip: ClipId,
    /// What the clip becomes; `null` removes it with its placements.
    pub state: Option<RestoredClip>,
}

domain! {
    domain = "record", schema = ("RecordCommand", "RecordOutput");

    /// Change the settings of recording; every field is optional and a field left out stays
    /// as it is. `offset_ms` (-500 to 500): every recorded note is moved this much earlier,
    /// on top of the output latency the host estimates (negative: later). `replace`: a take
    /// removes, in the clip it writes into, the notes that start in the range it recorded
    /// (from its first note to the end of its last), instead of adding to them (overdub,
    /// the default). `quantize_on_input` (`none`, `quarter`, `eighth`, `sixteenth`,
    /// `thirty-second`, `eighth-triplet`, `sixteenth-triplet`; default `none`) and
    /// `quantize_strength` (0 to 100 percent, default 100): the starts of the take's own
    /// notes are moved toward that grid of the clip when `record.commit_take` writes the
    /// take, never the notes the clip already had, and never their lengths. They belong to
    /// the machine, so they are session state: saved with the session, never in the
    /// project, and not undoable. A take uses the settings at the moment it is written, unless `record.commit_take` is given its own.
    "record.set_settings" (kind = "live", undo = "none", destructive = false)
    SetSettings {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset_ms: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replace: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantize_on_input: Option<InputGrid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 0, max = 100))]
        quantize_strength: Option<u8>,
    } => {
        /// The offset before the command.
        previous_offset_ms: i32,
        /// Replace mode before the command.
        previous_replace: bool,
        /// The input grid before the command.
        previous_quantize_on_input: InputGrid,
        /// The quantize strength before the command.
        previous_quantize_strength: u8,
    };

    /// Start recording the armed MIDI tracks (`track.arm`); refused when none is armed or
    /// when a take is still waiting to be committed. `target`: `{"at": "scene", "scene":
    /// <id>}` (the launch grid: overdub into the cell's clip, or a new clip in an empty
    /// cell) or `{"at": "arrangement"}` (the default: each note where it is played). From a
    /// stopped transport it plays from the play position, after the metronome's count-in
    /// bars (heard, not recorded), and with a scene target it launches that scene's clip
    /// of each armed track first; on a running transport it records at once. `latency_ms`
    /// is the output latency the host estimates (0 to 1000, default 0). The notes come
    /// back from the engine, already moved `shift_ms` earlier (the latency plus the session's
    /// offset) on the engine's clock, so that a note heard before a loop wrap stays before it;
    /// `record.stop` ends the take, which the host then writes with `record.commit_take`
    /// (`shift_ms` 0), one history entry. Not undoable itself.
    "record.start" (kind = "live", undo = "none", destructive = false)
    Start {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<RecordTarget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        latency_ms: Option<f64>,
    } => {
        /// The take's number.
        take: u64,
        target: RecordTarget,
        /// The tracks that record.
        tracks: Vec<TrackId>,
        /// How much earlier the engine places the notes: `latency_ms` plus the session's
        /// offset (`record.set_settings`), in milliseconds. The notes it reports are already
        /// moved: they are written with `record.commit_take`'s `shift_ms` 0.
        shift_ms: f64,
        /// Whether a count-in runs before the take.
        count_in: bool,
    };

    /// End the capture of the running take; the transport keeps playing. The notes still
    /// held are cut here. The host writes the take with `record.commit_take` once the
    /// engine has returned its last notes (within one report, about 33 ms). Does nothing
    /// when no take is recording. Not undoable itself.
    "record.stop" (kind = "live", undo = "none", destructive = false)
    Stop {} => {
        /// The take ended; `null` when none was recording.
        take: Option<u64>,
    };

    /// Drop the current take without writing anything (undo during a take): the capture
    /// stops if it runs. Does nothing when there is no take. Not undoable: it wrote nothing.
    "record.discard_take" (kind = "live", undo = "none", destructive = false)
    DiscardTake {} => {
        /// The take dropped, if there was one.
        take: Option<u64>,
    };

    /// Write a take into the project, every track at once, as **one** history entry. Per
    /// track, `notes` are the notes as played: `start` in transport ticks, `length` in
    /// ticks. Every start is first moved `shift_ms` earlier at the project's tempo; when the
    /// transport loops and the start is strictly inside the loop, the move wraps round the
    /// loop (a note that would go before the loop's start comes from its end). The notes the
    /// engine reported for a `record.start` take are already moved: write them with
    /// `shift_ms` 0 (the default); `shift_ms` is for notes not moved yet. With a
    /// scene target, a cell that holds a clip takes the notes on top of its own (overdub):
    /// a note lands where the clip played at that moment, from `anchor` (the transport tick
    /// of the clip's position 0) or else from the bar of the first note, round its loop;
    /// a clip without a loop grows to the bar after the last note. An empty cell gets a new
    /// clip from the bar of the first note to the bar after the last one (at least one
    /// bar), looping over its length. With the arrangement target, a note inside a
    /// placement goes into its clip where the placement shows it; the other notes make one
    /// new clip per gap between placements, from the bar of its first note to the bar after
    /// its last, cut at the next placement. A note that starts at most a sixteenth before
    /// tick 0 starts there with its length; one that starts earlier starts there, cut, when
    /// it ends after it, and is dropped and counted otherwise, as a note before its clip; a
    /// note longer than its clip is cut at the clip's end. With
    /// `replace` on (`record.set_settings`), a clip that takes notes first loses the notes
    /// that start in the range the track recorded (its first note to the end of its last),
    /// counted in `replaced`. With `quantize_on_input` on, the starts of the take's notes
    /// move toward the grid of their clip by `quantize_strength` (a note stays inside its
    /// clip, lengths never change). Undo: `record.restore` puts the clips back as they
    /// were, replaced notes included. `replace`, `quantize_on_input` and `quantize_strength`
    /// override the session's settings (`record.set_settings`) for this write only; left out,
    /// the session's apply.
    "record.commit_take" (kind = "edit", undo = "inverse", destructive = false)
    CommitTake {
        /// The take of `record.start` these notes are, when they are one: it is then
        /// written and the next take may start. Leave it out to write notes of your own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        take: Option<u64>,
        target: RecordTarget,
        #[serde(default)]
        shift_ms: f64,
        tracks: Vec<TakeTrack>,
        /// Replace mode for this take; leave it out for the session's setting.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replace: Option<bool>,
        /// The input grid for this take; leave it out for the session's setting.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quantize_on_input: Option<InputGrid>,
        /// The quantize strength (0 to 100) for this take; leave it out for the session's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 0, max = 100))]
        quantize_strength: Option<u8>,
    } => {
        clips: Vec<TakeClip>,
        /// Notes left out: before tick 0, before their clip, or outside a clip without a loop.
        dropped: u32,
        /// Notes of the clips removed by replace mode.
        replaced: u32,
    };

    /// Put clips back exactly as given, with their identifiers: `state` is the clip's
    /// snapshot and its placements, or `null` to remove the clip with its placements. The
    /// inverse of `record.commit_take`. Refused when a track is not a MIDI track or when
    /// the result breaks a rule of the project (a note outside its clip, a slot taken, an
    /// identifier in use). Undo: the restore of what it changed.
    "record.restore" (kind = "edit", undo = "inverse", destructive = false)
    Restore {
        clips: Vec<ClipRestore>,
    } => {};
}

type Out = Outcome<Command, Output>;

fn live(output: Output, events: Vec<ProjectEvent>) -> Out {
    Outcome {
        output,
        undo: Undo::None,
        events,
        control: vec![],
    }
}

/// Applies a command of this domain.
pub fn apply(command: &Command, state: &mut State, ctx: &mut Context) -> Result<Out, CommandError> {
    match command {
        Command::SetSettings {
            offset_ms,
            replace,
            quantize_on_input,
            quantize_strength,
        } => {
            if let Some(offset_ms) = offset_ms
                && !(-MAX_OFFSET_MS..=MAX_OFFSET_MS).contains(offset_ms)
            {
                return Err(invalid(
                    "offset_ms",
                    format!("an offset is -{MAX_OFFSET_MS} to {MAX_OFFSET_MS} ms, not {offset_ms}"),
                ));
            }
            if let Some(strength) = quantize_strength
                && *strength > 100
            {
                return Err(invalid(
                    "quantize_strength",
                    format!("a strength is 0 to 100 percent, not {strength}"),
                ));
            }
            let previous = state.session.record;
            let mut next = previous;
            if let Some(v) = offset_ms {
                next.offset_ms = *v;
            }
            if let Some(v) = replace {
                next.replace = *v;
            }
            if let Some(v) = quantize_on_input {
                next.quantize_on_input = *v;
            }
            if let Some(v) = quantize_strength {
                next.quantize_strength = *v;
            }
            state.session.record = next;
            let events = if previous == next {
                vec![]
            } else {
                vec![ProjectEvent::RecordSettingsChanged {
                    offset_ms: next.offset_ms,
                    replace: next.replace,
                    quantize_on_input: next.quantize_on_input,
                    quantize_strength: next.quantize_strength,
                }]
            };
            Ok(live(
                Output::SetSettings {
                    previous_offset_ms: previous.offset_ms,
                    previous_replace: previous.replace,
                    previous_quantize_on_input: previous.quantize_on_input,
                    previous_quantize_strength: previous.quantize_strength,
                },
                events,
            ))
        }
        Command::Start { target, latency_ms } => start(state, ctx, target.as_ref(), *latency_ms),
        Command::Stop {} => {
            let event = end_capture(ctx);
            let take = match &event {
                Some(ProjectEvent::RecordingChanged { take, .. }) => Some(*take),
                _ => None,
            };
            Ok(live(Output::Stop { take }, event.into_iter().collect()))
        }
        Command::DiscardTake {} => {
            let take = ctx.engine.take.take();
            Ok(live(
                Output::DiscardTake {
                    take: take.map(|t| t.number),
                },
                take.map(|t| ProjectEvent::RecordingChanged {
                    take: t.number,
                    state: TakeState::Discarded,
                })
                .into_iter()
                .collect(),
            ))
        }
        Command::CommitTake {
            take,
            target,
            shift_ms,
            tracks,
            replace,
            quantize_on_input,
            quantize_strength,
        } => {
            let settings = state.session.record;
            let strength = quantize_strength.unwrap_or(settings.quantize_strength);
            if strength > 100 {
                return Err(invalid(
                    "quantize_strength",
                    format!("a strength is 0 to 100 percent, not {strength}"),
                ));
            }
            let opts = Opts {
                replace: replace.unwrap_or(settings.replace),
                grid: quantize_on_input
                    .unwrap_or(settings.quantize_on_input)
                    .ticks(),
                strength,
            };
            let mut out = commit(state, ctx, target, *shift_ms, tracks, opts)?;
            // The take the engine captured is written: the next one may start.
            if let Some(t) = ctx.engine.take
                && !t.recording
                && *take == Some(t.number)
            {
                ctx.engine.take = None;
                out.events.push(ProjectEvent::RecordingChanged {
                    take: t.number,
                    state: TakeState::Committed,
                });
            }
            Ok(out)
        }
        Command::Restore { clips } => restore(state, clips),
    }
}

/// The MIDI tracks that record: the armed ones that exist, in id order.
pub fn recording_tracks(state: &State) -> Vec<TrackId> {
    state
        .session
        .armed
        .iter()
        .filter(|id| {
            state
                .project
                .tracks
                .get(*id)
                .is_some_and(|t| t.kind == TrackKind::Midi)
        })
        .cloned()
        .collect()
}

fn start(
    state: &mut State,
    ctx: &mut Context,
    target: Option<&RecordTarget>,
    latency_ms: Option<f64>,
) -> Result<Out, CommandError> {
    if ctx.engine.take.is_some() {
        return Err(invalid(
            "take",
            "a take is still running or waiting to be written: stop it (record.stop) or drop it (record.discard_take)",
        ));
    }
    let latency = latency_ms.unwrap_or(0.0);
    if !(latency.is_finite() && (0.0..=MAX_LATENCY_MS).contains(&latency)) {
        return Err(invalid(
            "latency_ms",
            format!("a latency is 0 to {MAX_LATENCY_MS} ms, not {latency}"),
        ));
    }
    let target = target.cloned().unwrap_or(RecordTarget::Arrangement {});
    if let RecordTarget::Scene { scene } = &target
        && !state.project.scenes.contains_key(scene)
    {
        return Err(not_found("scene", scene));
    }
    let tracks = recording_tracks(state);
    if tracks.is_empty() {
        return Err(invalid(
            "tracks",
            "no MIDI track is armed: arm one (track.arm) to record it",
        ));
    }
    let project = &state.project;
    let metronome = project.transport.metronome;
    // The engine's rule (`engine::engine`, the count-in): from a stopped transport, with
    // bars to count, and a cue before the end of the loop when the transport loops.
    let loop_end = project
        .transport
        .loop_region
        .filter(|_| project.transport.loop_enabled)
        .map(|r| r.end());
    let count_in = !ctx.engine.playing
        && metronome.count_in_bars > 0
        && loop_end.is_none_or(|end| ctx.engine.cue < end);
    ctx.engine.takes += 1;
    let number = ctx.engine.takes;
    ctx.engine.take = Some(Take {
        number,
        recording: true,
    });
    Ok(live(
        Output::Start {
            take: number,
            target,
            tracks,
            shift_ms: take_shift_ms(Some(latency), state),
            count_in,
        },
        vec![ProjectEvent::RecordingChanged {
            take: number,
            state: TakeState::Recording,
        }],
    ))
}

/// How much earlier the notes of a take that `record.start` starts are placed, in
/// milliseconds: the host's output latency (`latency_ms`, 0 when left out) plus the session's
/// offset. The engine applies it (`SetRecording`, #560) and `record.start` answers it.
pub fn take_shift_ms(latency_ms: Option<f64>, state: &State) -> f64 {
    latency_ms.unwrap_or(0.0) + f64::from(state.session.record.offset_ms)
}

/// A shift in milliseconds as the engine takes it (`SetRecording`): whole microseconds,
/// saturating (a shift `record.start` accepts is at most 1500 ms either way).
pub fn shift_us(shift_ms: f64) -> i32 {
    // `as` saturates and sends NaN to zero.
    (shift_ms * 1000.0).round() as i32
}

/// Ends the capture of a recording take, if one runs (a transport stop or pause): the
/// event a domain adds to its outcome.
pub(crate) fn end_capture(ctx: &mut Context) -> Option<ProjectEvent> {
    match ctx.engine.take {
        Some(Take {
            number,
            recording: true,
        }) => {
            ctx.engine.take = Some(Take {
                number,
                recording: false,
            });
            Some(ProjectEvent::RecordingChanged {
                take: number,
                state: TakeState::Stopped,
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- commit

/// A clip's length and loop, and the position the clip plays `e` ticks after its
/// position 0, the way the engine reads it (`engine::clip_player`): straight up to the
/// loop's end, then round the loop (the same formula before 0); without a loop, nothing
/// outside `[0, length)`.
#[derive(Clone, Copy)]
struct Geo {
    length: i64,
    looping: Option<(i64, i64)>,
}

impl Geo {
    fn of(clip: &Clip) -> Option<Geo> {
        match &clip.content {
            ClipContent::Midi {
                length, looping, ..
            } => Some(Geo {
                length: length.get(),
                looping: looping.map(|l| (l.start.get(), l.end.get())),
            }),
            ClipContent::Audio { .. } => None,
        }
    }

    fn position(self, e: i64) -> Option<i64> {
        match self.looping {
            Some((ls, le)) if le > ls => {
                if (0..le).contains(&e) {
                    Some(e)
                } else {
                    Some(ls + (e - le).rem_euclid(le - ls))
                }
            }
            _ => (0..self.length).contains(&e).then_some(e),
        }
    }
}

fn floor_to(t: i64, step: i64) -> i64 {
    t.div_euclid(step) * step
}

fn ceil_to(t: i64, step: i64) -> i64 {
    floor_to(t + step - 1, step)
}

/// What a take changes in one clip.
enum Write {
    /// Notes into an existing clip, which may grow.
    Into {
        clip: ClipId,
        notes: Vec<Note>,
        length: Option<i64>,
        /// Replace mode: the clip positions `[from, to)` whose notes (by start) go.
        erase: Vec<(i64, i64)>,
    },
    /// A new clip: in a slot of the grid, or placed in the arrangement.
    New {
        slot: Option<SceneId>,
        start: i64,
        length: i64,
        looping: bool,
        notes: Vec<Note>,
    },
}

/// How early a note may be played before the bar a new clip starts on and still belong to it,
/// in ticks: a sixteenth. A downbeat played in time, then moved earlier by the latency shift,
/// lands a few ticks before the bar line; it starts the clip on that bar, not on the bar before.
/// The engine keeps a note heard that early before the cue of a count-in for the same reason
/// (#723): one tolerance for both.
pub const EARLY: i64 = engine::recorder::COUNT_IN_EARLY;

/// The bar a take's first note starts a clip on, with [`EARLY`] of tolerance.
fn bar_of(first: i64, bar: i64) -> i64 {
    floor_to(first.saturating_add(EARLY), bar)
}

/// A position `e` ticks from a clip start taken from the first note: up to [`EARLY`] before it
/// counts as the start.
fn early(e: i64) -> i64 {
    if (-EARLY..0).contains(&e) { 0 } else { e }
}

/// How a take is written: replace mode and the quantization on input (`record.set_settings`).
#[derive(Clone, Copy)]
struct Opts {
    replace: bool,
    grid: Option<i64>,
    strength: u8,
}

/// The clip positions `[from, to)` a stretch of `e0..e1` ticks from the clip's position 0
/// plays, the way [`Geo::position`] reads one tick: a loop is walked lap by lap (it wraps into
/// pieces), at most three pieces, after which a looping clip is covered whole.
fn stretch(geo: Geo, e0: i64, e1: i64) -> Vec<(i64, i64)> {
    if e1 <= e0 {
        return vec![];
    }
    match geo.looping {
        Some((ls, le)) if le > ls => {
            let p = le - ls;
            let mut out = Vec::new();
            let mut cur = e0;
            for round in 0..4 {
                if cur >= e1 {
                    break;
                }
                if round == 3 {
                    out.push((ls, le));
                    break;
                }
                let pos = if (0..le).contains(&cur) {
                    cur
                } else {
                    ls + (cur - le).rem_euclid(p)
                };
                let mut run = le - pos;
                if cur < 0 {
                    run = run.min(-cur);
                }
                let run = run.min(e1 - cur).max(1);
                out.push((pos, pos + run));
                cur += run;
            }
            out
        }
        _ => {
            let (a, b) = (e0.max(0), e1.min(geo.length));
            if b > a { vec![(a, b)] } else { vec![] }
        }
    }
}

/// A note at `pos` in a clip of `length`, cut at its end; `None` when it starts outside.
fn place(n: &TakeNote, pos: i64, length: i64, opts: Opts) -> Option<Note> {
    let pos = early(pos);
    if pos < 0 || pos >= length {
        return None;
    }
    let len = n.length.get().max(1).min(length - pos);
    let start = match opts.grid {
        Some(grid) => {
            crate::clip::quantized_start(pos, grid, opts.strength).clamp(0, (length - len).max(0))
        }
        None => pos,
    };
    Some(Note::new(Tick(start), n.pitch, Tick(len), n.velocity))
}

fn check_take(tracks: &[TakeTrack], project: &Project) -> Result<(), CommandError> {
    let mut seen = BTreeSet::new();
    for (i, t) in tracks.iter().enumerate() {
        if !seen.insert(&t.track) {
            return Err(invalid(
                format!("tracks[{i}].track"),
                "a track is listed twice",
            ));
        }
        midi_track(project, &t.track)?;
        for (k, n) in t.notes.iter().enumerate() {
            let field = format!("tracks[{i}].notes[{k}]");
            if n.pitch > 127 || !(1..=127).contains(&n.velocity) || n.length.get() < 1 {
                return Err(invalid(
                    field,
                    "a note has a pitch 0 to 127, a velocity 1 to 127 and a length of at least 1",
                ));
            }
            if n.start.get().abs() > MAX_TICK || n.length.get() > MAX_TICK {
                return Err(invalid(
                    field,
                    format!("a note's start and length are within {MAX_TICK} ticks"),
                ));
            }
        }
    }
    Ok(())
}

/// The writes of one track's notes (already shifted), and how many were dropped.
fn plan_track(
    project: &Project,
    target: &RecordTarget,
    track_id: &TrackId,
    notes: &[TakeNote],
    anchor: Option<Tick>,
    opts: Opts,
) -> (Vec<Write>, u32) {
    let bar = project.time_signature().ticks_per_bar().get().max(1);
    let Some(track) = project.tracks.get(track_id) else {
        return (vec![], 0);
    };
    let mut dropped = 0u32;
    let mut writes = Vec::new();
    if notes.is_empty() {
        return (writes, 0);
    }
    let first = notes.iter().map(|n| n.start.get()).min().unwrap_or(0);
    let last_end = notes
        .iter()
        .map(|n| n.start.get().saturating_add(n.length.get()))
        .max()
        .unwrap_or(first + 1);
    match target {
        RecordTarget::Scene { scene } => {
            let cell = track
                .clips
                .iter()
                .find(|(_, c)| c.slot.as_ref() == Some(scene))
                .and_then(|(id, c)| Some((id.clone(), Geo::of(c)?)));
            match cell {
                Some((clip, geo)) => {
                    let origin = anchor.map_or(bar_of(first, bar), Tick::get);
                    let grows = geo.looping.is_none();
                    let original = geo.length;
                    let mut length = geo.length;
                    if grows {
                        let end = ceil_to(last_end - origin, bar);
                        length = length.max(end);
                    }
                    let geo = Geo { length, ..geo };
                    let mut out = Vec::new();
                    for n in notes {
                        match geo
                            .position(if anchor.is_some() {
                                n.start.get() - origin
                            } else {
                                early(n.start.get() - origin)
                            })
                            .and_then(|pos| place(n, pos, length, opts))
                        {
                            Some(note) => out.push(note),
                            None => dropped += 1,
                        }
                    }
                    let erase = if opts.replace {
                        let from = first - origin;
                        let from = if anchor.is_some() { from } else { early(from) };
                        stretch(geo, from, last_end - origin)
                    } else {
                        vec![]
                    };
                    writes.push(Write::Into {
                        clip,
                        notes: out,
                        length: (length != original).then_some(length),
                        erase,
                    });
                }
                None => {
                    let origin = bar_of(first, bar);
                    let length = ceil_to(last_end - origin, bar).max(bar);
                    let out = notes
                        .iter()
                        .filter_map(|n| place(n, n.start.get() - origin, length, opts))
                        .collect();
                    writes.push(Write::New {
                        slot: Some(scene.clone()),
                        start: 0,
                        length,
                        looping: true,
                        notes: out,
                    });
                }
            }
        }
        RecordTarget::Arrangement {} => {
            // The track's MIDI placements, by start.
            let mut spans: Vec<(i64, i64, i64, &ClipId)> = track
                .placements
                .values()
                .filter_map(|p| match p.span {
                    Span::Ticks { length, offset } => {
                        Some((p.start.get(), length.get(), offset.get(), &p.clip))
                    }
                    Span::Frames { .. } => None,
                })
                .collect();
            spans.sort_by_key(|s| s.0);
            let mut into: BTreeMap<ClipId, Vec<Note>> = BTreeMap::new();
            let mut erase: BTreeMap<ClipId, Vec<(i64, i64)>> = BTreeMap::new();
            // Gap (its start and end, `None`: no placement after) → notes outside clips.
            let mut gaps: BTreeMap<(i64, Option<i64>), Vec<&TakeNote>> = BTreeMap::new();
            // A note that starts at most [`EARLY`] before tick 0 (the first downbeat played a
            // little early, or moved earlier by the latency shift, #723) is moved to 0 with its
            // length, as a new clip's early rule does; one that starts earlier starts at 0 when
            // it ends after it (cut there), and is dropped otherwise.
            let mut kept = Vec::with_capacity(notes.len());
            for n in notes {
                let (t, end) = (n.start.get(), n.start.get().saturating_add(n.length.get()));
                if t >= 0 {
                    kept.push(*n);
                } else if t >= -EARLY {
                    kept.push(TakeNote {
                        start: Tick::ZERO,
                        ..*n
                    });
                } else if end > 0 {
                    kept.push(TakeNote {
                        start: Tick::ZERO,
                        length: Tick(end),
                        ..*n
                    });
                } else {
                    dropped += 1;
                }
            }
            for n in &kept {
                let t = n.start.get();
                let inside = spans
                    .iter()
                    .find(|(start, len, _, _)| (*start..start + len).contains(&t));
                match inside {
                    Some((start, _, offset, clip)) => {
                        let placed = track.clips.get(*clip).and_then(Geo::of).and_then(|geo| {
                            let pos = geo.position(offset + t - start)?;
                            place(n, pos, geo.length, opts)
                        });
                        match placed {
                            Some(note) => into.entry((*clip).clone()).or_default().push(note),
                            None => dropped += 1,
                        }
                    }
                    None => {
                        let before = spans
                            .iter()
                            .filter(|(start, _, _, _)| *start <= t)
                            .map(|(start, len, _, _)| start + len)
                            .max()
                            .unwrap_or(0);
                        let after = spans
                            .iter()
                            .map(|(start, _, _, _)| *start)
                            .filter(|start| *start > t)
                            .min();
                        gaps.entry((before, after)).or_default().push(n);
                    }
                }
            }
            if opts.replace && !kept.is_empty() {
                let a = kept.iter().map(|n| n.start.get()).min().unwrap_or(0);
                let b = kept
                    .iter()
                    .map(|n| n.start.get().saturating_add(n.length.get()))
                    .max()
                    .unwrap_or(a);
                for (start, len, offset, clip) in &spans {
                    let (t0, t1) = (a.max(*start), b.min(start + len));
                    let Some(geo) = track.clips.get(*clip).and_then(Geo::of) else {
                        continue;
                    };
                    if t1 > t0 {
                        erase.entry((*clip).clone()).or_default().extend(stretch(
                            geo,
                            offset + t0 - start,
                            offset + t1 - start,
                        ));
                        into.entry((*clip).clone()).or_default();
                    }
                }
            }
            for (clip, notes) in into {
                let erase = erase.remove(&clip).unwrap_or_default();
                writes.push(Write::Into {
                    clip,
                    notes,
                    length: None,
                    erase,
                });
            }
            for ((before, after), group) in gaps {
                let first = group.iter().map(|n| n.start.get()).min().unwrap_or(0);
                let last = group
                    .iter()
                    .map(|n| n.start.get().saturating_add(n.length.get()))
                    .max()
                    .unwrap_or(first + 1);
                let start = bar_of(first, bar).max(before);
                let mut end = ceil_to(last, bar).max(start + 1);
                if let Some(next) = after {
                    end = end.min(next);
                }
                let length = end - start;
                let out = group
                    .iter()
                    .filter_map(|n| place(n, n.start.get() - start, length, opts))
                    .collect();
                writes.push(Write::New {
                    slot: None,
                    start,
                    length,
                    looping: false,
                    notes: out,
                });
            }
        }
    }
    (writes, dropped)
}

/// A clip's snapshot with its placements, for `record.restore`.
fn restored(track: &project::Track, clip_id: &ClipId) -> Result<RestoredClip, CommandError> {
    let clip = track
        .clips
        .get(clip_id)
        .ok_or_else(|| not_found("clip", clip_id))?;
    let snapshot = crate::clip::snapshot(clip)?;
    let mut placements = Vec::new();
    for (id, p) in &track.placements {
        if p.clip != *clip_id {
            continue;
        }
        if let Span::Ticks { length, offset } = p.span {
            placements.push(PlacementSnapshot {
                id: id.clone(),
                start: p.start,
                length,
                offset,
            });
        }
    }
    Ok(RestoredClip {
        snapshot,
        placements,
    })
}

/// A take's start tick moved `shift` ticks earlier. A note stamped inside the transport's loop
/// is a position of the loop: the engine wrapped it when the stamp fell after the loop's end
/// (the host stamps the frame heard plus the output latency, `shift` brings it back). The move
/// is made on the loop, not on the line, so a note heard in the last milliseconds of the loop
/// stays there instead of leaving before its start (#542). A stamp outside the loop, a move
/// that is not backwards, a stamp exactly on the loop's start (a note pressed before the capture
/// began is clamped to the take's first tick: it stays on the line, where the clip's early rule
/// takes it to its first beat) and a transport that does not loop move on the line. Known limit:
/// a command cannot tell a stamp the engine wrapped from a note anticipating the loop's start
/// while the playhead enters the loop from before it (first pass only), which is folded too.
/// The page's takes are moved by the engine instead, which knows the wraps (#560), and come
/// here with a shift of 0: this fold serves the notes given with a shift of their own.
fn shift_start(start: i64, shift: i64, looping: Option<project::LoopRegion>) -> i64 {
    let line = start.saturating_sub(shift);
    let Some(region) = looping else {
        return line;
    };
    let (from, to) = (region.start().get(), region.end().get());
    if shift <= 0 || start <= from || start >= to || (from..to).contains(&line) {
        return line;
    }
    let len = i128::from(to) - i128::from(from);
    let wrapped = i128::from(from)
        + (i128::from(start) - i128::from(shift) - i128::from(from)).rem_euclid(len);
    i64::try_from(wrapped).unwrap_or(line)
}

fn commit(
    state: &mut State,
    ctx: &mut Context,
    target: &RecordTarget,
    shift_ms: f64,
    tracks: &[TakeTrack],
    opts: Opts,
) -> Result<Out, CommandError> {
    if !shift_ms.is_finite() || shift_ms.abs() > MAX_LATENCY_MS + f64::from(MAX_OFFSET_MS) {
        return Err(invalid(
            "shift_ms",
            format!(
                "a shift is at most {} ms either way",
                MAX_LATENCY_MS + f64::from(MAX_OFFSET_MS)
            ),
        ));
    }
    let project = &state.project;
    if let RecordTarget::Scene { scene } = target
        && !project.scenes.contains_key(scene)
    {
        return Err(not_found("scene", scene));
    }
    check_take(tracks, project)?;
    let shift = (shift_ms * project.tempo().bpm() * PPQ as f64 / 60_000.0).round() as i64;
    let looping = project
        .transport
        .loop_region
        .filter(|_| project.transport.loop_enabled);
    let mut changes: Vec<ClipRestore> = Vec::new();
    let mut before: Vec<ClipRestore> = Vec::new();
    let mut clips = Vec::new();
    let mut dropped = 0u32;
    let mut replaced = 0u32;
    let mut names: Vec<String> = Vec::new();
    for t in tracks {
        let shifted: Vec<TakeNote> = t
            .notes
            .iter()
            .map(|n| TakeNote {
                start: Tick(shift_start(n.start.get(), shift, looping)),
                ..*n
            })
            .collect();
        let (writes, lost) = plan_track(project, target, &t.track, &shifted, t.anchor, opts);
        dropped += lost;
        let track = midi_track(project, &t.track)?;
        for write in writes {
            match write {
                Write::Into {
                    clip,
                    notes,
                    length,
                    erase,
                } => {
                    let old = restored(track, &clip)?;
                    let mut new = old.clone();
                    let had: BTreeSet<Note> = old.snapshot.notes.iter().copied().collect();
                    let mut all: BTreeSet<Note> = had
                        .iter()
                        .copied()
                        .filter(|n| {
                            let s = n.start.get();
                            !erase.iter().any(|(from, to)| (*from..*to).contains(&s))
                        })
                        .collect();
                    let kept = all.len();
                    all.extend(notes.iter().copied());
                    let added = all.len() - kept;
                    if all == had && length.is_none() {
                        continue;
                    }
                    // Notes the clip had and no longer has: one the take plays again is not replaced.
                    let gone = had.difference(&all).count();
                    replaced = replaced.saturating_add(crate::clip::count(gone));
                    new.snapshot.notes = all.into_iter().collect();
                    if let Some(length) = length {
                        new.snapshot.length = Tick(length);
                    }
                    clips.push(TakeClip {
                        track: t.track.clone(),
                        clip: clip.clone(),
                        created: false,
                        notes: crate::clip::count(added),
                    });
                    before.push(ClipRestore {
                        track: t.track.clone(),
                        clip: clip.clone(),
                        state: Some(old),
                    });
                    changes.push(ClipRestore {
                        track: t.track.clone(),
                        clip,
                        state: Some(new),
                    });
                }
                Write::New {
                    slot,
                    start,
                    length,
                    looping,
                    notes,
                } => {
                    if notes.is_empty() {
                        continue;
                    }
                    let id = ClipId::generate(&mut ctx.ids);
                    let name = project.next_clip_name_beside(&t.track, &names);
                    check_name(&name, "name")?;
                    names.push(name.clone());
                    let notes: Vec<Note> = notes
                        .into_iter()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    let placements = if slot.is_none() {
                        vec![PlacementSnapshot {
                            id: PlacementId::generate(&mut ctx.ids),
                            start: Tick(start),
                            length: Tick(length),
                            offset: Tick::ZERO,
                        }]
                    } else {
                        vec![]
                    };
                    clips.push(TakeClip {
                        track: t.track.clone(),
                        clip: id.clone(),
                        created: true,
                        notes: crate::clip::count(notes.len()),
                    });
                    before.push(ClipRestore {
                        track: t.track.clone(),
                        clip: id.clone(),
                        state: None,
                    });
                    changes.push(ClipRestore {
                        track: t.track.clone(),
                        clip: id,
                        state: Some(RestoredClip {
                            snapshot: ClipSnapshot {
                                name,
                                color: None,
                                slot,
                                launch_quantize: None,
                                length: Tick(length),
                                looping: looping.then_some(TickLoop {
                                    start: Tick::ZERO,
                                    end: Tick(length),
                                }),
                                notes,
                            },
                            placements,
                        }),
                    });
                }
            }
        }
    }
    let output = Output::CommitTake {
        clips,
        dropped,
        replaced,
    };
    if changes.is_empty() {
        return Ok(live(output, vec![]));
    }
    let (events, _) = write_clips(state, &changes)?;
    Ok(Outcome {
        output,
        undo: Undo::Inverse(Box::new(Command::Restore { clips: before })),
        events,
        control: vec![],
    })
}

// ---------------------------------------------------------------- restore

fn restore(state: &mut State, clips: &[ClipRestore]) -> Result<Out, CommandError> {
    if clips.is_empty() {
        return Err(invalid("clips", "name at least one clip"));
    }
    let (events, inverse) = write_clips(state, clips)?;
    Ok(Outcome {
        output: Output::Restore {},
        undo: Undo::Inverse(Box::new(Command::Restore { clips: inverse })),
        events,
        control: vec![],
    })
}

/// Writes every clip as given on a copy of the project, checks the copy against the rules
/// of the project, then keeps it. Returns the events and what puts the clips back.
fn write_clips(
    state: &mut State,
    clips: &[ClipRestore],
) -> Result<(Vec<ProjectEvent>, Vec<ClipRestore>), CommandError> {
    let mut project = state.project.clone();
    let mut events = Vec::new();
    let mut inverse = Vec::new();
    let mut seen = BTreeSet::new();
    let mut arranged = BTreeSet::new();
    for (i, c) in clips.iter().enumerate() {
        let field = format!("clips[{i}]");
        if !seen.insert(&c.clip) {
            return Err(invalid(&field, "a clip is listed twice"));
        }
        midi_track(&project, &c.track)?;
        // The clip must be in this track or nowhere.
        if let Some((other, _)) = project
            .tracks
            .iter()
            .find(|(id, t)| **id != c.track && t.clips.contains_key(&c.clip))
        {
            return Err(invalid(
                format!("{field}.track"),
                format!("clip {} is in track {other}", c.clip),
            ));
        }
        let track = project
            .tracks
            .get_mut(&c.track)
            .ok_or_else(|| not_found("track", &c.track))?;
        let old = if track.clips.contains_key(&c.clip) {
            Some(restored(track, &c.clip)?)
        } else {
            None
        };
        inverse.push(ClipRestore {
            track: c.track.clone(),
            clip: c.clip.clone(),
            state: old.clone(),
        });
        let old_placements: Vec<PlacementId> = old
            .as_ref()
            .map(|o| o.placements.iter().map(|p| p.id.clone()).collect())
            .unwrap_or_default();
        track.clips.remove(&c.clip);
        track.placements.retain(|_, p| p.clip != c.clip);
        if let Some(new) = &c.state {
            let s = &new.snapshot;
            track.clips.insert(
                c.clip.clone(),
                Clip {
                    name: s.name.clone(),
                    color: s.color,
                    slot: s.slot.clone(),
                    launch_quantize: s.launch_quantize,
                    content: ClipContent::Midi {
                        length: s.length,
                        looping: s.looping,
                        notes: s.notes.iter().copied().collect(),
                    },
                },
            );
            for p in &new.placements {
                track.placements.insert(
                    p.id.clone(),
                    Placement {
                        clip: c.clip.clone(),
                        start: p.start,
                        span: Span::Ticks {
                            length: p.length,
                            offset: p.offset,
                        },
                    },
                );
            }
        }
        let new_placements: Vec<PlacementId> = c
            .state
            .as_ref()
            .map(|n| n.placements.iter().map(|p| p.id.clone()).collect())
            .unwrap_or_default();
        match (&old, &c.state) {
            (None, None) => {}
            (Some(o), Some(n))
                if o.snapshot.slot == n.snapshot.slot && o.placements == n.placements =>
            {
                if o != n {
                    events.push(ProjectEvent::ClipChanged {
                        clip: c.clip.clone(),
                        part: ClipPart::Content,
                    });
                }
            }
            _ => {
                if old.is_some() {
                    events.push(ProjectEvent::ClipDeleted {
                        track: c.track.clone(),
                        clip: c.clip.clone(),
                        placements: old_placements.clone(),
                    });
                }
                if c.state.is_some() {
                    events.push(ProjectEvent::ClipCreated {
                        track: c.track.clone(),
                        clip: c.clip.clone(),
                        placements: new_placements.clone(),
                    });
                }
                if !old_placements.is_empty() || !new_placements.is_empty() {
                    arranged.insert(c.track.clone());
                }
            }
        }
    }
    if let Err(violations) = project.validate() {
        let first = violations
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
        return Err(invalid(
            "clips",
            format!("the result breaks a rule: {first}"),
        ));
    }
    for track in arranged {
        events.push(ProjectEvent::ArrangementChanged { track });
    }
    state.project = project;
    Ok((events, inverse))
}
