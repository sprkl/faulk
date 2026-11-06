//! DV-8: the device commands. The index of a command becomes an order key, the instrument
//! stays first, a removed device comes back with its settings, a parameter is checked
//! against its descriptor, `track.set_instrument` swaps the instrument and undoes exactly,
//! and the engine messages of every chain change leave the engine as a snapshot would.
//!
//! The registry holds one effect (Echo): the tests also run with the engine's test effect
//! (`faulk.test_gain`, feature `testing`, enabled by this crate's dev-dependency on itself).

use ::project::{
    AutomationLane, AutomationPoint, AutomationTarget, Device, DeviceId, DeviceRole, LaneId,
    OrderKey, Shape, Tick, TrackId,
};
use commands::engine_msgs::{Synced, resync, snapshot};
use commands::testing::{check, check_refusal, context};
use commands::{
    Command, CommandError, Context, ControlMsg, Output, ProjectEvent, State, Undo, device, history,
    run, track,
};
use std::collections::BTreeMap;

use crate::device_examples;
use device_examples::{
    EFFECT, MIDI_EFFECT, chain, d, insert, insert_master, midi_chain, own_context, t,
};

fn refused(command: &Command, state: &State) -> CommandError {
    check_refusal(command, state, &context()).unwrap()
}

fn code(command: &Command, state: &State) -> &'static str {
    refused(command, state).code()
}

/// The chain of a track, in order.
fn ids(state: &State, track: &TrackId) -> Vec<DeviceId> {
    state.project.tracks[track]
        .devices_in_order()
        .into_iter()
        .map(|(id, _)| id.clone())
        .collect()
}

fn undo() -> Command {
    Command::History(history::Command::Undo {})
}

fn redo() -> Command {
    Command::History(history::Command::Redo {})
}

fn remove(device: &DeviceId) -> Command {
    d(device::Command::Remove {
        device: device.clone(),
    })
}

fn mv(device: &DeviceId, index: u32) -> Command {
    d(device::Command::Move {
        device: device.clone(),
        index,
    })
}

fn ins(track: &TrackId, index: u32) -> Command {
    d(device::Command::Insert {
        track: Some(track.clone()),
        master: false,
        kind: EFFECT.into(),
        index,
    })
}

fn bypass(device: &DeviceId, bypass: bool) -> Command {
    d(device::Command::SetBypass {
        device: device.clone(),
        bypass,
    })
}

fn param(device: &DeviceId, param: &str, value: f64) -> Command {
    d(device::Command::SetParam {
        device: device.clone(),
        param: param.into(),
        value,
    })
}

fn set_instrument(track: &TrackId, instrument: &str) -> Command {
    t(track::Command::SetInstrument {
        track: track.clone(),
        instrument: instrument.into(),
    })
}
/// Runs a command with ids of its own (apart from the chain's and the fixture's).
/// The events of an outcome without the history's own.
fn events(outcome: &commands::Outcome) -> Vec<ProjectEvent> {
    outcome
        .events
        .iter()
        .filter(|e| !matches!(e, ProjectEvent::HistoryChanged { .. }))
        .cloned()
        .collect()
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Runs a command with ids of its own (apart from the chain's and the fixture's).
fn go(state: &mut State, command: &Command) -> commands::Outcome {
    let mut ctx = Context::new(
        commands::Author::User,
        ::project::IdGenerator::new(
            1_695_000_000_000 + NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            0x00BB_0000_0000_0000_0000,
        ),
    );
    run(command, state, &mut ctx).unwrap()
}

// ---------------------------------------------------------------- insert

#[test]
fn an_index_counts_the_effects_and_the_instrument_stays_first() {
    let c = chain();
    let [synth, a, b] = [c.synth.clone(), c.a.clone(), c.b.clone()];
    assert_eq!(
        ids(&c.state, &c.track),
        [synth.clone(), a.clone(), b.clone()]
    );
    let mut s = c.state.clone();
    let mut ctx = context();
    let new = |s: &mut State, ctx: &mut Context, index| {
        let out = run(&ins(&c.track, index), s, ctx).unwrap();
        s.project.validate().unwrap();
        let Output::Device(device::Output::Insert { device }) = out.output else {
            panic!()
        };
        device
    };
    // Index 0 is right after the instrument, before every other effect.
    let first = new(&mut s, &mut ctx, 0);
    assert_eq!(
        ids(&s, &c.track),
        [synth.clone(), first.clone(), a.clone(), b.clone()]
    );
    // In the middle, then past the end (any index beyond the effects means last).
    let middle = new(&mut s, &mut ctx, 2);
    assert_eq!(
        ids(&s, &c.track),
        [
            synth.clone(),
            first.clone(),
            a.clone(),
            middle.clone(),
            b.clone()
        ]
    );
    let last = new(&mut s, &mut ctx, u32::MAX);
    assert_eq!(ids(&s, &c.track).last(), Some(&last));
    assert_eq!(ids(&s, &c.track).first(), Some(&synth));
}

#[test]
fn an_inserted_device_is_an_enabled_effect_at_its_descriptor_defaults() {
    let c = chain();
    let device = &c.state.project.tracks[&c.track].devices[&c.a];
    assert_eq!(device.role, DeviceRole::Effect);
    assert!(device.enabled);
    assert_eq!(device.state, None);
    assert_eq!(device.params["gain"], 1.0);
    assert_eq!(device.params["mute"], 0.0);
    assert_eq!(device.params["trim"], 1.0);
    assert_eq!(device.params.len(), 3);
}

#[test]
fn an_effect_goes_after_the_clip_player_of_an_audio_track() {
    let c = chain();
    let mut s = c.state.clone();
    go(&mut s, &ins(&c.audio_track, 0));
    s.project.validate().unwrap();
    let chain = ids(&s, &c.audio_track);
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0], c.player);
}

#[test]
fn insert_refuses_what_it_cannot_build() {
    let c = chain();
    let missing = TrackId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(code(&ins(&missing, 0), &c.state), "not_found");
    let kind = |kind: &str| {
        d(device::Command::Insert {
            track: Some(c.track.clone()),
            master: false,
            kind: kind.into(),
            index: 0,
        })
    };
    assert_eq!(code(&kind("faulk.nothing"), &c.state), "not_found");
    // An instrument is replaced, not inserted; the clip player is no kind of the registry.
    let e = refused(&kind("faulk.osc_synth"), &c.state);
    assert!(e.to_string().contains("track.set_instrument"), "{e}");
    assert_eq!(code(&kind("faulk.drums"), &c.state), "invalid");
    assert_eq!(code(&kind("faulk.clip_player"), &c.state), "not_found");
}

