//! DV-2 through the command layer and the engine: `device.insert` puts the filter after the
//! synth of a track, `device.set_param` (by name) moves it, `device.set_bypass` makes it
//! transparent, and an automation lane on its `cutoff` sweeps it. The render, a saw note
//! through the low-pass with the cutoff swept from 200 Hz to 8 kHz at a resonance of 0.7, is
//! the same natively and in WASM and is written to `.qa/renders/filter-sweep.wav` for the
//! ear. (The device itself is tested in `crates/engine/tests/it/filter.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, automation, device, history, live, transport};
use project::{AutomationTarget, DeviceId, Shape, Tick};
use test_support::{Session, parity};

/// Ticks per second at 120 BPM (960 per quarter note, two quarter notes per second).
const TICKS_PER_SECOND: i64 = 1920;

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

fn insert(s: &mut Session, track: &project::TrackId) -> DeviceId {
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.filter".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };
    fx
}

/// How bright a window is: the rms of its first difference over its rms (a pure low note
/// is near 0, a buzzy one is high).
fn brightness(x: &[f32]) -> f32 {
    let rms = |v: &mut dyn Iterator<Item = f32>| {
        let (sum, n) = v.fold((0.0f32, 0usize), |(s, n), a| (s + a * a, n + 1));
        (sum / n as f32).sqrt()
    };
    let diff = rms(&mut x.windows(2).map(|w| w[1] - w[0]));
    diff / rms(&mut x.iter().copied()).max(1e-9)
}

#[test]
fn a_cutoff_lane_sweeps_the_low_pass_over_a_held_saw() {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let fx = insert(&mut s, &f.track);
    set(&mut s, &fx, "mode", 0.0);
    set(&mut s, &fx, "resonance", 0.7);
    set(&mut s, &fx, "cutoff", 200.0);
    // Eight steps of an exponential sweep, 200 Hz to 8 kHz in 4 s (even to the ear).
    let Output::Automation(automation::Output::LaneCreate { lane }) =
        s.exec(Command::Automation(automation::Command::LaneCreate {
            track: Some(f.track.clone()),
            master: false,
            target: AutomationTarget::Device {
                device: fx.clone(),
                param: "cutoff".into(),
            },
        }))
    else {
        panic!("a lane")
    };
    let points: Vec<automation::Point> = (0..=8)
        .map(|i| automation::Point {
            tick: Tick(i * TICKS_PER_SECOND / 2),
            value: 200.0 * 40f64.powf(i as f64 / 8.0),
            shape: Shape::Linear,
        })
        .collect();
    s.run(Command::Automation(automation::Command::PointsAdd {
        lane,
        points,
    }));
    s.run(Command::Transport(transport::Command::Play {}));
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 45,
        velocity: 110,
    }));
    s.render_secs(4.0);

    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.5..4.0, 0.01);
    let sr = r.sample_rate;
    let window = |from: f32| {
        let a = (from * sr) as usize;
        &r.left[a..a + (0.25 * sr) as usize]
    };
    let (early, late) = (brightness(window(0.25)), brightness(window(3.5)));
    println!("brightness: {early:.3} at 0.25 s, {late:.3} at 3.5 s");
    assert!(
        late > 3.0 * early,
        "the sweep opens the sound: {early} -> {late}"
    );
    r.write_wav("filter-sweep");
}

#[test]
fn bypassed_the_filter_changes_nothing_and_its_mix_blends() {
    let f = fixture();
    let render = |filter: Option<(&str, f64)>| {
        let mut s = Session::from_state(f.state.clone(), context());
        if let Some((param, value)) = filter {
            let fx = insert(&mut s, &f.track);
            set(&mut s, &fx, "cutoff", 300.0);
            if param == "bypass" {
                s.run(Command::Device(device::Command::SetBypass {
                    device: fx,
                    bypass: value != 0.0,
                }));
            } else {
                set(&mut s, &fx, param, value);
            }
        }
        s.run(Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note: 57,
            velocity: 100,
        }));
        s.render_secs(0.5);
        parity(s.scenario())
    };
    let plain = render(None);
    let bypassed = render(Some(("bypass", 1.0)));
    let dry = render(Some(("mix", 0.0)));
    let wet = render(Some(("mix", 1.0)));
    plain.assert_close(&bypassed, 1e-6);
    // The mix ramps for 20 ms from its default (1) to 0: the dry sound is exact after it.
    let from = (0.1 * plain.sample_rate) as usize;
    let tail = |r: &test_support::Render| {
        test_support::Render::new(
            r.sample_rate,
            r.left[from..].to_vec(),
            r.right[from..].to_vec(),
        )
    };
    tail(&plain).assert_close(&tail(&dry), 1e-6);
    let (diff, _) = plain.max_difference(&wet);
    assert!(
        diff > 0.05,
        "the filtered note differs from the dry one: {diff}"
    );
}

