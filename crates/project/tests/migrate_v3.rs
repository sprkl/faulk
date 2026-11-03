//! Format 3 (#490): a track may hold a row `height`. A format 2 folder reads as before, its
//! number alone migrated; a format 3 folder is "made by a newer build" to a format 2 build.

use project::file::*;
use project::*;

fn folder(height: Option<u16>) -> (MemFs, TrackId, Project) {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Song");
    let mut track = Track::new(TrackKind::Midi, "T", OrderKey::first());
    track.height = height;
    let tid = TrackId::generate(&mut ids);
    project.tracks.insert(tid.clone(), track);
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    (fs, tid, project)
}

fn set_format(fs: &mut MemFs, from: u32, to: u32) {
    let text = String::from_utf8(fs.read("project.json").unwrap()).unwrap();
    assert!(text.contains(&format!("\"format\": {from}")));
    fs.insert(
        "project.json",
        text.replacen(
            &format!("\"format\": {from}"),
            &format!("\"format\": {to}"),
            1,
        ),
    );
}

#[test]
fn a_format_2_folder_opens_as_before_with_no_height() {
    let (mut fs, tid, _) = folder(None);
    set_format(&mut fs, FORMAT_VERSION, 2);
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.report.migrated_from, Some(2));
    assert_eq!(opened.project.format, FORMAT_VERSION);
    assert_eq!(opened.project.tracks[&tid].height, None);
    assert_eq!(opened.project.tracks[&tid].name, "T");
}

#[test]
fn a_folder_with_a_height_is_format_3_and_a_format_2_build_would_call_it_newer() {
    let (fs, tid, _) = folder(Some(96));
    let text = String::from_utf8(fs.read("project.json").unwrap()).unwrap();
    assert!(text.contains("\"format\": 3"), "{text}");
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.project.tracks[&tid].height, Some(96));
    // The format 2 build then refuses it as newer: the reader compares the number before any
    // other field (`folder_errors.rs` checks that refusal for a number above the supported one).
}
