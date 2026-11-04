//! The state the clip commands run from, and one example of every branch of each of
//! them. `tests/it/contract.rs` adds [`examples`] to the catalog of examples (a command
//! without one fails there); `tests/it/clip.rs` also checks each against `validate`.

#![allow(dead_code)]

use ::project::{
    AudioMedia, CLIP_PLAYER, Clip, ClipContent, ClipId, Device, DeviceId, DeviceRole, DeviceSource,
    Frame, FrameLoop, Hash, IdGenerator, Note, OrderKey, Placement, PlacementId, SampleRate, Scene,
    SceneId, Span, Tick, TickLoop, Track, TrackId, TrackKind,
};
use commands::clip::{self, ClipTarget, NoteChange};
use commands::testing::fixture;
use commands::{Command, State};
use std::collections::BTreeSet;

/// The state and the ids the clip tests use.
pub struct ClipFx {
    pub state: State,
    /// A MIDI track (the fixture's).
    pub track: TrackId,
    /// An audio track with one audio clip.
    pub audio_track: TrackId,
    pub audio_clip: ClipId,
    pub scene_a: SceneId,
    pub scene_b: SceneId,
    /// A free scene: no clip sits in it.
    pub scene_free: SceneId,
    /// 3840 ticks, loop 0..1920, in `scene_a`, three notes, no placement.
    pub clip: ClipId,
    /// 3840 ticks, in `scene_b`, placed at 7680 (length 3840, offset 480), one note.
    pub placed: ClipId,
    pub placement: PlacementId,
}

pub fn n(start: i64, pitch: u8, length: i64, velocity: u8) -> Note {
    Note::new(Tick(start), pitch, Tick(length), velocity)
}

pub fn c(command: clip::Command) -> Command {
    Command::Clip(command)
}

pub fn fx() -> ClipFx {
    let f = fixture();
    let mut state = f.state;
    let mut ids = IdGenerator::new(1_660_000_000_000, 0x00DD_0000_0000_0000_0000);
    let scene = |ids: &mut IdGenerator, order: OrderKey| {
        (
            SceneId::generate(ids),
            Scene {
                name: String::new(),
                order,
            },
        )
    };
    let first = OrderKey::first();
    let second = OrderKey::between(Some(&first), None).unwrap();
    let third = OrderKey::between(Some(&second), None).unwrap();
    let (scene_a, a) = scene(&mut ids, first);
    let (scene_b, b) = scene(&mut ids, second);
    let (scene_free, free) = scene(&mut ids, third);
    state.project.scenes.insert(scene_a.clone(), a);
    state.project.scenes.insert(scene_b.clone(), b);
    state.project.scenes.insert(scene_free.clone(), free);

    let clip = ClipId::generate(&mut ids);
    let placed = ClipId::generate(&mut ids);
    let placement = PlacementId::generate(&mut ids);
    let midi = state.project.tracks.get_mut(&f.track).unwrap();
    midi.clips.insert(
        clip.clone(),
        Clip {
            name: "Riff".into(),
            color: None,
            slot: Some(scene_a.clone()),
            launch_quantize: None,
            content: ClipContent::Midi {
                length: Tick(3840),
                looping: Some(TickLoop {
                    start: Tick(0),
                    end: Tick(1920),
                }),
                notes: BTreeSet::from([
                    n(0, 60, 480, 100),
                    n(960, 64, 960, 90),
                    n(3360, 67, 480, 80),
                ]),
            },
        },
    );
    midi.clips.insert(
        placed.clone(),
        Clip {
            name: "Placed".into(),
            color: None,
            slot: Some(scene_b.clone()),
            launch_quantize: None,
            content: ClipContent::Midi {
                length: Tick(3840),
                looping: None,
                notes: BTreeSet::from([n(480, 72, 480, 100)]),
            },
        },
    );
    midi.placements.insert(
        placement.clone(),
        Placement {
            clip: placed.clone(),
            start: Tick(7680),
            span: Span::Ticks {
                length: Tick(3840),
                offset: Tick(480),
            },
        },
    );

    let audio_track = TrackId::generate(&mut ids);
    let audio_clip = ClipId::generate(&mut ids);
    let mut audio = Track::new(TrackKind::Audio, "Drums", OrderKey::first());
    audio.devices.insert(
        DeviceId::generate(&mut ids),
        Device {
            order: OrderKey::first(),
            role: DeviceRole::Instrument,
            enabled: true,
            source: DeviceSource::Builtin {
                id: CLIP_PLAYER.into(),
                version: 1,
            },
            params: Default::default(),
            state: None,
            preset: None,
        },
    );
    audio.clips.insert(
        audio_clip.clone(),
        Clip {
            name: "Loop".into(),
            color: None,
            slot: None,
            launch_quantize: None,
            content: ClipContent::Audio {
                media: AudioMedia {
                    hash: Hash([7; 32]),
                    sample_rate: SampleRate::HZ_44100,
                    channels: 2,
                    frames: Frame::new(88_200),
                    name: "loop.wav".into(),
                },
                start: Frame::new(0),
                frames: Frame::new(44_100),
                gain_db: 0.0,
                looping: None,
            },
        },
    );
    state.project.tracks.insert(audio_track.clone(), audio);
    state.project.validate().expect("the clip fixture is valid");
    ClipFx {
        state,
        track: f.track,
        audio_track,
        audio_clip,
        scene_a,
        scene_b,
        scene_free,
        clip,
        placed,
        placement,
    }
}