#[test]
fn a_cleared_cutoff_lane_goes_back_to_the_stored_cutoff_and_its_points_play_again_after_undo() {
    cleared_cutoff_lane(true, "filter-lane-clear");
}

#[test]
fn a_cutoff_lane_cleared_while_stopped_goes_back_to_the_stored_cutoff_too() {
    cleared_cutoff_lane(false, "filter-lane-clear-stopped");
}

fn cleared_cutoff_lane(playing: bool, wav: &str) {
    // #394: a lane that is cleared stops driving the cutoff, which goes back to the project's
    // value (200 Hz, dark); undo brings the points back and the lane takes the cutoff again
    // (the restoring message holds nothing, the lane having left the engine).
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let fx = insert(&mut s, &f.track);
    set(&mut s, &fx, "mode", 0.0);
    set(&mut s, &fx, "resonance", 0.7);
    set(&mut s, &fx, "cutoff", 200.0);
    let Output::Automation(automation::Output::LaneCreate { lane }) =
        s.exec(Command::Automation(automation::Command::LaneCreate {
            track: Some(f.track.clone()),
            master: false,
            target: AutomationTarget::Device {
                device: fx.clone(),
                param: "cutoff".into(),
            },
        }))
    else {
        panic!("a lane")
    };
    // The lane opens the filter at once: 8 kHz from the first tick.
    s.run(Command::Automation(automation::Command::PointsAdd {
        lane: lane.clone(),
        points: vec![automation::Point {
            tick: Tick(0),
            value: 8000.0,
            shape: Shape::Step,
        }],
    }));
    if playing {
        s.run(Command::Transport(transport::Command::Play {}));
    }
    s.run(Command::Live(live::Command::NoteOn {
        track: f.track.clone(),
        note: 45,
        velocity: 110,
    }));
    s.render_secs(1.0);
    s.run(Command::Automation(automation::Command::LaneClear {
        lane: lane.clone(),
    }));
    s.render_secs(1.0);
    s.run(Command::History(history::Command::Undo {}));
    s.render_secs(1.0);

    let r = parity(s.scenario());
    r.assert_clean();
    r.assert_audible(0.5..3.0, 0.01);
    let sr = r.sample_rate;
    let window = |from: f32| {
        let a = (from * sr) as usize;
        &r.left[a..a + (0.25 * sr) as usize]
    };
    let (open, closed, reopened) = (
        brightness(window(0.5)),
        brightness(window(1.75)),
        brightness(window(2.75)),
    );
    println!("brightness: {open:.3} (lane), {closed:.3} (cleared), {reopened:.3} (undone)");
    assert!(
        open > 3.0 * closed,
        "the cleared lane lets go: {open} -> {closed}"
    );
    assert!(
        reopened > 3.0 * closed,
        "the undone lane plays again: {closed} -> {reopened}"
    );
    r.write_wav(wav);
}

// ---------------------------------------------------------------- the movement (#686)

fn load(s: &mut Session, fx: &DeviceId, id: &str) {
    s.run(Command::Preset(commands::preset::Command::Load {
        device: fx.clone(),
        preset: commands::preset::PresetRef {
            location: project::preset::PresetLocation::Factory,
            id: id.into(),
        },
    }));
}

/// The two factory presets of DV-2 now name the sixteen parameters (the new ones at their
/// neutral defaults): they sound as their four old values set alone, bit for bit. With
/// `engine/tests/it/filter_reference.rs` (the Filter at those values renders what the frozen
/// DV-2 Filter did), a preset of then sounds as it did.
#[test]
fn the_presets_of_dv2_sound_as_their_four_old_values_do() {
    let f = fixture();
    for (id, old) in [
        (
            "faulk.filter/warm-lowpass",
            [
                ("cutoff", 1200.0),
                ("mix", 1.0),
                ("mode", 0.0),
                ("resonance", 0.25),
            ],
        ),
        (
            "faulk.filter/thin-highpass",
            [
                ("cutoff", 600.0),
                ("mix", 1.0),
                ("mode", 1.0),
                ("resonance", 0.15),
            ],
        ),
    ] {
        let render = |by_preset: bool| {
            let mut s = Session::from_state(f.state.clone(), context());
            let fx = insert(&mut s, &f.track);
            if by_preset {
                load(&mut s, &fx, id);
            } else {
                for (param, value) in old {
                    set(&mut s, &fx, param, value);
                }
            }
            for note in [45, 52, 57] {
                s.run(Command::Live(live::Command::NoteOn {
                    track: f.track.clone(),
                    note,
                    velocity: 100,
                }));
            }
            s.render_secs(1.0);
            test_support::render_native(s.scenario())
        };
        let (a, b) = (render(true), render(false));
        a.assert_audible(0.1..1.0, 0.001);
        assert_eq!(a.max_difference(&b), (0.0, 0), "{id}");
    }
}