#[test]
fn insert_refuses_neighbours_that_share_an_order_key() {
    let c = chain();
    let mut s = c.state.clone();
    // A merge made the two effects equal: no key fits between them.
    let key = s.project.tracks[&c.track].devices[&c.a].order.clone();
    s.project
        .tracks
        .get_mut(&c.track)
        .unwrap()
        .devices
        .get_mut(&c.b)
        .unwrap()
        .order = key;
    let e = refused(&ins(&c.track, 1), &s);
    assert_eq!(e.code(), "invalid");
    // Before and after them there is room.
    go(&mut s, &ins(&c.track, 0));
    go(&mut s, &ins(&c.track, 99));
}

#[test]
fn insert_refuses_at_the_safety_limit_of_devices() {
    let c = chain();
    let mut s = c.state.clone();
    let mut ctx = context();
    let max = project::limits::MAX_DEVICES_PER_TRACK;
    while s.project.tracks[&c.track].devices.len() < max {
        run(&ins(&c.track, u32::MAX), &mut s, &mut ctx).unwrap();
    }
    s.project.validate().unwrap();
    let e = refused(&ins(&c.track, 0), &s);
    assert!(matches!(e, CommandError::Limit { .. }), "{e:?}");
    // A removal makes room again.
    go(&mut s, &remove(&c.a));
    go(&mut s, &ins(&c.track, 0));
}

// ---------------------------------------------------------------- remove and restore

#[test]
fn remove_then_undo_brings_the_device_back_with_its_settings() {
    let c = chain();
    let mut s = c.state.clone();
    go(&mut s, &param(&c.a, "gain", 0.25));
    go(&mut s, &bypass(&c.a, true));
    let before = s.clone();
    let Undo::Inverse(inverse) = go(&mut s, &remove(&c.a)).undo else {
        panic!()
    };
    assert_eq!(ids(&s, &c.track), [c.synth.clone(), c.b.clone()]);
    // The documented inverse carries the device as it was.
    let Command::Device(device::Command::Restore {
        track: Some(track),
        master: false,
        device: id,
        snapshot,
        ..
    }) = *inverse
    else {
        panic!("{inverse:?}")
    };
    assert_eq!((&track, &id), (&c.track, &c.a));
    assert_eq!(snapshot.params["gain"], 0.25);
    assert!(!snapshot.enabled);
    check(&remove(&c.a), &before, &context()).unwrap();
    // Through the history: undo restores the project exactly, redo removes again.
    let mut h = before.clone();
    run(&remove(&c.a), &mut h, &mut context()).unwrap();
    run(&undo(), &mut h, &mut context()).unwrap();
    assert_eq!(h.project, before.project);
    h.project.validate().unwrap();
    run(&redo(), &mut h, &mut context()).unwrap();
    assert_eq!(h.project, s.project);
}

#[test]
fn an_instrument_is_not_removed_and_a_device_takes_its_lanes_with_it() {
    let c = chain();
    assert_eq!(code(&remove(&c.synth), &c.state), "invalid");
    assert_eq!(code(&remove(&c.player), &c.state), "invalid");
    let missing = DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(code(&remove(&missing), &c.state), "not_found");
    let mut s = c.state.clone();
    with_lane(&mut s, &c.track, &c.a);
    let before = s.clone();
    // A device the lane does not target goes, the lane stays.
    go(&mut s, &remove(&c.b));
    assert_eq!(s.project.tracks[&c.track].automation.len(), 1);
    go(&mut s, &undo());
    assert_eq!(s.project, before.project);
    // The device the lane targets goes with it, and undo brings both back.
    let out = go(&mut s, &remove(&c.a));
    assert!(s.project.tracks[&c.track].automation.is_empty());
    assert!(matches!(
        events(&out).as_slice(),
        [
            ProjectEvent::AutomationLaneDeleted { .. },
            ProjectEvent::DeviceChainChanged { .. }
        ]
    ));
    go(&mut s, &undo());
    assert_eq!(s.project, before.project);
    s.project.validate().unwrap();
}

fn with_lane(state: &mut State, track: &TrackId, device: &DeviceId) {
    state
        .project
        .tracks
        .get_mut(track)
        .unwrap()
        .automation
        .insert(
            LaneId::generate(&mut ::project::IdGenerator::new(
                1_680_000_000_000 + NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                7,
            )),
            AutomationLane {
                target: AutomationTarget::Device {
                    device: device.clone(),
                    param: "gain".into(),
                },
                points: BTreeMap::from([(
                    Tick(0),
                    AutomationPoint {
                        value: 0.5,
                        shape: Shape::Linear,
                    },
                )]),
            },
        );
    state.project.validate().unwrap();
}

#[test]
fn restore_refuses_what_is_not_an_effect_the_chain_can_hold() {
    let c = chain();
    let mut s = c.state.clone();
    let Undo::Inverse(inverse) = go(&mut s, &remove(&c.a)).undo else {
        panic!()
    };
    let Command::Device(device::Command::Restore {
        track: Some(track),
        master: false,
        device: id,
        snapshot,
        ..
    }) = *inverse
    else {
        panic!()
    };
    let restore = |track: &TrackId, id: &DeviceId, snapshot: Device| {
        d(device::Command::Restore {
            track: Some(track.clone()),
            master: false,
            device: id.clone(),
            snapshot,
            lanes: Default::default(),
        })
    };
    // The good one applies.
    check(&restore(&track, &id, snapshot.clone()), &s, &context()).unwrap();
    // The identifier is in use (by another device, or by a track).
    assert_eq!(
        code(&restore(&track, &c.b, snapshot.clone()), &s),
        "invalid"
    );
    let as_track = DeviceId(c.track.0.clone());
    assert_eq!(
        code(&restore(&track, &as_track, snapshot.clone()), &s),
        "invalid"
    );
    // An instrument, the clip player, an unknown kind, an unknown parameter.
    let mut instrument = snapshot.clone();
    instrument.role = DeviceRole::Instrument;
    assert_eq!(code(&restore(&track, &id, instrument), &s), "invalid");
    let mut unknown = snapshot.clone();
    unknown.source = ::project::DeviceSource::Builtin {
        id: "faulk.nothing".into(),
        version: 1,
    };
    assert_eq!(code(&restore(&track, &id, unknown), &s), "invalid");
    let mut extra = snapshot.clone();
    extra.params.insert("cutoff".into(), 0.5);
    assert_eq!(code(&restore(&track, &id, extra), &s), "invalid");
    let mut player = snapshot.clone();
    player.source = ::project::DeviceSource::Builtin {
        id: ::project::CLIP_PLAYER.into(),
        version: 1,
    };
    player.params.clear();
    assert_eq!(code(&restore(&track, &id, player), &s), "invalid");
    // A key that puts it before the instrument breaks the chain: refused, nothing changed.
    let mut before_instrument = snapshot.clone();
    before_instrument.order = OrderKey::between(None, Some(&OrderKey::first())).unwrap();
    assert_eq!(
        code(&restore(&track, &id, before_instrument), &s),
        "invalid"
    );
    // A non finite parameter.
    let mut nan = snapshot.clone();
    nan.params.insert("gain".into(), f64::NAN);
    assert_eq!(code(&restore(&track, &id, nan), &s), "invalid");
    // An unknown track.
    let missing = TrackId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(code(&restore(&missing, &id, snapshot), &s), "not_found");
}

