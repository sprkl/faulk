//! AU-3: audio clips reach the engine. Each media an audio clip on the engine uses is one
//! audio source, declared before its first clip and released after its last; an audio clip
//! is a `ClipAudioSet` (range, gain, loop); its placements travel in frames with the MIDI
//! ones. After every command (and its undo), what the engine was sent equals what a snapshot
//! of the resulting project sends, compared by content (the slots may differ).

use std::collections::{BTreeMap, BTreeSet};

use crate::clip_examples::{ClipFx, c, fx, with_audio_placement};
use ::project::{Frame, FrameLoop, Hash, Tick};
use commands::clip::{self, ClipTarget};
use commands::engine_msgs::{Synced, snapshot};
use commands::testing::{context, imported_media};
use commands::{Command, ControlMsg, history};
use engine::msg::{Span, TrackSlot};

/// A `ClipAudioSet` by content: source slot, start, length, gain bits, loop.
type AudioSet = (u32, u64, u64, u32, Option<(u64, u64)>);

/// What the engine holds of the audio side, by content.
#[derive(Clone, Debug, Default, PartialEq)]
struct AudioModel {
    /// Source slot → its media and metadata.
    sources: BTreeMap<u32, ([u8; 32], u32, u8, u64)>,
    /// Clip slot → (source slot, start, length, gain bits, loop).
    clips: BTreeMap<u32, AudioSet>,
    /// Track slot → its frame placements as (start, clip slot, offset, length).
    placements: BTreeMap<u32, Vec<(i64, u32, u64, u64)>>,
}

impl AudioModel {
    fn apply(&mut self, msgs: &[ControlMsg]) {
        for m in msgs {
            match m {
                ControlMsg::Reset { .. } => *self = AudioModel::default(),
                ControlMsg::AudioSourceDeclare {
                    source,
                    sample_rate,
                    channels,
                    frames,
                    media,
                } => {
                    let old = self
                        .sources
                        .insert(source.0, (*media, *sample_rate, *channels, frames.0));
                    assert!(old.is_none(), "source {} declared twice", source.0);
                }
                ControlMsg::AudioSourceRelease { source } => {
                    assert!(
                        self.sources.remove(&source.0).is_some(),
                        "released an unknown source"
                    );
                    assert!(
                        self.clips.values().all(|c| c.0 != source.0),
                        "source {} released while a clip uses it",
                        source.0
                    );
                }
                ControlMsg::ClipAudioSet {
                    clip,
                    source,
                    start,
                    length,
                    gain,
                    looping,
                } => {
                    assert!(
                        self.sources.contains_key(&source.0),
                        "clip {} names undeclared source {}",
                        clip.0,
                        source.0
                    );
                    self.clips.insert(
                        clip.0,
                        (
                            source.0,
                            start.0,
                            length.0,
                            gain.to_bits(),
                            looping.map(|(s, e)| (s.0, e.0)),
                        ),
                    );
                }
                ControlMsg::ClipRemove { clip } => {
                    self.clips.remove(&clip.0);
                }
                ControlMsg::ClipNotesUpload { clip, .. } => {
                    self.clips.remove(&clip.0);
                }
                ControlMsg::TrackRemove { track } => {
                    self.placements.remove(&track.0);
                }
                ControlMsg::TrackPlacementsUpload {
                    track,
                    first,
                    placements,
                    ..
                } => {
                    let row = self.placements.entry(track.0).or_default();
                    if *first == 0 {
                        row.clear();
                    }
                    row.extend(placements.iter().filter_map(|p| match p.span {
                        Span::Frames { offset, length } => {
                            Some((p.start.0, p.clip.0, offset.0, length.0))
                        }
                        Span::Ticks { .. } => None,
                    }));
                }
                _ => {}
            }
        }
    }

    /// The content with slots replaced by what they name: clips by their source's media.
    fn canonical(&self) -> (BTreeSet<[u8; 32]>, Vec<String>, Vec<String>) {
        let media = |s: u32| self.sources[&s].0;
        let sources = self.sources.values().map(|s| s.0).collect();
        let mut clips: Vec<String> = self
            .clips
            .values()
            .map(|c| format!("{:?}", (media(c.0), c.1, c.2, c.3, c.4)))
            .collect();
        clips.sort();
        let mut placed: Vec<String> = self
            .placements
            .values()
            .flatten()
            .map(|p| {
                let c = self.clips[&p.1];
                format!("{:?}", (p.0, media(c.0), c.1, c.2, p.2, p.3))
            })
            .collect();
        placed.sort();
        (sources, clips, placed)
    }
}

struct Rig {
    s: Synced,
    model: AudioModel,
}

impl Rig {
    fn new(state: commands::State) -> Self {
        let mut s = Synced::new(state, context(), 1);
        let mut model = AudioModel::default();
        let (msgs, _) = snapshot(&s.state.project, &s.state.session);
        model.apply(&msgs);
        s.setup().unwrap();
        Rig { s, model }
    }

