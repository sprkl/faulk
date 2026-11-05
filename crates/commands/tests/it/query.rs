//! PR-6: the query commands. A query changes nothing (state, context, history, engine), lists
//! are paged by one rule, and every answer is typed and documented.

use ::project::{Note, Tick};
use commands::testing::{check_refusal, context};
use commands::{
    Command, CommandError, Context, Kind, Output, State, automation, catalog, clip, device,
    project, run, scene, track, transport,
};

use crate::automation_examples;
use crate::clip_examples;
use crate::device_examples;
use crate::query_examples;
use clip_examples::fx;

fn q(c: Command, state: &State) -> Result<Output, CommandError> {
    let mut s = state.clone();
    run(&c, &mut s, &mut context()).map(|o| o.output)
}

fn notes_list(clip: &::project::ClipId, offset: Option<u32>, limit: Option<u32>) -> Command {
    Command::Clip(clip::Command::NotesList {
        clip: clip.clone(),
        offset,
        limit,
    })
}

/// The fixture with 250 more notes in its first clip (through the command).
fn many_notes() -> (clip_examples::ClipFx, State) {
    let f = fx();
    let mut state = f.state.clone();
    let notes: Vec<Note> = (0..250)
        .map(|i| Note::new(Tick(i * 10), 40 + (i % 40) as u8, Tick(5), 100))
        .collect();
    run(
        &Command::Clip(clip::Command::NotesAdd {
            clip: f.clip.clone(),
            notes,
        }),
        &mut state,
        &mut context(),
    )
    .unwrap();
    (f, state)
}

#[test]
fn a_query_changes_nothing_not_even_the_context_the_history_or_the_engine() {
    let mut queries = 0;
    for (command, state) in query_examples::examples() {
        assert_eq!(command.meta().kind, Kind::Query, "{}", command.name());
        let (mut s, mut ctx) = (state.clone(), context());
        let before_ctx: Context = ctx.clone();
        let out = run(&command, &mut s, &mut ctx).unwrap();
        assert_eq!(s, state, "{}: the state changed", command.name());
        assert_eq!(ctx, before_ctx, "{}: the context changed", command.name());
        assert!(
            out.events.is_empty(),
            "{}: it reported events",
            command.name()
        );
        assert!(
            out.control.is_empty(),
            "{}: it sent engine messages",
            command.name()
        );
        assert!(
            matches!(out.undo, commands::Undo::None),
            "{}",
            command.name()
        );
        assert_eq!(s.history, state.history, "{}", command.name());
        queries += 1;
    }
    assert!(queries >= 20);
}

/// The queries the host answers (their numbers come from the page): `midi.list_inputs`,
/// `project.list_stored`, and the two that measure levels (`mixer.levels`,
/// `project.analyze_render`, #665). Their contract is checked in `contract.rs` and `levels.rs`.
const HOST_QUERIES: [&str; 4] = [
    "midi.list_inputs",
    "project.list_stored",
    "mixer.levels",
    "project.analyze_render",
];

#[test]
fn every_query_command_is_covered_by_an_example() {
    let mut declared: Vec<_> = Command::metas()
        .iter()
        .filter(|m| m.kind == Kind::Query)
        .map(|m| m.name)
        .collect();
    let mut covered: Vec<_> = query_examples::examples()
        .iter()
        .map(|(c, _)| c.name())
        .collect();
    declared.sort();
    covered.sort();
    covered.dedup();
    // The two queries that predate PR-6 are the host's (`midi.list_inputs`, `project.list_stored`).
    let new: Vec<_> = declared
        .iter()
        .copied()
        .filter(|n| !HOST_QUERIES.contains(n))
        .collect();
    assert_eq!(new, covered);
    assert_eq!(
        new,
        [
            "arrangement.marker.list",
            "automation.lane.list",
            "automation.lane.points",
            "clip.get",
            "clip.notes.list",
            "device.describe",
            "preset.list",
            "project.get",
            "scene.list",
            "track.get",
            "track.list",
            "transport.get",
        ]
    );
}

