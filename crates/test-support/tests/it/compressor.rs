//! DV-5 through the command layer and the engine: `device.insert` puts the compressor after
//! the drums of a track, `device.set_param` (by name) and `device.set_bypass` move it, and
//! the render is the same natively and in WASM. The render, the drum loop dry, with a gentle
//! compression and with a heavy one, is written to `.qa/renders/compressor.wav` for the ear.
//! (The device itself is tested in `crates/engine/tests/it/compressor.rs`.)

use commands::testing::{context, fixture};
use commands::{Command, Output, device, live, track};
use project::{DeviceId, TrackId, TrackKind};
use test_support::{Session, parity};

/// One step of the loop, in blocks of 128 frames: about 0.2507 s (an eighth note at 120 BPM).
const STEP_BLOCKS: u32 = 94;
/// A section: two bars of eight steps.
const SECTION_STEPS: u32 = 16;

fn hit(s: &mut Session, track: &TrackId, note: u8, velocity: u8) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note,
        velocity,
    }));
}

/// Two bars of drums: kick (36), snare (38) and closed hat (42), with accents.
fn drum_loop(s: &mut Session, track: &TrackId) {
    // (kick, snare, hat velocity) per eighth note of a bar, played twice.
    const BAR: [(u8, u8, u8); 8] = [
        (120, 0, 90),
        (0, 0, 50),
        (0, 115, 90),
        (0, 0, 50),
        (110, 0, 90),
        (95, 0, 50),
        (0, 120, 90),
        (0, 0, 45),
    ];
    for _ in 0..2 {
        for (kick, snare, hat) in BAR {
            if kick > 0 {
                hit(s, track, 36, kick);
            }
            if snare > 0 {
                hit(s, track, 38, snare);
            }
            hit(s, track, 42, hat);
            s.render_blocks(STEP_BLOCKS);
        }
    }
}

fn set(s: &mut Session, fx: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: fx.clone(),
        param: param.into(),
        value,
    }));
}

fn bypass(s: &mut Session, fx: &DeviceId, bypass: bool) {
    s.run(Command::Device(device::Command::SetBypass {
        device: fx.clone(),
        bypass,
    }));
}

fn drums_track(s: &mut Session) -> TrackId {
    let out = s.exec(Command::Track(track::Command::Create {
        kind: TrackKind::Midi,
        instrument: Some("faulk.drums".into()),
        name: None,
        color: None,
    }));
    let Output::Track(track::Output::Create { track, .. }) = out else {
        panic!("{out:?}")
    };
    track
}