/// The state after `command`, with the inverse it returned.
pub fn after(mut state: State, command: &Command) -> (State, Command) {
    let outcome = commands::run(command, &mut state, &mut commands::testing::context()).unwrap();
    let commands::Undo::Inverse(inverse) = outcome.undo else {
        panic!("no inverse")
    };
    (state, *inverse)
}

/// The fixture's state with a second, empty MIDI track (a copy of the first one without its
/// clips), for the commands that cross tracks.
pub fn with_other_track(f: &ClipFx) -> (State, TrackId) {
    let mut state = f.state.clone();
    let mut other = state.project.tracks.get(&f.track).unwrap().clone();
    other.clips.clear();
    other.placements.clear();
    other.name = "Other".into();
    let mut ids = IdGenerator::new(1_670_000_000_000, 0x00DD_1111_0000_0000_0000);
    let id = TrackId::generate(&mut ids);
    let device = other.devices.keys().next().unwrap().clone();
    let moved = other.devices.remove(&device).unwrap();
    other.devices.insert(DeviceId::generate(&mut ids), moved);
    state.project.tracks.insert(id.clone(), other);
    state
        .project
        .validate()
        .expect("the two-track fixture is valid");
    (state, id)
}

/// One example of every branch of every clip command.
pub fn examples() -> Vec<(Command, State)> {
    let f = fx();
    let on = |command: clip::Command| (c(command), f.state.clone());
    let mut all = vec![
        on(clip::Command::Create {
            track: f.track.clone(),
            at: ClipTarget::Slot {
                scene: f.scene_free.clone(),
            },
            length: None,
            name: Some("  New  ".into()),
        }),
        on(clip::Command::Create {
            track: f.track.clone(),
            at: ClipTarget::Arrangement { start: Tick(0) },
            length: Some(Tick(1920)),
            name: None,
        }),
        on(clip::Command::Delete {
            clip: f.clip.clone(),
        }),
        on(clip::Command::Delete {
            clip: f.placed.clone(),
        }),
        on(clip::Command::Duplicate {
            clip: f.clip.clone(),
            slot: Some(f.scene_free.clone()),
            track: None,
        }),
        on(clip::Command::Duplicate {
            clip: f.placed.clone(),
            slot: None,
            track: None,
        }),
        on(clip::Command::Rename {
            clip: f.audio_clip.clone(),
            name: " Renamed ".into(),
        }),
        // Longer: nothing to cut.
        on(clip::Command::SetLength {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(7680),
            trim: false,
        }),
        // Shorter, past no note: the plain inverse.
        on(clip::Command::SetLength {
            placements: None,
            clip: f.placed.clone(),
            length: Tick(1920),
            trim: true,
        }),
        // Shorter, cutting notes and the loop: `set_content` is the inverse.
        on(clip::Command::SetLength {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(1200),
            trim: true,
        }),
        on(clip::Command::SetLoop {
            clip: f.clip.clone(),
            looping: Some(TickLoop {
                start: Tick(480),
                end: Tick(3840),
            }),
        }),
        on(clip::Command::SetLoop {
            clip: f.clip.clone(),
            looping: None,
        }),
        on(clip::Command::SetContent {
            placements: None,
            clip: f.clip.clone(),
            length: Tick(960),
            looping: None,
            notes: vec![n(0, 48, 960, 64)],
        }),
        on(clip::Command::NotesAdd {
            clip: f.clip.clone(),
            notes: vec![
                n(0, 60, 480, 100),
                n(1920, 62, 240, 70),
                Note {
                    cents: -20,
                    chance: 50,
                    repeat: 4,
                    ..n(1920, 62, 240, 70)
                },
            ],
        }),
        on(clip::Command::NotesRemove {
            clip: f.clip.clone(),
            notes: vec![n(0, 60, 480, 100), n(1, 1, 1, 1)],
        }),
        on(clip::Command::NotesUpdate {
            clip: f.clip.clone(),
            changes: vec![NoteChange {
                from: n(0, 60, 480, 100),
                to: n(120, 61, 240, 101),
            }],
        }),
        // A chain: A -> B and B -> C at once.
        on(clip::Command::NotesUpdate {
            clip: f.clip.clone(),
            changes: vec![
                NoteChange {
                    from: n(0, 60, 480, 100),
                    to: n(960, 64, 960, 90),
                },
                NoteChange {
                    from: n(960, 64, 960, 90),
                    to: n(2000, 64, 960, 90),
                },
            ],
        }),
        // PN-2: every note to a coarse grid (one halfway to the later line, one pinned by the clip end).
        on(clip::Command::NotesQuantize {
            clip: f.clip.clone(),
            notes: None,
            grid: Tick(1920),
            strength: 100,
        }),
        on(clip::Command::NotesQuantize {
            clip: f.clip.clone(),
            notes: Some(vec![n(960, 64, 960, 90)]),
            grid: Tick(1920),
            strength: 50,
        }),
        on(clip::Command::NotesTranspose {
            clip: f.clip.clone(),
            notes: vec![n(0, 60, 480, 100), n(960, 64, 960, 90)],
            semitones: -12,
        }),
        on(clip::Command::NotesDuplicate {
            clip: f.clip.clone(),
            notes: vec![n(0, 60, 480, 100)],
            offset: Some(Tick(480)),
        }),
        on(clip::Command::NotesDuplicate {
            clip: f.clip.clone(),
            notes: vec![n(960, 64, 960, 90)],
            offset: Some(Tick(-480)),
        }),
    ];
    // Moves: within the track, out of the grid (placed clip only), and across tracks.
    all.push(on(clip::Command::Move {
        clip: f.clip.clone(),
        track: None,
        slot: Some(f.scene_free.clone()),
    }));
    all.push(on(clip::Command::Move {
        clip: f.placed.clone(),
        track: None,
        slot: None,
    }));
    let (two, other) = with_other_track(&f);
    all.push((
        c(clip::Command::Move {
            clip: f.clip.clone(),
            track: Some(other.clone()),
            slot: Some(f.scene_a.clone()),
        }),
        two.clone(),
    ));
    all.push((
        c(clip::Command::Duplicate {
            clip: f.clip.clone(),
            slot: None,
            track: Some(other),
        }),
        two,
    ));
    // The inverse of a delete, from the state that delete leaves: with and without
    // placements.
    for clip in [&f.clip, &f.placed] {
        let (state, restore) = after(
            f.state.clone(),
            &c(clip::Command::Delete { clip: clip.clone() }),
        );
        all.push((restore, state));
    }
    all.extend(audio_examples(&f));
    all
}

