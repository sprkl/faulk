//! Format 2 (#473): a placement longer than a MIDI clip without a loop repeats the clip. A
//! folder written in format 1, where such a clip was silent after its end, is migrated so
//! that it sounds the same: the placement is cut at the clip's end, and a clip with a loop
//! is left alone.

use project::file::*;
use project::*;

fn clip(length: i64, looping: Option<(i64, i64)>) -> Clip {
    Clip {
        name: "C".into(),
        color: None,
        slot: None,
        launch_quantize: None,
        content: ClipContent::Midi {
            length: Tick(length),
            looping: looping.map(|(start, end)| TickLoop {
                start: Tick(start),
                end: Tick(end),
            }),
            notes: [Note::new(Tick(0), 60, Tick(240), 100)].into(),
        },
    }
}

fn placed(clip: &ClipId, start: i64, length: i64, offset: i64) -> Placement {
    Placement {
        clip: clip.clone(),
        start: Tick(start),
        span: Span::Ticks {
            length: Tick(length),
            offset: Tick(offset),
        },
    }
}

/// A format-1 folder: one-shot and looped clips, placements longer than them.
fn v1_folder() -> (MemFs, TrackId, [PlacementId; 4], Project) {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Old song");
    let mut track = Track::new(TrackKind::Midi, "Bass", OrderKey::first());
    track.color = Some(Rgb::new(1, 2, 3));
    let (one_shot, looped) = (ClipId::generate(&mut ids), ClipId::generate(&mut ids));
    track.clips.insert(one_shot.clone(), clip(3840, None));
    track
        .clips
        .insert(looped.clone(), clip(3840, Some((0, 1920))));
    let p = [(); 4].map(|_| PlacementId::generate(&mut ids));
    // Stretched to four bars; inside the clip; crossing the clip's end from an offset; a loop.
    track
        .placements
        .insert(p[0].clone(), placed(&one_shot, 0, 15360, 0));
    track
        .placements
        .insert(p[1].clone(), placed(&one_shot, 15360, 1920, 960));
    track
        .placements
        .insert(p[2].clone(), placed(&one_shot, 19200, 960, 3000));
    track
        .placements
        .insert(p[3].clone(), placed(&looped, 23040, 7680, 0));
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
    let text = String::from_utf8(fs.read("project.json").unwrap()).unwrap();
    fs.insert(
        "project.json",
        text.replacen("\"format\": 2", "\"format\": 1", 1),
    );
    (fs, tid, p, project)
}

#[test]
fn a_format_1_song_is_cut_where_it_was_silent_and_sounds_the_same() {
    let (mut fs, track, p, _) = v1_folder();
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.report.migrated_from, Some(1));
    assert_eq!(opened.project.format, FORMAT_VERSION);
    let placements = &opened.project.tracks[&track].placements;
    let span = |id: &PlacementId| match placements[id].span {
        Span::Ticks { length, offset } => (length.get(), offset.get()),
        Span::Frames { .. } => panic!(),
    };
    // Cut at the clip's end: what lay past it was silence.
    assert_eq!(span(&p[0]), (3840, 0));
    // Already inside the clip: as it was.
    assert_eq!(span(&p[1]), (1920, 960));
    // Crossing the end from an offset: cut there.
    assert_eq!(span(&p[2]), (840, 3000));
    // A looped clip repeated before: left alone.
    assert_eq!(span(&p[3]), (7680, 0));
    opened.project.validate().unwrap();

    // Saving rewrites the migrated files; the folder then opens in format 2, unchanged.
    let mut state = opened.state;
    save(
        &mut fs,
        &opened.project,
        &mut state,
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    let again = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(again.report.migrated_from, None);
    assert_eq!(again.project, opened.project);
}

#[test]
fn a_current_song_keeps_its_stretched_clips() {
    let (mut fs, track, p, project) = v1_folder();
    let text = String::from_utf8(fs.read("project.json").unwrap()).unwrap();
    fs.insert(
        "project.json",
        text.replacen("\"format\": 1", "\"format\": 2", 1),
    );
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.report.migrated_from, None);
    assert_eq!(
        opened.project.tracks[&track].placements[&p[0]], project.tracks[&track].placements[&p[0]],
        "format 2 repeats the clip: nothing is cut"
    );
}
