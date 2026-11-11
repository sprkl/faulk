//! #191, #156: every factory preset loaded through `preset.load` into its device, played and
//! rendered: no NaN, no denormal, no unexpected silence, and a sound that is audibly its own: its
//! spectrogram stands apart from the device at its defaults (an instrument) or from the source
//! without the device (an effect), and from every other preset of the same device. The load
//! happens while the sound plays, as a musician stepping through presets does.
//!
//! An instrument plays itself (the synth a low chord, a drum kit each sound it names). An
//! effect sits on the master over a mix with something in every band: a drum kit playing a
//! pattern (kick, snare, hats), a bass held on A1 and A2, and a voice-like lead (a saw through a
//! resonant low-pass at 3.5 kHz) singing a line of eighth notes between A3 and G4.
//!
//! The first preset of each device is also rendered through WASM (parity), and every render is
//! written to `.qa/renders/preset-<kind>-<slug>.wav` for the ear. Contour's presets, the source
//! without an effect and a band table go to `.qa/156/`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use commands::preset::PresetRef;
use commands::testing::{context, fixture};
use commands::{Command, Output, device, live, preset, track};
use engine::devices::REGISTRY;
use engine::devices::drums::{CLOSED_HAT_KEY, KICK_KEY, OPEN_HAT_KEY, SNARE_KEY};
use project::preset::{PresetLocation, factory};
use project::{DeviceId, DeviceRole, TrackId, TrackKind};
use test_support::{Render, Session, parity, render_native};

/// One step of the drum pattern: a sixteenth note at 120 BPM.
const STEP: f32 = 0.125;
/// Steps before the load, then after it; the tail follows, notes off.
const BEFORE: u32 = 2;
const AFTER: u32 = 8;
const TAIL: f32 = 1.5;
/// When the preset lands, in seconds.
const LOAD_AT: f32 = BEFORE as f32 * STEP;

/// What sits on the track or the master.
#[derive(Clone, Copy)]
enum Setup<'a> {
    /// No device added: the source alone (an effect's material without the effect).
    Source,
    /// The device at its defaults, no preset loaded.
    Defaults(&'a str),
    /// The device, then the preset loaded while the sound plays.
    Preset(&'a str, &'a str),
}

fn note_on(s: &mut Session, track: &TrackId, note: u8, velocity: u8) {
    s.run(Command::Live(live::Command::NoteOn {
        track: track.clone(),
        note,
        velocity,
    }));
}

fn note_off(s: &mut Session, track: &TrackId, note: u8) {
    s.run(Command::Live(live::Command::NoteOff {
        track: track.clone(),
        note,
    }));
}

/// The lead's line, one note per eighth.
const LINE: [u8; 8] = [57, 60, 64, 67, 65, 64, 62, 60];

/// The tracks an effect's material plays on.
struct Band {
    drums: TrackId,
    lead: TrackId,
}

/// Plays `steps` sixteenths of the drum pattern and the lead's line from `from`, rendering each.
fn play_steps(s: &mut Session, band: &Band, from: u32, steps: u32) {
    let drums = &band.drums;
    for step in from..from + steps {
        if step % 2 == 0 {
            let i = (step / 2) as usize;
            if i > 0 {
                note_off(s, &band.lead, LINE[(i - 1) % LINE.len()]);
            }
            note_on(s, &band.lead, LINE[i % LINE.len()], 90);
        }
        let mut hits = vec![CLOSED_HAT_KEY];
        if step % 4 == 0 {
            hits.push(KICK_KEY);
        }
        if step % 8 == 4 {
            hits = vec![SNARE_KEY, CLOSED_HAT_KEY];
        }
        if step % 8 == 6 {
            hits = vec![OPEN_HAT_KEY];
        }
        for &key in &hits {
            note_on(s, drums, key, 100);
        }
        s.render_secs(STEP);
        for &key in &hits {
            note_off(s, drums, key);
        }
    }
}

