//! #191: every factory preset loaded through `preset.load` into its device, played and
//! rendered: no NaN, no denormal, no unexpected silence, and a sound that differs from the
//! device at its defaults (the preset does something in the engine, not only in the project).
//! The load happens while a note already sounds, as a musician stepping through presets does.
//! The first preset of each device is also rendered through WASM (parity), and every render is
//! written to `.qa/renders/preset-<kind>-<slug>.wav` for the ear.

use commands::preset::PresetRef;
use commands::testing::{context, fixture};
use commands::{Command, Output, device, live, preset, track};
use engine::devices::REGISTRY;
use project::preset::{PresetLocation, factory};
use project::{DeviceId, DeviceRole};
use test_support::{Render, Session, parity, render_native};

/// The device that gets the preset, and the notes that play it: the synth plays a low saw
/// chord through an effect; an instrument plays itself (a drum kit, each sound it names).
fn session(kind: &str, preset: Option<&str>) -> (Session, Vec<u8>) {
    let f = fixture();
    let mut s = Session::from_state(f.state.clone(), context());
    let descriptor = REGISTRY
        .iter()
        .find(|b| b.descriptor.kind == kind)
        .unwrap()
        .descriptor;
    let target: DeviceId = match descriptor.role {
        engine::device::DeviceRole::Instrument if kind == "faulk.osc_synth" => f.device.clone(),
        engine::device::DeviceRole::Instrument => {
            s.run(Command::Track(track::Command::SetInstrument {
                track: f.track.clone(),
                instrument: kind.into(),
            }));
            let state = s.state();
            state.project.tracks[&f.track]
                .devices
                .iter()
                .find(|(_, d)| d.role == DeviceRole::Instrument)
                .map(|(id, _)| id.clone())
                .unwrap()
        }
        _ => {
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
        }
    };
    let notes: Vec<u8> = if descriptor.named_keys.is_empty() {
        vec![45, 52, 57]
    } else {
        descriptor.named_keys.iter().map(|k| k.key).collect()
    };
    for &note in &notes {
        s.run(Command::Live(live::Command::NoteOn {
            track: f.track.clone(),
            note,
            velocity: 100,
        }));
    }
    s.render_secs(0.2);
    // The load lands while the notes sound.
    if let Some(id) = preset {
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
    // A drum kit is hit again after the load, so that its new sounds are heard.
    if !descriptor.named_keys.is_empty() {
        for &note in &notes {
            s.run(Command::Live(live::Command::NoteOn {
                track: f.track.clone(),
                note,
                velocity: 100,
            }));
            s.render_secs(0.15);
        }
    } else {
        s.render_secs(1.0);
    }
    for &note in &notes {
        s.run(Command::Live(live::Command::NoteOff {
            track: f.track.clone(),
            note,
        }));
    }
    s.render_secs(1.5);
    (s, notes)
}

fn checked(render: &Render, id: &str) {
    render.assert_finite();
    render.assert_no_denormals();
    // Heard after the load (at 0.2 s), not only before it with the defaults.
    render.assert_audible(0.3..1.0, 1e-3);
    assert!(render.peak() <= 1.0, "{id}: peak {}", render.peak());
}

#[test]
fn every_factory_preset_loads_while_playing_and_sounds_clean_and_different_from_the_defaults() {
    let mut first_of_kind = std::collections::BTreeSet::new();
    for f in factory::all() {
        let kind = f.preset.device.as_str();
        let (s, _) = session(kind, Some(f.id));
        let with_preset = if first_of_kind.insert(kind.to_owned()) {
            parity(s.scenario())
        } else {
            render_native(s.scenario())
        };
        checked(&with_preset, f.id);
        let (plain, _) = session(kind, None);
        let plain = render_native(plain.scenario());
        let (difference, at) = with_preset.max_difference(&plain);
        println!(
            "{}: peak {:.3}, rms {:.4}, differs from the defaults by {difference:.3} (frame {at})",
            f.id,
            with_preset.peak(),
            with_preset.rms()
        );
        assert!(
            difference > 1e-2,
            "{}: sounds like the device at its defaults",
            f.id
        );
        let slug = f.id.replace("faulk.", "").replace('/', "-");
        with_preset.write_wav(&format!("preset-{slug}"));
    }
    // Every device of the registry was heard.
    for builtin in REGISTRY {
        assert!(
            first_of_kind.contains(builtin.descriptor.kind),
            "{} has no factory preset rendered",
            builtin.descriptor.kind
        );
    }
}
