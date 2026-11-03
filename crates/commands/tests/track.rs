//! PR-8: the track commands. No cap below the safety limit, a delete that undoes to the
//! identical project, a move that undoes to the identical key, a duplicate with fresh
//! identifiers, and `view.select_track` refusing a track that does not exist.

use ::project::{
    AutomationLane, AutomationPoint, AutomationTarget, Id, IdGenerator, LaneId, OrderKey, Shape,
    Tick, Track, TrackId, TrackKind,
};
use commands::testing::{check, check_refusal, context, fixture};
use commands::track::{self, DEFAULT_INSTRUMENT};
use commands::{Command, CommandError, Output, ProjectEvent, State, Undo, history, run, view};
use std::collections::{BTreeMap, BTreeSet};

mod clip_examples;
mod track_examples;

use clip_examples::fx;

fn tc(c: track::Command) -> Command {
    Command::Track(c)
}

fn create(kind: TrackKind) -> Command {
    tc(track::Command::Create {
        kind,
        instrument: None,
        name: None,
        color: None,
    })
}

fn delete(track: &TrackId) -> Command {
    tc(track::Command::Delete {
        track: track.clone(),
    })
}

fn mv(track: &TrackId, index: u32) -> Command {
    tc(track::Command::Move {
        track: track.clone(),
        index,
    })
}

fn undo() -> Command {
    Command::History(history::Command::Undo {})
}

fn redo() -> Command {
    Command::History(history::Command::Redo {})
}

fn refused(command: &Command, state: &State) -> CommandError {
    check_refusal(command, state, &context()).unwrap()
}

/// The tracks' ids, in display order.
fn order(state: &State) -> Vec<TrackId> {
    state
        .project
        .tracks_in_order()
        .into_iter()
        .map(|(id, _)| id.clone())
        .collect()
}

/// Every identifier of the project, as a list (a duplicate shows up twice).
fn ids(state: &State) -> Vec<Id> {
    let p = &state.project;
    let mut all = vec![p.id.0.clone()];
    all.extend(p.scenes.keys().map(|k| k.0.clone()));
    for (id, t) in &p.tracks {
        all.push(id.0.clone());
        all.extend(t.devices.keys().map(|k| k.0.clone()));
        all.extend(t.clips.keys().map(|k| k.0.clone()));
        all.extend(t.placements.keys().map(|k| k.0.clone()));
        all.extend(t.automation.keys().map(|k| k.0.clone()));
    }
    all
}

fn created(outcome: &commands::Outcome) -> (TrackId, ::project::DeviceId) {
    let Output::Track(track::Output::Create { track, device }) = &outcome.output else {
        panic!("{:?}", outcome.output)
    };
    (track.clone(), device.clone())
}

#[test]
fn a_midi_track_plays_the_oscillator_at_its_defaults_and_goes_last() {
    let f = fixture();
    let mut s = f.state.clone();
    let out = run(&create(TrackKind::Midi), &mut s, &mut context()).unwrap();
    let (id, device) = created(&out);
    s.project.validate().unwrap();
    assert_eq!(order(&s).last(), Some(&id));
    let t = &s.project.tracks[&id];
    assert_eq!(t.kind, TrackKind::Midi);
    assert_eq!(t.name, "");
    assert_eq!(
        t.color,
        Some(::project::colors::palette_hue(1)),
        "the fixture holds hue 0"
    );
    assert_eq!(t.mixer, ::project::Mixer::default());
    let d = &t.devices[&device];
    assert!(matches!(
        &d.source,
        ::project::DeviceSource::Builtin { id, version: 1 } if id == DEFAULT_INSTRUMENT
    ));
    // Its parameters are the descriptor's defaults.
    let defaults: BTreeMap<String, f64> = engine::devices::osc_synth::DESCRIPTOR
        .params
        .iter()
        .map(|p| (p.key.to_owned(), p.default))
        .collect();
    assert_eq!(defaults.len(), 6);
    assert_eq!(d.params, defaults);
    // The fixture's synth holds the same values, widened from the engine's f32.
    for (key, value) in &s.project.tracks[&f.track].devices[&f.device].params {
        assert!((value - defaults[key]).abs() < 1e-6, "{key}");
    }
    assert!(
        out.events
            .contains(&ProjectEvent::TrackCreated { track: id })
    );
}