/// The live clip commands (no inverse, so apart from [`examples`], whose every example is
/// undone and redone by `tests/it/clip.rs`): asking the page for audio files.
pub fn live_examples() -> Vec<(Command, State)> {
    let f = fx();
    vec![
        (
            c(clip::Command::ChooseAudio {
                track: f.audio_track.clone(),
            }),
            f.state.clone(),
        ),
        (c(clip::Command::CancelAudioImport {}), f.state),
    ]
}

/// The fixture's state with a placement (in frames) of its audio clip.
pub fn with_audio_placement(f: &ClipFx) -> (State, PlacementId) {
    let mut state = f.state.clone();
    let mut ids = IdGenerator::new(1_680_000_000_000, 0x00DD_2222_0000_0000_0000);
    let id = PlacementId::generate(&mut ids);
    state
        .project
        .tracks
        .get_mut(&f.audio_track)
        .unwrap()
        .placements
        .insert(
            id.clone(),
            Placement {
                clip: f.audio_clip.clone(),
                start: Tick(3840),
                span: Span::Frames {
                    length: Frame::new(22_050),
                    offset: Frame::new(11_025),
                },
            },
        );
    state
        .project
        .validate()
        .expect("the audio placement is valid");
    (state, id)
}

/// The audio clip commands of AU-1: import (a staged file, a file the project already
/// uses), delete and restore an audio clip (with and without a placement), choose files.
fn audio_examples(f: &ClipFx) -> Vec<(Command, State)> {
    let on = |command: clip::Command| (c(command), f.state.clone());
    let mut all = vec![
        on(clip::Command::ImportAudio {
            track: f.audio_track.clone(),
            media: commands::testing::imported_media().hash,
            name: None,
            at: None,
        }),
        on(clip::Command::ImportAudio {
            track: f.audio_track.clone(),
            media: Hash([7; 32]),
            name: Some("  Second loop  ".into()),
            at: None,
        }),
        on(clip::Command::Delete {
            clip: f.audio_clip.clone(),
        }),
    ];
    let (placed, _) = with_audio_placement(f);
    all.push((
        c(clip::Command::Delete {
            clip: f.audio_clip.clone(),
        }),
        placed.clone(),
    ));
    for from in [f.state.clone(), placed] {
        let (state, restore) = after(
            from,
            &c(clip::Command::Delete {
                clip: f.audio_clip.clone(),
            }),
        );
        all.push((restore, state));
    }
    all.extend(audio_edit_examples(f));
    all
}