/// Every new parameter is set by name with `device.set_param`, and a lane on each continuous
/// one is created and played: the render moves with it, the same natively and in WASM.
#[test]
fn every_movement_parameter_is_set_by_name_and_each_continuous_one_takes_a_lane() {
    let f = fixture();
    let base = |s: &mut Session, fx: &DeviceId| {
        set(s, fx, "cutoff", 600.0);
        set(s, fx, "resonance", 0.5);
        set(s, fx, "lfo_sync", 0.0);
        set(s, fx, "lfo_rate", 3.0);
    };
    let play = |lane: Option<(&str, f64, f64)>, extra: &[(&str, f64)]| {
        let mut s = Session::from_state(f.state.clone(), context());
        let fx = insert(&mut s, &f.track);
        base(&mut s, &fx);
        for (param, value) in extra {
            set(&mut s, &fx, param, *value);
        }
        if let Some((param, from, to)) = lane {
            let Output::Automation(automation::Output::LaneCreate { lane }) =
                s.exec(Command::Automation(automation::Command::LaneCreate {
                    track: Some(f.track.clone()),
                    master: false,
                    target: AutomationTarget::Device {
                        device: fx.clone(),
                        param: param.into(),
                    },
                }))
            else {
                panic!("a lane on {param}")
            };
            s.run(Command::Automation(automation::Command::PointsAdd {
                lane,
                points: vec![
                    automation::Point {
                        tick: Tick(0),
                        value: from,
                        shape: Shape::Linear,
                    },
                    automation::Point {
                        tick: Tick(TICKS_PER_SECOND),
                        value: to,
                        shape: Shape::Linear,
                    },
                ],
            }));
        }
        // A while stopped first: a free LFO runs on meanwhile, so `lfo_restart` is heard.
        s.render_secs(0.3);
        s.run(Command::Transport(transport::Command::Play {}));
        s.run(Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note: 45,
            velocity: 110,
        }));
        s.render_secs(1.0);
        s
    };
    // Each choice and value by name: the render differs from the filter without it.
    let still = test_support::render_native(play(None, &[]).scenario());
    for (param, value) in [
        ("mode", 3.0),
        ("drive", 0.8),
        ("lfo_shape", 4.0),
        ("lfo_division", 2.0),
        ("lfo_phase", 90.0),
        ("lfo_stereo", 180.0),
        ("lfo_restart", 0.0),
        ("env_amount", 3.0),
        ("env_attack", 0.3),
        ("env_release", 1.0),
    ] {
        // The LFO and envelope settings are heard only with some movement; the note value
        // only in synced mode.
        let sync = if param == "lfo_division" { 1.0 } else { 0.0 };
        let r = test_support::render_native(
            play(
                None,
                &[
                    ("lfo_depth", 1.0),
                    ("env_amount", 1.0),
                    ("lfo_sync", sync),
                    (param, value),
                ],
            )
            .scenario(),
        );
        r.assert_clean();
        let moving = test_support::render_native(
            play(
                None,
                &[("lfo_depth", 1.0), ("env_amount", 1.0), ("lfo_sync", sync)],
            )
            .scenario(),
        );
        let changed = moving.max_difference(&r).0 > 1e-3;
        assert!(changed, "{param} = {value} changes the sound");
        assert!(still.max_difference(&r).0 > 1e-3, "{param}");
    }
    // A lane on each continuous parameter of the movement plays (and agrees in WASM).
    for (param, from, to) in [
        ("drive", 0.0, 1.0),
        ("lfo_rate", 0.5, 12.0),
        ("lfo_phase", 0.0, 360.0),
        ("lfo_stereo", 0.0, 180.0),
        ("lfo_depth", 0.0, 3.0),
        ("env_amount", -3.0, 3.0),
        ("env_attack", 0.001, 0.4),
        ("env_release", 0.01, 1.5),
    ] {
        let extra: &[(&str, f64)] = &[("lfo_depth", 1.0), ("env_amount", 1.0)];
        let s = play(Some((param, from, to)), extra);
        let r = parity(s.scenario());
        r.assert_clean();
        let without = test_support::render_native(play(None, extra).scenario());
        assert!(
            without.max_difference(&r).0 > 1e-3,
            "the lane on {param} moves the sound"
        );
    }
}

