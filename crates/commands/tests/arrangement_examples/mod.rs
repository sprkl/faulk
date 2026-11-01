//! The state the arrangement commands run from, and one example of every branch of each
//! of them. `tests/contract.rs` adds [`examples`] to the catalog of examples (a command
//! without one fails there); `tests/arrangement.rs` checks the rule on top of them.

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
        }),
        // Over A (cut on its right) and B (trimmed at the front).
        on(arrangement::Command::Place {
            clip: f.straight.clone(),
            start: Tick(3000),
            length: Some(Tick(1000)),
            offset: Some(Tick(960)),
        }),
        on(arrangement::Command::Move {
            placements: vec![f.a.clone(), f.b.clone()],
            by: Tick(480),
        }),
        // C onto B: B is covered and removed.
        on(arrangement::Command::Move {
            placements: vec![f.c.clone()],
            by: Tick(-3840),
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
        }),
        // Linked, onto the next ones: they are carved.
        on(arrangement::Command::Duplicate {
            placements: vec![f.a.clone()],
            linked: true,
            by: Some(Tick(7000)),
        }),
    ];
    // From the launch grid: an independent copy, a linked placement, one over what is
    // there, and a whole scene both ways.
    let grid = f.grid_state();
    let from_grid = |command: arrangement::Command| (a(command), grid.clone());
    for (clip, start, linked) in [
        (&f.looped, 11520, false),
        (&f.looped, 11520, true),
        (&f.looped, 1000, false),
    ] {
        all.push(from_grid(arrangement::Command::PlaceFromSlot {
            clip: clip.clone(),
            start: Tick(start),
            linked,
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
        },
        arrangement::Command::Delete {
            placements: vec![f.b.clone()],
        },
    ] {
        let (state, restore) = after(f.state.clone(), &a(command));
        all.push((restore, state));
    }
    all
}
