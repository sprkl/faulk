//! `device.set_sidechain` (#952): a Compressor's detection listening to another track. Its
//! schema, description and undo through the harness, every refusal leaving the state as it
//! was, `track.delete` of a source setting its listeners back to their own track and its undo
//! giving them back, and the engine messages the compiler sends for each, live and in a
//! snapshot.

use commands::engine_msgs::{Synced, snapshot};
use commands::testing::{check, check_refusal, context, fixture};
use commands::{
    Command, CommandError, Context, ControlMsg, Output, ProjectEvent, State, Undo, device, run,
    track,
};

use project::limits::MAX_SIDECHAIN_SOURCES;
use project::{DeviceId, TrackId, TrackKind};

/// A pad (the fixture's synth track, with a Compressor) and a kick (a second synth track).
pub struct Fx {
    pub state: State,
    /// The context the fixture was built with: its id generator goes on from there.
    pub ctx: Context,
    pub pad: TrackId,
    pub kick: TrackId,
    pub comp: DeviceId,
}

fn apply(state: &mut State, ctx: &mut Context, command: &Command) -> Output {
    run(command, state, ctx)
        .unwrap_or_else(|e| panic!("{command:?}: {e}"))
        .output
}

fn create(state: &mut State, ctx: &mut Context, name: &str) -> TrackId {
    let out = apply(
        state,
        ctx,
        &Command::Track(track::Command::Create {
            kind: TrackKind::Midi,
            instrument: None,
            name: Some(name.into()),
            color: None,
        }),
    );
    let Output::Track(track::Output::Create { track, .. }) = out else {
        panic!("{out:?}")
    };
    track
}

fn compressor(state: &mut State, ctx: &mut Context, track: &TrackId) -> DeviceId {
    let out = apply(
        state,
        ctx,
        &Command::Device(device::Command::Insert {
            track: Some(track.clone()),
            master: false,
            kind: "faulk.compressor".into(),
            index: 99,
        }),
    );
    let Output::Device(device::Output::Insert { device }) = out else {
        panic!("{out:?}")
    };
    device
}

pub fn fx() -> Fx {
    let f = fixture();
    let mut state = f.state;
    let mut ctx = context();
    let comp = compressor(&mut state, &mut ctx, &f.track);
    let kick = create(&mut state, &mut ctx, "Kick");
    Fx {
        state,
        ctx,
        pad: f.track,
        kick,
        comp,
    }
}

pub fn set(device: &DeviceId, source: Option<&TrackId>) -> Command {
    Command::Device(device::Command::SetSidechain {
        device: device.clone(),
        source: source.cloned(),
    })
}

fn delete(track: &TrackId) -> Command {
    Command::Track(track::Command::Delete {
        track: track.clone(),
    })
}

fn invalid(err: &CommandError, field: &str) -> String {
    match err {
        CommandError::Invalid { field: f, message } if f == field => message.clone(),
        other => panic!("expected invalid {field}, got {other:?}"),
    }
}

#[test]
fn choosing_a_source_is_undone_and_redone() {
    let f = fx();
    let mut c = f.ctx.clone();
    check(&set(&f.comp, Some(&f.kick)), &f.state, &c).unwrap();
    let mut s = f.state.clone();
    let out = run(&set(&f.comp, Some(&f.kick)), &mut s, &mut c).unwrap();
    assert_eq!(
        out.output,
        Output::Device(device::Output::SetSidechain { previous: None })
    );
    assert_eq!(
        out.events[0],
        ProjectEvent::DeviceSidechainChanged {
            device: f.comp.clone(),
            source: Some(f.kick.clone()),
        }
    );
    assert_eq!(
        s.project.device(&f.comp).unwrap().sidechain,
        Some(f.kick.clone())
    );
    // Back to its own track, undone too.
    check(&set(&f.comp, None), &s, &c).unwrap();
    // The same source again changes nothing: no inverse, no event.
    let again = run(&set(&f.comp, Some(&f.kick)), &mut s, &mut c).unwrap();
    assert_eq!(again.undo, Undo::None);
    assert!(
        !again
            .events
            .iter()
            .any(|e| matches!(e, ProjectEvent::DeviceSidechainChanged { .. })),
        "{:?}",
        again.events
    );
}