fn load(s: &mut Session, target: &DeviceId, id: &str) {
    let out = s.exec(Command::Preset(preset::Command::Load {
        device: target.clone(),
        preset: PresetRef {
            location: PresetLocation::Factory,
            id: id.into(),
        },
    }));
    let Output::Preset(preset::Output::Load {
        ignored,
        clamped,
        defaulted,
        ..
    }) = out
    else {
        panic!("{out:?}")
    };
    assert!(
        ignored.is_empty() && clamped.is_empty() && defaulted.is_empty(),
        "{id}: {ignored:?} {clamped:?} {defaulted:?}"
    );
}

fn create_track(s: &mut Session, instrument: &str) -> (TrackId, DeviceId) {
    let Output::Track(track::Output::Create { track, device }) =
        s.exec(Command::Track(track::Command::Create {
            kind: TrackKind::Midi,
            instrument: Some(instrument.into()),
            name: None,
            color: None,
        }))
    else {
        panic!("a {instrument} track")
    };
    (track, device)
}

fn set(s: &mut Session, device: &DeviceId, param: &str, value: f64) {
    s.run(Command::Device(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    }));
}

/// An effect on the master over drums, a bass and a voice-like lead.
fn effect_session(setup: Setup) -> Session {
    effect_session_for(setup, AFTER)
}

/// [`effect_session`] playing `after` steps after the load instead of [`AFTER`].
fn effect_session_for(setup: Setup, after: u32) -> Session {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let (drums, _) = create_track(&mut s, "faulk.drums");
    let (lead, voice) = create_track(&mut s, "faulk.osc_synth");
    set(&mut s, &voice, "attack", 0.03);
    set(&mut s, &voice, "sustain", 0.8);
    set(&mut s, &voice, "gain", 0.45);
    let out = s.exec(Command::Device(device::Command::Insert {
        track: Some(lead.clone()),
        master: false,
        kind: "faulk.filter".into(),
        index: 0,
    }));
    let Output::Device(device::Output::Insert { device: formant }) = out else {
        panic!("{out:?}")
    };
    set(&mut s, &formant, "cutoff", 3500.0);
    set(&mut s, &formant, "resonance", 0.35);
    // The bass: the fixture's saw, quieter.
    set(&mut s, &f.device, "gain", 0.5);
    let band = Band { drums, lead };
    // Hiccup captures on the transport's grid (#689): with the clock stopped it does nothing, so
    // its session plays.
    if matches!(setup, Setup::Preset("faulk.hiccup", _)) {
        s.run(Command::Transport(commands::transport::Command::Play {}));
    }
    let kind = match setup {
        Setup::Source => None,
        Setup::Defaults(kind) | Setup::Preset(kind, _) => Some(kind),
    };
    let target = kind.map(|kind| {
        let out = s.exec(Command::Device(device::Command::Insert {
            track: None,
            master: true,
            kind: kind.into(),
            index: 0,
        }));
        let Output::Device(device::Output::Insert { device }) = out else {
            panic!("{out:?}")
        };
        device
    });
    let bass = [33, 45];
    for &n in &bass {
        note_on(&mut s, &f.track, n, 90);
    }
    play_steps(&mut s, &band, 0, BEFORE);
    if let (Setup::Preset(_, id), Some(target)) = (setup, &target) {
        load(&mut s, target, id);
    }
    play_steps(&mut s, &band, BEFORE, after);
    for &n in &bass {
        note_off(&mut s, &f.track, n);
    }
    note_off(
        &mut s,
        &band.lead,
        LINE[((BEFORE + after - 1) / 2) as usize % LINE.len()],
    );
    s.render_secs(TAIL);
    s
}

