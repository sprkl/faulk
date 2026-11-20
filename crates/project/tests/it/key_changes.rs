//! Format 8 (#953): the key changes. A project without any writes the same `project.json` as
//! before (no field, only the number differs); with some it writes and reads them back the
//! same, in the folder, the cache snapshot and the pack; a format 7 folder opens with none,
//! its number alone migrated; unordered, duplicate, tick 0 or too many are refused.

use project::file::*;
use project::*;
use std::io::Cursor;

const D_MAJOR: Scale = Scale {
    root: 2,
    mode: ScaleMode::Major,
};
const F_MAJOR: Scale = Scale {
    root: 5,
    mode: ScaleMode::Major,
};

fn song(changes: Vec<KeyChange>) -> Project {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Song");
    project.scale = Some(D_MAJOR);
    project.key_changes = changes;
    let mut track = Track::new(TrackKind::Midi, "T", OrderKey::first());
    track.color = Some(Rgb {
        r: 240,
        g: 128,
        b: 152,
    });
    project.tracks.insert(TrackId::generate(&mut ids), track);
    project
}

fn change(bar: i64, scale: Option<Scale>) -> KeyChange {
    KeyChange {
        tick: Tick(bar * 3840),
        scale,
    }
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
fn a_project_without_key_changes_writes_no_field() {
    let fs = save_all(&song(vec![]), &SaveOptions::default());
    assert!(!text(&fs).contains("key_changes"), "{}", text(&fs));
    assert!(
        open(&fs, &OpenOptions::default())
            .unwrap()
            .project
            .key_changes
            .is_empty()
    );
}

#[test]
fn key_changes_are_written_and_read_back_the_same() {
    let project = song(vec![change(64, Some(F_MAJOR)), change(96, None)]);
    let fs = save_all(&project, &SaveOptions::default());
    assert!(text(&fs).contains("key_changes"), "{}", text(&fs));
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.project, project);
    let again = save_all(&opened.project, &SaveOptions::default());
    assert_eq!(again.files(), fs.files());
}

#[test]
fn the_cache_snapshot_and_the_pack_carry_the_key_changes() {
    let project = song(vec![change(4, Some(F_MAJOR))]);
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    let fs = save_all(&project, &options);
    let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
    assert!(cached.report.files_from_cache > 0);
    assert_eq!(cached.project, project);
    let mut out = Cursor::new(Vec::new());
    export_pack(&fs, &mut out).unwrap();
    let mut dest = MemFs::new();
    let opened = import_pack(
        Cursor::new(out.into_inner()),
        &mut dest,
        &PackLimits::default(),
    )
    .unwrap();
    assert_eq!(opened.project, project);
}

#[test]
fn a_format_7_folder_opens_with_its_number_migrated_and_no_key_changes() {
    let project = song(vec![]);
    let mut fs = save_all(&project, &SaveOptions::default());
    let text = text(&fs);
    let current = format!("\"format\": {FORMAT_VERSION}");
    assert!(text.contains(&current), "{text}");
    fs.insert("project.json", text.replacen(&current, "\"format\": 7", 1));
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.report.migrated_from, Some(7));
    assert_eq!(opened.project, project);
}

#[test]
fn a_build_of_format_7_would_call_a_newer_folder_newer_not_corrupt() {
    // The same rule one step up: this build reads the number first and refuses format 9.
    let mut fs = save_all(
        &song(vec![change(4, Some(F_MAJOR))]),
        &SaveOptions::default(),
    );
    let text = text(&fs);
    let current = format!("\"format\": {FORMAT_VERSION}");
    let newer = format!("\"format\": {}", FORMAT_VERSION + 1);
    fs.insert("project.json", text.replacen(&current, &newer, 1));
    assert!(matches!(
        open(&fs, &OpenOptions::default()),
        Err(FileError::NewerFormat { .. })
    ));
}

#[test]
fn broken_lists_refuse_the_folder() {
    let ok = song(vec![change(4, Some(F_MAJOR)), change(8, None)]);
    let violates = |p: &Project| p.validate().is_err();
    let mut unordered = ok.clone();
    unordered.key_changes.reverse();
    let mut twice = ok.clone();
    twice.key_changes[1].tick = twice.key_changes[0].tick;
    let mut at_zero = ok.clone();
    at_zero.key_changes[0].tick = Tick(0);
    let mut too_far = ok.clone();
    too_far.key_changes[1].tick = Tick(limits::MAX_TICK + 1);
    let mut bad_root = ok.clone();
    bad_root.key_changes[0].scale = Some(Scale {
        root: 12,
        mode: ScaleMode::Major,
    });
    let mut many = ok.clone();
    many.key_changes = (1..=(limits::MAX_KEY_CHANGES as i64 + 1))
        .map(|b| change(b, None))
        .collect();
    assert!(ok.validate().is_ok());
    for (name, p) in [
        ("unordered", &unordered),
        ("twice", &twice),
        ("tick 0", &at_zero),
        ("too far", &too_far),
        ("bad root", &bad_root),
        ("too many", &many),
    ] {
        assert!(violates(p), "{name}");
    }
    // And on reading: the written file of a broken list is refused.
    let fs = save_all(&unordered, &SaveOptions::default());
    assert!(open(&fs, &OpenOptions::default()).is_err());
}

#[test]
fn the_engine_and_the_project_share_the_limit() {
    assert_eq!(limits::MAX_KEY_CHANGES, engine::keymap::MAX_KEY_CHANGES);
}