#[test]
fn every_refusal_leaves_the_state_as_it_was() {
    let f = fx();
    let mut c = f.ctx.clone();
    let mut s = f.state.clone();
    // A device that takes no source: the synth.
    let synth = s.project.tracks[&f.pad]
        .devices
        .iter()
        .find(|(_, d)| d.role == project::DeviceRole::Instrument)
        .map(|(id, _)| id.clone())
        .unwrap();
    let err = check_refusal(&set(&synth, Some(&f.kick)), &s, &c).unwrap();
    assert!(invalid(&err, "device").contains("faulk.osc_synth"));
    // Its own track: `null` is that.
    let err = check_refusal(&set(&f.comp, Some(&f.pad)), &s, &c).unwrap();
    invalid(&err, "source");
    // A track that does not exist, a device that does not exist.
    let gone = TrackId::generate(&mut project::IdGenerator::new(1_900_000_000_000, 5));
    let err = check_refusal(&set(&f.comp, Some(&gone)), &s, &c).unwrap();
    assert_eq!(err.code(), "not_found");
    let nobody = DeviceId::generate(&mut project::IdGenerator::new(1_900_000_000_000, 6));
    let err = check_refusal(&set(&nobody, None), &s, &c).unwrap();
    assert_eq!(err.code(), "not_found");
    // On the master's chain.
    let out = apply(
        &mut s,
        &mut c,
        &Command::Device(device::Command::Insert {
            track: None,
            master: true,
            kind: "faulk.compressor".into(),
            index: 0,
        }),
    );
    let Output::Device(device::Output::Insert { device: on_master }) = out else {
        panic!()
    };
    let err = check_refusal(&set(&on_master, Some(&f.kick)), &s, &c).unwrap();
    assert!(invalid(&err, "device").contains("master"));
    // A loop: the kick listens to the pad, so the pad cannot listen to the kick; the message
    // names the kick.
    let kick_comp = compressor(&mut s, &mut c, &f.kick);
    apply(&mut s, &mut c, &set(&kick_comp, Some(&f.pad)));
    let err = check_refusal(&set(&f.comp, Some(&f.kick)), &s, &c).unwrap();
    assert!(invalid(&err, "source").contains("\"Kick\""), "{err:?}");
    // Through a third track too: bass → pad, kick → bass, pad → kick refused.
    let mut s = f.state.clone();
    let bass = create(&mut s, &mut c, "Bass");
    let bass_comp = compressor(&mut s, &mut c, &bass);
    let kick_comp = compressor(&mut s, &mut c, &f.kick);
    apply(&mut s, &mut c, &set(&bass_comp, Some(&f.pad)));
    apply(&mut s, &mut c, &set(&kick_comp, Some(&bass)));
    let err = check_refusal(&set(&f.comp, Some(&f.kick)), &s, &c).unwrap();
    assert!(invalid(&err, "source").contains("\"Kick\""), "{err:?}");
}

#[test]
fn more_sources_than_the_engine_holds_are_refused() {
    let f = fx();
    let mut c = f.ctx.clone();
    let mut s = f.state.clone();
    let mut comps = Vec::new();
    for i in 0..=MAX_SIDECHAIN_SOURCES {
        let source = create(&mut s, &mut c, &format!("S{i}"));
        comps.push((compressor(&mut s, &mut c, &f.pad), source));
    }
    let (last, rest) = comps.split_last().unwrap();
    for (comp, source) in rest {
        apply(&mut s, &mut c, &set(comp, Some(source)));
    }
    let err = check_refusal(&set(&last.0, Some(&last.1)), &s, &c).unwrap();
    assert_eq!(
        err,
        CommandError::Limit {
            entity: "sidechain sources".into(),
            max: MAX_SIDECHAIN_SOURCES as u64
        }
    );
    // A source already listened to counts once.
    check(&set(&last.0, Some(&rest[0].1)), &s, &c).unwrap();
}

#[test]
fn deleting_the_source_sets_its_listeners_back_and_undo_gives_them_back() {
    let f = fx();
    let mut c = f.ctx.clone();
    let mut s = f.state.clone();
    apply(&mut s, &mut c, &set(&f.comp, Some(&f.kick)));
    // The harness: apply, undo to the identical state, redo.
    check(&delete(&f.kick), &s, &c).unwrap();
    let before = s.clone();
    let out = run(&delete(&f.kick), &mut s, &mut c).unwrap();
    let Output::Track(track::Output::Delete { sidechains, .. }) = &out.output else {
        panic!()
    };
    assert_eq!(sidechains, &vec![f.comp.clone()]);
    assert_eq!(s.project.device(&f.comp).unwrap().sidechain, None);
    assert!(out.events.contains(&ProjectEvent::DeviceSidechainChanged {
        device: f.comp.clone(),
        source: None,
    }));
    let Undo::Inverse(inverse) = out.undo else {
        panic!()
    };
    run(&inverse, &mut s, &mut c).unwrap();
    assert_eq!(s.project, before.project);
    // The restore's refusals: a listener that does not exist, one that listens elsewhere.
    let mut gone = before.clone();
    run(&delete(&f.kick), &mut gone, &mut c).unwrap();
    let Command::Track(track::Command::Restore {
        track, snapshot, ..
    }) = *inverse
    else {
        panic!()
    };
    let restore = |listeners: Vec<DeviceId>| {
        Command::Track(track::Command::Restore {
            track: track.clone(),
            snapshot: snapshot.clone(),
            listeners,
        })
    };
    let nobody = DeviceId::generate(&mut project::IdGenerator::new(1_900_000_000_000, 7));
    let err = check_refusal(&restore(vec![nobody]), &gone, &c).unwrap();
    assert_eq!(err.code(), "not_found");
    let mut elsewhere = gone.clone();
    let other = create(&mut elsewhere, &mut c, "Other");
    apply(&mut elsewhere, &mut c, &set(&f.comp, Some(&other)));
    let err = check_refusal(&restore(vec![f.comp.clone()]), &elsewhere, &c).unwrap();
    invalid(&err, "listeners");
    check(&restore(vec![f.comp.clone()]), &gone, &c).unwrap();
}