/// The audio clip commands of AU-5: an import into a slot and into the arrangement, the
/// range, gain and loop of a clip, and a move of an audio clip in the grid.
fn audio_edit_examples(f: &ClipFx) -> Vec<(Command, State)> {
    let on = |command: clip::Command| (c(command), f.state.clone());
    let clip = || f.audio_clip.clone();
    let (placed, _) = with_audio_placement(f);
    let (looped, _) = after(
        f.state.clone(),
        &c(clip::Command::AudioSetLoop {
            clip: clip(),
            looping: Some(FrameLoop {
                start: Frame::new(0),
                end: Frame::new(22_050),
            }),
        }),
    );
    vec![
        on(clip::Command::ImportAudio {
            track: f.audio_track.clone(),
            media: commands::testing::imported_media().hash,
            name: None,
            at: Some(ClipTarget::Slot {
                scene: f.scene_free.clone(),
            }),
        }),
        on(clip::Command::ImportAudio {
            track: f.audio_track.clone(),
            media: commands::testing::imported_media().hash,
            name: None,
            at: Some(ClipTarget::Arrangement { start: Tick(1920) }),
        }),
        on(clip::Command::AudioSetRange {
            clip: clip(),
            start: Frame::new(22_050),
            frames: Frame::new(66_150),
            placements: None,
        }),
        (
            c(clip::Command::AudioSetRange {
                clip: clip(),
                start: Frame::new(0),
                frames: Frame::new(30_000),
                placements: None,
            }),
            placed,
        ),
        on(clip::Command::AudioSetGain {
            clip: clip(),
            gain_db: Some(-6.5),
        }),
        on(clip::Command::AudioSetGain {
            clip: clip(),
            gain_db: None,
        }),
        on(clip::Command::AudioSetLoop {
            clip: clip(),
            looping: Some(FrameLoop {
                start: Frame::new(100),
                end: Frame::new(44_100),
            }),
        }),
        (
            c(clip::Command::AudioSetLoop {
                clip: clip(),
                looping: None,
            }),
            looped,
        ),
        on(clip::Command::Move {
            clip: clip(),
            track: None,
            slot: Some(f.scene_free.clone()),
        }),
    ]
}
