//! #219: MIDI effects in a track's chain. A chain is its MIDI effects, its instrument, then its
//! audio effects; `index` counts the devices of the same kind; a MIDI effect is refused on the
//! master and on an audio track; remove, restore, move and restore-order keep the order, and
//! each undoes exactly. The engine side is in `device.rs` (`World`).

use ::project::{DeviceId, DeviceRole, TrackId};
use commands::testing::{check, check_refusal, context};
use commands::{Command, CommandError, Output, State, device, history, run, track};

use crate::device_examples::{EFFECT, MIDI_EFFECT, MidiChain, d, midi_chain, t};

/// A context whose ids meet neither the fixture's nor the examples'.
fn fresh() -> commands::Context {
    commands::Context::new(
        commands::Author::User,
        ::project::IdGenerator::new(1_673_000_000_000, 0x00FF_0000_0000_0000_0000),
    )
}

fn ids(state: &State, track: &TrackId) -> Vec<DeviceId> {
    state.project.tracks[track]
        .devices_in_order()
        .into_iter()
        .map(|(id, _)| id.clone())
        .collect()
}

fn roles(state: &State, track: &TrackId) -> Vec<DeviceRole> {
    state.project.tracks[track]
        .devices_in_order()
        .into_iter()
        .map(|(_, d)| d.role)
        .collect()
}

fn insert(track: Option<&TrackId>, kind: &str, index: u32) -> Command {
    d(device::Command::Insert {
        track: track.cloned(),
        master: track.is_none(),
        kind: kind.into(),
        index,
    })
}

fn mv(device: &DeviceId, index: u32) -> Command {
    d(device::Command::Move {
        device: device.clone(),
        index,
    })
}

fn refused(command: &Command, state: &State) -> CommandError {
    check_refusal(command, state, &context()).unwrap()
}

fn inserted(out: commands::Outcome) -> DeviceId {
    let Output::Device(device::Output::Insert { device }) = out.output else {
        panic!("{:?}", out.output)
    };
    device
}

#[test]
fn a_midi_effect_goes_before_the_instrument_and_its_index_counts_the_midi_effects() {
    let MidiChain { chain: c, m1, m2 } = midi_chain();
    assert_eq!(
        ids(&c.state, &c.track),
        [
            m1.clone(),
            m2.clone(),
            c.synth.clone(),
            c.a.clone(),
            c.b.clone()
        ]
    );
    assert_eq!(
        roles(&c.state, &c.track),
        [
            DeviceRole::MidiEffect,
            DeviceRole::MidiEffect,
            DeviceRole::Instrument,
            DeviceRole::Effect,
            DeviceRole::Effect
        ]
    );
    let mut ctx = fresh();
    // Index 0: the start of the chain; 1: between the two; past the end: right before the
    // instrument, never after it.
    for (index, at) in [(0u32, 0usize), (1, 1), (2, 2), (u32::MAX, 2)] {
        let mut s = c.state.clone();
        let new = inserted(
            run(
                &insert(Some(&c.track), MIDI_EFFECT, index),
                &mut s,
                &mut ctx,
            )
            .unwrap(),
        );
        s.project.validate().unwrap();
        assert_eq!(ids(&s, &c.track)[at], new, "index {index}");
        assert_eq!(ids(&s, &c.track)[3], c.synth, "index {index}");
    }
    // An audio effect's index still counts the audio effects: 0 is right after the instrument.
    let mut s = c.state.clone();
    let new = inserted(run(&insert(Some(&c.track), EFFECT, 0), &mut s, &mut ctx).unwrap());
    assert_eq!(ids(&s, &c.track)[3], new);
    assert_eq!(ids(&s, &c.track)[2], c.synth);
}

#[test]
fn a_midi_effect_goes_into_a_track_without_an_instrument_and_into_an_empty_chain() {
    let MidiChain { chain: c, .. } = midi_chain();
    let mut s = c.state.clone();
    s.project.tracks.get_mut(&c.track).unwrap().devices.clear();
    let mut ctx = fresh();
    let first = inserted(run(&insert(Some(&c.track), MIDI_EFFECT, 0), &mut s, &mut ctx).unwrap());
    let fx = inserted(run(&insert(Some(&c.track), EFFECT, 0), &mut s, &mut ctx).unwrap());
    let second = inserted(run(&insert(Some(&c.track), MIDI_EFFECT, 9), &mut s, &mut ctx).unwrap());
    s.project.validate().unwrap();
    assert_eq!(ids(&s, &c.track), [first, second, fx]);
}