#[test]
fn a_restored_device_or_track_with_a_bad_source_is_refused() {
    let f = fx();
    let mut c = f.ctx.clone();
    let mut s = f.state.clone();
    apply(&mut s, &mut c, &set(&f.comp, Some(&f.kick)));
    // The Compressor removed, then its source deleted: restoring it would name a track that
    // is gone.
    let out = run(
        &Command::Device(device::Command::Remove {
            device: f.comp.clone(),
        }),
        &mut s,
        &mut c,
    )
    .unwrap();
    let Undo::Inverse(restore) = out.undo else {
        panic!()
    };
    run(&delete(&f.kick), &mut s, &mut c).unwrap();
    let err = check_refusal(restore.as_ref(), &s, &c).unwrap();
    assert_eq!(err.code(), "invalid");
}

fn keys(msgs: &[ControlMsg]) -> Vec<(u32, u16, Option<u32>)> {
    msgs.iter()
        .filter_map(|m| match m {
            ControlMsg::DeviceSidechain {
                track,
                device,
                source,
            } => Some((track.0, device.0, source.map(|s| s.0))),
            _ => None,
        })
        .collect()
}

#[test]
fn the_engine_hears_each_change_once_live_and_in_a_snapshot() {
    let f = fx();
    let mut synced = Synced::new(f.state.clone(), f.ctx.clone(), 1);
    synced.setup().unwrap();
    let slot = |s: &Synced, t: &TrackId| s.ctx.engine.tracks.get(t).unwrap();
    let (pad, kick) = (slot(&synced, &f.pad), slot(&synced, &f.kick));
    let comp = synced.ctx.engine.track[&f.pad]
        .devices
        .get(&f.comp)
        .unwrap() as u16;
    let (out, _) = synced.run(&set(&f.comp, Some(&f.kick))).unwrap();
    assert_eq!(keys(&out.control), vec![(pad, comp, Some(kick))]);
    // A snapshot of that project sends it after every track.
    let (msgs, _) = snapshot(&synced.state.project, &synced.state.session);
    assert_eq!(keys(&msgs), vec![(pad, comp, Some(kick))]);
    let last_track = msgs
        .iter()
        .rposition(|m| matches!(m, ControlMsg::TrackCreate { .. }))
        .unwrap();
    let key_at = msgs
        .iter()
        .position(|m| matches!(m, ControlMsg::DeviceSidechain { .. }))
        .unwrap();
    assert!(key_at > last_track);
    // A change that sends nothing new for it sends no source again.
    let (out, _) = synced
        .run(&Command::Track(track::Command::Rename {
            track: f.pad.clone(),
            name: "Pad".into(),
        }))
        .unwrap();
    assert!(keys(&out.control).is_empty());
    // A device inserted in front of it moves it in the engine (`DeviceMove`): its source
    // moves along, nothing to send.
    let (out, _) = synced
        .run(&Command::Device(device::Command::Insert {
            track: Some(f.pad.clone()),
            master: false,
            kind: "faulk.filter".into(),
            index: 0,
        }))
        .unwrap();
    let moved = out.control.iter().any(|m| {
        matches!(
            m,
            ControlMsg::DeviceMove { .. } | ControlMsg::DeviceInsert { .. }
        )
    });
    assert!(moved, "{:?}", out.control);
    let comp_now = synced.ctx.engine.track[&f.pad]
        .devices
        .get(&f.comp)
        .unwrap() as u16;
    let resent = keys(&out.control);
    // Whether the engine moved it or built it again, it ends up listening to the kick.
    if out
        .control
        .iter()
        .any(|m| matches!(m, ControlMsg::DeviceInsert { device, .. } if device.0 == comp_now))
    {
        assert_eq!(resent, vec![(pad, comp_now, Some(kick))]);
    } else {
        assert!(resent.is_empty(), "{resent:?}");
    }
    assert_eq!(synced.ctx.engine.keys.get(&(pad, comp_now)), Some(&kick));
    // Deleting the kick: its slot leaves, the pad's source is cleared.
    let (out, _) = synced.run(&delete(&f.kick)).unwrap();
    assert_eq!(keys(&out.control), vec![(pad, comp_now, None)]);
    assert!(synced.ctx.engine.keys.is_empty());
    // Undone: the kick comes back and the source is sent again, after it.
    let (out, _) = synced
        .run(&Command::History(commands::history::Command::Undo {}))
        .unwrap();
    let kick = slot(&synced, &f.kick);
    assert_eq!(keys(&out.control), vec![(pad, comp_now, Some(kick))]);
}