#[test]
fn an_audio_track_holds_the_clip_player() {
    let mut s = fixture().state;
    let out = run(&create(TrackKind::Audio), &mut s, &mut context()).unwrap();
    let (id, device) = created(&out);
    s.project.validate().unwrap();
    let t = &s.project.tracks[&id];
    assert_eq!(t.kind, TrackKind::Audio);
    assert!(matches!(
        &t.devices[&device].source,
        ::project::DeviceSource::Builtin { id, .. } if id == ::project::CLIP_PLAYER
    ));
}

#[test]
fn create_refuses_what_it_cannot_honour_and_changes_nothing() {
    let f = fixture();
    let with = |kind, instrument: Option<&str>, name: Option<String>| {
        tc(track::Command::Create {
            kind,
            instrument: instrument.map(Into::into),
            name,
            color: None,
        })
    };
    // An unknown instrument.
    let e = refused(&with(TrackKind::Midi, Some("faulk.nope"), None), &f.state);
    assert!(matches!(e, CommandError::NotFound { ref entity, .. } if entity == "instrument"));
    // An audio track takes no instrument, not even the synth.
    let e = refused(
        &with(TrackKind::Audio, Some(DEFAULT_INSTRUMENT), None),
        &f.state,
    );
    assert!(matches!(e, CommandError::Invalid { ref field, .. } if field == "instrument"));
    // The clip player is not in the registry: a MIDI track cannot take it.
    let e = refused(
        &with(TrackKind::Midi, Some(::project::CLIP_PLAYER), None),
        &f.state,
    );
    assert_eq!(e.code(), "not_found");
    // A name over 200 characters; 200 are fine.
    let long = "x".repeat(::project::limits::MAX_NAME_CHARS + 1);
    let e = refused(&with(TrackKind::Midi, None, Some(long)), &f.state);
    assert!(matches!(e, CommandError::Invalid { ref field, .. } if field == "name"));
    let max = "é".repeat(::project::limits::MAX_NAME_CHARS);
    check(
        &with(TrackKind::Midi, None, Some(max)),
        &f.state,
        &context(),
    )
    .unwrap();
}

#[test]
fn a_thousand_tracks_are_created_and_the_project_stays_valid() {
    let mut s = commands::testing::state();
    let mut c = context();
    for i in 0..1_000 {
        let kind = if i % 2 == 0 {
            TrackKind::Midi
        } else {
            TrackKind::Audio
        };
        run(&create(kind), &mut s, &mut c).unwrap();
    }
    assert_eq!(s.project.tracks.len(), 1_000);
    s.project.validate().unwrap();
    // Display order is creation order.
    let created_order: Vec<_> = order(&s);
    assert_eq!(created_order.len(), 1_000);
    let unique: BTreeSet<_> = ids(&s).into_iter().collect();
    assert_eq!(unique.len(), ids(&s).len());
}

/// A state holding `n` empty tracks (built directly: the test is about the limit).
fn with_tracks(n: usize) -> State {
    let mut s = commands::testing::state();
    let mut ids = IdGenerator::new(1_700_000_000_000, 0x00CC_0000_0000_0000_0000);
    let mut key: Option<OrderKey> = None;
    for _ in 0..n {
        let k = OrderKey::between(key.as_ref(), None).unwrap();
        s.project.tracks.insert(
            TrackId::generate(&mut ids),
            Track::new(TrackKind::Midi, "", k.clone()),
        );
        key = Some(k);
    }
    s
}

#[test]
fn the_safety_limit_is_refused_with_a_typed_error_and_one_below_it_is_not() {
    let max = ::project::limits::MAX_TRACKS;
    let below = with_tracks(max - 1);
    check(&create(TrackKind::Midi), &below, &context()).unwrap();
    let full = with_tracks(max);
    let want = CommandError::Limit {
        entity: "tracks".into(),
        max: max as u64,
    };
    assert_eq!(refused(&create(TrackKind::Midi), &full), want);
    assert_eq!(refused(&create(TrackKind::Audio), &full), want);
    let one = order(&full)[0].clone();
    let dup = tc(track::Command::Duplicate {
        track: one.clone(),
        name: None,
    });
    assert_eq!(refused(&dup, &full), want);
    // Restoring a track into a full project is refused the same way.
    let snapshot = full.project.tracks[&one].clone();
    let mut without = full.clone();
    without.project.tracks.remove(&one);
    let restore = |track: TrackId| {
        tc(track::Command::Restore {
            track,
            snapshot: snapshot.clone(),
        })
    };
    check(&restore(one.clone()), &without, &context()).unwrap();
    let mut other = IdGenerator::new(1_710_000_000_000, 1);
    assert_eq!(
        refused(&restore(TrackId::generate(&mut other)), &full),
        want
    );
}

