//! A folder that is not a valid project is refused with a typed error, never a panic: one
//! test per way a folder can be wrong, the migrations, and a time-bounded fuzz of the
//! files, the cache snapshot and the pack.

use project::file::*;
use project::*;
use std::io::Cursor;

use crate::common;
use common::reference::{LAYOUT, id, song};

fn options() -> SaveOptions {
    SaveOptions {
        layout: LAYOUT,
        support_files: false,
        write_cache: false,
    }
}

fn saved(project: &Project) -> MemFs {
    let mut fs = MemFs::new();
    save(&mut fs, project, &mut SaveState::new(), None, &options()).unwrap();
    fs
}

fn song_fs() -> MemFs {
    saved(&song())
}

fn t(n: u8) -> String {
    id(n).to_string()
}

fn bass(path: &str) -> String {
    format!("tracks/{}/{path}", t(50))
}

fn edit(fs: &mut MemFs, path: &str, f: impl FnOnce(String) -> String) {
    let text = String::from_utf8(fs.read(path).expect(path)).unwrap();
    fs.insert(path, f(text));
}

fn refused(fs: &MemFs) -> FileError {
    open(fs, &OpenOptions::default()).expect_err("the folder must be refused")
}

// ------------------------------------------------------------------ the project file

#[test]
fn project_json_that_is_not_json_is_corrupt_and_names_the_file() {
    for bad in ["", "{", "[]", "null", "{\"format\": \"one\"}", "\u{0}\u{0}"] {
        let mut fs = song_fs();
        fs.insert("project.json", bad);
        match refused(&fs) {
            FileError::Corrupt { path, .. } => assert_eq!(path, "project.json", "{bad:?}"),
            other => panic!("{bad:?}: {other:?}"),
        }
    }
}

#[test]
fn a_newer_project_is_refused_as_such_even_if_it_has_fields_this_build_does_not_know() {
    let mut fs = song_fs();
    edit(&mut fs, "project.json", |s| {
        s.replacen("\"format\": 4", "\"format\": 5,\n  \"swing\": 0.5", 1)
    });
    assert_eq!(
        refused(&fs),
        FileError::NewerFormat {
            found: 5,
            supported: 4
        }
    );
    // Never rewritten: the refusal comes before anything else is read.
    let mut fs = song_fs();
    edit(&mut fs, "project.json", |s| {
        s.replacen("\"format\": 4", "\"format\": 4000000000", 1)
    });
    assert!(matches!(
        refused(&fs),
        FileError::NewerFormat {
            found: 4_000_000_000,
            ..
        }
    ));
}

#[test]
fn format_zero_and_older_formats_without_a_migration_are_unsupported() {
    let mut fs = song_fs();
    edit(&mut fs, "project.json", |s| {
        s.replacen("\"format\": 4", "\"format\": 0", 1)
    });
    assert_eq!(refused(&fs), FileError::UnsupportedFormat { found: 0 });
}

#[test]
fn an_unknown_field_or_a_duplicate_key_is_corrupt() {
    let mut fs = song_fs();
    edit(&mut fs, &bass("track.json"), |s| {
        s.replacen("\"name\"", "\"nickname\"", 1)
    });
    assert!(
        matches!(refused(&fs), FileError::Corrupt { path, .. } if path.ends_with("track.json"))
    );

    let mut fs = song_fs();
    edit(&mut fs, "project.json", |s| {
        s.replacen("\"transport\"", "\"tempo\": [],\n  \"transport\"", 1)
    });
    assert!(matches!(refused(&fs), FileError::Corrupt { .. }));

    // Two entities with one id in a table: refused, not merged.
    let mut fs = song_fs();
    edit(&mut fs, "project.json", |s| {
        s.replacen(
            "\"scenes\": {",
            &format!(
                "\"scenes\": {{\n    \"{}\": {{\"name\": \"x\", \"order\": \"a0\"}},",
                t(2)
            ),
            1,
        )
    });
    assert!(matches!(refused(&fs), FileError::Corrupt { .. }));
}

#[test]
fn a_file_over_the_size_limit_is_refused_before_it_is_read() {
    let mut fs = song_fs();
    fs.insert("project.json", vec![b' '; MAX_TEXT_FILE_BYTES as usize + 1]);
    assert!(matches!(refused(&fs), FileError::TooLarge { .. }));
    let mut fs = song_fs();
    let notes = bass(&format!("clips/{}.notes.tsv", t(20)));
    fs.insert(notes.clone(), vec![b'0'; MAX_TEXT_FILE_BYTES as usize + 1]);
    assert!(matches!(refused(&fs), FileError::TooLarge { path, .. } if path == notes));
}