// ---------------------------------------------------------------- move

#[test]
fn move_puts_an_effect_at_an_index_among_the_effects() {
    let c = chain();
    let mut s = c.state.clone();
    let out = go(&mut s, &mv(&c.b, 0));
    assert_eq!(
        out.output,
        Output::Device(device::Output::Move { previous_index: 1 })
    );
    assert_eq!(
        ids(&s, &c.track),
        [c.synth.clone(), c.b.clone(), c.a.clone()]
    );
    s.project.validate().unwrap();
    // Past the end means last.
    go(&mut s, &mv(&c.b, 99));
    assert_eq!(
        ids(&s, &c.track),
        [c.synth.clone(), c.a.clone(), c.b.clone()]
    );
    // Already there: nothing to undo, nothing to tell.
    let out = go(&mut s, &mv(&c.b, 1));
    assert_eq!(out.undo, Undo::None);
    assert!(events(&out).is_empty());
}

#[test]
fn undoing_a_move_puts_the_exact_key_back() {
    let c = chain();
    let mut s = c.state.clone();
    run(&mv(&c.b, 0), &mut s, &mut context()).unwrap();
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, c.state.project);
    run(&redo(), &mut s, &mut context()).unwrap();
    assert_eq!(ids(&s, &c.track)[1], c.b);
}

#[test]
fn an_instrument_does_not_move_and_an_effect_does_not_pass_it() {
    let c = chain();
    assert_eq!(code(&mv(&c.synth, 1), &c.state), "invalid");
    assert_eq!(code(&mv(&c.player, 0), &c.state), "invalid");
    let missing = DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(code(&mv(&missing, 0), &c.state), "not_found");
    let restore_order = |device: &DeviceId, order: OrderKey| {
        d(device::Command::RestoreOrder {
            device: device.clone(),
            order,
        })
    };
    let below = OrderKey::between(None, Some(&OrderKey::first())).unwrap();
    assert_eq!(
        code(&restore_order(&c.a, below.clone()), &c.state),
        "invalid"
    );
    assert_eq!(code(&restore_order(&c.synth, below), &c.state), "invalid");
    assert_eq!(
        code(&restore_order(&missing, OrderKey::first()), &c.state),
        "not_found"
    );
    // The same key is a no-op; another valid key applies.
    let key = c.state.project.tracks[&c.track].devices[&c.a].order.clone();
    let out = go(&mut c.state.clone(), &restore_order(&c.a, key));
    assert_eq!(out.undo, Undo::None);
    let after_b = OrderKey::between(
        Some(&c.state.project.tracks[&c.track].devices[&c.b].order),
        None,
    )
    .unwrap();
    check(&restore_order(&c.a, after_b), &c.state, &context()).unwrap();
}

#[test]
fn move_refuses_neighbours_that_share_an_order_key() {
    let c = chain();
    let mut s = c.state.clone();
    let last = insert(&mut s, &mut context(), &c.track, u32::MAX);
    let key = s.project.tracks[&c.track].devices[&c.a].order.clone();
    s.project
        .tracks
        .get_mut(&c.track)
        .unwrap()
        .devices
        .get_mut(&c.b)
        .unwrap()
        .order = key;
    // The last effect would go between the two equal ones: none fits.
    assert_eq!(code(&mv(&last, 1), &s), "invalid");
    // Before them, or after, it fits.
    go(&mut s, &mv(&last, 0));
}

// ---------------------------------------------------------------- bypass

#[test]
fn bypass_sets_and_undoes_and_the_clip_player_refuses() {
    let c = chain();
    let mut s = c.state.clone();
    let out = go(&mut s, &bypass(&c.a, true));
    assert_eq!(
        out.output,
        Output::Device(device::Output::SetBypass { previous: false })
    );
    assert!(!s.project.tracks[&c.track].devices[&c.a].enabled);
    assert_eq!(
        events(&out),
        [ProjectEvent::DeviceBypassChanged {
            device: c.a.clone(),
            bypass: true
        }]
    );
    // Same value: nothing.
    let out = go(&mut s, &bypass(&c.a, true));
    assert_eq!((events(&out).len(), out.undo), (0, Undo::None));
    // The instrument is bypassable; the clip player is not.
    go(&mut s, &bypass(&c.synth, true));
    assert_eq!(code(&bypass(&c.player, true), &c.state), "invalid");
    let missing = DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(code(&bypass(&missing, true), &c.state), "not_found");
    // Undo puts the flag back.
    let mut h = c.state.clone();
    run(&bypass(&c.a, true), &mut h, &mut context()).unwrap();
    run(&undo(), &mut h, &mut context()).unwrap();
    assert_eq!(h.project, c.state.project);
}

// ---------------------------------------------------------------- set_param

#[test]
fn a_parameter_is_checked_against_the_descriptor_clamped_and_rounded_to_its_step() {
    let c = chain();
    let applied = |s: &mut State, device: &DeviceId, p: &str, v: f64| {
        let out = go(s, &param(device, p, v));
        let Output::Device(device::Output::SetParam { value, .. }) = out.output else {
            panic!()
        };
        (value, out)
    };
    let mut s = c.state.clone();
    // Rounded to the step: waveform has step 1, from 0 to 3.
    assert_eq!(applied(&mut s, &c.synth, "waveform", 2.6).0, 3.0);
    assert_eq!(applied(&mut s, &c.synth, "waveform", 0.4).0, 0.0);
    // Clamped to the range.
    assert_eq!(applied(&mut s, &c.synth, "sustain", 5.0).0, 1.0);
    assert_eq!(applied(&mut s, &c.synth, "sustain", -1.0).0, 0.0);
    assert_eq!(applied(&mut s, &c.synth, "attack", 0.0).0, 0.002);
    // A continuous value is kept as given (in f64).
    assert_eq!(applied(&mut s, &c.synth, "attack", 0.1).0, 0.1);
    assert_eq!(
        s.project.tracks[&c.track].devices[&c.synth].params["attack"],
        0.1
    );
    // The event and the stored value are the rounded one.
    let (value, out) = applied(&mut s, &c.synth, "waveform", 1.2);
    assert_eq!(value, 1.0);
    assert_eq!(
        events(&out),
        [ProjectEvent::DeviceParamChanged {
            device: c.synth.clone(),
            param: "waveform".into(),
            value: 1.0
        }]
    );
    // An effect, by its own descriptor.
    assert_eq!(applied(&mut s, &c.a, "gain", 7.0).0, 2.0);
    assert_eq!(applied(&mut s, &c.a, "mute", 0.6).0, 1.0);
}