#[test]
fn delete_then_undo_restores_the_track_its_devices_and_clips_identically() {
    let f = fx();
    for track in [&f.track, &f.audio_track] {
        let mut s = f.state.clone();
        let before = s.clone();
        let out = run(&delete(track), &mut s, &mut context()).unwrap();
        assert!(!s.project.tracks.contains_key(track));
        let Output::Track(track::Output::Delete { devices, clips }) = out.output else {
            panic!()
        };
        let source = &before.project.tracks[track];
        assert_eq!(devices as usize, source.devices.len());
        assert_eq!(clips as usize, source.clips.len());
        assert!(!source.clips.is_empty() && !source.devices.is_empty());
        s.project.validate().unwrap();
        // Undo through the history: same track, same ids, same order key.
        run(&undo(), &mut s, &mut context()).unwrap();
        assert_eq!(s.project, before.project);
        s.project.validate().unwrap();
        // Redo deletes again, undo again.
        run(&redo(), &mut s, &mut context()).unwrap();
        assert!(!s.project.tracks.contains_key(track));
        run(&undo(), &mut s, &mut context()).unwrap();
        assert_eq!(s.project, before.project);
    }
}

#[test]
fn deleting_the_selected_or_soloed_track_clears_the_session_and_undo_leaves_it_cleared() {
    let f = fx();
    let mut s = f.state.clone();
    s.session.selected_track = Some(f.track.clone());
    s.session.solo.insert(f.track.clone());
    s.session.solo.insert(f.audio_track.clone());
    let out = run(&delete(&f.track), &mut s, &mut context()).unwrap();
    assert_eq!(s.session.selected_track, None);
    assert_eq!(s.session.solo, BTreeSet::from([f.audio_track.clone()]));
    assert!(
        out.events
            .contains(&ProjectEvent::TrackSelected { track: None })
    );
    assert!(out.events.contains(&ProjectEvent::SoloChanged {
        track: f.track.clone(),
        solo: false
    }));
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.session.selected_track, None);
    assert!(!s.session.solo.contains(&f.track));
    // Deleting another track leaves the selection alone.
    let mut other = f.state.clone();
    other.session.selected_track = Some(f.track.clone());
    run(&delete(&f.audio_track), &mut other, &mut context()).unwrap();
    assert_eq!(other.session.selected_track, Some(f.track.clone()));
}

#[test]
fn deleting_an_unknown_track_is_refused() {
    let f = fx();
    let mut s = f.state.clone();
    run(&delete(&f.track), &mut s, &mut context()).unwrap();
    let e = refused(&delete(&f.track), &s);
    assert!(matches!(e, CommandError::NotFound { ref entity, .. } if entity == "track"));
}

#[test]
fn restore_refuses_a_collision_or_an_invalid_snapshot_and_leaves_the_state_alone() {
    let f = fx();
    let snapshot = f.state.project.tracks[&f.track].clone();
    let restore = |track: &TrackId, snapshot: Track| {
        tc(track::Command::Restore {
            track: track.clone(),
            snapshot,
        })
    };
    // The track is still there: its id is in use.
    let e = refused(&restore(&f.track, snapshot.clone()), &f.state);
    assert!(matches!(e, CommandError::Invalid { ref field, .. } if field == "track"));
    // A new track id, but the devices and clips are still in the other track: collision.
    let mut ids = IdGenerator::new(1_720_000_000_000, 2);
    let fresh = TrackId::generate(&mut ids);
    let e = refused(&restore(&fresh, snapshot.clone()), &f.state);
    assert!(matches!(e, CommandError::Invalid { ref field, .. } if field == "snapshot"));
    // The id of another entity (a scene) as the track id.
    let scene = f.state.project.scenes.keys().next().unwrap().0.clone();
    let mut gone = f.state.clone();
    gone.project.tracks.remove(&f.track);
    let e = refused(&restore(&TrackId(scene), snapshot.clone()), &gone);
    assert_eq!(e.code(), "invalid");
    // A snapshot that breaks the project's rules: a MIDI track holding an audio clip.
    let mut bad = snapshot.clone();
    let audio = f.state.project.tracks[&f.audio_track].clone();
    bad.clips.extend(audio.clips);
    let e = refused(&restore(&f.track, bad), &gone);
    assert!(matches!(e, CommandError::Invalid { ref field, .. } if field == "snapshot"));
    // The honest snapshot applies, and undoing it deletes the track.
    check(&restore(&f.track, snapshot), &gone, &context()).unwrap();
}