/// A device that takes notes: an instrument playing itself (the synth a low chord, a drum kit
/// each sound it names), or a MIDI effect in front of the synth. `with_device: false` plays a MIDI
/// effect's source: the synth without it.
fn note_session(kind: &str, preset: Option<&str>, with_device: bool) -> Session {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let descriptor = REGISTRY
        .iter()
        .find(|b| b.descriptor.kind == kind)
        .unwrap()
        .descriptor;
    let midi_effect = descriptor.role == engine::device::DeviceRole::MidiEffect;
    let target: Option<DeviceId> = if midi_effect {
        with_device.then(|| {
            let out = s.exec(Command::Device(device::Command::Insert {
                track: Some(f.track.clone()),
                master: false,
                kind: kind.into(),
                index: 0,
            }));
            let Output::Device(device::Output::Insert { device }) = out else {
                panic!("{out:?}")
            };
            device
        })
    } else if kind == "faulk.osc_synth" {
        Some(f.device.clone())
    } else {
        s.run(Command::Track(track::Command::SetInstrument {
            track: f.track.clone(),
            instrument: kind.into(),
        }));
        s.state().project.tracks[&f.track]
            .devices
            .iter()
            .find(|(_, d)| d.role == DeviceRole::Instrument)
            .map(|(id, _)| id.clone())
    };
    let notes: Vec<u8> = if descriptor.named_keys.is_empty() {
        vec![45, 52, 57]
    } else {
        descriptor.named_keys.iter().map(|k| k.key).collect()
    };
    for &note in &notes {
        note_on(&mut s, &f.track, note, 100);
    }
    s.render_secs(LOAD_AT);
    if let (Some(id), Some(target)) = (preset, &target) {
        load(&mut s, target, id);
    }
    // A drum kit is hit again after the load, so that its new sounds are heard, and so are the
    // notes through a MIDI effect (its settings apply to the notes received after them, #690).
    if !descriptor.named_keys.is_empty() || midi_effect {
        for &note in &notes {
            note_on(&mut s, &f.track, note, 100);
            s.render_secs(0.2);
        }
    } else {
        // Played again after the load as well: a pluck has died out by then.
        for &note in &notes {
            note_off(&mut s, &f.track, note);
            note_on(&mut s, &f.track, note, 100);
        }
        s.render_secs(AFTER as f32 * STEP);
    }
    for &note in &notes {
        note_off(&mut s, &f.track, note);
    }
    s.render_secs(TAIL);
    s
}

fn role(kind: &str) -> engine::device::DeviceRole {
    REGISTRY
        .iter()
        .find(|b| b.descriptor.kind == kind)
        .unwrap()
        .descriptor
        .role
}

fn checked(render: &Render, id: &str) {
    render.assert_finite();
    render.assert_no_denormals();
    // Heard after the load, not only before it with the defaults.
    render.assert_audible(LOAD_AT + 0.1..1.0, 1e-3);
    assert!(render.peak() <= 1.0, "{id}: peak {}", render.peak());
}

/// Octave bands, in Hz.
const BANDS: [f32; 11] = [
    20.0, 40.0, 80.0, 160.0, 315.0, 630.0, 1250.0, 2500.0, 5000.0, 10_000.0, 20_000.0,
];

/// Level (dB) of each octave band of `bands` in `window`, from the power of the spectrum's bins.
fn band_levels(render: &Render, window: std::ops::Range<f32>, bands: &[f32]) -> Vec<f32> {
    let spectrum = render.spectrum(window);
    bands
        .windows(2)
        .map(|edge| {
            let power: f32 = spectrum
                .magnitudes
                .iter()
                .enumerate()
                .filter(|(i, _)| (edge[0]..edge[1]).contains(&(*i as f32 * spectrum.bin_hz)))
                .map(|(_, m)| m * m)
                .sum();
            10.0 * (power + 1e-20).log10()
        })
        .collect()
}

/// The octave bands a listener compares presets in: from 40 Hz (the band under it holds little
/// that a listener hears on music).
const HEARD_BANDS: &[f32] = BANDS.split_at(1).1;
const HEARD: usize = HEARD_BANDS.len() - 1;