#[test]
fn set_param_refuses_an_id_the_descriptor_does_not_declare_and_a_value_that_is_not_finite() {
    let c = chain();
    // The synth has no `mute`, the effect has no `attack`, nothing has `cutoff`.
    assert_eq!(code(&param(&c.synth, "mute", 1.0), &c.state), "not_found");
    assert_eq!(code(&param(&c.a, "attack", 0.1), &c.state), "not_found");
    assert_eq!(code(&param(&c.a, "cutoff", 0.1), &c.state), "not_found");
    assert_eq!(code(&param(&c.player, "gain", 0.1), &c.state), "not_found");
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(code(&param(&c.a, "gain", v), &c.state), "invalid");
    }
    let missing = DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(code(&param(&missing, "gain", 1.0), &c.state), "not_found");
}

#[test]
fn a_value_that_rounds_to_the_current_one_changes_nothing() {
    let c = chain();
    let mut s = c.state.clone();
    // The synth's waveform is 1: 1.3 rounds to it.
    let out = go(&mut s, &param(&c.synth, "waveform", 1.3));
    assert_eq!((events(&out).len(), out.undo), (0, Undo::None));
    assert_eq!(s, c.state);
    // Above the range: clamped to the maximum, which is the current value once set.
    go(&mut s, &param(&c.a, "gain", 2.0));
    let out = go(&mut s, &param(&c.a, "gain", 9.0));
    assert_eq!((events(&out).len(), out.undo), (0, Undo::None));
}

#[test]
fn a_module_device_keeps_the_check_against_what_the_project_holds() {
    let c = chain();
    let mut s = c.state.clone();
    let device = s
        .project
        .tracks
        .get_mut(&c.track)
        .unwrap()
        .devices
        .get_mut(&c.a)
        .unwrap();
    device.source = ::project::DeviceSource::Module {
        name: "mine".into(),
        hash: ::project::Hash([1; 32]),
    };
    device.params = BTreeMap::from([("depth".to_owned(), 0.5)]);
    let out = go(&mut s, &param(&c.a, "depth", 9.0));
    assert_eq!(
        out.output,
        Output::Device(device::Output::SetParam {
            previous: 0.5,
            value: 9.0
        })
    );
    assert_eq!(code(&param(&c.a, "other", 1.0), &s), "not_found");
}

// ---------------------------------------------------------------- track.set_instrument

#[test]
fn set_instrument_replaces_the_instrument_in_its_place_and_undoes_exactly() {
    let c = chain();
    let mut s = c.state.clone();
    // The synth's settings, to prove the undo gives them back.
    go(&mut s, &param(&c.synth, "attack", 0.5));
    let before = s.clone();
    let out = go(&mut s, &set_instrument(&c.track, "faulk.drums"));
    let Output::Track(track::Output::SetInstrument {
        device, previous, ..
    }) = out.output.clone()
    else {
        panic!()
    };
    assert_eq!(previous, c.synth);
    s.project.validate().unwrap();
    // The drums come first, new identifier, at their defaults; the effects stay.
    assert_eq!(
        ids(&s, &c.track),
        [device.clone(), c.a.clone(), c.b.clone()]
    );
    assert_ne!(device, c.synth);
    let drums = &s.project.tracks[&c.track].devices[&device];
    assert_eq!(drums.role, DeviceRole::Instrument);
    assert!(matches!(
        &drums.source,
        ::project::DeviceSource::Builtin { id, .. } if id == "faulk.drums"
    ));
    assert_eq!(
        events(&out),
        [ProjectEvent::DeviceChainChanged {
            track: Some(c.track.clone())
        }]
    );
    check(
        &set_instrument(&c.track, "faulk.drums"),
        &before,
        &context(),
    )
    .unwrap();
    // Through the history, then redo.
    let mut h = before.clone();
    run(
        &set_instrument(&c.track, "faulk.drums"),
        &mut h,
        &mut context(),
    )
    .unwrap();
    let swapped = h.clone();
    run(&undo(), &mut h, &mut context()).unwrap();
    assert_eq!(h.project, before.project);
    assert_eq!(
        h.project.tracks[&c.track].devices[&c.synth].params["attack"],
        0.5
    );
    run(&redo(), &mut h, &mut context()).unwrap();
    assert_eq!(h.project, swapped.project);
}

#[test]
fn set_instrument_takes_the_lanes_of_the_old_instrument_and_undo_brings_them_back() {
    let c = chain();
    let mut s = c.state.clone();
    with_lane(&mut s, &c.track, &c.synth);
    with_lane(&mut s, &c.track, &c.a);
    // The lane of an effect stays; only the instrument's goes.
    let before = s.clone();
    let lane_ids: Vec<LaneId> = s.project.tracks[&c.track]
        .automation
        .keys()
        .cloned()
        .collect();
    let mut h = s.clone();
    let out = run(
        &set_instrument(&c.track, "faulk.drums"),
        &mut h,
        &mut context(),
    )
    .unwrap();
    let Output::Track(track::Output::SetInstrument { removed_lanes, .. }) = out.output.clone()
    else {
        panic!()
    };
    assert_eq!(removed_lanes, 1);
    h.project.validate().unwrap();
    assert_eq!(h.project.tracks[&c.track].automation.len(), 1);
    assert!(
        events(&out)
            .iter()
            .any(|e| matches!(e, ProjectEvent::AutomationLaneDeleted { .. }))
    );
    // One history entry: undo gives the lane back with its identifier, redo takes it again.
    let swapped = h.clone();
    let undone = run(&undo(), &mut h, &mut context()).unwrap();
    assert_eq!(h.project, before.project);
    assert!(events(&undone).iter().any(
        |e| matches!(e, ProjectEvent::AutomationLaneCreated { lane, .. } if lane_ids.contains(lane))
    ));
    run(&redo(), &mut h, &mut context()).unwrap();
    assert_eq!(h.project, swapped.project);
    run(&undo(), &mut h, &mut context()).unwrap();
    assert_eq!(h.project, before.project);
}

#[test]
fn restore_instrument_refuses_lanes_that_do_not_fit_and_leaves_the_state_alone() {
    let c = chain();
    let mut s = c.state.clone();
    with_lane(&mut s, &c.track, &c.synth);
    let Undo::Inverse(inverse) = go(&mut s, &set_instrument(&c.track, "faulk.drums")).undo else {
        panic!()
    };
    let Command::Track(track::Command::RestoreInstrument {
        track,
        current,
        device,
        snapshot,
        lanes,
    }) = *inverse
    else {
        panic!()
    };
    assert_eq!(lanes.len(), 1);
    let swap = |lanes: BTreeMap<LaneId, AutomationLane>| {
        t(track::Command::RestoreInstrument {
            track: track.clone(),
            current: current.clone(),
            device: device.clone(),
            snapshot: snapshot.clone(),
            lanes,
        })
    };
    check(&swap(lanes.clone()), &s, &context()).unwrap();
    // A lane that targets another device, or a parameter the instrument does not have.
    let (id, lane) = lanes
        .iter()
        .next()
        .map(|(i, l)| (i.clone(), l.clone()))
        .unwrap();
    let mut elsewhere = lane.clone();
    elsewhere.target = AutomationTarget::Device {
        device: c.a.clone(),
        param: "gain".into(),
    };
    assert_eq!(
        code(&swap(BTreeMap::from([(id.clone(), elsewhere)])), &s),
        "invalid"
    );
    let mut unknown = lane.clone();
    unknown.target = AutomationTarget::Device {
        device: device.clone(),
        param: "nothing".into(),
    };
    assert_eq!(
        code(&swap(BTreeMap::from([(id.clone(), unknown)])), &s),
        "invalid"
    );
    // A lane identifier that is in use (the track's own device).
    let used = LaneId::parse(&c.a.to_string()).unwrap();
    assert_eq!(code(&swap(BTreeMap::from([(used, lane)])), &s), "invalid");
}