    fn run(&mut self, command: &Command) {
        let (outcome, _) = self
            .s
            .run(command)
            .unwrap_or_else(|e| panic!("{command:?}: {e:?}"));
        self.model.apply(&outcome.control);
        self.check(&format!("{command:?}"));
    }

    fn check(&self, after: &str) {
        let mut fresh = AudioModel::default();
        let (msgs, _) = snapshot(&self.s.state.project, &self.s.state.session);
        fresh.apply(&msgs);
        assert_eq!(
            self.model.canonical(),
            fresh.canonical(),
            "after {after}: the engine differs from a snapshot"
        );
    }
}

fn undo() -> Command {
    Command::History(history::Command::Undo {})
}

fn import(f: &ClipFx, at: Option<ClipTarget>) -> Command {
    c(clip::Command::ImportAudio {
        track: f.audio_track.clone(),
        media: imported_media().hash,
        name: None,
        at,
    })
}

#[test]
fn the_snapshot_declares_each_media_once_before_its_clips_and_sends_frame_placements() {
    let f = fx();
    let (state, _) = with_audio_placement(&f);
    let (msgs, slots) = snapshot(&state.project, &state.session);
    let declare = msgs
        .iter()
        .position(|m| matches!(m, ControlMsg::AudioSourceDeclare { .. }))
        .expect("the audio clip's media is declared");
    let set = msgs
        .iter()
        .position(|m| matches!(m, ControlMsg::ClipAudioSet { .. }))
        .expect("the audio clip is sent");
    assert!(declare < set, "the source before its clip");
    let ControlMsg::AudioSourceDeclare { media, .. } = &msgs[declare] else {
        unreachable!()
    };
    let clip = &state.project.tracks[&f.audio_track].clips[&f.audio_clip];
    let ::project::ClipContent::Audio { media: m, .. } = &clip.content else {
        panic!("the fixture's audio clip")
    };
    assert_eq!(*media, m.hash.0, "the declare carries the blob's hash");
    assert_eq!(slots.sources.len(), 1);
    let mut model = AudioModel::default();
    model.apply(&msgs);
    assert_eq!(model.placements.values().flatten().count(), 1);
    let (_, _, placed) = model.canonical();
    assert!(
        placed[0].contains("22050"),
        "{placed:?}: frames span sent as is"
    );
    let audio_slot = TrackSlot(slots.tracks.get(&f.audio_track).unwrap());
    assert!(model.placements.contains_key(&audio_slot.0));
}

#[test]
fn every_audio_edit_and_its_undo_leave_the_engine_as_a_snapshot_would() {
    let f = fx();
    let (state, placement) = with_audio_placement(&f);
    let mut r = Rig::new(state);
    let steps = [
        c(clip::Command::AudioSetGain {
            clip: f.audio_clip.clone(),
            gain_db: Some(-6.0),
        }),
        // The placement goes (the clip and its source stay).
        Command::Arrangement(commands::arrangement::Command::Delete {
            placements: vec![placement],
        }),
        // The pool clip comes back to the arrangement (#666).
        Command::Arrangement(commands::arrangement::Command::Place {
            clip: f.audio_clip.clone(),
            start: Tick(3840),
            length: None,
            offset: None,
            if_occupied: Default::default(),
        }),
        c(clip::Command::AudioSetGain {
            clip: f.audio_clip.clone(),
            gain_db: None,
        }),
        c(clip::Command::AudioSetLoop {
            clip: f.audio_clip.clone(),
            looping: Some(FrameLoop {
                start: Frame::new(100),
                end: Frame::new(2_000),
            }),
        }),
        c(clip::Command::AudioSetRange {
            clip: f.audio_clip.clone(),
            start: Frame::new(1_000),
            frames: Frame::new(30_000),
            placements: None,
        }),
        import(&f, None),
        import(
            &f,
            Some(ClipTarget::Slot {
                scene: f.scene_free.clone(),
            }),
        ),
        c(clip::Command::Delete {
            clip: f.audio_clip.clone(),
        }),
    ];
    for step in &steps {
        r.run(step);
    }
    // Back through every step, one undo at a time.
    for _ in &steps {
        r.run(&undo());
    }
}

#[test]
fn a_source_is_released_with_the_last_clip_of_its_media_only() {
    let f = fx();
    let mut r = Rig::new(f.state.clone());
    let declared = |r: &Rig| r.model.sources.len();
    assert_eq!(declared(&r), 1, "the fixture's clip");
    // Two clips of the imported media: one source.
    r.run(&import(&f, None));
    r.run(&import(&f, None));
    let imported = Hash(imported_media().hash.0);
    let count = |r: &Rig| {
        r.model
            .sources
            .values()
            .filter(|s| s.0 == imported.0)
            .count()
    };
    assert_eq!(count(&r), 1, "one source per media");
    r.run(&undo());
    assert_eq!(count(&r), 1, "one clip of the media is still there");
    r.run(&undo());
    assert_eq!(count(&r), 0, "released with its last clip");
    // Deleting the audio track releases its clips' sources.
    r.run(&Command::Track(commands::track::Command::Delete {
        track: f.audio_track.clone(),
    }));
    assert_eq!(declared(&r), 0);
    r.run(&undo());
    assert_eq!(declared(&r), 1);
}