#[test]
fn rename_and_colour_change_one_thing_and_a_repeat_changes_nothing() {
    let f = fixture();
    let t = &f.track;
    let rename = |name: &str| {
        tc(track::Command::Rename {
            track: t.clone(),
            name: name.into(),
        })
    };
    let mut s = f.state.clone();
    let out = run(&rename("  Bass  "), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.tracks[t].name, "Bass");
    assert_eq!(
        out.output,
        Output::Track(track::Output::Rename {
            previous: "Synth".into()
        })
    );
    assert!(out.events.contains(&ProjectEvent::TrackChanged {
        track: t.clone(),
        part: commands::contract::TrackPart::Name
    }));
    // The same name, trimmed or not, changes nothing: no inverse, no event.
    for same in ["Bass", "  Bass "] {
        let again = run(&rename(same), &mut s, &mut context()).unwrap();
        assert!(again.events.is_empty());
        assert_eq!(again.undo, Undo::None);
    }
    // Empty clears; over 200 characters is refused.
    run(&rename(""), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.tracks[t].name, "");
    let e = refused(&rename(&"x".repeat(201)), &s);
    assert_eq!(e.code(), "invalid");
    // Colour.
    let color = |c: Option<::project::Rgb>| {
        tc(track::Command::SetColor {
            track: t.clone(),
            color: c,
        })
    };
    let red = Some(::project::Rgb::new(255, 0, 0));
    run(&color(red), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.tracks[t].color, red);
    let again = run(&color(red), &mut s, &mut context()).unwrap();
    assert!(again.events.is_empty() && again.undo == Undo::None);
    run(&color(None), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.tracks[t].color, None);
    s.project.validate().unwrap();
    // Height (#490): kept, no-op when equal, undone exactly, refused outside the range.
    let height = |h: Option<u16>| {
        tc(track::Command::SetHeight {
            track: t.clone(),
            height: h,
        })
    };
    let set = run(&height(Some(96)), &mut s, &mut context()).unwrap();
    assert_eq!(s.project.tracks[t].height, Some(96));
    let again = run(&height(Some(96)), &mut s, &mut context()).unwrap();
    assert!(again.events.is_empty() && again.undo == Undo::None);
    let Undo::Inverse(inverse) = set.undo else {
        panic!("an inverse")
    };
    run(&inverse, &mut s, &mut context()).unwrap();
    assert_eq!(s.project.tracks[t].height, None);
    for bad in [27u16, 177, 0, u16::MAX] {
        let before = s.project.clone();
        let err = run(&height(Some(bad)), &mut s, &mut context()).unwrap_err();
        assert!(
            matches!(err, CommandError::Invalid { ref field, .. } if field == "height"),
            "{bad}: {err:?}"
        );
        assert_eq!(s.project, before);
    }
    for edge in [28u16, 176] {
        run(&height(Some(edge)), &mut s, &mut context()).unwrap();
    }
    s.project.validate().unwrap();
    assert_eq!(
        (::project::MIN_TRACK_HEIGHT, ::project::MAX_TRACK_HEIGHT),
        (28, 176),
        "the schema's range literals follow the constants"
    );
    // An unknown track.
    let mut ids = IdGenerator::new(1_730_000_000_000, 3);
    let ghost = TrackId::generate(&mut ids);
    for command in [
        tc(track::Command::Rename {
            track: ghost.clone(),
            name: "x".into(),
        }),
        tc(track::Command::SetColor {
            track: ghost.clone(),
            color: None,
        }),
        tc(track::Command::SetHeight {
            track: ghost.clone(),
            height: None,
        }),
        mv(&ghost, 0),
        tc(track::Command::RestoreOrder {
            track: ghost.clone(),
            order: OrderKey::first(),
        }),
        tc(track::Command::Duplicate {
            track: ghost.clone(),
            name: None,
        }),
    ] {
        assert_eq!(refused(&command, &f.state).code(), "not_found");
    }
}

#[test]
fn a_colour_is_parsed_as_rrggbb_and_nothing_else() {
    for bad in ["red", "#fff", "#gggggg", "ff0000", "#ff00000"] {
        let json = format!(
            r##"{{"type":"track.set_color","track":"01ARZ3NDEKTSV4RRFFQ69G5FAV","color":"{bad}"}}"##
        );
        assert!(serde_json::from_str::<Command>(&json).is_err(), "{bad}");
    }
    let missing = r#"{"type":"track.set_color","track":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}"#;
    assert!(serde_json::from_str::<Command>(missing).is_err());
    let missing = r#"{"type":"track.set_height","track":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}"#;
    assert!(serde_json::from_str::<Command>(missing).is_err());
}