#[test]
fn the_compressor_tames_a_drum_loop_dry_gentle_then_heavy() {
    let mut s = Session::from_state(fixture().state, context());
    let track = drums_track(&mut s);
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.compressor".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: fx }) = out else {
        panic!("{out:?}")
    };

    // 1. Dry: the compressor is bypassed.
    bypass(&mut s, &fx, true);
    drum_loop(&mut s, &track);
    // 2. Gentle: 2:1 from -14 dB, slow, a little makeup.
    bypass(&mut s, &fx, false);
    for (param, value) in [
        ("threshold", -14.0),
        ("ratio", 2.0),
        ("attack", 0.02),
        ("release", 0.2),
        ("makeup", 2.0),
    ] {
        set(&mut s, &fx, param, value);
    }
    drum_loop(&mut s, &track);
    // 3. Heavy: 8:1 from -26 dB, 5 ms attack, 6 dB of makeup (a bigger makeup lets the
    // first milliseconds of each hit, which the attack has not yet turned down, clip).
    for (param, value) in [
        ("threshold", -26.0),
        ("ratio", 8.0),
        ("attack", 0.005),
        ("release", 0.1),
        ("makeup", 6.0),
    ] {
        set(&mut s, &fx, param, value);
    }
    drum_loop(&mut s, &track);

    let r = parity(s.scenario());
    r.assert_clean();
    let section = (SECTION_STEPS * STEP_BLOCKS * 128) as f32 / 48_000.0;
    let (dry, gentle, heavy) = (
        0.0..section,
        section..2.0 * section,
        2.0 * section..3.0 * section,
    );
    // How unevenly loud the steps of the loop are: the rms of the loudest step over the rms
    // of the quietest one (a hat alone), in decibels.
    let step = (STEP_BLOCKS * 128) as f32 / 48_000.0;
    let spread = |w: &std::ops::Range<f32>| {
        let levels: Vec<f32> = (0..SECTION_STEPS)
            .map(|k| {
                let from = w.start + k as f32 * step;
                r.rms_in(from..from + step)
            })
            .collect();
        let max = levels.iter().cloned().fold(0f32, f32::max);
        let min = levels.iter().cloned().fold(f32::MAX, f32::min);
        20.0 * (max / min).log10()
    };
    let (s_dry, s_gentle, s_heavy) = (spread(&dry), spread(&gentle), spread(&heavy));
    println!(
        "dry: peak {:.3} rms {:.4} step spread {s_dry:.1} dB
gentle: peak {:.3} rms {:.4} step spread {s_gentle:.1} dB
heavy: peak {:.3} rms {:.4} step spread {s_heavy:.1} dB",
        r.peak_in(dry.clone()),
        r.rms_in(dry.clone()),
        r.peak_in(gentle.clone()),
        r.rms_in(gentle.clone()),
        r.peak_in(heavy.clone()),
        r.rms_in(heavy.clone()),
    );
    r.assert_audible(dry.clone(), 0.05);
    // The more it compresses, the more even the steps are.
    assert!(
        s_dry > s_gentle && s_gentle > s_heavy,
        "{s_dry} {s_gentle} {s_heavy}"
    );

    // Bypassed, the loop is the drums alone: the same section of a session without any
    // compressor is identical.
    let mut plain = Session::from_state(fixture().state, context());
    let t2 = drums_track(&mut plain);
    drum_loop(&mut plain, &t2);
    let reference = parity(plain.scenario());
    let frames = (section * 48_000.0) as usize;
    let drift = (0..frames)
        .map(|i| {
            (r.left[i] - reference.left[i])
                .abs()
                .max((r.right[i] - reference.right[i]).abs())
        })
        .fold(0f32, f32::max);
    assert!(
        drift < 1e-6,
        "bypassed, the compressor changes nothing: {drift}"
    );
    r.write_wav("compressor");
}

/// The keys a Compressor had in the MVP (DV-5): a project saved then holds only these.
const MVP_KEYS: [&str; 5] = ["threshold", "ratio", "attack", "release", "makeup"];