#[test]
fn set_instrument_to_the_same_instrument_changes_nothing() {
    let c = chain();
    let mut s = c.state.clone();
    let out = go(&mut s, &set_instrument(&c.track, "faulk.osc_synth"));
    assert_eq!(
        out.output,
        Output::Track(track::Output::SetInstrument {
            device: c.synth.clone(),
            previous: c.synth.clone(),
            removed_lanes: 0
        })
    );
    assert_eq!((events(&out).len(), out.undo), (0, Undo::None));
    assert_eq!(s, c.state);
}

#[test]
fn set_instrument_refuses_an_audio_track_and_a_non_instrument() {
    let c = chain();
    assert_eq!(
        code(&set_instrument(&c.audio_track, "faulk.drums"), &c.state),
        "invalid"
    );
    assert_eq!(code(&set_instrument(&c.track, EFFECT), &c.state), "invalid");
    assert_eq!(
        code(&set_instrument(&c.track, "faulk.nothing"), &c.state),
        "not_found"
    );
    let missing = TrackId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(
        code(&set_instrument(&missing, "faulk.drums"), &c.state),
        "not_found"
    );
    // A MIDI track with no instrument has none to replace.
    let mut bare = c.state.clone();
    bare.project
        .tracks
        .get_mut(&c.track)
        .unwrap()
        .devices
        .remove(&c.synth);
    assert_eq!(
        code(&set_instrument(&c.track, "faulk.drums"), &bare),
        "invalid"
    );
}

#[test]
fn restore_instrument_refuses_what_does_not_fit_and_leaves_the_state_alone() {
    let c = chain();
    let mut s = c.state.clone();
    let Undo::Inverse(inverse) = go(&mut s, &set_instrument(&c.track, "faulk.drums")).undo else {
        panic!()
    };
    let Command::Track(track::Command::RestoreInstrument {
        track,
        current,
        device: id,
        snapshot,
        ..
    }) = *inverse
    else {
        panic!()
    };
    let swap = |track: &TrackId, current: &DeviceId, id: &DeviceId, snapshot: Device| {
        t(track::Command::RestoreInstrument {
            track: track.clone(),
            current: current.clone(),
            device: id.clone(),
            snapshot,
            lanes: Default::default(),
        })
    };
    check(
        &swap(&track, &current, &id, snapshot.clone()),
        &s,
        &context(),
    )
    .unwrap();
    // `current` is an effect, or missing; the identifier is in use.
    assert_eq!(
        code(&swap(&track, &c.a, &id, snapshot.clone()), &s),
        "invalid"
    );
    assert_eq!(
        code(&swap(&track, &c.synth, &id, snapshot.clone()), &s),
        "invalid"
    );
    assert_eq!(
        code(&swap(&track, &current, &c.a, snapshot.clone()), &s),
        "invalid"
    );
    // The snapshot is an effect, the clip player on a MIDI track, an unknown device.
    let mut effect = snapshot.clone();
    effect.role = DeviceRole::Effect;
    assert_eq!(code(&swap(&track, &current, &id, effect), &s), "invalid");
    let mut player = snapshot.clone();
    player.source = ::project::DeviceSource::Builtin {
        id: ::project::CLIP_PLAYER.into(),
        version: 1,
    };
    player.params.clear();
    assert_eq!(code(&swap(&track, &current, &id, player), &s), "invalid");
    let mut unknown = snapshot.clone();
    unknown.source = ::project::DeviceSource::Builtin {
        id: "faulk.nothing".into(),
        version: 1,
    };
    assert_eq!(code(&swap(&track, &current, &id, unknown), &s), "invalid");
    // An instrument that would not stay first is refused by the project's rules.
    let mut late = snapshot.clone();
    late.order =
        OrderKey::between(Some(&s.project.tracks[&track].devices[&c.b].order), None).unwrap();
    assert_eq!(code(&swap(&track, &current, &id, late), &s), "invalid");
    let missing = TrackId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert_eq!(
        code(&swap(&missing, &current, &id, snapshot), &s),
        "not_found"
    );
}

// ---------------------------------------------------------------- the engine

/// One device of the replayed engine: its kind, parameter values (as bits) and enabled flag.
#[derive(Debug, PartialEq)]
struct Built {
    kind: u32,
    params: BTreeMap<u32, u32>,
    enabled: bool,
}

/// What an engine holds of the devices, built by replaying control messages: refuses (by
/// panicking) what the engine would refuse, a removal or a change of an empty slot.
#[derive(Debug, Default, PartialEq)]
struct Replay {
    tracks: BTreeMap<u32, ()>,
    devices: BTreeMap<(u32, u16), Built>,
}

impl Replay {
    fn apply(&mut self, msgs: &[ControlMsg]) {
        for msg in msgs {
            match msg {
                ControlMsg::Reset { .. } => *self = Replay::default(),
                ControlMsg::TrackCreate { track, .. } => {
                    self.tracks.insert(track.0, ());
                }
                ControlMsg::TrackRemove { track } => {
                    self.tracks.remove(&track.0);
                    self.devices.retain(|(t, _), _| *t != track.0);
                }
                ControlMsg::DeviceInsert {
                    track,
                    device,
                    kind,
                    state,
                } => {
                    assert!(
                        track.is_master() || self.tracks.contains_key(&track.0),
                        "{msg:?}: no such track"
                    );
                    let values = engine::msg::state_pairs(state)
                        .map(|(id, v)| (id.0, v.to_bits()))
                        .collect();
                    // An insert on an occupied slot replaces the device there.
                    self.devices.insert(
                        (track.0, device.0),
                        Built {
                            kind: kind.0,
                            params: values,
                            enabled: true,
                        },
                    );
                }
                ControlMsg::DeviceRemove { track, device } => {
                    assert!(
                        self.devices.remove(&(track.0, device.0)).is_some(),
                        "{msg:?}: the slot is empty"
                    );
                }
                ControlMsg::DeviceSetEnabled {
                    track,
                    device,
                    enabled,
                } => {
                    self.devices
                        .get_mut(&(track.0, device.0))
                        .unwrap_or_else(|| panic!("{msg:?}: the slot is empty"))
                        .enabled = *enabled;
                }
                ControlMsg::ParamSet {
                    track,
                    device,
                    param,
                    value,
                } => {
                    self.devices
                        .get_mut(&(track.0, device.0))
                        .unwrap_or_else(|| panic!("{msg:?}: the slot is empty"))
                        .params
                        .insert(param.0, value.to_bits());
                }
                _ => {}
            }
        }
    }
}

