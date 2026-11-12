//! The state the arrangement commands run from, and one example of every branch of each
//! of them. `tests/it/contract.rs` adds [`examples`] to the catalog of examples (a command
//! without one fails there); `tests/it/arrangement.rs` checks the rule on top of them.

#![allow(dead_code)]

use super::clip_examples;
use ::project::{ClipId, Placement, PlacementId, Span, Tick, TrackId};
use commands::arrangement::{self, Edge};
use commands::testing::context;
use commands::{Command, State};

/// The clip fixture with a small arrangement on its MIDI track.
pub struct ArFx {
    pub state: State,
    pub track: TrackId,
    /// 3840 ticks, loop 0..1920, three notes; in the arrangement only (no grid slot).
    pub looped: ClipId,
    /// 3840 ticks, no loop, one note.
    pub straight: ClipId,
    /// `looped`, start 0, length 3840, offset 0.
    pub a: PlacementId,
    /// `looped`, start 3840, length 1920, offset 960.
    pub b: PlacementId,
    /// `straight`, start 7680, length 3840, offset 480.
    pub c: PlacementId,
    pub audio_track: TrackId,
    pub audio_clip: ClipId,
    pub scene_a: ::project::SceneId,
    pub scene_b: ::project::SceneId,
    pub scene_free: ::project::SceneId,
}

impl ArFx {
    /// The state where both clips of the track sit in the launch grid (`looped` in
    /// `scene_a`, `straight` in `scene_b`), so that they can be sent to the arrangement.
    pub fn grid_state(&self) -> State {
        let mut state = self.state.clone();
        let track = track_mut(&mut state, &self.track);
        track.clips.get_mut(&self.looped).unwrap().slot = Some(self.scene_a.clone());
        track.clips.get_mut(&self.straight).unwrap().slot = Some(self.scene_b.clone());
        state.project.validate().expect("the grid fixture is valid");
        state
    }
}

pub fn fx() -> ArFx {
    let f = clip_examples::fx();
    let mut state = f.state.clone();
    let mut ids = ::project::IdGenerator::new(1_670_000_000_000, 0x00EE_0000_0000_0000_0000);
    let a = PlacementId::generate(&mut ids);
    let b = PlacementId::generate(&mut ids);
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    let place = |clip: &ClipId, start: i64, length: i64, offset: i64| Placement {
        clip: clip.clone(),
        start: Tick(start),
        span: Span::Ticks {
            length: Tick(length),
            offset: Tick(offset),
        },
    };
    track
        .placements
        .insert(a.clone(), place(&f.clip, 0, 3840, 0));
    track
        .placements
        .insert(b.clone(), place(&f.clip, 3840, 1920, 960));
    // Both clips live in the arrangement only; the tests that need a grid clip give one a
    // slot.
    for clip in [&f.clip, &f.placed] {
        track_mut(&mut state, &f.track)
            .clips
            .get_mut(clip)
            .unwrap()
            .slot = None;
    }
    state
        .project
        .validate()
        .expect("the arrangement fixture is valid");
    ArFx {
        state,
        track: f.track,
        looped: f.clip,
        straight: f.placed,
        a,
        b,
        c: f.placement,
        audio_track: f.audio_track,
        audio_clip: f.audio_clip,
        scene_a: f.scene_a,
        scene_b: f.scene_b,
        scene_free: f.scene_free,
    }
}

fn track_mut<'a>(state: &'a mut State, id: &TrackId) -> &'a mut ::project::Track {
    state.project.tracks.get_mut(id).unwrap()
}

pub fn a(command: arrangement::Command) -> Command {
    Command::Arrangement(command)
}

/// The state after `command`, with the inverse it returned.
pub fn after(mut state: State, command: &Command) -> (State, Command) {
    let outcome = commands::run(command, &mut state, &mut context()).unwrap();
    let commands::Undo::Inverse(inverse) = outcome.undo else {
        panic!("no inverse")
    };
    (state, *inverse)
}