/// FNV-1a over the bits of every sample of both channels.
fn fingerprint(r: &test_support::Render) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for (l, rr) in r.left.iter().zip(&r.right) {
        for bits in [l.to_bits(), rr.to_bits()] {
            for byte in bits.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

/// #157: a project saved with the MVP's Compressor (its five parameters only, one on a drum
/// track and one on the master chain) opens and renders the very same samples as before the
/// extension. The fingerprint was taken by this very test on the build that preceded #157
/// (`feat/159-ceiling-limiter` at 2301aeb9 merged with `develop` at 36bf36a4), on the WASM
/// render: the module carries its own libm, so the bits are the same on every platform (the
/// native render goes through the platform's, and must stay within the parity tolerance).
#[test]
fn an_mvp_project_reopens_and_renders_the_same_samples() {
    use commands::contract::Folder;
    use project::file::{MemFs, SaveOptions, SaveState, save, save_session};

    let mut s = Session::from_state(fixture().state, context());
    let track = drums_track(&mut s);
    let insert = |s: &mut Session, track: Option<TrackId>| {
        let master = track.is_none();
        match s.exec(Command::Device(device::Command::Insert {
            track,
            master,
            kind: "faulk.compressor".into(),
            index: 0,
        })) {
            Output::Device(device::Output::Insert { device }) => device,
            other => panic!("{other:?}"),
        }
    };
    let on_track = insert(&mut s, Some(track.clone()));
    let on_master = insert(&mut s, None);
    for (fx, values) in [
        (&on_track, [-26.0, 6.0, 0.003, 0.08, 5.0]),
        (&on_master, [-12.0, 2.5, 0.02, 0.3, 2.0]),
    ] {
        for (key, value) in MVP_KEYS.iter().zip(values) {
            set(&mut s, fx, key, value);
        }
    }
    // The project as the MVP wrote it: no key of a later parameter.
    let mut state = s.state().clone();
    let mut devices = 0;
    for chain in state
        .project
        .tracks
        .values_mut()
        .map(|t| &mut t.devices)
        .chain([&mut state.project.master_devices])
    {
        for d in chain.values_mut() {
            if matches!(&d.source, project::DeviceSource::Builtin { id, .. } if id == "faulk.compressor")
            {
                d.params.retain(|k, _| MVP_KEYS.contains(&k.as_str()));
                assert_eq!(d.params.len(), MVP_KEYS.len());
                devices += 1;
            }
        }
    }
    assert_eq!(devices, 2);
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &state.project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .expect("the project saves");
    save_session(&mut fs, &state.session).expect("the session saves");

    let mut opened = Session::from_state(fixture().state, context());
    opened.stage(Folder::new("mvp", fs)).open("mvp");
    drum_loop(&mut opened, &track);
    let wasm = test_support::render_wasm(opened.scenario());
    let native = parity(opened.scenario());
    native.assert_clean();
    native.assert_audible(0.0..1.0, 0.05);
    let print = fingerprint(&wasm);
    println!(
        "fingerprint {print:#018x}, native {:#018x}",
        fingerprint(&native)
    );
    assert_eq!(print, MVP_FINGERPRINT, "{print:#018x}");
}

const MVP_FINGERPRINT: u64 = 0x5ac9_4a44_bda8_a855;
/// One drum-loop section through a Compressor set by `device.set_param` (by key), bypassed
/// when `params` is `None`.
fn drums_through(params: Option<&[(&str, f64)]>) -> test_support::Render {
    let mut s = Session::from_state(fixture().state, context());
    let track = drums_track(&mut s);
    let fx = match s.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.compressor".into(),
        index: 0,
    })) {
        Output::Device(device::Output::Insert { device }) => device,
        other => panic!("{other:?}"),
    };
    match params {
        None => bypass(&mut s, &fx, true),
        Some(params) => {
            for (key, value) in params {
                set(&mut s, &fx, key, *value);
            }
        }
    }
    drum_loop(&mut s, &track);
    let r = parity(s.scenario());
    r.assert_clean();
    r
}

/// #157 for the ear: the drum loop dry, through the Compressor at the MVP's defaults, and
/// with the look-ahead, a wide knee and parallel compression, written to `.qa/157/`. The
/// look-ahead with a short attack turns the hits down before they pass: the same settings
/// without it let the first milliseconds of each hit through, louder.
#[test]
fn the_drum_loop_with_lookahead_knee_and_mix() {
    let heavy: [(&str, f64); 5] = [
        ("threshold", -30.0),
        ("ratio", 8.0),
        ("attack", 0.001),
        ("release", 0.12),
        ("knee", 12.0),
    ];
    let with = |extra: &[(&str, f64)]| {
        let all: Vec<(&str, f64)> = heavy.iter().chain(extra).copied().collect();
        drums_through(Some(&all))
    };
    let dry = drums_through(None);
    let defaults = drums_through(Some(&[]));
    let caught = with(&[("lookahead", 0.005)]);
    let missed = with(&[]);
    let parallel = with(&[("lookahead", 0.005), ("mix", 0.5), ("makeup", 6.0)]);
    let all = 0.0..(SECTION_STEPS * STEP_BLOCKS * 128) as f32 / 48_000.0;
    let level = |r: &test_support::Render| {
        (
            20.0 * r.peak_in(all.clone()).log10(),
            20.0 * r.rms_in(all.clone()).log10(),
        )
    };
    for (name, r) in [
        ("dry", &dry),
        ("MVP defaults", &defaults),
        ("heavy, no lookahead", &missed),
        ("heavy, lookahead 5 ms", &caught),
        (
            "lookahead 5 ms + knee 12 dB + mix 50% + makeup 6 dB",
            &parallel,
        ),
    ] {
        let (p, rms) = level(r);
        println!(
            "{name}: peak {p:.2} dBFS, rms {rms:.2} dBFS, crest {:.2} dB",
            p - rms
        );
    }
    // The look-ahead catches the hits the 1 ms attack alone lets through.
    let (caught_peak, caught_rms) = level(&caught);
    let (missed_peak, _) = level(&missed);
    assert!(
        caught_peak < missed_peak - 3.0,
        "{caught_peak} {missed_peak}"
    );
    // Parallel: louder than the heavy compression alone, with the hits kept.
    let (par_peak, par_rms) = level(&parallel);
    assert!(par_rms > caught_rms, "{par_rms} {caught_rms}");
    // Half of the dry hits come through: the peak is far above the compressed one's.
    assert!(par_peak > caught_peak + 6.0, "{par_peak} {caught_peak}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.qa/157");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, r) in [
        ("drums-dry", &dry),
        ("drums-mvp-defaults", &defaults),
        ("drums-lookahead-knee-mix", &parallel),
    ] {
        std::fs::write(dir.join(format!("{name}.wav")), r.wav()).unwrap();
    }
}