#[test]
fn notes_are_paged_by_one_rule() {
    let (f, state) = many_notes();
    let all: Vec<Note> = {
        let ::project::ClipContent::Midi { notes, .. } =
            &state.project.tracks[&f.track].clips[&f.clip].content
        else {
            panic!()
        };
        notes.iter().copied().collect()
    };
    assert_eq!(all.len(), 253);
    // Default page: 100, next at 100.
    let Output::Clip(clip::Output::NotesList { notes, page }) =
        q(notes_list(&f.clip, None, None), &state).unwrap()
    else {
        panic!()
    };
    assert_eq!(notes.len(), 100);
    assert_eq!(
        (page.total, page.offset, page.next_offset),
        (253, 0, Some(100))
    );
    // Walking the pages gives the whole list once, in order.
    let mut got = Vec::new();
    let mut offset = Some(0);
    let mut pages = 0;
    while let Some(o) = offset {
        let Output::Clip(clip::Output::NotesList { notes, page }) =
            q(notes_list(&f.clip, Some(o), Some(100)), &state).unwrap()
        else {
            panic!()
        };
        got.extend(notes);
        offset = page.next_offset;
        pages += 1;
    }
    assert_eq!((pages, got), (3, all));
    // Past the end: empty, no next.
    let Output::Clip(clip::Output::NotesList { notes, page }) =
        q(notes_list(&f.clip, Some(u32::MAX), Some(1000)), &state).unwrap()
    else {
        panic!()
    };
    assert!(notes.is_empty());
    assert_eq!((page.total, page.next_offset), (253, None));
    // Exactly at the end of a full page: no next.
    let Output::Clip(clip::Output::NotesList { page, .. }) =
        q(notes_list(&f.clip, Some(250), Some(3)), &state).unwrap()
    else {
        panic!()
    };
    assert_eq!(page.next_offset, None);
    // A limit outside 1..=1000 is refused, never clamped.
    for limit in [0, 1001, u32::MAX] {
        let err = q(notes_list(&f.clip, None, Some(limit)), &state).unwrap_err();
        assert!(
            matches!(err, CommandError::Invalid { ref field, .. } if field == "limit"),
            "{err:?}"
        );
    }
    assert!(q(notes_list(&f.clip, None, Some(1000)), &state).is_ok());
    assert!(q(notes_list(&f.clip, None, Some(1)), &state).is_ok());
}

