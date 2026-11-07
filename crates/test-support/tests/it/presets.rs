//! #191, #156: every factory preset loaded through `preset.load` into its device, played and
//! rendered: no NaN, no denormal, no unexpected silence, and a sound that is audibly its own: its
//! spectrogram stands apart from the device at its defaults, from the source without the device
//! (an effect), and from every other preset of the same device. The load happens while the
//! sound plays, as a musician stepping through presets does.
//!
//! An instrument plays itself (the synth a low chord, a drum kit each sound it names). An
//! effect sits on the master over a mix with something in every band: a drum kit playing a
//! pattern (kick, snare, hats) and the synth holding a five-note chord.
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

/// Plays `steps` sixteenths of the drum pattern from `from`, rendering each.
fn drum_steps(s: &mut Session, drums: &TrackId, from: u32, steps: u32) {
    for step in from..from + steps {
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

/// An effect on the master over drums and a held chord.
fn effect_session(setup: Setup) -> Session {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let Output::Track(track::Output::Create { track: drums, .. }) =
        s.exec(Command::Track(track::Command::Create {
            kind: TrackKind::Midi,
            instrument: Some("faulk.drums".into()),
            name: None,
            color: None,
        }))
    else {
        panic!("a drum track")
    };
    let kind = match setup {
        Setup::Source => None,
        Setup::Preset(kind, _) => Some(kind),
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
    let chord = [45, 52, 57, 61, 64];
    for &n in &chord {
        note_on(&mut s, &f.track, n, 60);
    }
    drum_steps(&mut s, &drums, 0, BEFORE);
    if let (Setup::Preset(_, id), Some(target)) = (setup, &target) {
        load(&mut s, target, id);
    }
    drum_steps(&mut s, &drums, BEFORE, AFTER);
    for &n in &chord {
        note_off(&mut s, &f.track, n);
    }
    s.render_secs(TAIL);
    s
}

/// An instrument playing itself: the synth a low chord, a drum kit each sound it names.
fn instrument_session(kind: &str, preset: Option<&str>) -> Session {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let descriptor = REGISTRY
        .iter()
        .find(|b| b.descriptor.kind == kind)
        .unwrap()
        .descriptor;
    let target: DeviceId = if kind == "faulk.osc_synth" {
        f.device.clone()
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
            .unwrap()
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
    if let Some(id) = preset {
        load(&mut s, &target, id);
    }
    // A drum kit is hit again after the load, so that its new sounds are heard.
    if !descriptor.named_keys.is_empty() {
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

fn is_instrument(kind: &str) -> bool {
    REGISTRY
        .iter()
        .find(|b| b.descriptor.kind == kind)
        .is_some_and(|b| b.descriptor.role == engine::device::DeviceRole::Instrument)
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

/// The octave bands a listener compares presets in: from 80 Hz (what small speakers and
/// headphones at a low level hardly give back below it does not tell two presets apart).
const HEARD_BANDS: &[f32] = BANDS.split_at(2).1;

/// A spectrogram from just after the load to the end of the tail: the heard octave bands of
/// slices of 0.1 s (time effects, an echo or a reverb, differ in the tail as much as in the
/// spectrum).
fn spectrogram(render: &Render) -> Vec<f32> {
    let end = LOAD_AT + AFTER as f32 * STEP + TAIL - 0.1;
    let mut cells = Vec::new();
    let mut t = LOAD_AT + 0.1;
    while t < end {
        cells.extend(band_levels(render, t..t + 0.1, HEARD_BANDS));
        t += 0.1;
    }
    cells
}

/// How far apart two renders sound: the mean absolute level difference, in dB, over the cells
/// of the spectrogram that either render makes clearly heard (within 40 dB of the loudest cell
/// of both). A cell is read no lower than 50 dB under the loudest: silence against a quiet
/// sound counts as what is heard, not as hundreds of dB.
fn distance(a: &[f32], b: &[f32]) -> f32 {
    let loudest = a.iter().chain(b).fold(f32::MIN, |m, v| m.max(*v));
    let floor = loudest - 50.0;
    let (sum, count) = a
        .iter()
        .zip(b)
        .filter(|(x, y)| x.max(**y) > loudest - 40.0)
        .fold((0.0, 0), |(s, c), (x, y)| {
            (s + (x.max(floor) - y.max(floor)).abs(), c + 1)
        });
    sum / count.max(1) as f32
}

/// Two renders closer than this, in mean dB over the spectrogram, sound alike. Calibrated on
/// #156: Contour's first two presets, which the founder heard as the same sound, measured
/// 2.08 dB apart and 1.6 dB from the source.
const DISTINCT_DB: f32 = 3.0;

/// Presets known to sound like what they are compared with, measured when this test came
/// (#156) and left to their own devices' owners: each pair must still be alike, so that the
/// list only shrinks.
const KNOWN_ALIKE: &[(&str, &str)] = &[
    // A gentle bus compressor: 0.92 dB from the source.
    ("faulk.compressor/gentle-glue", "the source"),
    // A short room: 2.16 dB from the source.
    ("faulk.reverb/small-room", "the source"),
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
    let mut failures = Vec::new();
    for f in factory::all() {
        let kind = f.preset.device.as_str();
        let instrument = is_instrument(kind);
        let s = if instrument {
            instrument_session(kind, Some(f.id))
        } else {
            effect_session(Setup::Preset(kind, f.id))
        };
        let with_preset = if first_of_kind.insert(kind.to_owned()) {
            parity(s.scenario())
        } else {
            render_native(s.scenario())
        };
        checked(&with_preset, f.id);
        let refs = references.entry(kind.to_owned()).or_insert_with(|| {
            if instrument {
                let defaults = instrument_session(kind, None);
                vec![(
                    "the defaults".to_owned(),
                    spectrogram(&render_native(defaults.scenario())),
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
            if (d < DISTINCT_DB) != known {
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
        "Contour presets on drums and a held chord, octave bands over the bar after the load, \
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
}