/// The synced LFO and the envelope in an offline export give the very file the playback of
/// the same song gives, natively and in WASM (#686: export = playback).
#[test]
fn the_export_of_a_moving_filter_is_the_playback_sample_for_sample() {
    use commands::export::{ExportBitDepth, ExportSettings};
    use test_support::export::{demo_song, export_native, export_wasm};
    let mut build = Session::from_state(demo_song(2, None), context());
    let lead = build
        .state()
        .project
        .tracks
        .iter()
        .find(|(_, t)| t.name == "Lead")
        .map(|(id, _)| id.clone())
        .expect("the lead");
    let fx = insert(&mut build, &lead);
    for (param, value) in [
        ("cutoff", 900.0),
        ("resonance", 0.6),
        ("lfo_shape", 5.0),
        ("lfo_division", 3.0),
        ("lfo_depth", 2.0),
        ("lfo_stereo", 90.0),
        ("env_amount", 1.0),
        ("drive", 0.3),
    ] {
        set(&mut build, &fx, param, value);
    }
    // The song with its filter, as a project opened: the playback starts from its snapshot.
    let mut song = Session::from_state(build.state().clone(), context());
    let settings = ExportSettings {
        bit_depth: ExportBitDepth::Float32,
        ..ExportSettings::default()
    };
    let exported = export_native(song.state(), settings);
    let n = exported.body_frames as usize;
    let file = exported.render();
    file.assert_audible(0.2..3.5, 0.01);
    // Played: half a second stopped, then from bar 1.
    let q = test_support::QUANTUM as usize;
    let before = (0.5 * test_support::SAMPLE_RATE / q as f32).ceil() as usize * q;
    song.render_secs(0.5);
    song.run(Command::Transport(transport::Command::Play {}));
    song.render_secs(n as f32 / test_support::SAMPLE_RATE + 0.1);
    let played = test_support::render_native(song.scenario());
    let played = test_support::Render::new(
        played.sample_rate,
        played.left[before..before + n].to_vec(),
        played.right[before..before + n].to_vec(),
    );
    let body = test_support::Render::new(
        file.sample_rate,
        file.left[..n].to_vec(),
        file.right[..n].to_vec(),
    );
    let (diff, at) = body.max_difference(&played);
    assert_eq!(
        diff, 0.0,
        "the file differs from the playback at frame {at}"
    );
    let wasm = export_wasm(song.state(), settings);
    assert_eq!(wasm.body_frames, exported.body_frames);
    let (d, at) = wasm.render().max_difference(&file);
    assert!(
        d <= test_support::PARITY_TOLERANCE,
        "WASM export differs by {d} at {at}"
    );
    body.write_wav("filter-movement-export");
}

/// Renders for the ear (`.qa/renders/`, not committed): a held saw chord through the Filter
/// without and with drive at full resonance, and the quarter wobble under a rising cutoff lane.
#[test]
fn renders_of_the_movement_are_written_for_listening() {
    let f = fixture();
    let chord = |s: &mut Session| {
        for note in [33, 45, 52] {
            s.run(Command::Live(live::Command::NoteOn {
                track: f.track.clone(),
                note,
                velocity: 110,
            }));
        }
    };
    for (name, drive) in [("filter-drive-off", 0.0), ("filter-drive-full", 1.0)] {
        let mut s = Session::from_state(f.state.clone(), context());
        let fx = insert(&mut s, &f.track);
        for (param, value) in [
            ("cutoff", 500.0),
            ("resonance", 0.95),
            ("drive", drive),
            ("lfo_depth", 2.0),
            ("lfo_division", 8.0),
            ("lfo_shape", 0.0),
        ] {
            set(&mut s, &fx, param, value);
        }
        s.run(Command::Transport(transport::Command::Play {}));
        chord(&mut s);
        s.render_secs(4.0);
        let r = parity(s.scenario());
        r.assert_clean();
        r.write_wav(name);
    }
    let mut s = Session::from_state(f.state.clone(), context());
    let fx = insert(&mut s, &f.track);
    load(&mut s, &fx, "faulk.filter/quarter-wobble");
    let Output::Automation(automation::Output::LaneCreate { lane }) =
        s.exec(Command::Automation(automation::Command::LaneCreate {
            track: Some(f.track.clone()),
            master: false,
            target: AutomationTarget::Device {
                device: fx.clone(),
                param: "cutoff".into(),
            },
        }))
    else {
        panic!("a lane")
    };
    s.run(Command::Automation(automation::Command::PointsAdd {
        lane,
        points: vec![
            automation::Point {
                tick: Tick(0),
                value: 200.0,
                shape: Shape::Linear,
            },
            automation::Point {
                tick: Tick(8 * TICKS_PER_SECOND),
                value: 8000.0,
                shape: Shape::Linear,
            },
        ],
    }));
    s.run(Command::Transport(transport::Command::Play {}));
    chord(&mut s);
    s.render_secs(8.0);
    let r = parity(s.scenario());
    r.assert_clean();
    r.write_wav("filter-wobble-under-lane");
}