#[test]
fn duplicating_a_track_and_deleting_a_scene_keep_the_engine_in_step() {
    let f = fx();
    let (state, _) = with_audio_placement(&f);
    let mut r = Rig::new(state);
    // An audio clip in the free scene, so that deleting the scene takes it.
    r.run(&import(
        &f,
        Some(ClipTarget::Slot {
            scene: f.scene_free.clone(),
        }),
    ));
    let steps = [
        Command::Track(commands::track::Command::Duplicate {
            track: f.audio_track.clone(),
            name: None,
        }),
        Command::Scene(commands::scene::Command::Delete {
            scene: f.scene_free.clone(),
        }),
    ];
    for step in &steps {
        r.run(step);
    }
    for _ in &steps {
        r.run(&undo());
    }
}

#[test]
fn deleting_an_audio_placement_empties_the_track_but_keeps_the_source() {
    let f = fx();
    let (state, placement) = with_audio_placement(&f);
    let mut r = Rig::new(state);
    assert_eq!(r.model.placements.values().flatten().count(), 1);
    r.run(&Command::Arrangement(
        commands::arrangement::Command::Delete {
            placements: vec![placement],
        },
    ));
    assert_eq!(
        r.model.placements.values().flatten().count(),
        0,
        "no placement left for the engine to play"
    );
    assert_eq!(r.model.sources.len(), 1, "the clip still holds its source");
    assert_eq!(r.model.clips.len(), 1, "and the clip stays in the engine");
    r.run(&undo());
    assert_eq!(r.model.placements.values().flatten().count(), 1);
}

#[test]
fn the_arrangement_edits_of_audio_placements_keep_the_engine_in_step_both_ways() {
    use commands::arrangement::{Command as A, Edge};
    let f = fx();
    let (state, placement) = with_audio_placement(&f);
    let mut r = Rig::new(state);
    let sources = r.model.sources.len();
    let ar = Command::Arrangement;
    let steps = [
        ar(A::Move {
            placements: vec![placement.clone()],
            by: Tick(960),
            if_occupied: Default::default(),
        }),
        ar(A::Resize {
            placement: placement.clone(),
            edge: Edge::End,
            to: Tick(4800 + 480),
            if_occupied: Default::default(),
            clip_length: None,
        }),
        ar(A::Resize {
            placement: placement.clone(),
            edge: Edge::Start,
            to: Tick(4800 - 240),
            if_occupied: Default::default(),
            clip_length: None,
        }),
        ar(A::Split {
            placement: placement.clone(),
            at: Tick(4800),
        }),
        // An independent copy: a new clip of the same media, so no new source.
        ar(A::Duplicate {
            placements: vec![placement.clone()],
            linked: false,
            by: Some(Tick(7680)),
            if_occupied: commands::arrangement::IfOccupied::Replace,
        }),
        ar(A::Duplicate {
            placements: vec![placement.clone()],
            linked: true,
            by: Some(Tick(15_360)),
            if_occupied: commands::arrangement::IfOccupied::Replace,
        }),
        ar(A::Repeat {
            placement: placement.clone(),
            until: Tick(30_000),
            if_occupied: Default::default(),
        }),
        ar(A::InsertTime {
            at: Tick(4700),
            length: Tick(3840),
        }),
        ar(A::DeleteTime {
            from: Tick(100),
            to: Tick(2000),
        }),
        c(clip::Command::Duplicate {
            clip: f.audio_clip.clone(),
            slot: Some(f.scene_free.clone()),
            track: None,
        }),
    ];
    for step in &steps {
        r.run(step);
        assert_eq!(
            r.model.sources.len(),
            sources,
            "{step:?}: one source per media"
        );
    }
    for _ in &steps {
        r.run(&undo());
    }
    for _ in &steps {
        r.run(&Command::History(history::Command::Redo {}));
    }
}

#[test]
fn a_faster_tempo_that_shortens_an_audio_placement_reaches_the_engine_and_undoes() {
    use crate::audio_clip::with_whole;
    let f = fx();
    // Two whole placements back to back at 120 BPM: a faster tempo shortens the first (#709).
    let (state, _) = with_whole(&f, f.state.clone(), 0, 41);
    let (state, _) = with_whole(&f, state, 1_920, 42);
    let mut r = Rig::new(state);
    let faster = Command::Transport(commands::transport::Command::SetTempo {
        bpm: 240.0,
        placements: None,
    });
    r.run(&faster);
    r.run(&undo());
}