/// #754: a Compressor restored from a snapshot of the MVP (its five keys), by
/// `device.restore` and by `track.restore`, renders the very same samples as the one that was
/// never removed, and then takes `knee` and `mix`.
#[test]
fn an_mvp_compressor_restored_renders_the_same_samples_and_takes_the_new_parameters() {
    let mut base = Session::from_state(fixture().state, context());
    let track = drums_track(&mut base);
    let fx = match base.exec(Command::Device(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: "faulk.compressor".into(),
        index: 0,
    })) {
        Output::Device(device::Output::Insert { device }) => device,
        other => panic!("{other:?}"),
    };
    for (key, value) in MVP_KEYS.iter().zip([-26.0, 6.0, 0.003, 0.08, 5.0]) {
        set(&mut base, &fx, key, value);
    }
    let before = base.state().clone();
    let mvp_device = {
        let mut d = before.project.tracks[&track].devices[&fx].clone();
        d.params.retain(|k, _| MVP_KEYS.contains(&k.as_str()));
        d
    };
    let render = |s: &Session| {
        let mut s = Session::from_state(s.state().clone(), context());
        drum_loop(&mut s, &track);
        let r = test_support::render_native(s.scenario());
        r.assert_clean();
        fingerprint(&r)
    };
    let expected = render(&base);
    // device.restore
    let mut by_device = Session::from_state(before.clone(), context());
    by_device.run(Command::Device(device::Command::Remove {
        device: fx.clone(),
    }));
    by_device.run(Command::Device(device::Command::Restore {
        track: Some(track.clone()),
        master: false,
        device: fx.clone(),
        snapshot: mvp_device.clone(),
        lanes: Default::default(),
    }));
    // track.restore
    let mut by_track = Session::from_state(before.clone(), context());
    by_track.run(Command::Track(track::Command::Delete {
        track: track.clone(),
    }));
    let mut snapshot = before.project.tracks[&track].clone();
    *snapshot.devices.get_mut(&fx).unwrap() = mvp_device;
    by_track.run(Command::Track(track::Command::Restore {
        track: track.clone(),
        snapshot,
    }));
    for (how, mut s) in [("device.restore", by_device), ("track.restore", by_track)] {
        assert_eq!(s.state().project, before.project, "{how}");
        // Before any setting, the render is the one of the Compressor never removed.
        assert_eq!(render(&s), expected, "{how}");
        // The new parameters can be set, and they change the sound.
        set(&mut s, &fx, "knee", 24.0);
        set(&mut s, &fx, "mix", 0.5);
        assert_ne!(render(&s), expected, "{how}");
    }
}