/// The fixture with a second MIDI track (a duplicate of the first, with its own clips and
/// placements, ids of its own) and the identifier of that track.
pub fn with_second_track(f: &ArFx) -> (State, TrackId) {
    let mut state = f.state.clone();
    let out = commands::run(
        &Command::Track(commands::track::Command::Duplicate {
            track: f.track.clone(),
            name: None,
        }),
        &mut state,
        // Identifiers apart from every other context's, so the commands under test never
        // generate one that the second track already holds.
        &mut commands::Context::new(
            commands::Author::User,
            ::project::IdGenerator::new(1_680_000_000_000, 0x00DD_0000_0000_0000_0000),
        ),
    )
    .unwrap();
    let commands::Output::Track(commands::track::Output::Duplicate { track }) = out.output else {
        panic!("not a track duplicate")
    };
    (state, track)
}

/// One example of every branch of every arrangement command.
pub fn examples() -> Vec<(Command, State)> {
    let f = fx();
    let on = |command: arrangement::Command| (a(command), f.state.clone());
    let mut all = vec![
        // Free ground: the plain case.
        on(arrangement::Command::Place {
            clip: f.looped.clone(),
            start: Tick(11520),
            length: None,
            offset: None,
            if_occupied: Default::default(),
        }),
        // An audio clip of the audio track, whole.
        on(arrangement::Command::Place {
            clip: f.audio_clip.clone(),
            start: Tick(3840),
            length: None,
            offset: None,
            if_occupied: Default::default(),
        }),
        // Over A (cut on its right) and B (trimmed at the front).
        on(arrangement::Command::Place {
            clip: f.straight.clone(),
            start: Tick(3000),
            length: Some(Tick(1000)),
            offset: Some(Tick(960)),
            if_occupied: Default::default(),
        }),
        on(arrangement::Command::Move {
            placements: vec![f.a.clone(), f.b.clone()],
            by: Tick(480),
            if_occupied: Default::default(),
        }),
        // C onto B: B is covered and removed.
        on(arrangement::Command::Move {
            placements: vec![f.c.clone()],
            by: Tick(-3840),
            if_occupied: Default::default(),
        }),
        on(arrangement::Command::Resize {
            placement: f.a.clone(),
            edge: Edge::End,
            to: Tick(5000),
        }),
        on(arrangement::Command::Resize {
            placement: f.b.clone(),
            edge: Edge::Start,
            to: Tick(4800),
        }),
        on(arrangement::Command::Split {
            placement: f.c.clone(),
            at: Tick(9000),
        }),
        on(arrangement::Command::Delete {
            placements: vec![f.a.clone(), f.c.clone()],
        }),
        // Independent copies, after the selection.
        on(arrangement::Command::Duplicate {
            placements: vec![f.a.clone(), f.b.clone()],
            linked: false,
            by: None,
            if_occupied: arrangement::IfOccupied::Replace,
        }),
        // Linked, onto the next ones: they are carved.
        on(arrangement::Command::Duplicate {
            placements: vec![f.a.clone()],
            linked: true,
            by: Some(Tick(7000)),
            if_occupied: arrangement::IfOccupied::Replace,
        }),
        // Repeated end to end: whole copies over free ground, the last one shortened, and over
        // what is there (C is carved, then covered).
        on(arrangement::Command::Repeat {
            placement: f.a.clone(),
            until: Tick(3840 * 2),
            if_occupied: Default::default(),
        }),
        on(arrangement::Command::Repeat {
            placement: f.b.clone(),
            until: Tick(3840 + 1920 * 3 + 700),
            if_occupied: Default::default(),
        }),
        on(arrangement::Command::Repeat {
            placement: f.a.clone(),
            until: Tick(3840 * 6),
            if_occupied: Default::default(),
        }),
    ];
    // To another MIDI track: one placement of a clip another one still uses (the clip
    // stays), both placements of it (the clip goes), over what the target holds, and with a
    // shift in time.
    let (two, second) = with_second_track(&f);
    let to_second = |moves: Vec<(&PlacementId, &TrackId)>, by: Option<i64>| {
        (
            a(arrangement::Command::MoveToTrack {
                moves: moves
                    .into_iter()
                    .map(|(p, t)| arrangement::TrackMove {
                        placement: p.clone(),
                        track: t.clone(),
                    })
                    .collect(),
                by: by.map(Tick),
                if_occupied: arrangement::IfOccupied::Replace,
            }),
            two.clone(),
        )
    };
    all.push(to_second(vec![(&f.a, &second)], None));
    all.push(to_second(vec![(&f.a, &second), (&f.b, &second)], Some(480)));
    all.push(to_second(
        vec![(&f.c, &second), (&f.b, &f.track)],
        Some(-480),
    ));
    // #805: `refuse` and `skip` on free ground, and a copy that refuses to replace.
    for how in [
        arrangement::IfOccupied::Refuse,
        arrangement::IfOccupied::Skip,
    ] {
        let (mut command, state) = to_second(vec![(&f.a, &second)], Some(100_000));
        if let Command::Arrangement(arrangement::Command::MoveToTrack { if_occupied, .. }) =
            &mut command
        {
            *if_occupied = how;
        }
        all.push((command, state));
    }
    all.push((
        a(arrangement::Command::Duplicate {
            placements: vec![f.c.clone()],
            linked: false,
            by: Some(Tick(40_000)),
            if_occupied: arrangement::IfOccupied::Refuse,
        }),
        f.state.clone(),
    ));
    // The same promise on the other commands (#805): free ground, so a plain edit.
    for command in [
        arrangement::Command::Place {
            clip: f.looped.clone(),
            start: Tick(11520),
            length: None,
            offset: None,
            if_occupied: arrangement::IfOccupied::Refuse,
        },
        arrangement::Command::Move {
            placements: vec![f.c.clone()],
            by: Tick(40_000),
            if_occupied: arrangement::IfOccupied::Refuse,
        },
        arrangement::Command::Repeat {
            placement: f.c.clone(),
            until: Tick(3840 * 5),
            if_occupied: arrangement::IfOccupied::Refuse,
        },
    ] {
        all.push((a(command), f.state.clone()));
    }
    {
        let (state, restore) = after(
            two.clone(),
            &a(arrangement::Command::MoveToTrack {
                moves: vec![
                    arrangement::TrackMove {
                        placement: f.a.clone(),
                        track: second.clone(),
                    },
                    arrangement::TrackMove {
                        placement: f.b.clone(),
                        track: second.clone(),
                    },
                ],
                by: None,
                if_occupied: arrangement::IfOccupied::Replace,
            }),
        );
        all.push((restore, state));
    }
    // From the launch grid: an independent copy, a linked placement, one over what is
    // there, and a whole scene both ways.
    let grid = f.grid_state();
    let from_grid = |command: arrangement::Command| (a(command), grid.clone());
    all.push(from_grid(arrangement::Command::PlaceFromSlot {
        clip: f.looped.clone(),
        start: Tick(11520),
        linked: false,
        if_occupied: arrangement::IfOccupied::Refuse,
    }));
    for (clip, start, linked) in [
        (&f.looped, 11520, false),
        (&f.looped, 11520, true),
        (&f.looped, 1000, false),
    ] {
        all.push(from_grid(arrangement::Command::PlaceFromSlot {
            clip: clip.clone(),
            start: Tick(start),
            linked,
            if_occupied: Default::default(),
        }));
    }
    for (scene, linked) in [(&f.scene_a, false), (&f.scene_b, true)] {
        all.push(from_grid(arrangement::Command::PlaceScene {
            scene: scene.clone(),
            start: Tick(7000),
            linked,
        }));
    }
    // Time inserted and deleted across the tracks, and the restore of a time edit that
    // also moved an automation lane and the loop.
    all.push(on(arrangement::Command::InsertTime {
        at: Tick(1000),
        length: Tick(3840),
    }));
    all.push(on(arrangement::Command::DeleteTime {
        from: Tick(1000),
        to: Tick(5000),
    }));
    {
        let mut state = f.state.clone();
        let mut ids = ::project::IdGenerator::new(1_700_000_000_000, 0x00CC_0000_0000_0000_0000);
        let lane = ::project::LaneId::generate(&mut ids);
        state
            .project
            .tracks
            .get_mut(&f.track)
            .unwrap()
            .automation
            .insert(
                lane,
                ::project::AutomationLane {
                    target: ::project::AutomationTarget::Mixer {
                        param: ::project::MixerParam::Gain,
                    },
                    points: std::collections::BTreeMap::from([(
                        Tick(6000),
                        ::project::AutomationPoint {
                            value: -6.0,
                            shape: ::project::Shape::Linear,
                        },
                    )]),
                },
            );
        state.project.transport.loop_region = ::project::LoopRegion::new(Tick(2000), Tick(8000));
        let (state, restore) = after(
            state,
            &a(arrangement::Command::InsertTime {
                at: Tick(1000),
                length: Tick(3840),
            }),
        );
        all.push((restore, state));
    }
    // The inverse of an edit that creates a clip, and of one that carves: the restores.
    for command in [
        arrangement::Command::Duplicate {
            placements: vec![f.c.clone()],
            linked: false,
            by: None,
            if_occupied: arrangement::IfOccupied::Replace,
        },
        arrangement::Command::Delete {
            placements: vec![f.b.clone()],
        },
        arrangement::Command::Repeat {
            placement: f.a.clone(),
            until: Tick(3840 * 6),
            if_occupied: Default::default(),
        },
    ] {
        let (state, restore) = after(f.state.clone(), &a(command));
        all.push((restore, state));
    }
    // An audio placement: its delete, with a MIDI one in the same command, and the restore
    // that brings it back.
    let (placed, audio) = with_audio_placement(&f);
    let delete = a(arrangement::Command::Delete {
        placements: vec![audio, f.b.clone()],
    });
    all.push((delete.clone(), placed.clone()));
    let (state, restore) = after(placed.clone(), &delete);
    all.push((restore, state));
    // The audio branches (AU-6): an audio placement moved, resized at each edge, split,
    // copied (an independent copy makes an audio clip; its undo brings back nothing but
    // removes it) and repeated, each with the restore that undoes it.
    let audio = with_audio_placement(&f).1;
    for command in [
        arrangement::Command::Move {
            placements: vec![audio.clone()],
            by: Tick(960),
            if_occupied: Default::default(),
        },
        arrangement::Command::Resize {
            placement: audio.clone(),
            edge: Edge::End,
            to: Tick(4320),
        },
        arrangement::Command::Resize {
            placement: audio.clone(),
            edge: Edge::Start,
            to: Tick(3600),
        },
        arrangement::Command::Split {
            placement: audio.clone(),
            at: Tick(4320),
        },
        arrangement::Command::Duplicate {
            placements: vec![audio.clone()],
            linked: false,
            by: None,
            if_occupied: arrangement::IfOccupied::Replace,
        },
        arrangement::Command::Repeat {
            placement: audio.clone(),
            until: Tick(3840 * 3),
            if_occupied: Default::default(),
        },
    ] {
        let command = a(command);
        all.push((command.clone(), placed.clone()));
        let (state, restore) = after(placed.clone(), &command);
        all.push((restore, state));
    }
    all
}

/// The fixture's state with a placement of its audio clip (22 050 frames from frame
/// 11 025, at tick 3840).
pub fn with_audio_placement(f: &ArFx) -> (State, PlacementId) {
    let mut state = f.state.clone();
    let mut ids = ::project::IdGenerator::new(1_680_000_000_000, 0x00DD_3333_0000_0000_0000);
    let id = PlacementId::generate(&mut ids);
    track_mut(&mut state, &f.audio_track).placements.insert(
        id.clone(),
        Placement {
            clip: f.audio_clip.clone(),
            start: Tick(3840),
            span: Span::Frames {
                length: ::project::Frame::new(22_050),
                offset: ::project::Frame::new(11_025),
            },
        },
    );
    state.project.validate().unwrap();
    (state, id)
}