/// A spectrogram from just after the load to the end of the tail: the heard octave bands of
/// slices of 0.1 s (time effects, an echo or a reverb, differ in the tail as much as in the
/// spectrum). Each slice holds `HEARD` levels, lowest band first.
fn spectrogram(render: &Render) -> Vec<f32> {
    let end = (LOAD_AT + AFTER as f32 * STEP + TAIL).min(render.seconds()) - 0.1;
    let mut cells = Vec::new();
    let mut t = LOAD_AT + 0.1;
    while t < end {
        cells.extend(band_levels(render, t..t + 0.1, HEARD_BANDS));
        t += 0.1;
    }
    cells
}

/// How far apart two renders sound, in dB: per octave band, the mean absolute level difference
/// over the slices where either render makes that band clearly heard (within 40 dB of the
/// loudest cell of both); the largest of the bands. A change confined to one region of the
/// spectrum (a presence lift, a low cut) counts at its full size, not diluted by the bands it
/// leaves alone. A cell is read no lower than 50 dB under the loudest: silence against a quiet
/// sound counts as what is heard, not as hundreds of dB. A band heard in fewer than 3 slices
/// (0.3 s) is left out.
fn distance(a: &[f32], b: &[f32]) -> f32 {
    let loudest = a.iter().chain(b).fold(f32::MIN, |m, v| m.max(*v));
    let floor = loudest - 50.0;
    let mut sums = [(0.0f32, 0u32); HEARD];
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        if x.max(*y) > loudest - 40.0 {
            let band = &mut sums[i % HEARD];
            band.0 += (x.max(floor) - y.max(floor)).abs();
            band.1 += 1;
        }
    }
    sums.iter()
        .filter(|(_, n)| *n >= 3)
        .fold(0.0, |m, (sum, n)| m.max(sum / *n as f32))
}

/// Two renders closer than this, in dB (see [`distance`]), sound alike: a whole octave band 3 dB
/// louder or softer through the passage is a change a listener names without an A/B switch
/// (#156: the founder asked for Contour's two presets to differ clearly by ear; they measure far
/// above it, see the band table under `.qa/156/`).
const DISTINCT_DB: f32 = 3.0;

/// How far above [`DISTINCT_DB`] a [`KNOWN_ALIKE`] pair must stand before the test calls it stale.
const STALE_MARGIN_DB: f32 = 1.0;

/// Presets known to sound like what they are compared with, measured when this test came
/// (#156) and left to their own devices' owners. A listed pair that now stands clearly apart
/// ([`STALE_MARGIN_DB`] above the threshold) fails the test, so that the list only shrinks; a
/// pair near the threshold (small-room) does not flip another change's test.
const KNOWN_ALIKE: &[(&str, &str)] = &[
    // A gentle bus compressor: 0.85 dB from the source.
    ("faulk.compressor/gentle-glue", "the source"),
    // A short room: 2.98 dB from the source (just under the threshold).
    ("faulk.reverb/small-room", "the source"),
    // A vibrato moves the pitch, not the level of a band: 2.20 dB from the source.
    ("faulk.bloom/vibrato", "the source"),
];

fn qa_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.qa")
        .join(name);
    std::fs::create_dir_all(&dir).expect("the QA folder is created");
    dir
}

