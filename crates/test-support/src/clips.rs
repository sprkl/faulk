//! A two-track clip project (CL-2): built by commands, its clips launched by raw engine
//! messages (no `launch.*` command exists before GR-2). Shared by the engine's render test,
//! the native/WASM parity test and the listening render.

use commands::testing;
use commands::{Command, Output, clip, track, transport};
use engine::msg::{ControlMsg, SceneSlot, TrackSlot};
use project::{Note, Scene, SceneId, Tick, TickLoop, TrackId};

use crate::scenario::{Scenario, Session};

/// When the second track's clip starts: the first bar line after its launch, 2 s at
/// 120 BPM in 4/4.
pub const SECOND_START_SECS: f32 = 2.0;
/// When `transport.stop` is sent.
pub const STOP_SECS: f32 = 6.0;
/// The length of the render.
pub const END_SECS: f32 = 7.0;

fn n(start: i64, pitch: u8, length: i64, velocity: u8) -> Note {
    Note {
        start: Tick(start),
        pitch,
        length: Tick(length),
        velocity,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    }
}

fn slot_of(s: &Session, track: &TrackId) -> TrackSlot {
    TrackSlot(
        s.ctx()
            .engine
            .tracks
            .get(track)
            .expect("the track has a slot"),
    )
}

/// The project: the synth track of `commands::testing::fixture` and its duplicate, one
/// scene; a one-bar bass loop on the first track, launched at once; a two-bar clip on the
/// second (an intro bar, then a loop over most of its second bar, a note with fine tuning, a note
/// crossing the loop end), launched during the first bar when `second` is true; the
/// transport stops at [`STOP_SECS`].
pub fn two_tracks(second: bool) -> Scenario {
    let fixture = testing::fixture();
    let mut state = fixture.state.clone();
    let mut ids = project::IdGenerator::new(1_700_000_000_000, 0x00CC_0000_0000_0000_0000);
    let scene = SceneId::generate(&mut ids);
    state.project.scenes.insert(
        scene.clone(),
        Scene {
            name: "A".into(),
            order: project::OrderKey::first(),
        },
    );
    let mut s = Session::from_state(state, testing::context());
    let bass = fixture.track.clone();
    let Output::Track(track::Output::Duplicate { track: lead }) =
        s.exec(Command::Track(track::Command::Duplicate {
            track: bass.clone(),
            name: Some("Lead".into()),
        }))
    else {
        panic!("a track")
    };
    let mut clips = Vec::new();
    for (track, length) in [(&bass, 3840), (&lead, 7680)] {
        let Output::Clip(clip::Output::Create { clip, .. }) =
            s.exec(Command::Clip(clip::Command::Create {
                track: track.clone(),
                at: clip::ClipTarget::Slot {
                    scene: scene.clone(),
                },
                length: Some(Tick(length)),
                name: None,
            }))
        else {
            panic!("a clip")
        };
        clips.push(clip);
    }
    let bass_notes = vec![
        n(0, 36, 400, 110),
        n(960, 43, 400, 90),
        n(1920, 36, 400, 110),
        n(2880, 46, 700, 90),
    ];
    let mut tuned = n(5760, 69, 900, 80);
    tuned.cents = 30;
    let lead_notes = vec![
        n(0, 64, 1800, 70),
        n(1920, 67, 1800, 70),
        n(3840, 72, 900, 85),
        n(4800, 71, 900, 85),
        tuned,
        // Crosses the loop end (7200): cut at each wrap.
        n(6720, 76, 900, 75),
    ];
    for (clip, notes, looping) in [
        (&clips[0], bass_notes, (0, 3840)),
        (&clips[1], lead_notes, (3840, 7200)),
    ] {
        s.run(Command::Clip(clip::Command::NotesAdd {
            clip: clip.clone(),
            notes,
        }));
        s.run(Command::Clip(clip::Command::SetLoop {
            clip: clip.clone(),
            looping: Some(TickLoop {
                start: Tick(looping.0),
                end: Tick(looping.1),
            }),
        }));
    }
    let scene_slot = SceneSlot(s.ctx().engine.scenes.get(&scene).expect("scene slot") as u16);
    let (bass_slot, lead_slot) = (slot_of(&s, &bass), slot_of(&s, &lead));
    s.messages(&[ControlMsg::LaunchClip {
        track: bass_slot,
        scene: scene_slot,
    }]);
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(1.3);
    if second {
        s.messages(&[ControlMsg::LaunchClip {
            track: lead_slot,
            scene: scene_slot,
        }]);
    }
    s.render_secs(STOP_SECS - 1.3);
    s.run(Command::Transport(transport::Command::Stop {}));
    s.render_secs(END_SECS - STOP_SECS);
    s.into_scenario()
}

