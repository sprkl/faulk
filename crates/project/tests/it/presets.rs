//! #191: the presets of a project are saved in `presets/<id>.json`, one preset file each, and a
//! device keeps the preset it was set from. Format 4: a format-3 folder opens with its number
//! migrated; a project without presets writes the same files as before but for that number; the
//! presets go through the full save (a removed preset's file is removed), the cache snapshot and
//! the pack; a file of `presets/` that is not a preset id is ignored and left alone, a preset
//! file that does not read refuses the folder.

use project::file::*;
use project::preset::{DevicePreset, Preset, PresetId, PresetLocation};
use project::*;
use std::collections::BTreeMap;
use std::io::Cursor;

struct Song {
    project: Project,
    track: TrackId,
    device: DeviceId,
    warm: PresetId,
    bright: PresetId,
}

fn preset(name: &str, cutoff: f64) -> Preset {
    Preset {
        device: "faulk.filter".into(),
        version: 1,
        name: name.into(),
        category: Some("Tone".into()),
        params: BTreeMap::from([("cutoff".into(), cutoff)]),
    }
}

fn song() -> Song {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Presets");
    let track = TrackId::generate(&mut ids);
    let device = DeviceId::generate(&mut ids);
    let warm = PresetId::generate(&mut ids);
    let bright = PresetId::generate(&mut ids);
    let mut t = Track::new(TrackKind::Midi, "Bass", OrderKey::first());
    t.color = Some(Rgb::new(240, 128, 152));
    t.devices.insert(
        device.clone(),
        Device {
            order: OrderKey::first(),
            role: DeviceRole::Effect,
            enabled: true,
            source: DeviceSource::Builtin {
                id: "faulk.filter".into(),
                version: 1,
            },
            params: BTreeMap::from([("cutoff".into(), 900.0)]),
            state: None,
            preset: Some(Box::new(DevicePreset {
                location: PresetLocation::Project,
                id: warm.to_string(),
                name: "Warm".into(),
            })),
        },
    );
    project.tracks.insert(track.clone(), t);
    project.presets.insert(warm.clone(), preset("Warm", 900.0));
    project
        .presets
        .insert(bright.clone(), preset("Bright", 5000.0));
    Song {
        project,
        track,
        device,
        warm,
        bright,
    }
}

fn save_all(project: &Project, options: &SaveOptions) -> (MemFs, SaveState) {
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    save(&mut fs, project, &mut state, None, options).unwrap();
    (fs, state)
}

fn preset_path(id: &PresetId) -> String {
    format!("presets/{id}.json")
}

#[test]
fn presets_are_one_file_each_and_open_again_with_the_device_s_preset() {
    let s = song();
    let (fs, _) = save_all(&s.project, &SaveOptions::default());
    let warm = fs.read(&preset_path(&s.warm)).unwrap();
    assert_eq!(warm, s.project.presets[&s.warm].to_file().unwrap());
    assert!(fs.exists(&preset_path(&s.bright)));
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.project, s.project);
    let device = &opened.project.tracks[&s.track].devices[&s.device];
    assert_eq!(device.preset.as_ref().unwrap().name, "Warm");
    let text = String::from_utf8(
        fs.read(&format!("tracks/{}/devices/{}.json", s.track, s.device))
            .unwrap(),
    )
    .unwrap();
    assert!(text.contains("\"preset\": {"), "{text}");
}

#[test]
fn a_project_without_presets_writes_no_presets_folder_and_no_device_preset() {
    let mut s = song();
    s.project.presets.clear();
    s.project
        .tracks
        .get_mut(&s.track)
        .unwrap()
        .devices
        .get_mut(&s.device)
        .unwrap()
        .preset = None;
    let (fs, _) = save_all(&s.project, &SaveOptions::default());
    assert!(fs.files().keys().all(|p| !p.starts_with("presets")));
    assert!(
        fs.files()
            .values()
            .all(|b| !String::from_utf8_lossy(b).contains("\"preset\""))
    );
}

