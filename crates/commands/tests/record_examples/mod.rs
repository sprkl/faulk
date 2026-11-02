//! Examples of the `record.*` commands, with the state they run from. `tests/contract.rs`
//! adds [`examples`] to the catalog of examples (a command without one fails there);
//! `tests/record.rs` checks the take mapping on the same states.

#![allow(dead_code)]

use ::project::{ClipId, Note, PlacementId, SceneId, Tick, TrackId};
use commands::clip::{ClipTarget, Command as Cl};
use commands::record::{ClipRestore, Command as R, RecordTarget, TakeNote, TakeTrack};
use commands::scene::Command as Sc;
use commands::testing::fixture;
use commands::{Author, Command, Context, Output, State, run};

/// The fixture's MIDI track, armed, with two scenes: in the first a looping clip of one bar
/// (notes at 0 and 1920), the second empty; and a clip of one bar placed at bar 3 (tick
/// 7680) in the arrangement.
pub struct Rec {
    pub state: State,
    pub track: TrackId,
    pub scene: SceneId,
    pub empty: SceneId,
    pub grid_clip: ClipId,
    pub placed_clip: ClipId,
    pub placement: PlacementId,
}

/// Its ids come from a seed of their own, apart from the fixture's and the test context's.
fn ran(state: &mut State, ctx: &mut Context, command: Command) -> Output {
    run(&command, state, ctx)
        .unwrap_or_else(|e| panic!("{command:?}: {e}"))
        .output
}

pub fn rec() -> Rec {
    let f = fixture();
    let mut state = f.state;
    let track = f.track;
    let ctx = &mut Context::new(
        Author::User,
        ::project::IdGenerator::new(1_660_000_000_000, 0x00AB_0000_0000_0000_0000),
    );
    let scene = match ran(&mut state, ctx, Command::Scene(Sc::Create { name: None })) {
        Output::Scene(commands::scene::Output::Create { scene }) => scene,
        o => panic!("{o:?}"),
    };
    let empty = match ran(&mut state, ctx, Command::Scene(Sc::Create { name: None })) {
        Output::Scene(commands::scene::Output::Create { scene }) => scene,
        o => panic!("{o:?}"),
    };
    let grid_clip = match ran(
        &mut state,
        ctx,
        Command::Clip(Cl::Create {
            track: track.clone(),
            at: ClipTarget::Slot {
                scene: scene.clone(),
            },
            length: None,
            name: None,
        }),
    ) {
        Output::Clip(commands::clip::Output::Create { clip, .. }) => clip,
        o => panic!("{o:?}"),
    };
    ran(
        &mut state,
        ctx,
        Command::Clip(Cl::SetLoop {
            clip: grid_clip.clone(),
            looping: Some(::project::TickLoop {
                start: Tick(0),
                end: Tick(3840),
            }),
        }),
    );
    ran(
        &mut state,
        ctx,
        Command::Clip(Cl::NotesAdd {
            clip: grid_clip.clone(),
            notes: vec![
                Note::new(Tick(0), 48, Tick(480), 90),
                Note::new(Tick(1920), 50, Tick(480), 90),
            ],
        }),
    );
    let (placed_clip, placement) = match ran(
        &mut state,
        ctx,
        Command::Clip(Cl::Create {
            track: track.clone(),
            at: ClipTarget::Arrangement { start: Tick(7680) },
            length: None,
            name: None,
        }),
    ) {
        Output::Clip(commands::clip::Output::Create { clip, placement }) => {
            (clip, placement.unwrap())
        }
        o => panic!("{o:?}"),
    };
    state.session.armed.insert(track.clone());
    Rec {
        state,
        track,
        scene,
        empty,
        grid_clip,
        placed_clip,
        placement,
    }
}

pub fn note(start: i64, pitch: u8, length: i64) -> TakeNote {
    TakeNote {
        start: Tick(start),
        length: Tick(length),
        pitch,
        velocity: 100,
    }
}

pub fn take(r: &Rec, notes: Vec<TakeNote>, anchor: Option<i64>) -> Vec<TakeTrack> {
    vec![TakeTrack {
        track: r.track.clone(),
        notes,
        anchor: anchor.map(Tick),
    }]
}

pub fn examples() -> Vec<(Command, State)> {
    let r = rec();
    let on = |c: R| (Command::Record(c), r.state.clone());
    vec![
        on(R::SetSettings { offset_ms: 12 }),
        on(R::Start {
            target: Some(RecordTarget::Scene {
                scene: r.scene.clone(),
            }),
            latency_ms: Some(20.0),
        }),
        on(R::Stop {}),
        on(R::DiscardTake {}),
        // Overdub into the looping grid clip.
        on(R::CommitTake {
            target: RecordTarget::Scene {
                scene: r.scene.clone(),
            },
            shift_ms: 0.0,
            tracks: take(&r, vec![note(4800, 60, 240)], Some(3840)),
        }),
        // A new grid clip.
        on(R::CommitTake {
            target: RecordTarget::Scene {
                scene: r.empty.clone(),
            },
            shift_ms: 0.0,
            tracks: take(&r, vec![note(4000, 60, 240), note(9000, 62, 240)], None),
        }),
        // The arrangement: one note into the placed clip, one in a new clip before it.
        on(R::CommitTake {
            target: RecordTarget::Arrangement {},
            shift_ms: 10.0,
            tracks: take(&r, vec![note(8000, 60, 240), note(1000, 62, 240)], None),
        }),
        on(R::Restore {
            clips: vec![ClipRestore {
                track: r.track.clone(),
                clip: r.placed_clip.clone(),
                state: None,
            }],
        }),
    ]
}