#[test]
fn every_factory_preset_loads_while_playing_and_sounds_unlike_its_source_and_its_siblings() {
    let mut first_of_kind = std::collections::BTreeSet::new();
    // Per kind: the render to compare with (an instrument at its defaults, an effect's source
    // without it) and the presets heard.
    let mut references: BTreeMap<String, Vec<(String, Vec<f32>)>> = BTreeMap::new();
    let mut heard: BTreeMap<String, Vec<(String, Vec<f32>, Render)>> = BTreeMap::new();
    // Per kind: the device at its defaults, which every preset must change (#191).
    let mut defaults: BTreeMap<String, Render> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut visited = Vec::new();
    for f in factory::all() {
        let kind = f.preset.device.as_str();
        let role = role(kind);
        let s = if role == engine::device::DeviceRole::Effect {
            effect_session(Setup::Preset(kind, f.id))
        } else {
            note_session(kind, Some(f.id), true)
        };
        let with_preset = if first_of_kind.insert(kind.to_owned()) {
            parity(s.scenario())
        } else {
            render_native(s.scenario())
        };
        checked(&with_preset, f.id);
        // The preset reached the engine: its sound is not the device's at its defaults, whatever
        // the device (a MIDI effect included, whose distances below are not judged).
        let plain = defaults.entry(kind.to_owned()).or_insert_with(|| {
            render_native(
                if role == engine::device::DeviceRole::Effect {
                    effect_session(Setup::Defaults(kind))
                } else {
                    note_session(kind, None, true)
                }
                .scenario(),
            )
        });
        let (difference, at) = with_preset.max_difference(plain);
        if difference <= 1e-2 {
            failures.push(format!(
                "{} plays like the device at its defaults (largest difference {difference:.4} at frame {at})",
                f.id
            ));
        }
        let refs = references.entry(kind.to_owned()).or_insert_with(|| {
            if role == engine::device::DeviceRole::Instrument {
                let defaults = note_session(kind, None, true);
                vec![(
                    "the defaults".to_owned(),
                    spectrogram(&render_native(defaults.scenario())),
                )]
            } else if role == engine::device::DeviceRole::MidiEffect {
                let source = note_session(kind, None, false);
                vec![(
                    "the source".to_owned(),
                    spectrogram(&render_native(source.scenario())),
                )]
            } else {
                let source = effect_session(Setup::Source);
                vec![(
                    "the source".to_owned(),
                    spectrogram(&render_native(source.scenario())),
                )]
            }
        });
        let grid = spectrogram(&with_preset);
        let mut line = format!("{}: peak {:.3}", f.id, with_preset.peak());
        let siblings = heard.entry(kind.to_owned()).or_default();
        let others = refs
            .iter()
            .map(|(name, g)| (name.as_str(), g))
            .chain(siblings.iter().map(|(id, g, _)| (id.as_str(), g)));
        for (name, other) in others {
            let d = distance(&grid, other);
            let _ = write!(line, ", {d:.2} dB from {name}");
            let known = KNOWN_ALIKE.contains(&(f.id, name));
            if known {
                visited.push((f.id, name.to_owned()));
            }
            // A MIDI effect moves pitches and timing, which octave-band levels do not tell apart
            // (a major and a minor triad hold the same energy per octave): its distances are
            // printed, not judged.
            let wrong = if known {
                d >= DISTINCT_DB + STALE_MARGIN_DB
            } else {
                d < DISTINCT_DB
            };
            if role != engine::device::DeviceRole::MidiEffect && wrong {
                failures.push(if known {
                    format!(
                        "{} no longer sounds like {name} ({d:.2} dB): drop it from KNOWN_ALIKE",
                        f.id
                    )
                } else {
                    format!("{} sounds like {name} ({d:.2} dB)", f.id)
                });
            }
        }
        println!("{line}");
        let slug = f.id.replace("faulk.", "").replace('/', "-");
        with_preset.write_wav(&format!("preset-{slug}"));
        siblings.push((f.id.to_owned(), grid, with_preset));
    }
    // Every listed exception names a pair the loop compared: a renamed preset leaves no dead entry.
    for (id, name) in KNOWN_ALIKE {
        if !visited.iter().any(|(i, n)| i == id && n == name) {
            failures.push(format!(
                "KNOWN_ALIKE lists {id} against {name}, which no longer exists"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "presets against what they must not sound like (alike under {DISTINCT_DB} dB):\n{}",
        failures.join("\n")
    );
    // Every device of the registry was heard.
    for builtin in REGISTRY {
        assert!(
            first_of_kind.contains(builtin.descriptor.kind),
            "{} has no factory preset rendered",
            builtin.descriptor.kind
        );
    }

    // #156: Contour's presets on the mix, with the source, and their band table.
    let dir = qa_dir("156");
    let source = render_native(effect_session(Setup::Source).scenario());
    std::fs::write(dir.join("contour-source.wav"), source.wav()).unwrap();
    let window = LOAD_AT + 0.1..LOAD_AT + AFTER as f32 * STEP;
    let base = band_levels(&source, window.clone(), &BANDS);
    let mut table = String::from(
        "Contour presets on drums, a bass and a voice-like lead, octave bands over the bar after the load, \
         in dB relative to the source without an effect.\n\n| Band (Hz) |",
    );
    let names: Vec<_> = heard["faulk.contour"]
        .iter()
        .map(|(id, _, _)| id.clone())
        .collect();
    for id in &names {
        let _ = write!(table, " {id} |");
    }
    table.push_str(" source level (dB) |\n|---|");
    table.push_str(&"---:|".repeat(names.len() + 1));
    table.push('\n');
    let levels: Vec<Vec<f32>> = heard["faulk.contour"]
        .iter()
        .map(|(id, _, render)| {
            let slug = id.replace("faulk.contour/", "");
            std::fs::write(dir.join(format!("contour-{slug}.wav")), render.wav()).unwrap();
            band_levels(render, window.clone(), &BANDS)
        })
        .collect();
    for (b, edge) in BANDS.windows(2).enumerate() {
        let _ = write!(table, "| {:.0}-{:.0} |", edge[0], edge[1]);
        for l in &levels {
            let _ = write!(table, " {:+.1} |", l[b] - base[b]);
        }
        let _ = writeln!(table, " {:.1} |", base[b]);
    }
    std::fs::write(dir.join("contour-presets-bands.md"), &table).unwrap();
    println!("{table}");

    // For the ear: 8 s of the same material, without and with each preset loaded from the start
    // of its second step (the comparison above stays on the short renders).
    let listen = |setup: Setup, name: &str| {
        let render = render_native(effect_session_for(setup, 64).scenario());
        std::fs::write(dir.join(format!("listen-{name}.wav")), render.wav()).unwrap();
    };
    listen(Setup::Source, "source");
    for id in &names {
        listen(
            Setup::Preset("faulk.contour", id),
            &id.replace("faulk.contour/", ""),
        );
    }

    // What each one is for, as the founder asked (#156): Low Cut Tidy a steep low cut and
    // nothing above it, Vocal Presence the lows untouched, a presence lift and some air.
    let change = |id: &str, band: usize| {
        let i = names.iter().position(|n| n == id).unwrap();
        levels[i][band] - base[band]
    };
    let (lct, vp) = ("faulk.contour/low-cut-tidy", "faulk.contour/vocal-presence");
    // Bands: 0 is 20-40 Hz, 1 40-80, 2 80-160, 3 160-315 ... 7 2.5-5 kHz, 9 10-20 kHz.
    assert!(
        change(lct, 1) < -20.0,
        "{lct}: 40-80 Hz {:+.1} dB",
        change(lct, 1)
    );
    for band in 3..BANDS.len() - 1 {
        assert!(
            change(lct, band).abs() < 0.5,
            "{lct}: band {band} {:+.1} dB, flat expected",
            change(lct, band)
        );
    }
    for band in 0..5 {
        assert!(
            change(vp, band).abs() < 0.5,
            "{vp}: band {band} {:+.1} dB, untouched expected",
            change(vp, band)
        );
    }
    assert!(
        (3.0..5.5).contains(&change(vp, 7)),
        "{vp}: 2.5-5 kHz {:+.1} dB",
        change(vp, 7)
    );
    assert!(
        change(vp, 9) > 1.5,
        "{vp}: 10-20 kHz {:+.1} dB",
        change(vp, 9)
    );
}
