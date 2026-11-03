//! #191: the preset commands. A save keeps every parameter of the device under a new id and
//! the device shows it; a load applies a preset in one undoable step (unknown values ignored,
//! out-of-range ones clamped, a newer device version refused, an older one migrated); names are
//! unique per device kind and location; factory presets are read-only; the library outlives
//! the project; every refusal leaves the state as it was.

use ::project::preset::{DevicePreset, PresetId, PresetLocation, factory as factory_presets};
use ::project::{DeviceId, DeviceSource};
use commands::preset::{DeviceRestore, PresetRef};
use commands::testing::{check_refusal, context};
use commands::{Command, CommandError, Output, ProjectEvent, State, history, preset, run};
use std::collections::BTreeMap;

mod clip_examples;
mod preset_examples;

use preset_examples::{ROUND_BASS, factory, fx, p, reference, synth_preset};

fn refused(command: &Command, state: &State) -> CommandError {
    check_refusal(command, state, &context()).unwrap()
}

fn invalid_field(command: &Command, state: &State) -> String {
    match refused(command, state) {
        CommandError::Invalid { field, .. } => field,
        other => panic!("{command:?}: {other:?}"),
    }
}

fn ran(state: &mut State, command: &Command) -> commands::Outcome {
    run(command, state, &mut context()).unwrap_or_else(|e| panic!("{command:?}: {e}"))
}

fn undo() -> Command {
    Command::History(history::Command::Undo {})
}

fn load(device: &DeviceId, preset: PresetRef) -> Command {
    p(preset::Command::Load {
        device: device.clone(),
        preset,
    })
}

fn save(device: &DeviceId, name: &str, location: PresetLocation) -> Command {
    p(preset::Command::Save {
        device: device.clone(),
        name: name.into(),
        category: None,
        location,
    })
}

fn list(state: &mut State, device: &DeviceId) -> preset::Output {
    let out = ran(
        state,
        &p(preset::Command::List {
            device: Some(device.clone()),
            kind: None,
        }),
    );
    let Output::Preset(out) = out.output else {
        panic!()
    };
    out
}