#[test]
fn a_duplicate_keeps_the_height_of_its_source() {
    let mut s = commands::testing::state();
    let mut c = context();
    let (source, _) = created(&run(&create(TrackKind::Midi), &mut s, &mut c).unwrap());
    run(
        &tc(track::Command::SetHeight {
            track: source.clone(),
            height: Some(120),
        }),
        &mut s,
        &mut c,
    )
    .unwrap();
    let out = run(
        &tc(track::Command::Duplicate {
            track: source.clone(),
            name: None,
        }),
        &mut s,
        &mut c,
    )
    .unwrap();
    let copies: Vec<_> = s
        .project
        .tracks
        .iter()
        .filter(|(id, _)| **id != source)
        .collect();
    assert_eq!(copies.len(), 1, "{out:?}");
    assert_eq!(copies[0].1.height, Some(120));
}

#[test]
fn move_places_the_track_and_undo_restores_the_exact_key() {
    let mut s = commands::testing::state();
    let mut c = context();
    let tracks: Vec<TrackId> = (0..5)
        .map(|_| created(&run(&create(TrackKind::Midi), &mut s, &mut c).unwrap()).0)
        .collect();
    let start = order(&s);
    assert_eq!(start, tracks);
    // To the front, to the middle, past the end (clamped), and from the end.
    for (track, index, expected) in [
        (&tracks[4], 0, vec![4, 0, 1, 2, 3]),
        (&tracks[0], 2, vec![4, 1, 0, 2, 3]),
        (&tracks[4], 99, vec![1, 0, 2, 3, 4]),
        (&tracks[3], 1, vec![1, 3, 0, 2, 4]),
    ] {
        let before = s.clone();
        let out = run(&mv(track, index), &mut s, &mut c).unwrap();
        assert_eq!(
            order(&s),
            expected
                .iter()
                .map(|&i| tracks[i].clone())
                .collect::<Vec<_>>()
        );
        s.project.validate().unwrap();
        // Undo through the history puts back the exact order key, not just the order.
        run(&undo(), &mut s, &mut c).unwrap();
        assert_eq!(s.project, before.project);
        run(&redo(), &mut s, &mut c).unwrap();
        assert!(!out.events.is_empty());
    }
}

#[test]
fn moving_to_the_current_place_changes_nothing() {
    let mut s = commands::testing::state();
    let mut c = context();
    let tracks: Vec<TrackId> = (0..3)
        .map(|_| created(&run(&create(TrackKind::Midi), &mut s, &mut c).unwrap()).0)
        .collect();
    for (i, t) in tracks.iter().enumerate() {
        let out = run(&mv(t, i as u32), &mut s, &mut c).unwrap();
        assert!(out.events.is_empty() && out.undo == Undo::None);
        let Output::Track(track::Output::Move { previous_index }) = out.output else {
            panic!()
        };
        assert_eq!(previous_index as usize, i);
    }
    // The last, by an index past the end.
    let out = run(&mv(&tracks[2], 50), &mut s, &mut c).unwrap();
    assert!(out.events.is_empty());
}

#[test]
fn move_separates_tracks_that_share_an_order_key() {
    // Two branches made the same key (the fixture's tracks): ids break the tie, and a
    // move still lands where it was asked.
    let f = fx();
    let before = order(&f.state);
    assert_eq!(before.len(), 2);
    let mut s = f.state.clone();
    run(&mv(&before[0], 1), &mut s, &mut context()).unwrap();
    assert_eq!(order(&s), [before[1].clone(), before[0].clone()]);
    s.project.validate().unwrap();
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, f.state.project);
}

#[test]
fn restore_order_refuses_a_bad_key_when_read_and_is_a_no_op_on_the_same_key() {
    let f = fx();
    let json = format!(
        r#"{{"type":"track.restore_order","track":"{}","order":"!"}}"#,
        f.track
    );
    assert!(serde_json::from_str::<Command>(&json).is_err());
    let same = tc(track::Command::RestoreOrder {
        track: f.track.clone(),
        order: f.state.project.tracks[&f.track].order.clone(),
    });
    let mut s = f.state.clone();
    let out = run(&same, &mut s, &mut context()).unwrap();
    assert!(out.events.is_empty() && out.undo == Undo::None);
}