/// A project host: every command runs through the history and its engine messages are
/// replayed, then the engine's devices must be what a snapshot of the project would give,
/// and the slot table must be the snapshot's.
struct World {
    s: Synced,
    replay: Replay,
}

impl World {
    fn new(state: State) -> Self {
        let mut ctx = own_context();
        let msgs = resync(&state, &mut ctx, 1);
        let mut replay = Replay::default();
        replay.apply(&msgs);
        // The ids commands generate come from a seed apart from the chain's.
        let ctx = Context::new(
            commands::Author::User,
            ::project::IdGenerator::new(1_690_000_000_000, 0x00AA_0000_0000_0000_0000),
        );
        let mut s = Synced::new(state, ctx, 1);
        s.setup().unwrap();
        World { s, replay }
    }

    fn exec(&mut self, command: &Command) -> commands::Outcome {
        let (outcome, _) = self
            .s
            .run(command)
            .unwrap_or_else(|e| panic!("{command:?}: {e:?}"));
        self.replay.apply(&outcome.control);
        self.check();
        outcome
    }

    /// The messages of one command, for tests of their exact shape.
    fn check(&self) {
        let (msgs, slots) = snapshot(&self.s.state.project, &self.s.state.session);
        let mut fresh = Replay::default();
        fresh.apply(&msgs);
        assert_eq!(self.replay, fresh, "the engine drifted from the project");
        assert_eq!(self.s.ctx.engine.track, slots.track, "slots drifted");
        assert_eq!(
            self.s.ctx.engine.master_devices, slots.master_devices,
            "the master's slots drifted"
        );
        self.s.state.project.validate().unwrap();
    }
}

fn kinds(msgs: &[ControlMsg]) -> Vec<&'static str> {
    msgs.iter()
        .map(|m| match m {
            ControlMsg::DeviceInsert { .. } => "insert",
            ControlMsg::DeviceRemove { .. } => "remove",
            ControlMsg::DeviceSetEnabled { .. } => "enabled",
            ControlMsg::AutomationLaneUpload { .. } => "lane",
            _ => "other",
        })
        .collect()
}

#[test]
fn every_midi_effect_command_and_its_undo_leave_the_engine_as_a_snapshot_would() {
    // #219: the MIDI effects take the first slots, the instrument and the audio effects the
    // ones after; each change and its undo leave the engine as a snapshot of the project.
    let m = midi_chain();
    let c = &m.chain;
    let (msgs, _) = snapshot(&c.state.project, &c.state.session);
    let transpose = engine::device::testing::TRANSPOSE.kind_id;
    let slots: Vec<u16> = msgs
        .iter()
        .filter_map(|msg| match msg {
            ControlMsg::DeviceInsert {
                track,
                device,
                kind,
                ..
            } if *kind == transpose && !track.is_master() => Some(device.0),
            _ => None,
        })
        .collect();
    assert_eq!(slots, [0, 1]);
    let mut w = World::new(c.state.clone());
    let midi = |index| {
        d(device::Command::Insert {
            track: Some(c.track.clone()),
            master: false,
            kind: MIDI_EFFECT.into(),
            index,
        })
    };
    let commands = [
        midi(0),
        mv(&m.m2, 0),
        bypass(&m.m1, true),
        param(&m.m1, "semitones", 5.0),
        remove(&m.m1),
        set_instrument(&c.track, "faulk.drums"),
        midi(u32::MAX),
        bypass(&m.m1, false),
    ];
    let mut done = 0;
    for command in &commands {
        match w.s.run(command) {
            Ok((outcome, _)) => {
                w.replay.apply(&outcome.control);
                w.check();
                done += 1;
            }
            Err(commands::engine_msgs::SyncError::Refused(_)) => {}
            Err(e) => panic!("{command:?}: {e:?}"),
        }
    }
    assert_eq!(
        done,
        commands.len() - 1,
        "only the bypass of the removed one is refused"
    );
    for _ in 0..done {
        w.exec(&undo());
    }
    assert_eq!(w.s.state.project, c.state.project);
    for _ in 0..done {
        w.exec(&redo());
    }
}

#[test]
fn every_chain_command_and_its_undo_leave_the_engine_as_a_snapshot_would() {
    let c = chain();
    let mut w = World::new(c.state.clone());
    let track = c.track.clone();
    let mut extra = Vec::new();
    for command in [
        ins(&track, 0),
        ins(&track, 2),
        ins(&track, u32::MAX),
        mv(&c.b, 0),
        bypass(&c.a, true),
        param(&c.a, "gain", 0.5),
        remove(&c.a),
        set_instrument(&track, "faulk.drums"),
        remove(&c.b),
        bypass(&c.a, false),
    ] {
        let name = format!("{command:?}");
        // A removed device cannot be addressed any more: skip what no longer applies.
        match w.s.run(&command) {
            Ok((outcome, _)) => {
                w.replay.apply(&outcome.control);
                w.check();
                extra.push(name);
            }
            Err(commands::engine_msgs::SyncError::Refused(_)) => {}
            Err(e) => panic!("{name}: {e:?}"),
        }
    }
    assert!(extra.len() >= 8, "{extra:?}");
    // Undo everything, then redo everything: the engine follows the project each time.
    for _ in 0..extra.len() {
        w.exec(&undo());
    }
    assert_eq!(w.s.state.project, c.state.project);
    for _ in 0..extra.len() {
        w.exec(&redo());
    }
}

#[test]
fn a_device_that_keeps_its_place_is_not_touched_and_one_that_shifts_is_built_again() {
    let c = chain();
    let mut w = World::new(c.state.clone());
    // Appending touches nothing before it.
    let out = w.exec(&ins(&c.track, u32::MAX));
    assert_eq!(kinds(&out.control), ["insert"], "{:?}", out.control);
    // Inserting right after the instrument shifts the three effects behind it: each one
    // leaves its slot and is built again one slot further, the last into a new slot.
    let out = w.exec(&ins(&c.track, 0));
    assert_eq!(
        kinds(&out.control),
        ["insert", "insert", "insert", "insert"],
        "{:?}",
        out.control
    );
    // Removing the last effect empties its slot and touches nothing else.
    let last = w.s.state.project.tracks[&c.track]
        .devices_in_order()
        .last()
        .map(|(id, _)| (*id).clone())
        .unwrap();
    let out = w.exec(&remove(&last));
    assert_eq!(kinds(&out.control), ["remove"], "{:?}", out.control);
    // Swapping two neighbours builds both again, in each other's slot, with no removal.
    let out = w.exec(&mv(&c.b, 0));
    assert!(
        !kinds(&out.control).contains(&"remove"),
        "{:?}",
        out.control
    );
    assert!(kinds(&out.control).iter().all(|k| *k == "insert"));
}