#[test]
fn a_load_sets_every_parameter_at_once_and_one_undo_takes_it_all_back() {
    let f = fx();
    let mut state = f.state.clone();
    let before = state.project.clone();
    let out = ran(&mut state, &load(&f.synth, factory(ROUND_BASS)));
    let bass = &factory_presets::get(ROUND_BASS).unwrap().preset;
    let device = state.project.device(&f.synth).unwrap();
    assert_eq!(device.params, bass.params);
    assert_eq!(
        device.preset,
        Some(Box::new(DevicePreset {
            location: PresetLocation::Factory,
            id: ROUND_BASS.into(),
            name: "Round Bass".into(),
        }))
    );
    // One event per changed value, then the device's preset.
    let changed: Vec<&str> = out
        .events
        .iter()
        .filter_map(|e| match e {
            ProjectEvent::DeviceParamChanged { param, .. } => Some(param.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(changed.len(), bass.params.len());
    let last = out
        .events
        .iter()
        .rfind(|e| !matches!(e, ProjectEvent::HistoryChanged { .. }));
    assert!(matches!(
        last,
        Some(ProjectEvent::DevicePresetChanged { .. })
    ));
    // The engine gets the values like any parameter change.
    ran(&mut state, &undo());
    assert_eq!(state.project, before);
    // Loading the same preset twice changes nothing the second time.
    ran(&mut state, &load(&f.synth, factory(ROUND_BASS)));
    let again = run(
        &load(&f.synth, factory(ROUND_BASS)),
        &mut state,
        &mut context(),
    )
    .unwrap();
    assert!(again.events.is_empty());
    assert!(matches!(again.undo, commands::Undo::None));
}

#[test]
fn a_preset_with_unknown_or_out_of_range_values_is_ignored_or_clamped_never_a_panic() {
    let f = fx();
    let mut state = f.state.clone();
    let mut odd = synth_preset("Odd", 99.0);
    odd.params.insert("cutoff".into(), 1000.0);
    odd.params.insert("waveform".into(), -3.0);
    odd.params.remove("gain");
    let id = PresetId::parse("01HZY00000000000000000PRS1").unwrap();
    state.library.insert(id.clone(), odd);
    let out = ran(
        &mut state,
        &load(&f.synth, reference(PresetLocation::Library, &id)),
    );
    let Output::Preset(preset::Output::Load {
        ignored,
        clamped,
        defaulted,
        migrated_from,
    }) = out.output
    else {
        panic!()
    };
    assert_eq!(ignored, ["cutoff"]);
    assert_eq!(clamped, ["waveform", "attack"]);
    assert!(defaulted.contains(&"gain".to_owned()));
    assert_eq!(migrated_from, None);
    let params = &state.project.device(&f.synth).unwrap().params;
    assert_eq!(params["attack"], 30.0);
    assert_eq!(params["waveform"], 0.0);
    assert_eq!(params["gain"], 0.8);
    assert!(!params.contains_key("cutoff"));
}

#[test]
fn a_preset_of_a_newer_device_version_or_of_another_device_is_refused_clearly() {
    let f = fx();
    let mut state = f.state.clone();
    let mut newer = synth_preset("Future", 0.1);
    newer.version = 7;
    let id = PresetId::parse("01HZY00000000000000000PRS2").unwrap();
    state.library.insert(id.clone(), newer);
    match refused(
        &load(&f.synth, reference(PresetLocation::Library, &id)),
        &state,
    ) {
        CommandError::Invalid { field, message } => {
            assert_eq!(field, "preset");
            assert!(
                message.contains("version 7") && message.contains("version 1"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        invalid_field(
            &load(&f.synth, factory("faulk.filter/warm-lowpass")),
            &state
        ),
        "preset"
    );
    // The list shows it as one this Faulk cannot load.
    let preset::Output::List { presets, .. } = list(&mut state, &f.synth) else {
        panic!()
    };
    let entry = presets.iter().find(|e| e.name == "Future").unwrap();
    assert!(entry.newer);
}

#[test]
fn every_refusal_leaves_the_state_as_it_was() {
    let f = fx();
    let s = &f.state;
    let synth = &f.synth;
    let warm = reference(PresetLocation::Project, &f.warm);
    let bright = reference(PresetLocation::Library, &f.bright);
    let unknown = reference(
        PresetLocation::Library,
        &PresetId::parse("01HZY00000000000000000NXNE").unwrap(),
    );
    let ghost = DeviceId::parse("01HZY00000000000000000GH0S").unwrap();
    let not_found = |c: &Command| assert_eq!(refused(c, s).code(), "not_found", "{c:?}");

    // Save.
    assert_eq!(
        invalid_field(&save(synth, "X", PresetLocation::Factory), s),
        "location"
    );
    assert_eq!(
        invalid_field(&save(synth, "  ", PresetLocation::Project), s),
        "name"
    );
    assert_eq!(
        invalid_field(&save(synth, &"x".repeat(201), PresetLocation::Project), s),
        "name"
    );
    assert_eq!(
        invalid_field(&save(synth, " warm ", PresetLocation::Project), s),
        "name"
    );
    assert_eq!(
        invalid_field(&save(synth, "BRIGHT", PresetLocation::Library), s),
        "name"
    );
    assert_eq!(
        invalid_field(
            &p(preset::Command::Save {
                device: synth.clone(),
                name: "Ok".into(),
                category: Some(" ".into()),
                location: PresetLocation::Project,
            }),
            s
        ),
        "category"
    );
    not_found(&save(&ghost, "X", PresetLocation::Project));
    // A device without presets: the clip player (an audio track's instrument).
    let mut audio = s.clone();
    let player = DeviceId::parse("01HZY00000000000000000PXAY").unwrap();
    let mut track = ::project::Track::new(
        ::project::TrackKind::Audio,
        "Audio",
        ::project::OrderKey::between(Some(&::project::OrderKey::first()), None).unwrap(),
    );
    track.color = Some(::project::colors::palette_hue(1));
    track.devices.insert(
        player.clone(),
        ::project::Device {
            order: ::project::OrderKey::first(),
            role: ::project::DeviceRole::Instrument,
            enabled: true,
            source: DeviceSource::Builtin {
                id: ::project::CLIP_PLAYER.into(),
                version: 1,
            },
            params: BTreeMap::new(),
            state: None,
            preset: None,
        },
    );
    audio.project.tracks.insert(
        ::project::TrackId::parse("01HZY00000000000000000TRAK").unwrap(),
        track,
    );
    audio.project.validate().unwrap();
    assert_eq!(
        invalid_field(&save(&player, "X", PresetLocation::Project), &audio),
        "device"
    );
    assert_eq!(
        invalid_field(&load(&player, factory(ROUND_BASS)), &audio),
        "device"
    );

    // Load.
    not_found(&load(synth, unknown.clone()));
    not_found(&load(synth, factory("faulk.osc_synth/none")));
    not_found(&load(&ghost, factory(ROUND_BASS)));

    // Delete, rename, copy.
    let delete = |preset: PresetRef| p(preset::Command::Delete { preset });
    let rename = |preset: PresetRef, name: &str| {
        p(preset::Command::Rename {
            preset,
            name: name.into(),
        })
    };
    let copy = |preset: PresetRef, location, name: Option<&str>| {
        p(preset::Command::Copy {
            preset,
            location,
            name: name.map(str::to_owned),
        })
    };
    assert_eq!(invalid_field(&delete(factory(ROUND_BASS)), s), "location");
    not_found(&delete(unknown.clone()));
    assert_eq!(
        invalid_field(
            &delete(PresetRef {
                location: PresetLocation::Project,
                id: "../x".into()
            }),
            s
        ),
        "preset"
    );
    assert_eq!(
        invalid_field(&rename(factory(ROUND_BASS), "X"), s),
        "location"
    );
    assert_eq!(invalid_field(&rename(warm.clone(), ""), s), "name");
    not_found(&rename(unknown.clone(), "X"));
    assert_eq!(
        invalid_field(&copy(warm.clone(), PresetLocation::Factory, None), s),
        "location"
    );
    // The library already has nothing called Warm, but the project does.
    assert_eq!(
        invalid_field(&copy(warm.clone(), PresetLocation::Project, None), s),
        "name"
    );
    assert_eq!(
        invalid_field(
            &copy(bright.clone(), PresetLocation::Library, Some("bright")),
            s
        ),
        "name"
    );
    not_found(&copy(unknown.clone(), PresetLocation::Project, None));

    // List.
    assert_eq!(
        invalid_field(
            &p(preset::Command::List {
                device: Some(synth.clone()),
                kind: Some("faulk.filter".into()),
            }),
            s
        ),
        "kind"
    );
    not_found(&p(preset::Command::List {
        device: Some(ghost.clone()),
        kind: None,
    }));

    // Restore.
    let restore = |preset: PresetRef, snapshot, device| {
        p(preset::Command::Restore {
            preset,
            snapshot,
            device,
        })
    };
    assert_eq!(
        invalid_field(&restore(factory(ROUND_BASS), None, None), s),
        "location"
    );
    assert_eq!(
        invalid_field(
            &restore(
                PresetRef {
                    location: PresetLocation::Library,
                    id: "not-an-id".into()
                },
                None,
                None
            ),
            s
        ),
        "preset"
    );
    let mut bad = synth_preset("Bad", 0.1);
    bad.name = " Bad".into();
    assert_eq!(
        invalid_field(&restore(unknown.clone(), Some(bad), None), s),
        "name"
    );
    // An id the project uses for something else.
    let taken = reference(PresetLocation::Project, &PresetId(f.track.0.clone()));
    assert_eq!(
        invalid_field(&restore(taken, Some(synth_preset("T", 0.1)), None), s),
        "preset"
    );
    not_found(&restore(
        warm.clone(),
        None,
        Some(DeviceRestore {
            device: ghost.clone(),
            preset: None,
        }),
    ));
    assert_eq!(
        invalid_field(
            &restore(
                warm.clone(),
                None,
                Some(DeviceRestore {
                    device: synth.clone(),
                    preset: Some(Box::new(DevicePreset {
                        location: PresetLocation::Library,
                        id: "../../escape".into(),
                        name: "X".into(),
                    })),
                }),
            ),
            s
        ),
        "device"
    );

    // Restore a device's values.
    let restore_device = |params: BTreeMap<String, f64>, preset| {
        p(preset::Command::RestoreDevice {
            device: synth.clone(),
            params,
            preset,
        })
    };
    not_found(&restore_device(
        BTreeMap::from([("cutoff".into(), 1.0)]),
        None,
    ));
    assert_eq!(
        invalid_field(
            &restore_device(BTreeMap::from([("gain".into(), f64::NAN)]), None),
            s
        ),
        "params"
    );
    assert_eq!(
        invalid_field(
            &restore_device(
                BTreeMap::new(),
                Some(Box::new(DevicePreset {
                    location: PresetLocation::Factory,
                    id: "no-slash".into(),
                    name: "X".into(),
                }))
            ),
            s
        ),
        "preset"
    );
    not_found(&p(preset::Command::RestoreDevice {
        device: ghost,
        params: BTreeMap::new(),
        preset: None,
    }));
}

#[test]
fn a_full_location_is_refused_with_limit() {
    let f = fx();
    let mut state = f.state.clone();
    let mut ids = ::project::IdGenerator::new(1_630_000_000_000, 7);
    while state.project.presets.len() < ::project::preset::MAX_PROJECT_PRESETS {
        let name = format!("P{}", state.project.presets.len());
        state
            .project
            .presets
            .insert(PresetId::generate(&mut ids), synth_preset(&name, 0.1));
    }
    assert_eq!(
        refused(&save(&f.synth, "One more", PresetLocation::Project), &state).code(),
        "limit"
    );
    assert_eq!(
        refused(
            &p(preset::Command::Copy {
                preset: factory(ROUND_BASS),
                location: PresetLocation::Project,
                name: None,
            }),
            &state
        )
        .code(),
        "limit"
    );
    // The library is not full: a save there passes.
    ran(
        &mut state,
        &save(&f.synth, "One more", PresetLocation::Library),
    );
}

#[test]
fn a_save_keeps_every_parameter_and_the_device_then_shows_the_preset_unmodified() {
    let f = fx();
    let mut state = f.state.clone();
    ran(
        &mut state,
        &p(preset::Command::RestoreDevice {
            device: f.synth.clone(),
            params: BTreeMap::from([("attack".into(), 1.5)]),
            preset: Some(Box::new(DevicePreset {
                location: PresetLocation::Project,
                id: f.warm.to_string(),
                name: "Warm".into(),
            })),
        }),
    );
    let out = ran(&mut state, &save(&f.synth, "Slow", PresetLocation::Library));
    let Output::Preset(preset::Output::Save { preset: saved }) = out.output else {
        panic!()
    };
    let id = PresetId::parse(&saved.id).unwrap();
    let kept = &state.library[&id];
    assert_eq!(kept.params, state.project.device(&f.synth).unwrap().params);
    assert_eq!(kept.device, "faulk.osc_synth");
    let preset::Output::List {
        current, modified, ..
    } = list(&mut state, &f.synth)
    else {
        panic!()
    };
    assert_eq!(current.unwrap().name, "Slow");
    assert_eq!(modified, Some(false));
    // A knob turned: modified.
    ran(
        &mut state,
        &Command::Device(commands::device::Command::SetParam {
            device: f.synth.clone(),
            param: "sustain".into(),
            value: 0.1,
        }),
    );
    let preset::Output::List { modified, .. } = list(&mut state, &f.synth) else {
        panic!()
    };
    assert_eq!(modified, Some(true));
    // Its undo takes the preset out of the library and gives the device its preset back.
    ran(&mut state, &undo());
    ran(&mut state, &undo());
    assert!(!state.library.contains_key(&id));
    assert_eq!(
        state
            .project
            .device(&f.synth)
            .unwrap()
            .preset
            .as_ref()
            .unwrap()
            .name,
        "Warm"
    );
}

#[test]
fn the_list_puts_factory_then_project_then_library_by_name_and_says_what_is_unknown() {
    let f = fx();
    let mut state = f.state.clone();
    let preset::Output::List {
        presets,
        current,
        modified,
    } = list(&mut state, &f.synth)
    else {
        panic!()
    };
    let shown: Vec<(PresetLocation, &str)> = presets
        .iter()
        .map(|e| (e.preset.location, e.name.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            (PresetLocation::Factory, "Glass Pluck"),
            (PresetLocation::Factory, "Round Bass"),
            (PresetLocation::Factory, "Soft Pad"),
            (PresetLocation::Factory, "Square Lead"),
            (PresetLocation::Project, "Warm"),
            (PresetLocation::Library, "Bright"),
        ]
    );
    assert!(presets[0].read_only && !presets[4].read_only);
    // Every entry holds a value for every parameter of the synth.
    assert!(presets.iter().all(|e| e.params.len() == 6));
    assert_eq!(current.unwrap().name, "Warm");
    // The synth's values are not Warm's (attack 0.5, square wave): modified.
    assert_eq!(modified, Some(true));
    // A preset the device shows that is not here (a library preset of another computer).
    state.library.clear();
    state.project.device_mut(&f.synth).unwrap().preset = Some(Box::new(DevicePreset {
        location: PresetLocation::Library,
        id: f.bright.to_string(),
        name: "Bright".into(),
    }));
    let preset::Output::List { modified, .. } = list(&mut state, &f.synth) else {
        panic!()
    };
    assert_eq!(modified, None);
    // Every preset, every kind.
    let all = ran(
        &mut state,
        &p(preset::Command::List {
            device: None,
            kind: None,
        }),
    );
    let Output::Preset(preset::Output::List { presets, .. }) = all.output else {
        panic!()
    };
    assert_eq!(presets.len(), factory_presets::all().len() + 1);
}

#[test]
fn the_library_outlives_the_project_and_its_changes_are_not_project_changes() {
    let f = fx();
    let mut state = f.state.clone();
    let out = ran(&mut state, &save(&f.synth, "Kept", PresetLocation::Library));
    let library = state.library.clone();
    // The library event does not mark the project as changed; the device's does.
    let marks: Vec<bool> = out
        .events
        .iter()
        .filter(|e| !matches!(e, ProjectEvent::HistoryChanged { .. }))
        .map(ProjectEvent::changes_project)
        .collect();
    assert_eq!(marks, [false, true]);
    ran(
        &mut state,
        &Command::Project(commands::project::Command::New { name: None }),
    );
    assert_eq!(state.library, library);
    ran(
        &mut state,
        &Command::Project(commands::project::Command::Open {
            folder: commands::testing::OPENED_FOLDER.into(),
        }),
    );
    assert_eq!(state.library, library);
    // A project preset is a project change.
    let copy = ran(
        &mut state,
        &p(preset::Command::Copy {
            preset: factory(ROUND_BASS),
            location: PresetLocation::Project,
            name: None,
        }),
    );
    assert!(
        copy.events
            .iter()
            .filter(|e| !matches!(e, ProjectEvent::HistoryChanged { .. }))
            .all(ProjectEvent::changes_project)
    );
}

#[test]
fn a_copy_goes_into_the_library_without_touching_the_devices_and_names_stay_unique() {
    let f = fx();
    let mut state = f.state.clone();
    let device_before = state.project.device(&f.synth).cloned();
    let out = ran(
        &mut state,
        &p(preset::Command::Copy {
            preset: reference(PresetLocation::Project, &f.warm),
            location: PresetLocation::Library,
            name: None,
        }),
    );
    assert_eq!(state.project.device(&f.synth).cloned(), device_before);
    let Output::Preset(preset::Output::Copy { preset: copy }) = out.output else {
        panic!()
    };
    let id = PresetId::parse(&copy.id).unwrap();
    assert_eq!(state.library[&id], state.project.presets[&f.warm]);
    // Saving the same name again in the library is refused, case and spaces aside; renaming
    // a preset to its own name in another case is not.
    assert_eq!(
        invalid_field(&save(&f.synth, " WARM ", PresetLocation::Library), &state),
        "name"
    );
    ran(
        &mut state,
        &p(preset::Command::Rename {
            preset: copy.clone(),
            name: "WARM".into(),
        }),
    );
    assert_eq!(state.library[&id].name, "WARM");
    // A preset of another device may share the name.
    ran(
        &mut state,
        &p(preset::Command::Copy {
            preset: factory("faulk.filter/warm-lowpass"),
            location: PresetLocation::Library,
            name: Some("Warm".into()),
        }),
    );
}

#[test]
fn a_load_into_a_device_that_lacks_parameters_tells_the_engine_and_undoes_to_what_it_played() {
    // A filter as an older Faulk wrote it: only its cutoff (the engine plays the others at their
    // defaults: low-pass, resonance 0.2, mix 1).
    let f = fx();
    let mut state = f.state.clone();
    let filter = DeviceId::parse("01HZY00000000000000000F1TR").unwrap();
    let track = state.project.tracks.get_mut(&f.track).unwrap();
    track.devices.insert(
        filter.clone(),
        ::project::Device {
            order: ::project::OrderKey::between(Some(&::project::OrderKey::first()), None).unwrap(),
            role: ::project::DeviceRole::Effect,
            enabled: true,
            source: DeviceSource::Builtin {
                id: "faulk.filter".into(),
                version: 1,
            },
            params: BTreeMap::from([("cutoff".into(), 800.0)]),
            state: None,
            preset: None,
        },
    );
    state.project.validate().unwrap();
    let out = ran(
        &mut state,
        &load(&filter, factory("faulk.filter/thin-highpass")),
    );
    let mut told: Vec<&str> = out
        .events
        .iter()
        .filter_map(|e| match e {
            ProjectEvent::DeviceParamChanged { param, .. } => Some(param.as_str()),
            _ => None,
        })
        .collect();
    told.sort_unstable();
    // `mix` keeps its default (1): nothing to tell, nothing written.
    assert_eq!(told, ["cutoff", "mode", "resonance"]);
    let params = &state.project.device(&filter).unwrap().params;
    assert_eq!(params.get("mix"), None);
    assert_eq!(params["mode"], 1.0);
    let preset::Output::List { modified, .. } = list(&mut state, &filter) else {
        panic!()
    };
    assert_eq!(modified, Some(false));
    // The undo gives back what the engine played: the cutoff, and the defaults of the others.
    let undone = ran(&mut state, &undo());
    let params = &state.project.device(&filter).unwrap().params;
    assert_eq!(params["cutoff"], 800.0);
    assert_eq!(params["mode"], 0.0);
    assert_eq!(params["resonance"], 0.2);
    assert_eq!(
        undone
            .events
            .iter()
            .filter(|e| matches!(e, ProjectEvent::DeviceParamChanged { .. }))
            .count(),
        3
    );
}

#[test]
fn the_other_refusals_of_rename_restore_and_restore_device() {
    let f = fx();
    let mut state = f.state.clone();
    // Rename to a name another preset of the synth uses in that location.
    let other = PresetId::parse("01HZY00000000000000000XTRA").unwrap();
    state
        .project
        .presets
        .insert(other.clone(), synth_preset("Other", 0.1));
    assert_eq!(
        invalid_field(
            &p(preset::Command::Rename {
                preset: reference(PresetLocation::Project, &other),
                name: "warm".into(),
            }),
            &state
        ),
        "name"
    );
    // Restore into a full project, and into a full library.
    let mut ids = ::project::IdGenerator::new(1_620_000_000_000, 11);
    let mut full = state.clone();
    while full.project.presets.len() < ::project::preset::MAX_PROJECT_PRESETS {
        full.project
            .presets
            .insert(PresetId::generate(&mut ids), synth_preset("P", 0.1));
    }
    while full.library.len() < ::project::preset::MAX_LIBRARY_PRESETS {
        full.library
            .insert(PresetId::generate(&mut ids), synth_preset("L", 0.1));
    }
    let fresh = PresetId::parse("01HZY00000000000000000NEW1").unwrap();
    for location in [PresetLocation::Project, PresetLocation::Library] {
        let restore = p(preset::Command::Restore {
            preset: reference(location, &fresh),
            snapshot: Some(synth_preset("Back", 0.1)),
            device: None,
        });
        assert_eq!(refused(&restore, &full).code(), "limit", "{location:?}");
    }
    // Restore a device's values on a device without presets: the clip player.
    let mut audio = state.clone();
    let player = DeviceId::parse("01HZY00000000000000000PXA2").unwrap();
    let mut track = ::project::Track::new(
        ::project::TrackKind::Audio,
        "Audio",
        ::project::OrderKey::between(Some(&::project::OrderKey::first()), None).unwrap(),
    );
    track.color = Some(::project::colors::palette_hue(1));
    track.devices.insert(
        player.clone(),
        ::project::Device {
            order: ::project::OrderKey::first(),
            role: ::project::DeviceRole::Instrument,
            enabled: true,
            source: DeviceSource::Builtin {
                id: ::project::CLIP_PLAYER.into(),
                version: 1,
            },
            params: BTreeMap::new(),
            state: None,
            preset: None,
        },
    );
    audio.project.tracks.insert(
        ::project::TrackId::parse("01HZY00000000000000000TRK2").unwrap(),
        track,
    );
    assert_eq!(
        invalid_field(
            &p(preset::Command::RestoreDevice {
                device: player,
                params: BTreeMap::new(),
                preset: None,
            }),
            &audio
        ),
        "device"
    );
}
