//! Format 5 (#695): the project's key and scale. A project without one writes the same
//! `project.json` as before (no field); one with a scale writes it and reads it back the
//! same; a format 4 folder opens with no scale, its number alone migrated; a root outside
//! 0 to 11 or an unknown scale refuses the folder.

use project::file::*;
use project::*;
use std::io::Cursor;

fn song(scale: Option<Scale>) -> Project {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Song");
    project.scale = scale;
    let mut track = Track::new(TrackKind::Midi, "T", OrderKey::first());
    // A track without a colour gets one on reading: given here, the project reads back equal.
    track.color = Some(Rgb {
        r: 240,
        g: 128,
        b: 152,
    });
    project.tracks.insert(TrackId::generate(&mut ids), track);
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

const F_MINOR: Scale = Scale {
    root: 5,
    mode: ScaleMode::NaturalMinor,
};

#[test]
fn every_scale_has_the_degrees_of_its_table_on_every_root() {
    // The table of each scale, by hand, as pitch classes over C.
    let table: [(ScaleMode, [bool; 12]); 12] = {
        let row = |degrees: &[u8]| {
            let mut r = [false; 12];
            for d in degrees {
                r[*d as usize] = true;
            }
            r
        };
        [
            (ScaleMode::Major, row(&[0, 2, 4, 5, 7, 9, 11])),
            (ScaleMode::NaturalMinor, row(&[0, 2, 3, 5, 7, 8, 10])),
            (ScaleMode::HarmonicMinor, row(&[0, 2, 3, 5, 7, 8, 11])),
            (ScaleMode::MelodicMinor, row(&[0, 2, 3, 5, 7, 9, 11])),
            (ScaleMode::Dorian, row(&[0, 2, 3, 5, 7, 9, 10])),
            (ScaleMode::Phrygian, row(&[0, 1, 3, 5, 7, 8, 10])),
            (ScaleMode::Lydian, row(&[0, 2, 4, 6, 7, 9, 11])),
            (ScaleMode::Mixolydian, row(&[0, 2, 4, 5, 7, 9, 10])),
            (ScaleMode::Locrian, row(&[0, 1, 3, 5, 6, 8, 10])),
            (ScaleMode::MajorPentatonic, row(&[0, 2, 4, 7, 9])),
            (ScaleMode::MinorPentatonic, row(&[0, 3, 5, 7, 10])),
            (ScaleMode::Blues, row(&[0, 3, 5, 6, 7, 10])),
        ]
    };
    assert_eq!(
        table.iter().map(|(m, _)| *m).collect::<Vec<_>>(),
        ScaleMode::ALL.to_vec()
    );
    for (mode, over_c) in table {
        let intervals = mode.intervals();
        assert_eq!(intervals[0], 0, "{mode:?} starts on its root");
        assert!(intervals.windows(2).all(|w| w[0] < w[1]), "{mode:?} rises");
        for root in 0..=scale::MAX_ROOT {
            let scale = Scale { root, mode };
            let classes = scale.pitch_classes();
            assert!(classes.windows(2).all(|w| w[0] < w[1]));
            assert_eq!(classes.len(), intervals.len(), "{mode:?} on {root}");
            for pitch in 0..=127u8 {
                // The table moved up by the root, wrapping past B.
                let expected = over_c[((pitch % 12 + 12 - root) % 12) as usize];
                assert_eq!(
                    scale.contains(pitch),
                    expected,
                    "{mode:?} on {root}: {pitch}"
                );
                assert_eq!(classes.contains(&(pitch % 12)), expected);
            }
        }
    }
    // F minor: F G A♭ B♭ C D♭ E♭.
    assert_eq!(F_MINOR.pitch_classes(), vec![0, 1, 3, 5, 7, 8, 10]);
}

#[test]
fn a_project_without_a_scale_writes_no_field() {
    let fs = save_all(&song(None), &SaveOptions::default());
    let text = text(&fs);
    assert!(!text.contains("scale"), "{text}");
    assert_eq!(
        open(&fs, &OpenOptions::default()).unwrap().project.scale,
        None
    );
}

#[test]
fn a_scale_is_written_and_read_back_the_same() {
    let project = song(Some(F_MINOR));
    let fs = save_all(&project, &SaveOptions::default());
    let text = text(&fs);
    assert!(
        text.contains(
            "\"scale\": {
    \"mode\": \"natural_minor\",
    \"root\": 5
  }"
        ),
        "{text}"
    );
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.project, project);
    // Written again, the folder is byte for byte the same.
    let again = save_all(&opened.project, &SaveOptions::default());
    assert_eq!(again.files(), fs.files());
}

#[test]
fn the_cache_snapshot_and_the_pack_carry_the_scale() {
    let project = song(Some(Scale {
        root: 2,
        mode: ScaleMode::Dorian,
    }));
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
fn a_format_4_folder_opens_with_its_number_migrated_and_no_scale() {
    let project = song(None);
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    let mut fs = save_all(&project, &options);
    let text = text(&fs);
    let current = format!("\"format\": {FORMAT_VERSION}");
    assert!(text.contains(&current), "{text}");
    fs.insert("project.json", text.replacen(&current, "\"format\": 4", 1));
    // With or without the snapshot (made by this build), the folder migrates from its files.
    for use_cache in [false, true] {
        let opened = open(&fs, &OpenOptions { use_cache }).unwrap();
        assert_eq!(opened.report.migrated_from, Some(4));
        assert_eq!(opened.project.format, FORMAT_VERSION);
        assert_eq!(opened.project.scale, None);
        assert_eq!(opened.project, project);
    }
}

#[test]
fn a_format_5_folder_is_newer_than_a_format_4_build_says() {
    // A format-4 build reads the number first: 5 is above what it supports, so it says
    // "newer" rather than "corrupt" for the `scale` field. The same rule, one step up: this
    // build refuses format 6.
    let mut fs = save_all(&song(Some(F_MINOR)), &SaveOptions::default());
    let text = text(&fs);
    let current = format!("\"format\": {FORMAT_VERSION}");
    let newer = format!("\"format\": {}", FORMAT_VERSION + 1);
    fs.insert("project.json", text.replacen(&current, &newer, 1));
    assert!(matches!(
        open(&fs, &OpenOptions::default()),
        Err(FileError::NewerFormat { found, supported }) if found == FORMAT_VERSION + 1 && supported == FORMAT_VERSION
    ));
}

#[test]
fn a_bad_root_or_an_unknown_scale_refuses_the_folder() {
    let fs = save_all(&song(Some(F_MINOR)), &SaveOptions::default());
    let text = text(&fs);
    for (from, to, why) in [
        ("\"root\": 5", "\"root\": 12", "root above 11"),
        ("\"root\": 5", "\"root\": -1", "negative root"),
        ("\"natural_minor\"", "\"ionian_sharp\"", "unknown scale"),
        (
            "\"natural_minor\"",
            "\"chromatic\"",
            "chromatic is no field, not a scale",
        ),
        ("\"root\": 5", "\"root\": 5, \"cells\": []", "unknown field"),
    ] {
        let mut broken = fs.clone();
        broken.insert("project.json", text.replacen(from, to, 1));
        assert!(
            open(&broken, &OpenOptions::default()).is_err(),
            "{why} should be refused"
        );
    }
}