#[test]
fn a_bypassed_device_is_built_again_bypassed_and_a_bypass_reaches_the_engine() {
    let c = chain();
    let mut w = World::new(c.state.clone());
    let out = w.exec(&bypass(&c.b, true));
    assert_eq!(kinds(&out.control), ["enabled"]);
    let out = w.exec(&ins(&c.track, 1));
    // The bypassed effect shifted: it comes back with its `DeviceSetEnabled`.
    assert!(
        kinds(&out.control).contains(&"enabled"),
        "{:?}",
        out.control
    );
    // The replay holds it disabled, as the snapshot does (checked by `exec`).
    let out = w.exec(&bypass(&c.b, false));
    assert_eq!(kinds(&out.control), ["enabled"]);
}

#[test]
fn an_effect_on_an_audio_track_takes_the_slot_after_the_clip_players_hole() {
    let c = chain();
    let mut w = World::new(c.state.clone());
    // The clip player is not built by the engine: its slot 0 stays empty.
    let out = w.exec(&ins(&c.audio_track, 0));
    assert_eq!(kinds(&out.control), ["insert"], "{:?}", out.control);
    let ControlMsg::DeviceInsert { device, .. } = &out.control[0] else {
        panic!()
    };
    assert_eq!(*device, engine::msg::DeviceSlot(1));
    // A second effect in front shifts the first one; the hole is never removed.
    let out = w.exec(&ins(&c.audio_track, 0));
    assert!(
        !kinds(&out.control).contains(&"remove"),
        "{:?}",
        out.control
    );
    w.exec(&undo());
    w.exec(&undo());
}

#[test]
fn the_lanes_of_a_device_that_shifts_follow_it() {
    let c = chain();
    let mut state = c.state.clone();
    with_lane(&mut state, &c.track, &c.b);
    let mut w = World::new(state);
    let out = w.exec(&ins(&c.track, 0));
    // A, B and the new one: A and B shift, and B's lane is uploaded again to its new slot.
    assert!(kinds(&out.control).contains(&"lane"), "{:?}", out.control);
    let ControlMsg::AutomationLaneUpload { target, .. } = out
        .control
        .iter()
        .find(|m| matches!(m, ControlMsg::AutomationLaneUpload { .. }))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        *target,
        engine::msg::AutomationTarget::DeviceParam {
            device: engine::msg::DeviceSlot(3),
            param: engine::msg::ParamId(0)
        }
    );
}

#[test]
fn a_track_the_engine_does_not_know_gets_no_message() {
    let c = chain();
    let mut w = World::new(c.state.clone());
    // Forget the track's slot, as an engine table that was full would.
    w.s.ctx.engine.tracks.release(&c.track);
    w.s.ctx.engine.track.remove(&c.track);
    let (outcome, _) = w.s.run(&ins(&c.track, 0)).unwrap();
    assert!(outcome.control.is_empty(), "{:?}", outcome.control);
    let (outcome, _) = w.s.run(&bypass(&c.a, true)).unwrap();
    assert!(outcome.control.is_empty(), "{:?}", outcome.control);
}

#[test]
fn the_undo_of_set_instrument_swaps_the_instrument_in_the_engine() {
    let c = chain();
    let mut w = World::new(c.state.clone());
    let out = w.exec(&set_instrument(&c.track, "faulk.drums"));
    // One insert on slot 0, replacing the synth there; nothing else moves.
    assert_eq!(kinds(&out.control), ["insert"], "{:?}", out.control);
    let out = w.exec(&undo());
    assert_eq!(kinds(&out.control), ["insert"], "{:?}", out.control);
}

#[test]
fn a_swap_that_takes_a_lane_and_its_undo_leave_the_engine_as_a_snapshot_would() {
    let c = chain();
    let mut state = c.state.clone();
    with_lane(&mut state, &c.track, &c.synth);
    let mut w = World::new(state);
    // `exec` replays the messages and compares the engine with a fresh snapshot each time.
    w.exec(&set_instrument(&c.track, "faulk.drums"));
    w.exec(&undo());
    w.exec(&redo());
    w.exec(&undo());
}

// ---------------------------------------------------------------- the master's chain (#364)

/// The master's chain in order.
fn master_ids(state: &State) -> Vec<DeviceId> {
    state
        .project
        .master_devices_in_order()
        .into_iter()
        .map(|(id, _)| id.clone())
        .collect()
}

fn ins_master(index: u32) -> Command {
    d(device::Command::Insert {
        track: None,
        master: true,
        kind: EFFECT.into(),
        index,
    })
}

#[test]
fn the_master_holds_effects_in_order_and_each_command_finds_them_by_id() {
    let c = chain();
    assert_eq!(master_ids(&c.state), [c.ma.clone(), c.mb.clone()]);
    // The track's chain is untouched by the master's.
    assert_eq!(ids(&c.state, &c.track).len(), 3);
    let mut s = c.state.clone();
    // An index counts the effects: first, last, past the end.
    let out = go(&mut s, &ins_master(0));
    let Output::Device(device::Output::Insert { device: first }) = out.output else {
        panic!()
    };
    assert_eq!(
        events(&go(&mut s, &ins_master(u32::MAX))),
        [ProjectEvent::DeviceChainChanged { track: None }]
    );
    assert_eq!(master_ids(&s)[0], first);
    assert_eq!(master_ids(&s).len(), 4);
    s.project.validate().unwrap();
    // Move, bypass, set a parameter and remove go through the device identifier.
    go(&mut s, &mv(&c.mb, 0));
    assert_eq!(master_ids(&s)[0], c.mb);
    go(&mut s, &bypass(&c.ma, true));
    assert!(!s.project.master_devices[&c.ma].enabled);
    let out = go(&mut s, &param(&c.ma, "gain", 0.25));
    assert_eq!(
        out.output,
        Output::Device(device::Output::SetParam {
            previous: 1.0,
            value: 0.25
        })
    );
    let out = go(&mut s, &remove(&c.ma));
    assert_eq!(
        out.output,
        Output::Device(device::Output::Remove { track: None })
    );
    assert!(!s.project.master_devices.contains_key(&c.ma));
    s.project.validate().unwrap();
}

#[test]
fn every_master_chain_command_undoes_and_redoes_exactly() {
    let c = chain();
    for command in [
        ins_master(0),
        ins_master(u32::MAX),
        remove(&c.ma),
        mv(&c.mb, 0),
        bypass(&c.mb, true),
        param(&c.ma, "gain", 0.5),
    ] {
        let mut s = c.state.clone();
        let mut ctx = context();
        run(&command, &mut s, &mut ctx).unwrap();
        let after = s.project.clone();
        run(&undo(), &mut s, &mut ctx).unwrap();
        assert_eq!(s.project, c.state.project, "undo of {command:?}");
        run(&redo(), &mut s, &mut ctx).unwrap();
        assert_eq!(s.project, after, "redo of {command:?}");
    }
}