#[test]
fn duplicate_copies_next_to_the_source_with_fresh_identifiers_everywhere() {
    let mut f = fx();
    // An automation lane on the MIDI track's synth, so that the copy must remap it.
    let device = f.state.project.tracks[&f.track]
        .devices
        .keys()
        .next()
        .unwrap()
        .clone();
    let mut gen_ids = IdGenerator::new(1_740_000_000_000, 4);
    f.state
        .project
        .tracks
        .get_mut(&f.track)
        .unwrap()
        .automation
        .insert(
            LaneId::generate(&mut gen_ids),
            AutomationLane {
                target: AutomationTarget::Device {
                    device: device.clone(),
                    param: "attack".into(),
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
    f.state.project.validate().unwrap();
    // The fixture's two tracks share an order key; give the audio one a later key so that
    // "right after the source" is distinguishable from "last".
    let mut s = f.state.clone();
    let first = OrderKey::first();
    s.project.tracks.get_mut(&f.audio_track).unwrap().order =
        OrderKey::between(Some(&first), None).unwrap();
    let mut c = context();
    let source = order(&s)[0].clone();
    let out = run(
        &tc(track::Command::Duplicate {
            track: source.clone(),
            name: None,
        }),
        &mut s,
        &mut c,
    )
    .unwrap();
    let Output::Track(track::Output::Duplicate { track: copy }) = out.output else {
        panic!()
    };
    s.project.validate().unwrap();
    let now = order(&s);
    assert_eq!(now.len(), 3);
    let at = now.iter().position(|t| *t == source).unwrap();
    assert_eq!(now[at + 1], copy);
    // Every identifier of the project is still unique, and the copy shares none.
    let all = ids(&s);
    assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), all.len());
    let (a, b) = (&s.project.tracks[&source], &s.project.tracks[&copy]);
    assert_eq!(a.name, b.name);
    assert_eq!(a.mixer, b.mixer);
    assert_eq!(a.kind, b.kind);
    assert_eq!(a.devices.len(), b.devices.len());
    assert_eq!(a.clips.len(), b.clips.len());
    assert_eq!(a.placements.len(), b.placements.len());
    // Params, clip content and placements are equal; the placement names the copy's clip.
    assert_eq!(
        a.devices.values().collect::<Vec<_>>(),
        b.devices.values().collect::<Vec<_>>()
    );
    for p in b.placements.values() {
        assert!(b.clips.contains_key(&p.clip) && !a.clips.contains_key(&p.clip));
    }
    // The lane follows the copy's own device.
    let lane = b.automation.values().next().unwrap();
    let AutomationTarget::Device { device: target, .. } = &lane.target else {
        panic!()
    };
    assert!(b.devices.contains_key(target) && !a.devices.contains_key(target));
    // Independent: editing a clip of the copy leaves the source's alone.
    let (copy_clip, _) = b.clips.iter().next().unwrap();
    let source_before = s.project.tracks[&source].clone();
    run(
        &Command::Clip(commands::clip::Command::Rename {
            clip: copy_clip.clone(),
            name: "changed".into(),
        }),
        &mut s,
        &mut c,
    )
    .unwrap();
    assert_eq!(s.project.tracks[&source], source_before);
}

#[test]
fn duplicate_takes_an_optional_name_and_undo_deletes_the_copy() {
    let f = fx();
    let name = |n: Option<&str>| {
        tc(track::Command::Duplicate {
            track: f.audio_track.clone(),
            name: n.map(Into::into),
        })
    };
    let mut s = f.state.clone();
    let out = run(&name(Some("  Drums 2 ")), &mut s, &mut context()).unwrap();
    let Output::Track(track::Output::Duplicate { track: copy }) = out.output else {
        panic!()
    };
    assert_eq!(s.project.tracks[&copy].name, "Drums 2");
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, f.state.project);
    assert_eq!(
        refused(&name(Some(&"x".repeat(201))), &f.state).code(),
        "invalid"
    );
}

#[test]
fn select_track_refuses_a_track_that_does_not_exist_and_always_lets_you_deselect() {
    let f = fixture();
    let mut ids = IdGenerator::new(1_750_000_000_000, 5);
    let ghost = TrackId::generate(&mut ids);
    let select = |t: Option<TrackId>| Command::View(view::Command::SelectTrack { track: t });
    let e = refused(&select(Some(ghost.clone())), &f.state);
    assert!(
        matches!(e, CommandError::NotFound { ref entity, ref id } if entity == "track" && *id == ghost.to_string())
    );
    // A selection gone stale (the track was deleted behind it) can still be cleared.
    let mut stale = f.state.clone();
    stale.session.selected_track = Some(ghost);
    run(&select(None), &mut stale, &mut context()).unwrap();
    assert_eq!(stale.session.selected_track, None);
    // And a real track is selected.
    let mut s = f.state.clone();
    run(&select(Some(f.track.clone())), &mut s, &mut context()).unwrap();
    assert_eq!(s.session.selected_track, Some(f.track));
}