/// When scene B takes over from scene A in [`scene_grid`]: the bar line after its launch.
pub const SCENE_SWITCH_SECS: f32 = 2.0;
/// When `StopAll` takes effect in [`scene_grid`]: the bar line after its order.
pub const STOP_ALL_SECS: f32 = 6.0;
/// The length of the render of [`scene_grid`].
pub const GRID_END_SECS: f32 = 8.0;

/// A two-scene, two-track project (GR-1): each scene holds a one-bar loop on both tracks (a
/// bass and a lead). Scene A is launched with the transport, scene B at 1.3 s when `switch`
/// is true (both tracks change at the bar line of [`SCENE_SWITCH_SECS`]), and `StopAll` at
/// 4.3 s when `stop_all` is true (everything stops at [`STOP_ALL_SECS`]). The orders are raw
/// engine messages (no `launch.*` command exists before GR-2).
pub fn scene_grid(switch: bool, stop_all: bool) -> Scenario {
    let fixture = testing::fixture();
    let mut state = fixture.state.clone();
    let mut ids = project::IdGenerator::new(1_700_000_000_000, 0x00CD_0000_0000_0000_0000);
    let (a, b) = (SceneId::generate(&mut ids), SceneId::generate(&mut ids));
    let first = project::OrderKey::first();
    let second = project::OrderKey::between(Some(&first), None).expect("a key after the first");
    for (id, name, order) in [(&a, "A", first), (&b, "B", second)] {
        state.project.scenes.insert(
            id.clone(),
            Scene {
                name: name.into(),
                order,
            },
        );
    }
    let mut s = Session::from_state(state, testing::context());
    let bass = fixture.track.clone();
    let Output::Track(track::Output::Duplicate { track: lead }) =
        s.exec(Command::Track(track::Command::Duplicate {
            track: bass.clone(),
            name: Some("Lead".into()),
        }))
    else {
        panic!("a track")
    };
    // (scene, track, notes): one bar each, looped over the whole clip.
    let cells = [
        (&a, &bass, vec![n(0, 36, 1800, 110), n(1920, 43, 1800, 100)]),
        (&a, &lead, vec![n(0, 64, 3600, 70)]),
        (&b, &bass, vec![n(0, 41, 1800, 110), n(1920, 48, 1800, 100)]),
        (&b, &lead, vec![n(0, 69, 3600, 70)]),
    ];
    for (scene, track, notes) in cells {
        let Output::Clip(clip::Output::Create { clip, .. }) =
            s.exec(Command::Clip(clip::Command::Create {
                track: track.clone(),
                at: clip::ClipTarget::Slot {
                    scene: scene.clone(),
                },
                length: Some(Tick(3840)),
                name: None,
            }))
        else {
            panic!("a clip")
        };
        s.run(Command::Clip(clip::Command::NotesAdd {
            clip: clip.clone(),
            notes,
        }));
        s.run(Command::Clip(clip::Command::SetLoop {
            clip,
            looping: Some(TickLoop {
                start: Tick(0),
                end: Tick(3840),
            }),
        }));
    }
    let slot = |id: &SceneId| SceneSlot(s.ctx().engine.scenes.get(id).expect("scene slot") as u16);
    let (scene_a, scene_b) = (slot(&a), slot(&b));
    s.messages(&[ControlMsg::LaunchScene { scene: scene_a }]);
    s.run(Command::Transport(transport::Command::Play {}));
    s.render_secs(1.3);
    if switch {
        s.messages(&[ControlMsg::LaunchScene { scene: scene_b }]);
    }
    s.render_secs(3.0);
    if stop_all {
        s.messages(&[ControlMsg::StopAll]);
    }
    s.render_secs(GRID_END_SECS - 4.3);
    s.into_scenario()
}