#[test]
fn a_midi_effect_is_refused_on_the_master_and_on_an_audio_track() {
    let MidiChain { chain: c, .. } = midi_chain();
    let on_master = refused(&insert(None, MIDI_EFFECT, 0), &c.state);
    assert_eq!(on_master.code(), "invalid");
    assert!(on_master.to_string().contains("master"), "{on_master}");
    let on_audio = refused(&insert(Some(&c.audio_track), MIDI_EFFECT, 0), &c.state);
    assert_eq!(on_audio.code(), "invalid");
    assert!(on_audio.to_string().contains("audio track"), "{on_audio}");
}

#[test]
fn a_midi_effect_moves_among_the_midi_effects_and_never_past_the_instrument() {
    let MidiChain { chain: c, m1, m2 } = midi_chain();
    let mut s = c.state.clone();
    let mut ctx = context();
    let out = run(&mv(&m2, 0), &mut s, &mut ctx).unwrap();
    let Output::Device(device::Output::Move { previous_index }) = out.output else {
        panic!()
    };
    assert_eq!(previous_index, 1);
    assert_eq!(
        ids(&s, &c.track)[..3],
        [m2.clone(), m1.clone(), c.synth.clone()]
    );
    // Past the end: last of the MIDI effects, still before the instrument.
    let mut s = c.state.clone();
    run(&mv(&m1, 99), &mut s, &mut ctx).unwrap();
    s.project.validate().unwrap();
    assert_eq!(
        ids(&s, &c.track)[..3],
        [m2.clone(), m1.clone(), c.synth.clone()]
    );
    // An audio effect moved to 0 stays after the instrument.
    let mut s = c.state.clone();
    run(&mv(&c.b, 0), &mut s, &mut ctx).unwrap();
    assert_eq!(
        ids(&s, &c.track)[2..],
        [c.synth.clone(), c.b.clone(), c.a.clone()]
    );
    // The same place changes nothing.
    let s = c.state.clone();
    let mut same = s.clone();
    let out = run(&mv(&m1, 0), &mut same, &mut ctx).unwrap();
    assert!(matches!(out.undo, commands::Undo::None));
    assert_eq!(same.project, s.project);
}

#[test]
fn restore_order_refuses_a_key_that_crosses_the_instrument_either_way() {
    let MidiChain { chain: c, m1, .. } = midi_chain();
    let synth_key = c.state.project.tracks[&c.track].devices[&c.synth]
        .order
        .clone();
    let after_synth = ::project::OrderKey::between(
        Some(&synth_key),
        Some(&c.state.project.tracks[&c.track].devices[&c.a].order),
    )
    .unwrap();
    let midi_late = d(device::Command::RestoreOrder {
        device: m1.clone(),
        order: after_synth,
    });
    assert_eq!(refused(&midi_late, &c.state).code(), "invalid");
    let before_synth = ::project::OrderKey::between(
        None,
        Some(&c.state.project.tracks[&c.track].devices[&m1].order),
    )
    .unwrap();
    let effect_early = d(device::Command::RestoreOrder {
        device: c.a.clone(),
        order: before_synth,
    });
    assert_eq!(refused(&effect_early, &c.state).code(), "invalid");
}