#[test]
fn every_example_of_the_domain_meets_the_contract_and_is_valid() {
    for (command, state) in track_examples::examples() {
        check(&command, &state, &context()).unwrap_or_else(|e| panic!("{command:?}: {e}"));
        let mut after = state.clone();
        run(&command, &mut after, &mut context()).unwrap();
        after.project.validate().unwrap();
    }
}

#[test]
fn move_refuses_when_tied_keys_leave_no_room_instead_of_reporting_a_move() {
    // A and B share a key, C comes after: nothing fits between A and B.
    let f = fx();
    let mut s = f.state.clone();
    let mut c = context();
    let third = created(&run(&create(TrackKind::Midi), &mut s, &mut c).unwrap()).0;
    let now = order(&s);
    assert_eq!(now.len(), 3);
    assert_eq!(now[2], third);
    let (a, b) = (&now[0], &now[1]);
    assert_eq!(
        s.project.tracks[a].order, s.project.tracks[b].order,
        "the fixture ties its two tracks"
    );
    let e = refused(&mv(&third, 1), &s);
    assert!(matches!(e, CommandError::Invalid { ref field, .. } if field == "index"));
    // Moving to the end of the tied group, or to the front, still works.
    check(&mv(&third, 0), &s, &context()).unwrap();
    check(&mv(a, 2), &s, &context()).unwrap();
}

#[test]
fn duplicate_of_a_tied_track_goes_after_the_whole_tied_group() {
    let f = fx();
    let before = order(&f.state);
    let mut s = f.state.clone();
    let out = run(
        &tc(track::Command::Duplicate {
            track: before[0].clone(),
            name: None,
        }),
        &mut s,
        &mut context(),
    )
    .unwrap();
    let Output::Track(track::Output::Duplicate { track: copy }) = out.output else {
        panic!()
    };
    assert_eq!(order(&s)[2], copy);
    s.project.validate().unwrap();
}

#[test]
fn restore_refuses_a_device_the_registry_does_not_have_and_leaves_the_state_alone() {
    let f = fx();
    let mut gone = f.state.clone();
    let snapshot = gone.project.tracks.remove(&f.track).unwrap();
    let restore = |snapshot: Track| {
        tc(track::Command::Restore {
            track: f.track.clone(),
            snapshot,
        })
    };
    let device = snapshot.devices.keys().next().unwrap().clone();
    let broken = |edit: &dyn Fn(&mut ::project::Device)| {
        let mut t = snapshot.clone();
        edit(t.devices.get_mut(&device).unwrap());
        t
    };
    // An unknown built-in device.
    let unknown = broken(&|d| {
        d.source = ::project::DeviceSource::Builtin {
            id: "faulk.nope".into(),
            version: 9,
        }
    });
    let e = refused(&restore(unknown), &gone);
    assert!(matches!(e, CommandError::Invalid { ref field, .. } if field == "snapshot"));
    // A parameter the synth does not declare.
    let extra = broken(&|d| {
        d.params.insert("cutoff".into(), 1.0);
    });
    let e = refused(&restore(extra), &gone);
    assert!(
        matches!(e, CommandError::Invalid { ref field, ref message } if field == "snapshot" && message.contains("cutoff"))
    );
    // The clip player takes no parameter.
    let mut audio = f.state.clone();
    let audio_snapshot = audio.project.tracks.remove(&f.audio_track).unwrap();
    let mut with_param = audio_snapshot.clone();
    with_param
        .devices
        .values_mut()
        .next()
        .unwrap()
        .params
        .insert("gain".into(), 1.0);
    let command = tc(track::Command::Restore {
        track: f.audio_track.clone(),
        snapshot: with_param,
    });
    assert_eq!(refused(&command, &audio).code(), "invalid");
    // The honest ones apply.
    check(&restore(snapshot.clone()), &gone, &context()).unwrap();
    let honest = tc(track::Command::Restore {
        track: f.audio_track.clone(),
        snapshot: audio_snapshot,
    });
    check(&honest, &audio, &context()).unwrap();
}

