//! Format 6 (#219): a MIDI track's chain may hold MIDI effects (role `midi_effect`). A format 5
//! folder opens with its number alone migrated and nothing lost; a folder with a MIDI effect
//! writes and reads back the same; a build that does not know the format (a format 5 build
//! meeting this format, here this build meeting the next one) says "newer", never "corrupt",
//! even when the folder holds a role it cannot read.

use project::file::*;
use project::*;
use std::collections::BTreeMap;

fn device(role: DeviceRole, id: &str, order: OrderKey) -> Device {
    Device {
        order,
        role,
        enabled: true,
        source: DeviceSource::Builtin {
            id: id.into(),
            version: 1,
        },
        params: BTreeMap::new(),
        state: None,
        preset: None,
    }
}

/// A song in A minor with a MIDI track (synth and echo), and, with `midi_effect`, a MIDI effect
/// before its synth.
fn song(midi_effect: bool) -> Project {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Song");
    project.scale = Some(Scale {
        root: 9,
        mode: ScaleMode::NaturalMinor,
    });
    let mut track = Track::new(TrackKind::Midi, "Keys", OrderKey::first());
    track.color = Some(Rgb {
        r: 240,
        g: 128,
        b: 152,
    });
    let synth = OrderKey::first();
    let echo = OrderKey::between(Some(&synth), None).unwrap();
    track.devices.insert(
        DeviceId::generate(&mut ids),
        device(DeviceRole::Instrument, "faulk.osc_synth", synth.clone()),
    );
    track.devices.insert(
        DeviceId::generate(&mut ids),
        device(DeviceRole::Effect, "faulk.echo", echo),
    );
    if midi_effect {
        let mut stack = device(
            DeviceRole::MidiEffect,
            "faulk.stack",
            OrderKey::between(None, Some(&synth)).unwrap(),
        );
        stack.params.insert("strum".into(), 0.2);
        track.devices.insert(DeviceId::generate(&mut ids), stack);
    }
    project.tracks.insert(TrackId::generate(&mut ids), track);
    project.validate().unwrap();
    project
}

fn save_all(project: &Project, options: &SaveOptions) -> MemFs {
    let mut fs = MemFs::new();
    save(&mut fs, project, &mut SaveState::new(), None, options).unwrap();
    fs
}

fn text(fs: &MemFs) -> String {
    String::from_utf8(fs.read("project.json").unwrap()).unwrap()
}

#[test]
fn a_format_5_folder_opens_with_its_number_migrated_and_nothing_lost() {
    let project = song(false);
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    let mut fs = save_all(&project, &options);
    let text = text(&fs);
    let current = format!("\"format\": {FORMAT_VERSION}");
    assert_eq!(FORMAT_VERSION, 6);
    assert!(text.contains(&current), "{text}");
    fs.insert("project.json", text.replacen(&current, "\"format\": 5", 1));
    // With or without the snapshot (made by this build), the folder migrates from its files.
    for use_cache in [false, true] {
        let opened = open(&fs, &OpenOptions { use_cache }).unwrap();
        assert_eq!(opened.report.migrated_from, Some(5));
        assert_eq!(opened.project.format, FORMAT_VERSION);
        assert_eq!(opened.project, project, "use_cache {use_cache}");
    }
}

#[test]
fn a_midi_effect_writes_and_reads_back_the_same() {
    let project = song(true);
    let fs = save_all(&project, &SaveOptions::default());
    let files: String = fs
        .files()
        .iter()
        .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
        .collect();
    assert!(files.contains("\"midi_effect\""), "the role is written");
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.report.migrated_from, None);
    assert_eq!(opened.project, project);
}

#[test]
fn a_build_that_does_not_know_the_format_says_newer_even_with_a_role_it_cannot_read() {
    // A format-5 build reads the number before anything else: 6 is above what it supports,
    // so it refuses the folder as newer rather than corrupt for the `midi_effect` role. The
    // same rule one step up: this build meets format 7 and a role it does not know.
    let fs = save_all(&song(true), &SaveOptions::default());
    let mut newer = fs.clone();
    for (path, bytes) in fs.files() {
        let text = String::from_utf8_lossy(bytes)
            .replace("\"midi_effect\"", "\"unknown_role\"")
            .replace(
                &format!("\"format\": {FORMAT_VERSION}"),
                &format!("\"format\": {}", FORMAT_VERSION + 1),
            );
        newer.insert(path, text.into_bytes());
    }
    assert!(matches!(
        open(&newer, &OpenOptions::default()),
        Err(FileError::NewerFormat { found, supported })
            if found == FORMAT_VERSION + 1 && supported == FORMAT_VERSION
    ));
    // At the current number, the unknown role is a corrupt folder: the number is what tells
    // the two apart.
    let mut corrupt = fs.clone();
    for (path, bytes) in fs.files() {
        let text = String::from_utf8_lossy(bytes).replace("\"midi_effect\"", "\"unknown_role\"");
        corrupt.insert(path, text.into_bytes());
    }
    assert!(open(&corrupt, &OpenOptions::default()).is_err());
}
