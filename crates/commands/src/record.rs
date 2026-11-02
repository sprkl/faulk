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
//! 2. The engine stamps the notes played on those tracks in transport ticks and returns them
//!    in its report (`engine::recorder`). Nothing is written in the project meanwhile.
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
    Clip, ClipContent, ClipId, Note, PPQ, Placement, PlacementId, Project, SceneId, Span, Take,
    Tick, TickLoop, TrackId, TrackKind,
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

    /// Set the manual latency offset of recording, in milliseconds, -500 to 500: every
    /// recorded note is moved this much earlier, on top of the output latency the host
    /// estimates (negative: later). It belongs to the machine, so it is session state:
    /// saved with the session, never in the project, and not undoable.
    "record.set_settings" (kind = "live", undo = "none", destructive = false)
    SetSettings {
        offset_ms: i32,
    } => {
        /// The offset before the command.
        previous_offset_ms: i32,
    };

    /// Start recording the armed MIDI tracks (`track.arm`); refused when none is armed or
    /// when a take is still waiting to be committed. `target`: `{"at": "scene", "scene":
    /// <id>}` (the launch grid: overdub into the cell's clip, or a new clip in an empty
    /// cell) or `{"at": "arrangement"}` (the default: each note where it is played). From a
    /// stopped transport it plays from the play position, after the metronome's count-in
    /// bars (heard, not recorded), and with a scene target it launches that scene's clip
    /// of each armed track first; on a running transport it records at once. `latency_ms`
    /// is the output latency the host estimates (0 to 1000, default 0). The notes come
    /// back from the engine; `record.stop` ends the take, which the host then writes with
    /// `record.commit_take`, one history entry. Not undoable itself.
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
        /// How much earlier the notes will be placed: `latency_ms` plus the session's
        /// offset (`record.set_settings`), in milliseconds.
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
    /// ticks. Every start is first moved `shift_ms` earlier at the project's tempo. With a
    /// scene target, a cell that holds a clip takes the notes on top of its own (overdub):
    /// a note lands where the clip played at that moment, from `anchor` (the transport tick
    /// of the clip's position 0) or else from the bar of the first note, round its loop;
    /// a clip without a loop grows to the bar after the last note. An empty cell gets a new
    /// clip from the bar of the first note to the bar after the last one (at least one
    /// bar), looping over its length. With the arrangement target, a note inside a
    /// placement goes into its clip where the placement shows it; the other notes make one
    /// new clip per gap between placements, from the bar of its first note to the bar after
    /// its last, cut at the next placement. A note before tick 0 or before its clip is
    /// dropped and counted; a note longer than its clip is cut at the clip's end. Undo:
    /// `record.restore` puts the clips back as they were.
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
    } => {
        clips: Vec<TakeClip>,
        /// Notes left out: before tick 0, before their clip, or outside a clip without a loop.
        dropped: u32,
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
        Command::SetSettings { offset_ms } => {
            if !(-MAX_OFFSET_MS..=MAX_OFFSET_MS).contains(offset_ms) {
                return Err(invalid(
                    "offset_ms",
                    format!("an offset is -{MAX_OFFSET_MS} to {MAX_OFFSET_MS} ms, not {offset_ms}"),
                ));
            }
            let previous = state.session.record.offset_ms;
            state.session.record.offset_ms = *offset_ms;
            let events = if previous == *offset_ms {
                vec![]
            } else {
                vec![ProjectEvent::RecordSettingsChanged {
                    offset_ms: *offset_ms,
                }]
            };
            Ok(live(
                Output::SetSettings {
                    previous_offset_ms: previous,
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
        } => {
            let mut out = commit(state, ctx, target, *shift_ms, tracks)?;
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
            shift_ms: latency + f64::from(state.session.record.offset_ms),
            count_in,
        },
        vec![ProjectEvent::RecordingChanged {
            take: number,
            state: TakeState::Recording,
        }],
    ))
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

/// A note at `pos` in a clip of `length`, cut at its end; `None` when it starts outside.
fn place(n: &TakeNote, pos: i64, length: i64) -> Option<Note> {
    if pos < 0 || pos >= length {
        return None;
    }
    let len = n.length.get().max(1).min(length - pos);
    Some(Note::new(Tick(pos), n.pitch, Tick(len), n.velocity))
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
                    let origin = anchor.map_or(floor_to(first, bar), Tick::get);
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
                            .position(n.start.get() - origin)
                            .and_then(|pos| place(n, pos, length))
                        {
                            Some(note) => out.push(note),
                            None => dropped += 1,
                        }
                    }
                    writes.push(Write::Into {
                        clip,
                        notes: out,
                        length: (length != original).then_some(length),
                    });
                }
                None => {
                    let origin = floor_to(first, bar);
                    let length = ceil_to(last_end - origin, bar).max(bar);
                    let out = notes
                        .iter()
                        .filter_map(|n| place(n, n.start.get() - origin, length))
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
            // Gap (its start and end, `None`: no placement after) → notes outside clips.
            let mut gaps: BTreeMap<(i64, Option<i64>), Vec<&TakeNote>> = BTreeMap::new();
            // A note that starts before tick 0 (played on the first downbeat, moved earlier by the
            // latency shift) starts at 0 when it ends after it; one that ends before is dropped.
            let mut kept = Vec::with_capacity(notes.len());
            for n in notes {
                let (t, end) = (n.start.get(), n.start.get().saturating_add(n.length.get()));
                if t >= 0 {
                    kept.push(*n);
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
                            place(n, pos, geo.length)
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
            for (clip, notes) in into {
                writes.push(Write::Into {
                    clip,
                    notes,
                    length: None,
                });
            }
            for ((before, after), group) in gaps {
                let first = group.iter().map(|n| n.start.get()).min().unwrap_or(0);
                let last = group
                    .iter()
                    .map(|n| n.start.get().saturating_add(n.length.get()))
                    .max()
                    .unwrap_or(first + 1);
                let start = floor_to(first, bar).max(before);
                let mut end = ceil_to(last, bar).max(start + 1);
                if let Some(next) = after {
                    end = end.min(next);
                }
                let length = end - start;
                let out = group
                    .iter()
                    .filter_map(|n| place(n, n.start.get() - start, length))
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

fn commit(
    state: &mut State,
    ctx: &mut Context,
    target: &RecordTarget,
    shift_ms: f64,
    tracks: &[TakeTrack],
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
    let mut changes: Vec<ClipRestore> = Vec::new();
    let mut before: Vec<ClipRestore> = Vec::new();
    let mut clips = Vec::new();
    let mut dropped = 0u32;
    let mut names: Vec<String> = Vec::new();
    for t in tracks {
        let shifted: Vec<TakeNote> = t
            .notes
            .iter()
            .map(|n| TakeNote {
                start: Tick(n.start.get().saturating_sub(shift)),
                ..*n
            })
            .collect();
        let (writes, lost) = plan_track(project, target, &t.track, &shifted, t.anchor);
        dropped += lost;
        let track = midi_track(project, &t.track)?;
        for write in writes {
            match write {
                Write::Into {
                    clip,
                    notes,
                    length,
                } => {
                    let old = restored(track, &clip)?;
                    let mut new = old.clone();
                    let had: BTreeSet<Note> = old.snapshot.notes.iter().copied().collect();
                    let mut all = had.clone();
                    all.extend(notes.iter().copied());
                    let added = all.len() - had.len();
                    if added == 0 && length.is_none() {
                        continue;
                    }
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
                    let name = project.next_clip_name_beside(&names);
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
    let output = Output::CommitTake { clips, dropped };
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