#[test]
fn delete_then_undo_keeps_the_automation_lanes_of_the_track() {
    let mut f = fx();
    let device = f.state.project.tracks[&f.track]
        .devices
        .keys()
        .next()
        .unwrap()
        .clone();
    let mut gen_ids = IdGenerator::new(1_770_000_000_000, 7);
    let lane = LaneId::generate(&mut gen_ids);
    f.state
        .project
        .tracks
        .get_mut(&f.track)
        .unwrap()
        .automation
        .insert(
            lane.clone(),
            AutomationLane {
                target: AutomationTarget::Device {
                    device,
                    param: "attack".into(),
                },
                points: BTreeMap::from([(
                    Tick(0),
                    AutomationPoint {
                        value: 0.5,
                        shape: Shape::Step,
                    },
                )]),
            },
        );
    f.state.project.validate().unwrap();
    let mut s = f.state.clone();
    run(&delete(&f.track), &mut s, &mut context()).unwrap();
    run(&undo(), &mut s, &mut context()).unwrap();
    assert_eq!(s.project, f.state.project);
    assert!(s.project.tracks[&f.track].automation.contains_key(&lane));
}

/// The colours of the tracks, in display order.
fn colors(state: &State) -> Vec<Option<::project::Rgb>> {
    order(state)
        .iter()
        .map(|id| state.project.tracks[id].color)
        .collect()
}

/// #479: a created track (agent: no colour given) and a duplicate store a hue of the
/// palette at creation; reordering, deleting and duplicating then change no other colour,
/// and each step undoes to the identical state.
#[test]
fn a_track_keeps_its_stored_colour_whatever_happens_to_the_others() {
    use ::project::colors::{TRACK_PALETTE, palette_hue};
    let mut s = commands::testing::state();
    let mut ctx = context();
    let mut ids = Vec::new();
    for _ in 0..5 {
        let out = run(&create(TrackKind::Midi), &mut s, &mut ctx).unwrap();
        ids.push(created(&out).0);
    }
    let hues: Vec<_> = (0..5).map(|i| Some(palette_hue(i))).collect();
    assert_eq!(colors(&s), hues, "the next free hue, in turn");

    let color_of = |s: &State, id: &TrackId| s.project.tracks[id].color;
    let before: Vec<_> = ids.iter().map(|id| color_of(&s, id)).collect();

    // Reorder.
    check(&mv(&ids[4], 0), &s, &ctx).unwrap();
    run(&mv(&ids[4], 0), &mut s, &mut ctx).unwrap();
    for (id, was) in ids.iter().zip(&before) {
        assert_eq!(&color_of(&s, id), was);
    }
    // Delete, and undo.
    check(&delete(&ids[1]), &s, &ctx).unwrap();
    run(&delete(&ids[1]), &mut s, &mut ctx).unwrap();
    for (i, (id, was)) in ids.iter().zip(&before).enumerate() {
        if i != 1 {
            assert_eq!(&color_of(&s, id), was);
        }
    }
    // The freed hue is the next one handed out.
    let out = run(&create(TrackKind::Audio), &mut s, &mut ctx).unwrap();
    assert_eq!(color_of(&s, &created(&out).0), before[1]);

    // A duplicate keeps the source's colour; the source and the others keep theirs.
    let dup = tc(track::Command::Duplicate {
        track: ids[0].clone(),
        name: None,
    });
    let snapshot: Vec<_> = ids
        .iter()
        .filter(|id| s.project.tracks.contains_key(*id))
        .map(|id| color_of(&s, id))
        .collect();
    check(&dup, &s, &ctx).unwrap();
    let out = run(&dup, &mut s, &mut ctx).unwrap();
    let Output::Track(track::Output::Duplicate { track: copy }) = out.output else {
        panic!()
    };
    assert_eq!(
        color_of(&s, &copy),
        color_of(&s, &ids[0]),
        "a copy keeps its source's colour"
    );
    let after: Vec<_> = ids
        .iter()
        .filter(|id| s.project.tracks.contains_key(*id))
        .map(|id| color_of(&s, id))
        .collect();
    assert_eq!(after, snapshot);

    // Past the sixteen hues, the least used comes first; an explicit colour is kept.
    for _ in 0..TRACK_PALETTE.len() {
        run(&create(TrackKind::Midi), &mut s, &mut ctx).unwrap();
    }
    s.project.validate().unwrap();
    let explicit = tc(track::Command::Create {
        kind: TrackKind::Midi,
        instrument: None,
        name: None,
        color: Some(::project::Rgb::new(1, 2, 3)),
    });
    let out = run(&explicit, &mut s, &mut ctx).unwrap();
    assert_eq!(
        color_of(&s, &created(&out).0),
        Some(::project::Rgb::new(1, 2, 3))
    );
}