#[test]
fn a_full_save_removes_the_file_of_a_deleted_preset_and_rewrites_only_what_changed() {
    let mut s = song();
    let (mut fs, mut state) = save_all(&s.project, &SaveOptions::default());
    s.project.presets.remove(&s.bright);
    s.project.presets.get_mut(&s.warm).unwrap().name = "Warmer".into();
    let report = save(
        &mut fs,
        &s.project,
        &mut state,
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    assert_eq!(report.written, [preset_path(&s.warm)]);
    assert_eq!(report.removed, [preset_path(&s.bright)]);
    assert_eq!(
        open(&fs, &OpenOptions::default()).unwrap().project,
        s.project
    );
}

#[test]
fn the_cache_snapshot_and_the_pack_carry_the_presets() {
    let s = song();
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    let (fs, _) = save_all(&s.project, &options);
    let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
    assert_eq!(cached.project, s.project);
    assert!(cached.report.files_from_cache > 0);
    // `project.json` is read for its format; every other file, the presets included, comes from
    // the snapshot.
    assert_eq!(cached.report.files_read, 1);

    let mut out = Cursor::new(Vec::new());
    export_pack(&fs, &mut out).unwrap();
    let mut dest = MemFs::new();
    let opened = import_pack(
        Cursor::new(out.into_inner()),
        &mut dest,
        &PackLimits::default(),
    )
    .unwrap();
    assert_eq!(opened.project, s.project);
    assert_eq!(
        dest.files().get(&preset_path(&s.warm)),
        fs.files().get(&preset_path(&s.warm))
    );
}

#[test]
fn a_stray_file_in_presets_is_ignored_and_a_broken_preset_refuses_the_folder() {
    let s = song();
    let (mut fs, _) = save_all(&s.project, &SaveOptions::default());
    fs.insert("presets/notes.txt", "mine");
    fs.insert("presets/../escape.json", "{}");
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.project, s.project);
    assert!(
        opened
            .report
            .ignored
            .contains(&"presets/notes.txt".to_owned()),
        "{:?}",
        opened.report.ignored
    );

    for (bad, why) in [
        ("{", "not JSON"),
        (
            "{\"format\": 1, \"device\": \"faulk.filter\", \"version\": 1, \"name\": \"\", \"params\": {}}",
            "empty name",
        ),
        (
            "{\"format\": 2, \"device\": \"faulk.filter\", \"version\": 1, \"name\": \"X\", \"params\": {}}",
            "newer file format",
        ),
    ] {
        let mut broken = fs.clone();
        broken.insert(preset_path(&s.bright), bad);
        match open(&broken, &OpenOptions::default()) {
            Err(FileError::Corrupt { path, .. }) => {
                assert_eq!(path, preset_path(&s.bright), "{why}")
            }
            other => panic!("{why}: {other:?}"),
        }
    }
    let mut big = fs.clone();
    big.insert(
        preset_path(&s.bright),
        vec![b' '; project::preset::MAX_PRESET_BYTES + 1],
    );
    assert!(matches!(
        open(&big, &OpenOptions::default()),
        Err(FileError::Corrupt { .. } | FileError::TooLarge { .. })
    ));
}

#[test]
fn a_format_3_folder_opens_with_its_number_migrated_and_no_presets() {
    let mut s = song();
    s.project.presets.clear();
    s.project
        .tracks
        .get_mut(&s.track)
        .unwrap()
        .devices
        .get_mut(&s.device)
        .unwrap()
        .preset = None;
    let (mut fs, _) = save_all(&s.project, &SaveOptions::default());
    let text = String::from_utf8(fs.read("project.json").unwrap()).unwrap();
    let current = format!("\"format\": {FORMAT_VERSION}");
    assert!(text.contains(&current), "{text}");
    fs.insert("project.json", text.replacen(&current, "\"format\": 3", 1));
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.report.migrated_from, Some(3));
    assert_eq!(opened.project, s.project);
    // The format-3 build refuses a format-4 folder by its number, before reading a device file
    // whose `preset` field it would call corrupt (`folder_errors.rs` checks that refusal).
}

#[test]
fn the_documented_preset_file_is_the_factory_file_byte_for_byte() {
    // `docs/architecture/02-modele-projet.md` § 2.8 shows a factory file as it is written.
    let doc = include_str!("../../../../docs/architecture/02-modele-projet.md");
    let section = &doc[doc.find("### 2.8").expect("§ 2.8")..];
    let start = section.find("```json\n").expect("a json block") + "```json\n".len();
    let block = &section[start..start + section[start..].find("  ```").unwrap()];
    let shown: String = block
        .lines()
        .map(|l| format!("{}\n", l.strip_prefix("  ").unwrap_or(l)))
        .collect();
    let file = project::preset::factory::get("faulk.osc_synth/round-bass").unwrap();
    assert_eq!(shown.into_bytes(), file.preset.to_file().unwrap());
}
