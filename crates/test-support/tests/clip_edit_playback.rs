//! CL-3: a thousand edits of a clip while it loops, through the commands, the project
//! compiler and the real synth. The pad (a long note that starts the clip) never drops out,
//! no sample is NaN or denormal, the track is silent after the last note and its release,
//! and the native and WASM renders agree.

use commands::testing;
use commands::{Command, Output, clip, launch, scene, transport};
use project::{Note, Tick, TickLoop};
use test_support::{Render, Scenario, Session, parity, render_native};

fn n(start: i64, pitch: u8, length: i64) -> Note {
    Note {
        start: Tick(start),
        pitch,
        length: Tick(length),
        velocity: 100,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    }
}

/// One-bar loop at 120 BPM (2 s). The pad is 1.5 s long. `edits` notes are added and
/// removed after the pad, two blocks apart, during the first 5.3 s. The transport stops
/// at 6 s.
fn scenario(edits: usize) -> Scenario {
    let fixture = testing::fixture();
    let mut s = Session::from_state(fixture.state.clone(), testing::context());
    let Output::Scene(scene::Output::Create { scene }) =
        s.exec(Command::Scene(scene::Command::Create { name: None }))
    else {
        panic!("a scene")
    };
    let Output::Clip(clip::Output::Create { clip, .. }) =
        s.exec(Command::Clip(clip::Command::Create {
            track: fixture.track.clone(),
            at: clip::ClipTarget::Slot { scene },
            length: Some(Tick(3840)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    s.run(Command::Clip(clip::Command::NotesAdd {
        clip: clip.clone(),
        notes: vec![n(0, 48, 3000)],
    }));
    s.run(Command::Clip(clip::Command::SetLoop {
        clip: clip.clone(),
        looping: Some(TickLoop {
            start: Tick(0),
            end: Tick(3840),
        }),
    }));
    s.run(Command::Launch(launch::Command::Clip {
        clip: clip.clone(),
    }));
    let mut present: Vec<Note> = Vec::new();
    for i in 0..edits {
        // Notes after the pad in start order: the pad keeps its place in the table.
        let note = n(3100 + (i as i64 % 40) * 15, 60 + (i % 24) as u8, 40);
        if present.contains(&note) {
            s.run(Command::Clip(clip::Command::NotesRemove {
                clip: clip.clone(),
                notes: vec![note],
            }));
            present.retain(|p| *p != note);
        } else {
            s.run(Command::Clip(clip::Command::NotesAdd {
                clip: clip.clone(),
                notes: vec![note],
            }));
            present.push(note);
        }
        s.render_blocks(2);
    }
    let rendered = s.scenario().seconds();
    s.render_secs((6.0 - rendered).max(0.0));
    s.run(Command::Transport(transport::Command::Stop {}));
    s.render_secs(1.5);
    s.into_scenario()
}

/// The lowest peak over 10 ms windows inside the pad of each pass, `from` to `to` seconds
/// of the pass (the pad sounds 0 to 1.5 s of a 2 s pass).
fn quietest_pad_window(r: &Render, until: f32) -> f32 {
    let mut lowest = f32::MAX;
    let mut pass = 0.0f32;
    while pass * 2.0 + 0.2 <= until {
        let mut t = pass * 2.0 + 0.1;
        while t + 0.01 <= (pass * 2.0 + 1.45).min(until) {
            lowest = lowest.min(r.peak_in(t..t + 0.01));
            t += 0.01;
        }
        pass += 1.0;
    }
    lowest
}

#[test]
fn a_thousand_edits_during_playback_leave_the_pad_unbroken_and_the_track_silent_after() {
    let reference = render_native(&scenario(0));
    let edited = scenario(1000);
    let r = parity(&edited);
    r.assert_clean();
    let base = quietest_pad_window(&reference, 5.3);
    assert!(base > 0.01, "the reference pad is audible: {base}");
    let lowest = quietest_pad_window(&r, 5.3);
    assert!(
        lowest > base * 0.5,
        "a 10 ms window of the pad dropped to {lowest} (reference {base})"
    );
    // The transport stopped at 6 s: silent once the release is over.
    r.assert_silent(7.0..7.5);
}