#[test]
fn the_undo_of_a_master_remove_is_a_restore_into_the_master_with_its_settings() {
    let c = chain();
    let mut s = c.state.clone();
    go(&mut s, &param(&c.ma, "gain", 0.5));
    go(&mut s, &bypass(&c.ma, true));
    let before = s.project.master_devices[&c.ma].clone();
    let out = go(&mut s, &remove(&c.ma));
    let Undo::Inverse(inverse) = &out.undo else {
        panic!("{:?}", out.undo)
    };
    let Command::Device(device::Command::Restore {
        track,
        master,
        device,
        snapshot,
        ..
    }) = &**inverse
    else {
        panic!("{inverse:?}")
    };
    assert_eq!((track, *master, device), (&None, true, &c.ma));
    assert_eq!(snapshot, &before);
    go(&mut s, &undo());
    assert_eq!(s.project.master_devices[&c.ma], before);
}

#[test]
fn the_master_chain_is_named_by_exactly_one_of_track_and_master() {
    let c = chain();
    let both = d(device::Command::Insert {
        track: Some(c.track.clone()),
        master: true,
        kind: EFFECT.into(),
        index: 0,
    });
    assert_eq!(code(&both, &c.state), "invalid");
    let neither = d(device::Command::Insert {
        track: None,
        master: false,
        kind: EFFECT.into(),
        index: 0,
    });
    assert_eq!(code(&neither, &c.state), "invalid");
    let snapshot = c.state.project.master_devices[&c.ma].clone();
    for (track, master) in [(Some(c.track.clone()), true), (None, false)] {
        let restore = d(device::Command::Restore {
            track,
            master,
            device: DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            snapshot: snapshot.clone(),
            lanes: Default::default(),
        });
        assert_eq!(code(&restore, &c.state), "invalid");
    }
    // The wire form: `master` is optional and `track` too.
    let json =
        serde_json::json!({"type": "device.insert", "master": true, "kind": EFFECT, "index": 0});
    let command: Command = serde_json::from_value(json).unwrap();
    assert_eq!(command, ins_master(0));
    let json = serde_json::json!({"type": "device.insert", "track": c.track.to_string(), "kind": EFFECT, "index": 0});
    let command: Command = serde_json::from_value(json).unwrap();
    assert_eq!(command, ins(&c.track, 0));
    assert!(
        !serde_json::to_string(&ins(&c.track, 0))
            .unwrap()
            .contains("master")
    );
}

#[test]
fn the_master_refuses_an_instrument_an_unknown_effect_and_a_chain_past_the_limit() {
    let c = chain();
    let kind = |kind: &str| {
        d(device::Command::Insert {
            track: None,
            master: true,
            kind: kind.into(),
            index: 0,
        })
    };
    let e = refused(&kind("faulk.osc_synth"), &c.state);
    assert_eq!(e.code(), "invalid");
    assert!(e.to_string().contains("master"), "{e}");
    assert_eq!(code(&kind("faulk.drums"), &c.state), "invalid");
    assert_eq!(code(&kind("faulk.nothing"), &c.state), "not_found");
    // A snapshot of an instrument is not restored into the master either.
    let synth = c.state.project.tracks[&c.track].devices[&c.synth].clone();
    let restore = d(device::Command::Restore {
        track: None,
        master: true,
        device: DeviceId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
        snapshot: synth,
        lanes: Default::default(),
    });
    assert_eq!(code(&restore, &c.state), "invalid");
    // The safety limit of devices per chain.
    let mut s = c.state.clone();
    let mut ctx = own_context();
    while s.project.master_devices.len() < ::project::limits::MAX_DEVICES_PER_TRACK {
        insert_master(&mut s, &mut ctx, u32::MAX);
    }
    assert_eq!(code(&ins_master(0), &s), "limit");
    // The restore of an effect already in a track is refused: ids are unique.
    let dup = d(device::Command::Restore {
        track: None,
        master: true,
        device: c.a.clone(),
        snapshot: c.state.project.tracks[&c.track].devices[&c.a].clone(),
        lanes: Default::default(),
    });
    assert_eq!(code(&dup, &c.state), "invalid");
}

#[test]
fn the_master_chain_reaches_the_engine_as_a_snapshot_would_give_it() {
    let c = chain();
    let mut w = World::new(c.state.clone());
    let mut done = 0;
    for command in [
        ins_master(0),
        ins_master(u32::MAX),
        mv(&c.mb, 0),
        bypass(&c.ma, true),
        param(&c.ma, "gain", 0.5),
        remove(&c.ma),
        bypass(&c.ma, false),
    ] {
        // Once removed, a device cannot be addressed: skip what no longer applies.
        match w.s.run(&command) {
            Ok((outcome, _)) => {
                assert!(
                    outcome.control.iter().all(|m| match m {
                        ControlMsg::DeviceInsert { track, .. }
                        | ControlMsg::DeviceRemove { track, .. }
                        | ControlMsg::DeviceSetEnabled { track, .. }
                        | ControlMsg::ParamSet { track, .. } => track.is_master(),
                        _ => true,
                    }),
                    "{command:?}: {:?}",
                    outcome.control
                );
                w.replay.apply(&outcome.control);
                w.check();
                done += 1;
            }
            Err(commands::engine_msgs::SyncError::Refused(_)) => {}
            Err(e) => panic!("{command:?}: {e:?}"),
        }
    }
    assert!(done >= 6, "{done}");
    for _ in 0..done {
        w.exec(&undo());
    }
    assert_eq!(w.s.state.project, c.state.project);
    for _ in 0..done {
        w.exec(&redo());
    }
}

#[test]
fn the_snapshot_sends_the_master_chain_after_the_master_and_before_the_tracks() {
    let c = chain();
    let (msgs, _) = snapshot(&c.state.project, &c.state.session);
    let master_at = msgs
        .iter()
        .position(|m| matches!(m, ControlMsg::SetMaster { .. }))
        .unwrap();
    let first_track = msgs
        .iter()
        .position(|m| matches!(m, ControlMsg::TrackCreate { .. }))
        .unwrap();
    let inserts: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m, ControlMsg::DeviceInsert { track, .. } if track.is_master()))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(inserts.len(), 2);
    assert!(inserts.iter().all(|i| *i > master_at && *i < first_track));
}

#[test]
fn a_bypass_and_a_parameter_on_the_master_reach_the_engine_and_the_track_ones_do_not_change() {
    let c = chain();
    let mut w = World::new(c.state.clone());
    let out = w.exec(&bypass(&c.ma, true));
    assert_eq!(kinds(&out.control), ["enabled"]);
    let out = w.exec(&param(&c.ma, "gain", 0.5));
    assert_eq!(out.control.len(), 1, "{:?}", out.control);
    let out = w.exec(&bypass(&c.a, true));
    assert!(out.control.iter().all(|m| !matches!(
        m,
        ControlMsg::DeviceSetEnabled { track, .. } if track.is_master()
    )));
}
