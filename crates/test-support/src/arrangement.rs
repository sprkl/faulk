//! A two-track song in the arrangement (AR-2), built by commands only: clips created in the
//! arrangement, notes added, the transport played, sought and looped, a clip launched from
//! the grid and the tracks given back with `launch.back_to_arrangement`. Shared by the
//! engine's render test and the native/WASM parity test. At 120 BPM in 4/4 a bar is 2 s and
//! 960 ticks are 0.5 s.

use commands::testing;
use commands::{Command, Output, clip, launch, track, transport};
use engine::msg::{ControlMsg, SceneSlot, TrackSlot};
use project::{Note, Scene, SceneId, Tick, TickLoop};

use crate::scenario::{Scenario, Session};

/// How the song is played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Play {
    /// Played from the start to [`STOP_SECS`].
    Straight,
    /// Sought to the middle of the bass's first note at [`SEEK_SECS`].
    SeekIntoNote,
    /// A loop over the first bar, played for the whole render.
    RegionLoop,
    /// A clip of the lead's grid launched at [`LAUNCH_SECS`] (it takes the track at the bar
    /// line), the tracks given back at [`BACK_SECS`].
    GridTakeover,
    /// All of it in one render, for the native/WASM comparison (an interpreted render is
    /// slow): a loop over both bars, a seek into a note at 0.5 s (the loop wraps at 3.72 s),
    /// the lead's grid clip launched at 4.5 s (it takes the track at the bar line), the
    /// tracks given back at 6.5 s inside a note of the lead's clip.
    Everything,
}

/// When [`Play::Straight`] stops.
pub const STOP_SECS: f32 = 6.0;
/// When [`Play::SeekIntoNote`] seeks.
pub const SEEK_SECS: f32 = 0.5;
/// The tick it seeks to: 0.78 s into the song, inside the bass's first note (0 to 3000).
pub const SEEK_TICK: i64 = 1500;
/// When [`Play::GridTakeover`] launches the lead's grid clip.
pub const LAUNCH_SECS: f32 = 1.3;
/// When [`Play::GridTakeover`] gives the tracks back: inside the lead's second note.
pub const BACK_SECS: f32 = 3.2;
/// The length of every render.
pub const END_SECS: f32 = 8.0;

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

/// The song: the synth track of `commands::testing::fixture` (the bass) and its duplicate
/// (the lead). The bass has a two-bar clip from the start, a long note then a second one a
/// bar later; the lead has a one-bar clip from the start of bar 2 (three notes, the second
/// one still sounding when the tracks are given back, the third later), and a one-bar loop
/// in the grid.
pub fn song(play: Play) -> Scenario {
    let fixture = testing::fixture();
    let mut state = fixture.state.clone();
    let mut ids = project::IdGenerator::new(1_700_000_000_000, 0x00CE_0000_0000_0000_0000);
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
    let create = |s: &mut Session, track, at, length| {
        let Output::Clip(clip::Output::Create { clip, .. }) =
            s.exec(Command::Clip(clip::Command::Create {
                track,
                at,
                length: Some(Tick(length)),
                name: None,
            }))
        else {
            panic!("a clip")
        };
        clip
    };
    let arrangement = |start| clip::ClipTarget::Arrangement { start: Tick(start) };
    let bass_clip = create(&mut s, bass.clone(), arrangement(0), 7680);
    let lead_clip = create(&mut s, lead.clone(), arrangement(3840), 3840);
    let grid_clip = create(
        &mut s,
        lead.clone(),
        clip::ClipTarget::Slot {
            scene: scene.clone(),
        },
        3840,
    );
    for (clip, notes) in [
        (
            &bass_clip,
            vec![n(0, 36, 3000, 110), n(3840, 43, 3000, 100)],
        ),
        (
            &lead_clip,
            vec![
                n(0, 64, 1800, 70),
                n(1920, 67, 900, 70),
                n(3000, 72, 600, 70),
            ],
        ),
        (&grid_clip, vec![n(0, 79, 3600, 60)]),
    ] {
        s.run(Command::Clip(clip::Command::NotesAdd {
            clip: clip.clone(),
            notes,
        }));
    }
    s.run(Command::Clip(clip::Command::SetLoop {
        clip: grid_clip,
        looping: Some(TickLoop {
            start: Tick(0),
            end: Tick(3840),
        }),
    }));
    match play {
        Play::Straight => {
            s.run(Command::Transport(transport::Command::Play {}));
            s.render_secs(STOP_SECS);
            s.run(Command::Transport(transport::Command::Stop {}));
        }
        Play::SeekIntoNote => {
            s.run(Command::Transport(transport::Command::Play {}));
            s.render_secs(SEEK_SECS);
            s.run(Command::Transport(transport::Command::Seek {
                tick: Tick(SEEK_TICK),
            }));
        }
        Play::RegionLoop => {
            s.run(Command::Transport(transport::Command::SetLoop {
                loop_region: Some(TickLoop {
                    start: Tick(0),
                    end: Tick(3840),
                }),
            }));
            s.run(Command::Transport(transport::Command::ToggleLoop {}));
            s.run(Command::Transport(transport::Command::Play {}));
        }
        Play::GridTakeover => {
            let scene_slot =
                SceneSlot(s.ctx().engine.scenes.get(&scene).expect("scene slot") as u16);
            let lead_slot = TrackSlot(s.ctx().engine.tracks.get(&lead).expect("track slot"));
            s.run(Command::Transport(transport::Command::Play {}));
            s.render_secs(LAUNCH_SECS);
            // No `launch.clip` command before GR-2: the engine message.
            s.messages(&[ControlMsg::LaunchClip {
                track: lead_slot,
                scene: scene_slot,
            }]);
            s.render_secs(BACK_SECS - LAUNCH_SECS);
            s.run(Command::Launch(launch::Command::BackToArrangement {}));
        }
        Play::Everything => {
            let scene_slot =
                SceneSlot(s.ctx().engine.scenes.get(&scene).expect("scene slot") as u16);
            let lead_slot = TrackSlot(s.ctx().engine.tracks.get(&lead).expect("track slot"));
            s.run(Command::Transport(transport::Command::SetLoop {
                loop_region: Some(TickLoop {
                    start: Tick(0),
                    end: Tick(7680),
                }),
            }));
            s.run(Command::Transport(transport::Command::ToggleLoop {}));
            s.run(Command::Transport(transport::Command::Play {}));
            s.render_secs(SEEK_SECS);
            s.run(Command::Transport(transport::Command::Seek {
                tick: Tick(SEEK_TICK),
            }));
            s.render_secs(4.5 - SEEK_SECS);
            s.messages(&[ControlMsg::LaunchClip {
                track: lead_slot,
                scene: scene_slot,
            }]);
            s.render_secs(2.0);
            s.run(Command::Launch(launch::Command::BackToArrangement {}));
        }
    }
    let rendered = s.scenario().seconds();
    s.render_secs(END_SECS - rendered);
    s.into_scenario()
}