// ------------------------------------------------------------------ the tracks

#[test]
fn a_track_without_its_header_is_missing_a_file_a_folder_of_strays_is_not_a_track() {
    let mut fs = song_fs();
    fs.remove(&bass("track.json")).unwrap();
    assert_eq!(
        refused(&fs),
        FileError::Missing {
            path: bass("track.json")
        }
    );

    // What a track's removal leaves when the system dropped a `.DS_Store` there.
    let mut fs = song_fs();
    let (a, b) = (format!("tracks/{}", t(50)), format!("tracks/{}", t(51)));
    for path in fs
        .files()
        .keys()
        .filter(|p| p.starts_with(&b))
        .cloned()
        .collect::<Vec<_>>()
    {
        fs.remove(&path).unwrap();
    }
    fs.insert(format!("{b}/.DS_Store"), "x");
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.project.tracks.len(), 1);
    assert_eq!(opened.report.ignored, vec![b]);
    let _ = a;
}

#[test]
fn what_is_not_part_of_the_format_is_ignored_reported_and_never_deleted() {
    let mut fs = song_fs();
    let strays = [
        bass(".DS_Store"),
        bass("devices/readme.txt"),
        bass(&format!("clips/{}.bak", t(20))),
        "tracks/notes.txt".to_owned(),
        "tracks/not-an-id/track.json".to_owned(),
        // Notes whose clip header is gone.
        bass(&format!("clips/{}.notes.tsv", t(25))),
    ];
    for s in &strays {
        fs.insert(s.clone(), "stray");
    }
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    let mut expected = strays.to_vec();
    expected.retain(|s| s != "tracks/not-an-id/track.json");
    expected.push("tracks/not-an-id".to_owned());
    expected.sort();
    assert_eq!(opened.report.ignored, expected);
    let mut state = opened.state;
    let report = save(&mut fs, &opened.project, &mut state, None, &options()).unwrap();
    assert!(report.removed.is_empty(), "{report:?}");
    for s in &strays {
        assert!(fs.exists(s), "{s} was deleted");
    }
}

#[test]
fn a_midi_clip_without_its_notes_is_missing_a_file() {
    let mut fs = song_fs();
    let notes = bass(&format!("clips/{}.notes.tsv", t(20)));
    fs.remove(&notes).unwrap();
    assert_eq!(refused(&fs), FileError::Missing { path: notes });
}

#[test]
fn bad_names_in_the_structural_folders_do_not_break_the_open() {
    let mut fs = song_fs();
    fs.insert(bass("devices/lowercase.json"), "{}");
    fs.insert(bass("automation/x.tsv"), "garbage");
    fs.insert(bass("clips/short.json"), "{");
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.report.ignored.len(), 3);
}