#[test]
fn points_tracks_and_scenes_follow_the_same_rule() {
    let f = automation_examples::fx();
    let mut state = f.state.clone();
    let mut ctx = automation_examples::lane_context();
    let points: Vec<_> = (0..300)
        .map(|i| automation_examples::pt(10_000 + i * 10, -3.0, ::project::Shape::Linear))
        .collect();
    automation_examples::add(&mut state, &mut ctx, &f.gain_lane, points);
    let points = |offset, limit| {
        q(
            Command::Automation(automation::Command::LanePoints {
                lane: f.gain_lane.clone(),
                offset,
                limit,
            }),
            &state,
        )
    };
    let Output::Automation(automation::Output::LanePoints {
        lane,
        points: p,
        page,
    }) = points(Some(250), None).unwrap()
    else {
        panic!()
    };
    assert_eq!(lane.point_count, 303);
    assert_eq!(p.len(), 53);
    assert_eq!((page.total, page.next_offset), (303, None));
    // Points come in tick order.
    assert!(p.windows(2).all(|w| w[0].tick < w[1].tick));
    assert!(points(None, Some(0)).is_err());

    // Tracks and scenes, one at a time.
    let clips = fx();
    let Output::Track(track::Output::List { tracks, page }) = q(
        Command::Track(track::Command::List {
            offset: Some(0),
            limit: Some(1),
        }),
        &clips.state,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(
        (tracks.len(), page.total, page.next_offset),
        (1, 2, Some(1))
    );
    let Output::Scene(scene::Output::List { scenes, page }) = q(
        Command::Scene(scene::Command::List {
            offset: Some(1),
            limit: Some(1),
        }),
        &clips.state,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(scenes.len(), 1);
    assert_eq!(page.offset, 1);
}

#[test]
fn the_answers_say_what_the_project_holds() {
    let f = fx();
    let Output::Project(project::Output::Get {
        name,
        track_count,
        scene_count,
        clip_count,
        placement_count,
        can_undo,
        tempo_bpm,
        master_devices,
        ..
    }) = q(Command::Project(project::Command::Get {}), &f.state).unwrap()
    else {
        panic!()
    };
    assert_eq!(name, "Demo");
    assert_eq!((track_count, scene_count), (2, 3));
    assert_eq!(clip_count as usize, f.state.project.counts().clips);
    assert_eq!(
        placement_count as usize,
        f.state.project.counts().placements
    );
    assert!(can_undo);
    assert_eq!(tempo_bpm, 120.0);
    assert!(master_devices.is_empty());

    // The clip with a slot, the scene it fills, the placed clip's placement.
    let Output::Scene(scene::Output::List { scenes, .. }) = q(
        Command::Scene(scene::Command::List {
            offset: None,
            limit: None,
        }),
        &f.state,
    )
    .unwrap() else {
        panic!()
    };
    let a = scenes.iter().find(|s| s.id == f.scene_a).unwrap();
    assert!(
        a.clips
            .iter()
            .any(|slot| slot.clip == f.clip && slot.track == f.track)
    );
    let free = scenes.iter().find(|s| s.id == f.scene_free).unwrap();
    assert!(free.clips.is_empty());
    let Output::Clip(clip::Output::Get {
        placements,
        page,
        track,
        ..
    }) = q(
        Command::Clip(clip::Command::Get {
            clip: f.placed.clone(),
            offset: None,
            limit: None,
        }),
        &f.state,
    )
    .unwrap()
    else {
        panic!()
    };
    assert_eq!(track, f.track);
    assert_eq!(page.total, 1);
    assert_eq!(placements[0].id, f.placement);

    // The transport settings follow the edit.
    let mut s = f.state.clone();
    run(
        &Command::Transport(transport::Command::SetTempo {
            bpm: 90.0,
            placements: None,
        }),
        &mut s,
        &mut context(),
    )
    .unwrap();
    let Output::Transport(transport::Output::Get {
        tempo_bpm,
        numerator,
        loop_region,
        ..
    }) = q(Command::Transport(transport::Command::Get {}), &s).unwrap()
    else {
        panic!()
    };
    assert_eq!((tempo_bpm, numerator, loop_region), (90.0, 4, None));
}

#[test]
fn refusals_name_the_field_or_the_entity() {
    let f = fx();
    let ghost = ::project::ClipId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let err = q(notes_list(&ghost, None, None), &f.state).unwrap_err();
    assert!(matches!(err, CommandError::NotFound { ref entity, .. } if entity == "clip"));
    // An audio clip has no notes.
    let err = q(notes_list(&f.audio_clip, None, None), &f.state).unwrap_err();
    assert!(matches!(err, CommandError::Invalid { ref field, .. } if field == "clip"));
    // Exactly one of device and kind.
    let describe = |device, kind| {
        q(
            Command::Device(device::Command::Describe { device, kind }),
            &f.state,
        )
    };
    assert!(matches!(
        describe(None, None).unwrap_err(),
        CommandError::Invalid { .. }
    ));
    assert!(matches!(
        describe(
            Some(device_examples::chain().synth),
            Some(device::SYNTH_KIND.into())
        )
        .unwrap_err(),
        CommandError::Invalid { .. }
    ));
    assert!(matches!(
        describe(None, Some("faulk.nothing".into())).unwrap_err(),
        CommandError::NotFound { ref entity, .. } if entity == "device_kind"
    ));
    // A track and the master together.
    let lane_list = |track, master| {
        q(
            Command::Automation(automation::Command::LaneList {
                track,
                master,
                offset: None,
                limit: None,
            }),
            &f.state,
        )
    };
    assert!(matches!(
        lane_list(Some(f.track.clone()), true).unwrap_err(),
        CommandError::Invalid { .. }
    ));
    let ghost_track = ::project::TrackId::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    assert!(matches!(
        lane_list(Some(ghost_track), false).unwrap_err(),
        CommandError::NotFound { .. }
    ));
    // The harness agrees: a refused query changes nothing either.
    let c = notes_list(&ghost, None, None);
    check_refusal(&c, &f.state, &context()).unwrap();
}

#[test]
fn a_device_is_described_with_its_parameters_and_its_kind() {
    let f = automation_examples::fx();
    let Output::Device(device::Output::Describe {
        device,
        track,
        in_master,
        params,
        descriptor,
    }) = q(
        Command::Device(device::Command::Describe {
            device: Some(f.chain.synth.clone()),
            kind: None,
        }),
        &f.state,
    )
    .unwrap()
    else {
        panic!()
    };
    assert_eq!(device.unwrap().id, f.chain.synth);
    assert_eq!(track, Some(f.track.clone()));
    let d = descriptor.unwrap();
    assert_eq!(d.kind, device::SYNTH_KIND);
    for p in &d.params {
        assert!(!p.description.is_empty(), "{}", p.key);
        assert!(p.min <= p.default && p.default <= p.max, "{}", p.key);
        assert!(params.contains_key(&p.key), "{}", p.key);
    }
    // A master device has no track; the clip player has no descriptor.
    assert!(!in_master);
    let Output::Device(device::Output::Describe {
        track, in_master, ..
    }) = q(
        Command::Device(device::Command::Describe {
            device: Some(f.chain.ma.clone()),
            kind: None,
        }),
        &f.state,
    )
    .unwrap()
    else {
        panic!()
    };
    assert_eq!(track, None);
    assert!(in_master);
    let Output::Device(device::Output::Describe {
        descriptor, device, ..
    }) = q(
        Command::Device(device::Command::Describe {
            device: Some(f.chain.player.clone()),
            kind: None,
        }),
        &f.state,
    )
    .unwrap()
    else {
        panic!()
    };
    assert!(descriptor.is_none());
    assert!(device.unwrap().name.is_none());
    // The master's chain is listed by project.get.
    let Output::Project(project::Output::Get { master_devices, .. }) =
        q(Command::Project(project::Command::Get {}), &f.state).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        master_devices
            .iter()
            .map(|d| d.id.clone())
            .collect::<Vec<_>>(),
        vec![f.chain.ma.clone(), f.chain.mb.clone()]
    );
}

#[test]
fn every_field_of_every_query_answer_is_documented() {
    let tools: Vec<_> = catalog()
        .into_iter()
        .filter(|t| t.kind == Kind::Query && !HOST_QUERIES.contains(&t.name.as_str()))
        .collect();
    assert_eq!(tools.len(), 12);
    for tool in tools {
        assert!(!tool.description.is_empty(), "{}", tool.name);
        assert!(
            tool.description.to_lowercase().contains("changes nothing"),
            "{}",
            tool.name
        );
        for (what, schema) in [
            ("input", &tool.input_schema),
            ("output", &tool.output_schema),
        ] {
            for (field, def) in schema["properties"].as_object().unwrap() {
                assert!(
                    def.get("description").is_some_and(|d| d.is_string()),
                    "{}: {what} field `{field}` has no description",
                    tool.name
                );
            }
        }
    }
}

/// The types the query answers are made of (those of `crates/commands/src/query.rs` that a
/// schema names): every field of each is documented, nested ones included.
#[test]
fn every_field_of_the_types_of_the_answers_is_documented() {
    // The root's definitions: a tool's schema only holds the types it references (#559).
    let output = commands::output_schema();
    let defs = &output["$defs"];
    let mut checked = 0;
    for name in [
        "PageInfo",
        "TrackSummary",
        "DeviceSummary",
        "ClipSummary",
        "ClipContentInfo",
        "PlacementInfo",
        "SceneInfo",
        "SceneSlot",
        "LaneSummary",
        "ParamSpec",
        "KeyName",
        "DeviceKindInfo",
        "PresetEntry",
        "PresetRef",
        "DevicePreset",
    ] {
        let def = defs
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not in the schema"));
        // A struct has `properties`; the tagged enum has them in each `oneOf` variant.
        let mut objects = vec![def];
        objects.extend(def["oneOf"].as_array().into_iter().flatten());
        for object in objects {
            for (field, d) in object["properties"].as_object().into_iter().flatten() {
                if field == "kind" && d.get("const").is_some() {
                    continue; // the tag of an enum variant
                }
                assert!(
                    d.get("description").is_some_and(|x| x.is_string()),
                    "{name}.{field} has no description"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 40, "{checked}");
}