#[test]
fn a_removed_midi_effect_comes_back_in_its_place_and_a_misplaced_snapshot_is_refused() {
    let MidiChain { chain: c, m1, m2 } = midi_chain();
    let mut s = c.state.clone();
    let mut ctx = context();
    let out = run(
        &d(device::Command::Remove { device: m1.clone() }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(ids(&s, &c.track)[0], m2);
    let commands::Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    run(&inverse, &mut s, &mut ctx).unwrap();
    assert_eq!(s.project, c.state.project);
    // A MIDI-effect snapshot restored into an audio track, or into the master, is refused.
    let mut gone = c.state.clone();
    let out = run(
        &d(device::Command::Remove { device: m1.clone() }),
        &mut gone,
        &mut ctx,
    )
    .unwrap();
    let commands::Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    let Command::Device(device::Command::Restore {
        device, snapshot, ..
    }) = *inverse
    else {
        panic!()
    };
    for (track, master) in [(Some(c.audio_track.clone()), false), (None, true)] {
        let elsewhere = d(device::Command::Restore {
            track,
            master,
            device: device.clone(),
            snapshot: snapshot.clone(),
            lanes: Default::default(),
        });
        assert_eq!(refused(&elsewhere, &gone).code(), "invalid");
    }
}

#[test]
fn every_midi_effect_command_passes_the_contract_check() {
    let MidiChain { chain: c, m1, m2 } = midi_chain();
    let ctx = context();
    for command in [
        insert(Some(&c.track), MIDI_EFFECT, 0),
        insert(Some(&c.track), MIDI_EFFECT, u32::MAX),
        mv(&m2, 0),
        d(device::Command::Remove { device: m1.clone() }),
        d(device::Command::SetBypass {
            device: m1.clone(),
            bypass: true,
        }),
        d(device::Command::SetParam {
            device: m1.clone(),
            param: "semitones".into(),
            value: -5.0,
        }),
    ] {
        check(&command, &c.state, &ctx).unwrap();
    }
}

#[test]
fn the_descriptions_for_agents_say_where_a_midi_effect_goes() {
    let schema = serde_json::to_string(&commands::schema()).unwrap();
    for needle in [
        "goes before the instrument, `index` counting the MIDI effects",
        "a MIDI effect among the MIDI effects",
    ] {
        assert!(schema.contains(needle), "{needle}");
    }
}

#[test]
fn undo_and_redo_of_a_midi_effect_insert_and_move() {
    let MidiChain { chain: c, m1, m2 } = midi_chain();
    let mut s = c.state.clone();
    let mut ctx = context();
    run(&insert(Some(&c.track), MIDI_EFFECT, 1), &mut s, &mut ctx).unwrap();
    run(&mv(&m2, 0), &mut s, &mut ctx).unwrap();
    let after = s.project.clone();
    let undo = Command::History(history::Command::Undo {});
    let redo = Command::History(history::Command::Redo {});
    run(&undo, &mut s, &mut ctx).unwrap();
    run(&undo, &mut s, &mut ctx).unwrap();
    assert_eq!(s.project, c.state.project);
    run(&redo, &mut s, &mut ctx).unwrap();
    run(&redo, &mut s, &mut ctx).unwrap();
    assert_eq!(s.project, after);
    let _ = m1;
}

#[test]
fn the_instrument_swap_keeps_the_midi_effects_first() {
    let MidiChain { chain: c, m1, m2 } = midi_chain();
    let mut s = c.state.clone();
    let mut ctx = context();
    run(
        &t(track::Command::SetInstrument {
            track: c.track.clone(),
            instrument: "faulk.drums".into(),
        }),
        &mut s,
        &mut ctx,
    )
    .unwrap();
    s.project.validate().unwrap();
    assert_eq!(ids(&s, &c.track)[..2], [m1, m2]);
    assert_eq!(roles(&s, &c.track)[2], DeviceRole::Instrument);
}

#[test]
fn a_snapshot_whose_role_is_not_its_kinds_is_refused() {
    // A MIDI effect restored as an audio effect, or an audio effect as a MIDI effect, would sit
    // in the wrong section of the chain while the engine runs it as what it is.
    let MidiChain { chain: c, m1, .. } = midi_chain();
    for (device, role) in [(&m1, DeviceRole::Effect), (&c.a, DeviceRole::MidiEffect)] {
        let mut gone = c.state.clone();
        let out = run(
            &d(device::Command::Remove {
                device: device.clone(),
            }),
            &mut gone,
            &mut fresh(),
        )
        .unwrap();
        let commands::Undo::Inverse(inverse) = out.undo else {
            panic!()
        };
        let Command::Device(device::Command::Restore {
            track,
            master,
            device,
            mut snapshot,
            lanes,
        }) = *inverse
        else {
            panic!()
        };
        snapshot.role = role;
        let wrong = d(device::Command::Restore {
            track,
            master,
            device,
            snapshot,
            lanes,
        });
        let error = refused(&wrong, &gone);
        assert_eq!(error.code(), "invalid");
        assert!(error.to_string().contains("not a"), "{error}");
    }
}

#[test]
fn restore_order_refuses_the_instruments_own_key() {
    // Equal keys leave the order to the identifiers' sort: a MIDI effect or an audio effect
    // never takes the instrument's key, whatever the identifiers.
    let MidiChain { chain: c, m1, .. } = midi_chain();
    let synth_key = c.state.project.tracks[&c.track].devices[&c.synth]
        .order
        .clone();
    for device in [&m1, &c.a] {
        let same = d(device::Command::RestoreOrder {
            device: device.clone(),
            order: synth_key.clone(),
        });
        assert_eq!(refused(&same, &c.state).code(), "invalid");
    }
}

#[test]
fn a_track_holds_at_most_sixteen_midi_effects() {
    let MidiChain { chain: c, .. } = midi_chain();
    let mut s = c.state.clone();
    let mut ctx = fresh();
    for _ in 2..::project::limits::MAX_MIDI_EFFECTS_PER_TRACK {
        run(&insert(Some(&c.track), MIDI_EFFECT, 0), &mut s, &mut ctx).unwrap();
    }
    s.project.validate().unwrap();
    let error = refused(&insert(Some(&c.track), MIDI_EFFECT, 0), &s);
    assert_eq!(error.code(), "limit");
    // Audio effects still go in.
    run(&insert(Some(&c.track), EFFECT, 0), &mut s, &mut ctx).unwrap();
}

/// #690, #691: every parameter of each MIDI effect of the registry (Stack, Scatter) is set by
/// `device.set_param` through the contract check (schema, description, undo), at both ends of
/// its range and beyond (clamped), and its value reads back as the descriptor sanitizes it.
#[test]
fn every_parameter_of_each_midi_effect_is_set_and_undone_through_device_set_param() {
    let kinds: Vec<&str> = engine::devices::REGISTRY
        .iter()
        .filter(|b| b.descriptor.role == engine::device::DeviceRole::MidiEffect)
        .map(|b| b.descriptor.kind)
        .collect();
    assert!(
        kinds.contains(&"faulk.stack") && kinds.contains(&"faulk.scatter"),
        "{kinds:?}"
    );
    for builtin in engine::devices::REGISTRY
        .iter()
        .filter(|b| b.descriptor.role == engine::device::DeviceRole::MidiEffect)
    {
        let descriptor = builtin.descriptor;
        let MidiChain { chain: c, .. } = midi_chain();
        let mut s = c.state.clone();
        let fx = inserted(
            run(
                &insert(Some(&c.track), descriptor.kind, 0),
                &mut s,
                &mut fresh(),
            )
            .unwrap(),
        );
        assert_eq!(
            s.project.tracks[&c.track].devices[&fx].role,
            DeviceRole::MidiEffect
        );
        assert_eq!(
            s.project.tracks[&c.track].devices[&fx].params.len(),
            descriptor.params.len(),
            "{}",
            descriptor.kind
        );
        let ctx = context();
        for p in descriptor.params {
            for value in [
                p.min,
                p.max,
                p.max + 100.0,
                p.min - 100.0,
                (p.min + p.max) / 2.0,
            ] {
                let command = d(device::Command::SetParam {
                    device: fx.clone(),
                    param: p.key.into(),
                    value,
                });
                // A value that lands on the one stored changes nothing (no inverse, no event).
                if p.sanitize_f64(value) == Some(p.default) {
                    continue;
                }
                check(&command, &s, &ctx)
                    .unwrap_or_else(|e| panic!("{} {} = {value}: {e}", descriptor.kind, p.key));
                let mut after = s.clone();
                run(&command, &mut after, &mut fresh()).unwrap();
                let stored = after.project.tracks[&c.track].devices[&fx].params[p.key];
                assert_eq!(
                    Some(stored),
                    p.sanitize_f64(value),
                    "{} {} = {value}",
                    descriptor.kind,
                    p.key
                );
            }
        }
    }
    // The agents read every one of them in the schema bundle's descriptors.
    let schema = serde_json::to_string(&commands::schema()).unwrap();
    assert!(schema.contains("device.set_param"));
}