#[test]
fn a_broken_notes_file_names_its_file_and_line() {
    let mut fs = song_fs();
    let notes = bass(&format!("clips/{}.notes.tsv", t(20)));
    edit(&mut fs, &notes, |s| {
        s.replacen("480\t240\t36\t100", "480\t240\tx\t100", 1)
    });
    match refused(&fs) {
        FileError::Corrupt { path, message } => {
            assert_eq!(path, notes);
            assert!(message.contains("line 4"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_placement_of_a_clip_that_is_not_there_is_a_violation_not_a_guess() {
    let mut fs = song_fs();
    edit(&mut fs, &bass("arrangement.tsv"), |s| {
        s.replacen(&t(21), &t(29), 1)
    });
    match refused(&fs) {
        FileError::Invalid { violations } => {
            assert!(
                violations
                    .iter()
                    .any(|v| v.kind == ViolationKind::UnknownClip),
                "{violations:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_project_that_reads_fine_but_breaks_its_invariants_is_invalid_with_every_violation() {
    let mut fs = song_fs();
    // A clip in a scene that does not exist, and a note outside its clip.
    edit(&mut fs, &bass(&format!("clips/{}.json", t(21))), |s| {
        s.replacen(&t(3), &t(9), 1)
    });
    edit(&mut fs, &bass(&format!("clips/{}.notes.tsv", t(20))), |s| {
        s.replacen("960\t120", "99960\t120", 1)
    });
    match refused(&fs) {
        FileError::Invalid { violations } => {
            let kinds: Vec<_> = violations.iter().map(|v| &v.kind).collect();
            assert!(
                kinds.contains(&&ViolationKind::UnknownScene),
                "{violations:?}"
            );
            assert!(
                kinds.contains(&&ViolationKind::NoteOutsideClip),
                "{violations:?}"
            );
            assert!(
                violations.iter().all(|v| v.path.starts_with("tracks/")),
                "{violations:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_same_id_in_two_places_is_a_duplicate() {
    let mut fs = song_fs();
    // A device that carries the id of a clip.
    let from = bass(&format!("devices/{}.json", t(11)));
    let bytes = fs.read(&from).unwrap();
    fs.insert(bass(&format!("devices/{}.json", t(21))), bytes);
    match refused(&fs) {
        FileError::Invalid { violations } => {
            assert!(
                violations
                    .iter()
                    .any(|v| matches!(v.kind, ViolationKind::DuplicateId(_))),
                "{violations:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

// ------------------------------------------------------------------ lanes and blobs

fn lane(n: u8) -> String {
    bass(&format!("automation/{}.tsv", t(n)))
}

fn blob_path(fs: &MemFs) -> String {
    fs.files()
        .keys()
        .find(|p| p.starts_with("blobs/"))
        .cloned()
        .expect("a blob")
}

#[test]
fn a_lane_blob_that_is_missing_wrong_or_lying_is_refused() {
    // Missing.
    let mut fs = song_fs();
    let blob = blob_path(&fs);
    fs.remove(&blob).unwrap();
    assert_eq!(refused(&fs), FileError::Missing { path: blob.clone() });

    // Its bytes are not its hash.
    let mut fs = song_fs();
    let mut bytes = fs.read(&blob).unwrap();
    bytes[10] ^= 1;
    fs.insert(blob.clone(), bytes);
    assert!(matches!(refused(&fs), FileError::Corrupt { path, .. } if path == blob));

    // The text announces more points than the blob holds.
    let mut fs = song_fs();
    edit(&mut fs, &lane(41), |s| s.replace("points=6", "points=7"));
    assert!(matches!(refused(&fs), FileError::Corrupt { path, .. } if path == lane(41)));

    // A hash that is not a hash, or a path in its place, is never turned into a path.
    for bad in [
        "../../../../etc/passwd",
        "../x",
        "/dev/zero",
        "BLOBS",
        &"A".repeat(64),
    ] {
        let mut fs = song_fs();
        let hex = blob.strip_prefix("blobs/").unwrap().to_owned();
        edit(&mut fs, &lane(41), |s| s.replace(&hex, bad));
        assert!(
            matches!(refused(&fs), FileError::Corrupt { path, .. } if path == lane(41)),
            "{bad}"
        );
    }

    // A text lane that has points and a blob header.
    let mut fs = song_fs();
    edit(&mut fs, &lane(41), |s| format!("{s}0\t1\tlinear\n"));
    assert!(matches!(refused(&fs), FileError::Corrupt { .. }));
}

#[test]
fn a_huge_lane_blob_is_refused_by_its_size_before_it_is_read() {
    let mut fs = song_fs();
    let blob = blob_path(&fs);
    // Same name, but far past the most a lane can hold: the listing says so.
    struct Big(MemFs, String);
    impl FileSystem for Big {
        fn read(&self, p: &str) -> Result<Vec<u8>, FsError> {
            assert_ne!(p, self.1, "a blob over the limit must not be read");
            self.0.read(p)
        }
        fn write(&mut self, p: &str, b: &[u8]) -> Result<(), FsError> {
            self.0.write(p, b)
        }
        fn remove(&mut self, p: &str) -> Result<(), FsError> {
            self.0.remove(p)
        }
        fn exists(&self, p: &str) -> bool {
            self.0.exists(p)
        }
        fn list(&self, d: &str) -> Result<Vec<DirEntry>, FsError> {
            let mut v = self.0.list(d)?;
            if d == "blobs" {
                for e in &mut v {
                    e.size = u64::MAX / 2;
                }
            }
            Ok(v)
        }
    }
    fs.insert("keep", "x");
    let big = Big(fs, blob);
    assert!(matches!(
        open(&big, &OpenOptions::default()),
        Err(FileError::TooLarge { .. })
    ));
}

// ------------------------------------------------------------------ migrations

fn v0_folder() -> MemFs {
    // The song as a format-0 folder would have had it: another header on the notes files and
    // a project name under `title`.
    let mut fs = song_fs();
    edit(&mut fs, "project.json", |s| {
        s.replacen("\"format\": 4", "\"format\": 0", 1)
            .replacen("\"name\"", "\"title\"", 1)
    });
    for path in fs
        .files()
        .keys()
        .filter(|p| p.ends_with(".notes.tsv"))
        .cloned()
        .collect::<Vec<_>>()
    {
        edit(&mut fs, &path, |s| {
            s.replacen("# faulk-notes 1", "# faulk-notes 0", 1)
        });
    }
    fs
}

/// A stand-in for the real step from format 1 (which cuts placements): the format only.
fn one_to_two(fs: &mut MemFs) -> Result<(), String> {
    let text = String::from_utf8(fs.read("project.json").map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs.insert(
        "project.json",
        text.replacen("\"format\": 1", "\"format\": 2", 1),
    );
    Ok(())
}

fn two_to_three(fs: &mut MemFs) -> Result<(), String> {
    let text = String::from_utf8(fs.read("project.json").map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs.insert(
        "project.json",
        text.replacen("\"format\": 2", "\"format\": 3", 1),
    );
    Ok(())
}

fn three_to_four(fs: &mut MemFs) -> Result<(), String> {
    let text = String::from_utf8(fs.read("project.json").map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs.insert(
        "project.json",
        text.replacen("\"format\": 3", "\"format\": 4", 1),
    );
    Ok(())
}

fn zero_to_one(fs: &mut MemFs) -> Result<(), String> {
    let paths: Vec<String> = fs.files().keys().cloned().collect();
    for path in paths {
        let text = String::from_utf8(fs.read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let text = if path == "project.json" {
            text.replacen("\"format\": 0", "\"format\": 1", 1)
                .replacen("\"title\"", "\"name\"", 1)
        } else if path.ends_with(".notes.tsv") {
            text.replacen("# faulk-notes 0", "# faulk-notes 1", 1)
        } else {
            continue;
        };
        fs.insert(path, text);
    }
    Ok(())
}

#[test]
fn an_older_project_is_migrated_in_memory_and_saving_rewrites_what_the_migration_changed() {
    let table = [
        Migration {
            from: 0,
            apply: zero_to_one,
        },
        Migration {
            from: 1,
            apply: one_to_two,
        },
        Migration {
            from: 2,
            apply: two_to_three,
        },
        Migration {
            from: 3,
            apply: three_to_four,
        },
    ];
    let mut fs = v0_folder();
    let before = fs.clone();
    let opened = open_with(&fs, &OpenOptions::default(), &table).unwrap();
    assert_eq!(opened.report.migrated_from, Some(0));
    assert_eq!(opened.project, song());
    assert_eq!(fs.files(), before.files(), "opening never writes");

    // The state remembers the bytes on disk: the save writes every file the migration
    // changed (project.json and the notes), and only those.
    let mut state = opened.state;
    let report = save(&mut fs, &opened.project, &mut state, None, &options()).unwrap();
    let mut changed: Vec<String> = report.written.clone();
    changed.sort();
    let mut expected: Vec<String> = before
        .files()
        .iter()
        .filter(|(p, b)| song_fs().files().get(*p) != Some(*b))
        .map(|(p, _)| p.clone())
        .collect();
    expected.sort();
    assert_eq!(changed, expected);
    assert!(changed.len() >= 3);
    assert_eq!(fs.files(), song_fs().files());
    // And the folder now opens as a current project, without a table.
    let again = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(again.report.migrated_from, None);
    assert_eq!(again.project, song());
}

#[test]
fn a_migration_that_fails_or_leaves_the_wrong_format_is_a_typed_error() {
    fn fails(_: &mut MemFs) -> Result<(), String> {
        Err("no".into())
    }
    fn forgets(_: &mut MemFs) -> Result<(), String> {
        Ok(())
    }
    let fs = v0_folder();
    let err = open_with(
        &fs,
        &OpenOptions::default(),
        &[Migration {
            from: 0,
            apply: fails,
        }],
    )
    .unwrap_err();
    assert_eq!(
        err,
        FileError::Migration {
            from: 0,
            message: "no".into()
        }
    );
    let err = open_with(
        &fs,
        &OpenOptions::default(),
        &[
            Migration {
                from: 0,
                apply: forgets,
            },
            Migration {
                from: 1,
                apply: one_to_two,
            },
            Migration {
                from: 2,
                apply: two_to_three,
            },
            Migration {
                from: 3,
                apply: three_to_four,
            },
        ],
    )
    .unwrap_err();
    assert!(
        matches!(err, FileError::Corrupt { .. } | FileError::Migration { .. }),
        "{err:?}"
    );
    // No step from 0: unsupported.
    assert_eq!(
        open_with(&fs, &OpenOptions::default(), &[]).unwrap_err(),
        FileError::UnsupportedFormat { found: 0 }
    );
}

// ------------------------------------------------------------------ the pack

fn pack_bytes(fs: &MemFs) -> (Vec<u8>, PackReport) {
    let mut out = Cursor::new(Vec::new());
    let report = export_pack(fs, &mut out).unwrap();
    (out.into_inner(), report)
}

fn import(bytes: &[u8]) -> (MemFs, Result<Opened, FileError>) {
    let mut dest = MemFs::new();
    let result = import_pack(
        Cursor::new(bytes.to_vec()),
        &mut dest,
        &PackLimits::default(),
    );
    (dest, result)
}

#[test]
fn a_pack_round_trips_and_leaves_out_what_the_project_does_not_use() {
    let mut fs = song_fs();
    // An old blob nobody refers to any more, the session, the cache, a git folder.
    fs.insert(format!("blobs/{}", "e".repeat(64)), "old take");
    fs.insert(".faulk/session.json", "{}");
    fs.insert(".git/config", "x");
    fs.insert(".gitignore", ".faulk/\n");
    let (bytes, report) = pack_bytes(&fs);
    assert_eq!(
        report.missing_blobs,
        vec![Hash([0xab; 32])],
        "the audio blob is not in the folder"
    );
    let (dest, result) = import(&bytes);
    let opened = result.unwrap();
    assert_eq!(opened.project, song());
    let names: Vec<&String> = dest.files().keys().collect();
    assert!(
        names
            .iter()
            .all(|p| !p.starts_with(".faulk") && !p.starts_with(".git/")),
        "{names:?}"
    );
    assert!(!dest.files().keys().any(|p| p.ends_with(&"e".repeat(64))));
    assert!(dest.exists(".gitignore"));
    assert!(
        dest.files().keys().any(|p| p.starts_with("blobs/")),
        "the lane blob is in"
    );
    // The same project gives the same pack, byte for byte.
    assert_eq!(pack_bytes(&fs).0, bytes);
    // The folder it makes is the folder it came from, files and bytes.
    for (path, b) in dest.files() {
        assert_eq!(fs.files().get(path), Some(b), "{path}");
    }
}

#[test]
fn a_pack_written_in_pieces_is_the_pack_of_the_whole_files() {
    let fs = song_fs();
    let plan = plan_pack(&fs).unwrap();
    let mut pack = PackWriter::new(Vec::new());
    for path in &plan.paths {
        let bytes = fs.read(path).unwrap();
        pack.start(path, bytes.len() as u64).unwrap();
        for piece in bytes.chunks(3) {
            pack.write(piece).unwrap();
        }
    }
    // The compressor may cut its blocks where the pieces end, so the bytes can differ; the files do not.
    let contents = |bytes: Vec<u8>| {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut file = zip.by_index(i).unwrap();
                let mut body = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut body).unwrap();
                (file.name().unwrap().into_owned(), body)
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(
        contents(pack.finish().unwrap()),
        contents(pack_bytes(&fs).0)
    );
}

#[test]
fn a_file_of_a_gibibyte_or_more_gets_the_64_bit_fields_and_the_pack_reads_back() {
    // The size announced is what decides the fields; the bytes given can be few.
    let mut pack = PackWriter::new(Vec::new());
    pack.start("project.json", 10).unwrap();
    pack.write(b"{}").unwrap();
    pack.start("blobs/big", 3 << 30).unwrap();
    pack.write(b"audio").unwrap();
    pack.start("tracks/after", 4).unwrap();
    pack.write(b"next").unwrap();
    let bytes = pack.finish().unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(zip.len(), 3);
    let mut big = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("blobs/big").unwrap(), &mut big).unwrap();
    assert_eq!(big, "audio");
    let mut after = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("tracks/after").unwrap(), &mut after).unwrap();
    assert_eq!(after, "next");
}

#[test]
fn an_invalid_project_is_not_exported() {
    let mut fs = song_fs();
    fs.remove(&bass("track.json")).unwrap();
    let mut out = Cursor::new(Vec::new());
    assert!(matches!(
        export_pack(&fs, &mut out),
        Err(FileError::Missing { .. })
    ));
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn a_pack_with_a_path_outside_the_format_is_refused_and_nothing_is_left() {
    let project = saved(&Project::new(ProjectId(id(1)), "P"));
    let good = project.files()["project.json"].clone();
    for bad in [
        "../project.json",
        "/etc/passwd",
        "tracks/../../x",
        "..\\x",
        "C:\\x",
        "tracks\\x",
        ".faulk/session.json",
        ".git/config",
        "blobs/not-a-hash",
        "other.txt",
        "tracks/x/track.json",
        // Right prefix, wrong last component: separators, drives, `..`, not an id.
        &format!("tracks/{}/devices/..\\..\\x.json", t(50)),
        &format!("tracks/{}/devices/C:\\x.json", t(50)),
        &format!("tracks/{}/devices/../x.json", t(50)),
        &format!("tracks/{}/devices/x.json", t(50)),
        &format!("tracks/{}/clips/{}.notes.tsv.bak", t(50), t(20)),
        &format!("tracks/{}/automation/x.tsv", t(50)),
    ] {
        let bytes = zip_of(&[("project.json", &good), (bad, b"x")]);
        let (dest, result) = import(&bytes);
        assert!(
            matches!(result, Err(FileError::BadPack { .. })),
            "{bad}: {result:?}"
        );
        assert!(
            dest.is_empty(),
            "{bad}: left {:?}",
            dest.files().keys().collect::<Vec<_>>()
        );
    }
}

#[test]
fn a_pack_with_a_blob_that_is_not_its_name_is_refused() {
    let project = saved(&Project::new(ProjectId(id(1)), "P"));
    let bytes = zip_of(&[
        ("project.json", &project.files()["project.json"]),
        (&format!("blobs/{}", "c".repeat(64)), b"not that hash"),
    ]);
    assert!(matches!(import(&bytes).1, Err(FileError::BadPack { .. })));
}

#[test]
fn a_pack_that_is_not_a_zip_or_not_a_project_is_refused() {
    for bytes in [&b""[..], b"PK", b"not a zip at all", &[0u8; 100]] {
        assert!(matches!(import(bytes).1, Err(FileError::BadPack { .. })));
    }
    // A zip that is not a project.
    let (dest, result) = import(&zip_of(&[("project.json", b"{}")]));
    assert!(matches!(result, Err(FileError::Corrupt { .. })));
    assert!(dest.is_empty());
    assert!(matches!(
        import(&zip_of(&[])).1,
        Err(FileError::NotAProject)
    ));
    // A valid pack of an invalid project: validated, then nothing left behind.
    let mut fs = song_fs();
    edit(&mut fs, &bass(&format!("clips/{}.json", t(21))), |s| {
        s.replacen(&t(3), &t(9), 1)
    });
    let entries: Vec<(String, Vec<u8>)> = fs
        .files()
        .iter()
        .map(|(p, b)| (p.clone(), b.clone()))
        .collect();
    let refs: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let (dest, result) = import(&zip_of(&refs));
    assert!(
        matches!(result, Err(FileError::Invalid { .. })),
        "{result:?}"
    );
    assert!(dest.is_empty());
}

#[test]
fn a_pack_goes_into_an_empty_folder_only_and_is_bounded() {
    let fs = song_fs();
    let (bytes, _) = pack_bytes(&fs);
    let mut dest = MemFs::new();
    dest.insert("mine.txt", "x");
    assert_eq!(
        import_pack(
            Cursor::new(bytes.clone()),
            &mut dest,
            &PackLimits::default()
        )
        .unwrap_err(),
        FileError::DestinationNotEmpty
    );
    assert_eq!(dest.len(), 1);
    // A text file over the limit (by what it produces), too many entries, too many bytes.
    for limits in [
        PackLimits {
            max_text: 100,
            ..PackLimits::default()
        },
        PackLimits {
            max_entries: 3,
            ..PackLimits::default()
        },
        PackLimits {
            max_total: 100,
            ..PackLimits::default()
        },
        PackLimits {
            max_blob: 10,
            ..PackLimits::default()
        },
    ] {
        let mut dest = MemFs::new();
        let r = import_pack(Cursor::new(bytes.clone()), &mut dest, &limits);
        assert!(
            matches!(r, Err(FileError::BadPack { .. })),
            "{limits:?}: {r:?}"
        );
        assert!(dest.is_empty());
    }
    // A pack that deflates a lot: the produced length is what counts.
    let big = vec![b' '; 1 << 20];
    let mut entries = vec![(
        "project.json".to_owned(),
        fs.files()["project.json"].clone(),
    )];
    entries.push((format!("tracks/{}/track.json", t(50)), big));
    let refs: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let mut dest = MemFs::new();
    let r = import_pack(
        Cursor::new(zip_of(&refs)),
        &mut dest,
        &PackLimits {
            max_text: 1000,
            ..PackLimits::default()
        },
    );
    assert!(matches!(r, Err(FileError::BadPack { .. })));
    assert!(dest.is_empty());
}

// ------------------------------------------------------------------ the fuzz

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// One random change to some bytes: what a bad merge, a bad editor or a bad disk does.
fn mutate(rng: &mut Rng, bytes: &[u8]) -> Vec<u8> {
    let mut v = bytes.to_vec();
    let at = rng.below(v.len() + 1);
    let end = (at + 1 + rng.below(16)).min(v.len());
    match rng.below(14) {
        0 => {
            if let Some(b) = v.get_mut(at) {
                *b ^= 1 << rng.below(8);
            }
        }
        1 => {
            if let Some(b) = v.get_mut(at) {
                *b = rng.next() as u8;
            }
        }
        2 => {
            v.drain(at..end);
        }
        3 => v.truncate(at),
        4 => {
            let copy = v[at..end].to_vec();
            v.splice(at..at, copy);
        }
        5 => {
            let junk: Vec<u8> = (0..1 + rng.below(8)).map(|_| rng.next() as u8).collect();
            v.splice(at..at, junk);
        }
        6 => {
            // Swap two lines.
            let text = String::from_utf8_lossy(&v).into_owned();
            let mut lines: Vec<&str> = text.split('\n').collect();
            let (a, b) = (rng.below(lines.len()), rng.below(lines.len()));
            lines.swap(a, b);
            v = lines.join("\n").into_bytes();
        }
        7 => {
            // A number turned into a huge one.
            let text = String::from_utf8_lossy(&v).into_owned();
            let digits: Vec<usize> = text
                .char_indices()
                .filter(|(_, c)| c.is_ascii_digit())
                .map(|(i, _)| i)
                .collect();
            if !digits.is_empty() {
                let i = digits[rng.below(digits.len())];
                let huge = [
                    "99999999999999999999999",
                    "-9223372036854775808",
                    "9223372036854775807",
                    "1e999",
                    "-0",
                    "NaN",
                    "inf",
                ][rng.below(7)];
                v = format!("{}{huge}{}", &text[..i], &text[i + 1..]).into_bytes();
            }
        }
        8 => v.splice(at..at, *b"\t").for_each(drop),
        9 => v.splice(at..at, *b"\n").for_each(drop),
        10 => {
            let mut bom = vec![0xef, 0xbb, 0xbf];
            bom.extend_from_slice(&v);
            v = bom;
        }
        11 => {
            v = String::from_utf8_lossy(&v)
                .replace('\n', "\r\n")
                .into_bytes()
        }
        12 => v.clear(),
        _ => {
            if let Some(b) = v.get_mut(at) {
                *b = b'"';
            }
        }
    }
    v
}

fn fuzz_seconds() -> f64 {
    std::env::var("FAULK_FUZZ_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.5)
}

fn fuzz_seed() -> u64 {
    std::env::var("FAULK_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5eed_f00d_cafe_1234)
}

/// What an accepted folder must be: valid, and writable back to something that reads equal.
fn accepted(opened: &Opened) {
    assert_eq!(opened.project.validate(), Ok(()));
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &opened.project,
        &mut SaveState::new(),
        None,
        &options(),
    )
    .unwrap();
    let again = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(again.project, opened.project);
}

fn bases() -> Vec<MemFs> {
    let mut with_cache = MemFs::new();
    let o = SaveOptions {
        write_cache: true,
        ..options()
    };
    save(&mut with_cache, &song(), &mut SaveState::new(), None, &o).unwrap();
    vec![
        song_fs(),
        saved(&Project::new(ProjectId(id(1)), "Empty")),
        with_cache,
        saved(&common::decorate::decorate(song(), 5)),
    ]
}

#[test]
fn a_mutated_folder_is_accepted_validly_or_refused_with_a_type_never_a_panic() {
    let seed = fuzz_seed();
    println!("fuzz seed {seed} (FAULK_FUZZ_SEED to replay)");
    let mut rng = Rng(seed);
    let bases = bases();
    let start = std::time::Instant::now();
    let (mut runs, mut ok, mut err) = (0u64, 0u64, 0u64);
    while start.elapsed().as_secs_f64() < fuzz_seconds() {
        let mut fs = bases[rng.below(bases.len())].clone();
        // One to three files changed, the snapshot and the blobs included.
        for _ in 0..1 + rng.below(3) {
            let paths: Vec<String> = fs.files().keys().cloned().collect();
            let path = &paths[rng.below(paths.len())];
            let bytes = mutate(&mut rng, &fs.files()[path]);
            fs.insert(path.clone(), bytes);
        }
        for use_cache in [false, true] {
            match open(&fs, &OpenOptions { use_cache }) {
                Ok(opened) => {
                    ok += 1;
                    accepted(&opened);
                }
                Err(_) => err += 1,
            }
        }
        runs += 1;
    }
    println!("{runs} folders, {ok} accepted, {err} refused");
    assert!(runs > 50, "the fuzz ran only {runs} times");
    assert!(
        err > 0 && ok > 0,
        "a fuzz that only accepts or only refuses tests nothing"
    );
}

#[test]
fn a_mutated_pack_is_imported_validly_or_refused_never_a_panic() {
    let seed = fuzz_seed() ^ 0xA5A5;
    let mut rng = Rng(seed);
    let (pack, _) = pack_bytes(&song_fs());
    let start = std::time::Instant::now();
    let mut runs = 0u64;
    while start.elapsed().as_secs_f64() < fuzz_seconds() / 2.0 {
        let bytes = mutate(&mut rng, &pack);
        let (dest, result) = import(&bytes);
        match result {
            Ok(opened) => accepted(&opened),
            Err(_) => assert!(dest.is_empty(), "a refused pack leaves nothing"),
        }
        runs += 1;
    }
    assert!(runs > 20, "{runs}");
}

#[test]
fn a_mutated_session_never_stops_the_open() {
    let project = song();
    let mut rng = Rng(fuzz_seed() ^ 0x5E55);
    let mut fs = song_fs();
    let session = SessionState {
        selected_track: Some(TrackId(id(50))),
        solo: [TrackId(id(50))].into(),
        midi_input: Some("Keys".into()),
        armed: Default::default(),
        monitor: Default::default(),
        record: Default::default(),
    };
    save_session(&mut fs, &session).unwrap();
    let base = fs.read(".faulk/session.json").unwrap();
    for _ in 0..500 {
        fs.insert(".faulk/session.json", mutate(&mut rng, &base));
        let _ = load_session(&fs, &project);
        assert_eq!(open(&fs, &OpenOptions::default()).unwrap().project, project);
    }
}

#[test]
fn a_failed_import_into_a_real_folder_leaves_it_empty_and_usable() {
    let dir = std::env::temp_dir().join(format!("faulk-sv1-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut fs = song_fs();
    edit(&mut fs, &bass(&format!("clips/{}.json", t(21))), |s| {
        s.replacen(&t(3), &t(9), 1)
    });
    let entries: Vec<(String, Vec<u8>)> = fs
        .files()
        .iter()
        .map(|(p, b)| (p.clone(), b.clone()))
        .collect();
    let refs: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let bad = zip_of(&refs);
    let mut dest = StdFs::new(&dir);
    let r = import_pack(Cursor::new(bad.clone()), &mut dest, &PackLimits::default());
    assert!(matches!(r, Err(FileError::Invalid { .. })), "{r:?}");
    assert!(
        std::fs::read_dir(&dir).unwrap().next().is_none(),
        "the folder is empty again"
    );
    // So the retry with a good pack is not refused as "not empty".
    let (good, _) = pack_bytes(&song_fs());
    let opened = import_pack(Cursor::new(good), &mut dest, &PackLimits::default()).unwrap();
    assert_eq!(opened.project, song());
    let _ = std::fs::remove_dir_all(&dir);
}
