//! The folder format (SV-1): write, read, rewrite; dirty-file saves; the QA-7 thresholds;
//! the cache snapshot; the session file; the git support files.

use project::file::*;
use project::*;
use proptest::prelude::*;
use std::collections::BTreeMap;

mod common;
use common::decorate::decorated;
use common::{ClipSpec, TrackSpec, name, track_spec};

fn save_all(project: &Project) -> (MemFs, SaveState) {
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    save(&mut fs, project, &mut state, None, &SaveOptions::default()).expect("save");
    (fs, state)
}

fn open_default(fs: &MemFs) -> Opened {
    open(fs, &OpenOptions::default()).expect("open")
}

fn lane_project(points: usize) -> (Project, TrackId, LaneId) {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Lanes");
    let mut track = Track::new(TrackKind::Midi, "T", OrderKey::first());
    track.color = Some(project::Rgb::new(1, 2, 3));
    let lane = LaneId::generate(&mut ids);
    track.automation.insert(
        lane.clone(),
        AutomationLane {
            target: AutomationTarget::Mixer {
                param: MixerParam::Pan,
            },
            points: lane_points(points),
        },
    );
    let tid = TrackId::generate(&mut ids);
    project.tracks.insert(tid.clone(), track);
    (project, tid, lane)
}

fn lane_points(n: usize) -> BTreeMap<Tick, AutomationPoint> {
    (0..n)
        .map(|i| {
            (
                Tick(i as i64 * 10),
                AutomationPoint {
                    value: (i % 199) as f64 / 100.0 - 1.0,
                    shape: if i % 7 == 0 {
                        Shape::Step
                    } else {
                        Shape::Linear
                    },
                },
            )
        })
        .collect()
}

fn set_points(project: &mut Project, t: &TrackId, l: &LaneId, n: usize) {
    project
        .tracks
        .get_mut(t)
        .and_then(|t| t.automation.get_mut(l))
        .expect("lane")
        .points = lane_points(n);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, failure_persistence: None, ..ProptestConfig::default() })]

    /// Write, read, rewrite: the project comes back equal and the second write has the same
    /// bytes, through the folder of the first write or a fresh one; and a save after the open
    /// writes nothing.
    #[test]
    fn write_read_rewrite_is_identical(
        seed in any::<u128>(),
        name in name(),
        tracks in prop::collection::vec(track_spec(), 0..6),
        scenes in 0usize..5,
    ) {
        let project = decorated(seed, name, tracks, scenes);
        prop_assert_eq!(project.validate(), Ok(()));
        let (mut fs, _) = save_all(&project);
        common::rules::check(&fs);
        let opened = open_default(&fs);
        prop_assert_eq!(&opened.project, &project);
        prop_assert_eq!(&opened.report.ignored, &Vec::<String>::new());

        // Rewritten into a fresh folder: the same bytes.
        let (fs2, state2) = save_all(&opened.project);
        prop_assert_eq!(fs.files(), fs2.files());
        // The state of a text open is the state of the write.
        prop_assert_eq!(&opened.state, &state2);

        // Saved again over the first folder with the state the open gave: nothing written.
        let mut state = opened.state;
        fs.take_log();
        let report = save(&mut fs, &project, &mut state, None, &SaveOptions::default()).unwrap();
        prop_assert!(report.written.is_empty() && report.removed.is_empty(), "{report:?}");
        prop_assert_eq!(fs.take_log(), vec![]);
    }

    /// The same through the cache snapshot: the snapshot and the text give the same project
    /// and the same state.
    #[test]
    fn the_cache_gives_what_the_text_gives(
        seed in any::<u128>(),
        name in name(),
        tracks in prop::collection::vec(track_spec(), 0..5),
        scenes in 0usize..4,
    ) {
        let project = decorated(seed, name, tracks, scenes);
        let mut fs = MemFs::new();
        let mut state = SaveState::new();
        let options = SaveOptions { write_cache: true, ..SaveOptions::default() };
        let report = save(&mut fs, &project, &mut state, None, &options).unwrap();
        prop_assert!(report.cache_written);
        let text = open_default(&fs);
        let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
        prop_assert_eq!(cached.report.cache, CacheUse::Used);
        prop_assert_eq!(cached.report.files_read, 1, "only project.json is read");
        prop_assert!(cached.report.files_from_cache > 0 || project.tracks.is_empty());
        prop_assert_eq!(&cached.project, &text.project);
        prop_assert_eq!(&cached.state, &text.state);
        prop_assert_eq!(&cached.project, &project);
    }
}

#[test]
fn an_empty_folder_is_not_a_project() {
    assert_eq!(
        open(&MemFs::new(), &OpenOptions::default()).unwrap_err(),
        FileError::NotAProject
    );
}

#[test]
fn a_new_project_writes_its_files_and_the_git_support_files() {
    let project = Project::new(ProjectId::generate(&mut IdGenerator::for_tests()), "Empty");
    let (fs, _) = save_all(&project);
    let names: Vec<&str> = fs.files().keys().map(String::as_str).collect();
    assert_eq!(names, [".gitattributes", ".gitignore", "project.json"]);
    assert_eq!(
        String::from_utf8_lossy(&fs.files()[".gitignore"]),
        ".faulk/\n"
    );
    let attributes = String::from_utf8_lossy(&fs.files()[".gitattributes"]).into_owned();
    assert!(attributes.contains("blobs/** filter=lfs diff=lfs merge=lfs -text"));
    assert!(attributes.contains("*.json text eol=lf merge=faulk diff=faulk"));
    assert!(attributes.contains("*.tsv text eol=lf merge=faulk diff=faulk"));
}

#[test]
fn support_files_are_written_when_absent_and_never_overwritten() {
    let project = Project::new(ProjectId::generate(&mut IdGenerator::for_tests()), "Empty");
    let (mut fs, _) = save_all(&project);
    fs.insert(".gitignore", "mine\n");
    let mut state = SaveState::new();
    save(&mut fs, &project, &mut state, None, &SaveOptions::default()).unwrap();
    assert_eq!(fs.files()[".gitignore"], b"mine\n");
    fs.remove(".gitattributes").unwrap();
    let mut state = SaveState::new();
    save(&mut fs, &project, &mut state, None, &SaveOptions::default()).unwrap();
    assert!(fs.exists(".gitattributes"));
    let off = SaveOptions {
        support_files: false,
        ..SaveOptions::default()
    };
    fs.remove(".gitattributes").unwrap();
    save(&mut fs, &project, &mut SaveState::new(), None, &off).unwrap();
    assert!(!fs.exists(".gitattributes"));
}

fn song() -> Project {
    // Two MIDI tracks (notes, an effect, a lane, placements) and an audio track.
    let spec = |midi: bool, clips: usize| TrackSpec {
        midi,
        name: "Track".into(),
        gain: Some(-3.5),
        pan: 0.25,
        mute: false,
        effects: vec![("cutoff".into(), 800.0)],
        clips: (0..clips)
            .map(|i| ClipSpec {
                length: 3840,
                notes: (0..8)
                    .map(|n| (n * 480, 36 + n as u8, 240, 90 + n as u8))
                    .collect(),
                slot: true,
                placed_at: Some(i as i64 * 3840),
            })
            .collect(),
        points: (0..30)
            .map(|i| (i * 100, f64::from(i as i32) * 3.0 - 40.0, i % 4 == 0))
            .collect(),
    };
    decorated(
        7,
        "Song".into(),
        vec![spec(true, 3), spec(true, 2), spec(false, 2)],
        3,
    )
}

fn first_midi_clip(p: &Project) -> Option<(TrackId, ClipId)> {
    p.tracks.iter().find_map(|(t, tr)| {
        tr.clips
            .iter()
            .find(|(_, c)| c.content.is_midi())
            .map(|(c, _)| (t.clone(), c.clone()))
    })
}

/// Edits a note of a MIDI clip (a note is a value: remove, insert).
fn edit_note(p: &mut Project, t: &TrackId, c: &ClipId) {
    let ClipContent::Midi { notes, .. } = &mut p
        .tracks
        .get_mut(t)
        .and_then(|t| t.clips.get_mut(c))
        .expect("clip")
        .content
    else {
        panic!("not midi")
    };
    let old = *notes.iter().next().expect("a note");
    notes.remove(&old);
    notes.insert(Note {
        velocity: if old.velocity == 1 {
            2
        } else {
            old.velocity - 1
        },
        ..old
    });
}

#[test]
fn editing_one_note_rewrites_one_file() {
    let mut project = song();
    let (t, c) = first_midi_clip(&project).expect("the song has a MIDI clip");
    let (mut fs, mut state) = save_all(&project);
    fs.take_log();
    edit_note(&mut project, &t, &c);

    let mut dirty = Dirty::new();
    dirty.mark(Unit::Notes(t.clone(), c.clone()));
    let report = save(
        &mut fs,
        &project,
        &mut state,
        Some(&dirty),
        &SaveOptions::default(),
    )
    .unwrap();
    assert_eq!(
        report.written,
        vec![format!("tracks/{t}/clips/{c}.notes.tsv")]
    );
    assert_eq!(fs.take_log().len(), 1, "one write, nothing else");

    // The dirty save leaves the folder exactly as a full save of the project does.
    let (full, _) = save_all(&project);
    assert_eq!(fs.files(), full.files());

    // A full save with the same state finds nothing more to write.
    let report = save(&mut fs, &project, &mut state, None, &SaveOptions::default()).unwrap();
    assert!(report.written.is_empty());
    assert_eq!(fs.take_log(), vec![]);
}

#[test]
fn editing_one_note_with_the_cache_on_writes_the_file_and_the_snapshot_only() {
    let mut project = song();
    let (t, c) = first_midi_clip(&project).expect("a MIDI clip");
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    save(&mut fs, &project, &mut state, None, &options).unwrap();
    fs.take_log();
    edit_note(&mut project, &t, &c);
    let mut dirty = Dirty::new();
    dirty.mark(Unit::Notes(t.clone(), c.clone()));
    let report = save(&mut fs, &project, &mut state, Some(&dirty), &options).unwrap();
    assert!(report.cache_written);
    let log = fs.take_log();
    assert_eq!(
        log,
        vec![
            Op::Write(format!("tracks/{t}/clips/{c}.notes.tsv")),
            Op::Write(".faulk/cache/project.snap".into())
        ]
    );
    // The updated snapshot is current: it reads back the edited project.
    let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
    assert_eq!(cached.project, project);
    assert_eq!(cached.report.files_read, 1);
}

#[test]
fn a_dirty_save_removes_the_files_of_what_is_gone() {
    let mut project = song();
    let (mut fs, mut state) = save_all(&project);
    let (t, track) = project
        .tracks
        .iter()
        .next()
        .map(|(t, tr)| (t.clone(), tr.clone()))
        .unwrap();
    let mut dirty = Dirty::new();

    // A clip.
    if let Some(c) = track.clips.keys().next().cloned() {
        project.tracks.get_mut(&t).unwrap().clips.remove(&c);
        project
            .tracks
            .get_mut(&t)
            .unwrap()
            .placements
            .retain(|_, p| p.clip != c);
        dirty.mark(Unit::Clip(t.clone(), c.clone()));
        dirty.mark(Unit::Arrangement(t.clone()));
        let report = save(
            &mut fs,
            &project,
            &mut state,
            Some(&dirty),
            &SaveOptions::default(),
        )
        .unwrap();
        assert!(
            report
                .removed
                .iter()
                .any(|p| p.ends_with(&format!("{c}.json")))
        );
        assert!(!fs.files().keys().any(|p| p.contains(c.0.as_str())));
    }
    // The whole track.
    project.tracks.remove(&t);
    let mut dirty = Dirty::new();
    dirty.mark(Unit::Track(t.clone()));
    save(
        &mut fs,
        &project,
        &mut state,
        Some(&dirty),
        &SaveOptions::default(),
    )
    .unwrap();
    assert!(!fs.files().keys().any(|p| p.contains(t.0.as_str())));
    let (full, _) = save_all(&project);
    assert_eq!(fs.files(), full.files());

    // A full save removes too: what the state knows and the project lost.
    let mut project2 = project.clone();
    let t2 = project2.tracks.keys().next().cloned().unwrap();
    project2.tracks.remove(&t2);
    save(
        &mut fs,
        &project2,
        &mut state,
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    assert!(!fs.files().keys().any(|p| p.contains(t2.0.as_str())));
}

#[test]
fn a_failed_write_is_not_remembered_and_the_retry_completes() {
    struct Failing {
        inner: MemFs,
        fail_at: usize,
        writes: usize,
    }
    impl FileSystem for Failing {
        fn read(&self, p: &str) -> Result<Vec<u8>, FsError> {
            self.inner.read(p)
        }
        fn write(&mut self, p: &str, b: &[u8]) -> Result<(), FsError> {
            self.writes += 1;
            if self.writes == self.fail_at {
                return Err(FsError::Other("disk full".into()));
            }
            self.inner.write(p, b)
        }
        fn remove(&mut self, p: &str) -> Result<(), FsError> {
            self.inner.remove(p)
        }
        fn exists(&self, p: &str) -> bool {
            self.inner.exists(p)
        }
        fn list(&self, d: &str) -> Result<Vec<DirEntry>, FsError> {
            self.inner.list(d)
        }
    }
    let project = song();
    let (reference, _) = save_all(&project);
    let total = reference.len();
    for fail_at in 1..=total {
        let mut fs = Failing {
            inner: MemFs::new(),
            fail_at,
            writes: 0,
        };
        let mut state = SaveState::new();
        let err = save(&mut fs, &project, &mut state, None, &SaveOptions::default()).unwrap_err();
        assert!(matches!(err, FileError::Io { .. }), "{err:?}");
        // Whatever was written, the folder never panics a reader: it opens or is refused.
        let _ = open(&fs.inner, &OpenOptions::default());
        // The retry (the same state) writes what is left and reaches the full result.
        fs.fail_at = usize::MAX;
        save(&mut fs, &project, &mut state, None, &SaveOptions::default()).unwrap();
        assert_eq!(
            fs.inner.files(),
            reference.files(),
            "failed at write {fail_at}"
        );
    }
}

#[test]
fn a_value_the_format_cannot_write_is_a_typed_error_not_a_panic() {
    let project = song();
    let (t, d) = project
        .tracks
        .iter()
        .find_map(|(t, tr)| tr.devices.keys().next().map(|d| (t.clone(), d.clone())))
        .unwrap();

    // A non-finite parameter, in a JSON file.
    let mut nan = project.clone();
    nan.tracks
        .get_mut(&t)
        .unwrap()
        .devices
        .get_mut(&d)
        .unwrap()
        .params
        .insert("x".into(), f64::NAN);
    let err = save(
        &mut MemFs::new(),
        &nan,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, FileError::Unwritable { .. }), "{err:?}");

    // A non-finite point, in a TSV file; a parameter id with a tab, in a lane header.
    for tab in [false, true] {
        let mut bad = project.clone();
        let track = bad.tracks.get_mut(&t).unwrap();
        let lane = track.automation.values_mut().next().unwrap();
        if tab {
            lane.target = AutomationTarget::Device {
                device: d.clone(),
                param: "a\tb".into(),
            };
        } else if let Some(p) = lane.points.values_mut().next() {
            p.value = f64::INFINITY;
        }
        let err = save(
            &mut MemFs::new(),
            &bad,
            &mut SaveState::new(),
            None,
            &SaveOptions::default(),
        )
        .unwrap_err();
        assert!(
            matches!(err, FileError::Unwritable { .. }),
            "{tab}: {err:?}"
        );
    }
}

#[test]
fn saving_into_another_folder_needs_a_fresh_state() {
    let project = song();
    let (fs, _) = save_all(&project);
    let opened = open_default(&fs);
    // The state of the open knows the files of the first folder: a second folder saved with
    // it would get nothing. A fresh state writes everything.
    let mut other = MemFs::new();
    let mut state = opened.state;
    let report = save(
        &mut other,
        &project,
        &mut state,
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    assert!(
        report.written.len() <= 2,
        "only the support files: {report:?}"
    );
    let mut other = MemFs::new();
    save(
        &mut other,
        &project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    assert_eq!(other.files(), fs.files());
}

// ------------------------------------------------------------------------ QA-7

fn lane_is_blob(fs: &MemFs, t: &TrackId, l: &LaneId) -> bool {
    let text =
        String::from_utf8(fs.read(&format!("tracks/{t}/automation/{l}.tsv")).unwrap()).unwrap();
    text.contains("\tblob=")
}

fn blobs(fs: &MemFs) -> usize {
    fs.files()
        .keys()
        .filter(|p| p.starts_with("blobs/"))
        .count()
}

#[test]
fn a_lane_goes_to_a_blob_above_100_000_points_and_back_to_text_below_50_000() {
    let (mut project, t, l) = lane_project(1000);
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    let options = SaveOptions::default();
    let mut step = |n: usize, expect_blob: bool, project: &mut Project| {
        set_points(project, &t, &l, n);
        save(&mut fs, project, &mut state, None, &options).unwrap();
        assert_eq!(lane_is_blob(&fs, &t, &l), expect_blob, "{n} points");
        // Whatever the form, it reads back the same.
        assert_eq!(open_default(&fs).project, *project, "{n} points");
        assert_eq!(state.is_blob_lane(&t, &l), expect_blob);
    };
    step(1_000, false, &mut project);
    step(99_999, false, &mut project); // no history: text
    step(100_000, false, &mut project); // not above the threshold
    step(100_001, true, &mut project);
    step(99_999, true, &mut project); // hysteresis: stays a blob
    step(50_000, true, &mut project); // not below the threshold
    step(49_999, false, &mut project);
    step(99_999, false, &mut project); // and stays text
    step(100_001, true, &mut project);
    // The old blobs stay (§ 3.4: never collected on their own).
    assert!(blobs(&fs) >= 2);
}

#[test]
fn a_blob_lane_reopens_as_a_blob_and_a_no_op_save_writes_nothing() {
    let (project, t, l) = lane_project(70_000);
    // 70 000 points with a state that says blob (it came from above the threshold).
    let options = SaveOptions {
        layout: Layout {
            blob_above: 60_000,
            text_below: 50_000,
        },
        ..SaveOptions::default()
    };
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    save(&mut fs, &project, &mut state, None, &options).unwrap();
    assert!(lane_is_blob(&fs, &t, &l));
    // 70 000 is above 60 000: blob. Now between the thresholds only: shrink the layout's
    // upper bound after the fact, as if the lane had come down from above.
    let between = SaveOptions {
        layout: Layout::QA7,
        ..SaveOptions::default()
    };
    for use_cache in [false, true] {
        let mut fs2 = fs.clone();
        if use_cache {
            // A snapshot of the folder as it is.
            let o = open_default(&fs2);
            let mut s = o.state;
            let with_cache = SaveOptions {
                write_cache: true,
                layout: options.layout,
                ..SaveOptions::default()
            };
            save(&mut fs2, &project, &mut s, None, &with_cache).unwrap();
        }
        let opened = open(&fs2, &OpenOptions { use_cache }).unwrap();
        assert!(opened.state.is_blob_lane(&t, &l), "cache: {use_cache}");
        let mut state = opened.state;
        fs2.take_log();
        // 70 000 is between 50 000 and 100 000: it keeps its form with the QA-7 layout.
        let report = save(&mut fs2, &project, &mut state, None, &between).unwrap();
        assert!(
            report.written.is_empty() && report.blobs.is_empty(),
            "{report:?}"
        );
        assert_eq!(fs2.take_log(), vec![]);
        assert!(lane_is_blob(&fs2, &t, &l));
    }
}

#[test]
fn blobs_are_named_by_their_hash_and_never_rewritten() {
    let (project, t, l) = lane_project(100_001);
    let (mut fs, mut state) = save_all(&project);
    let blob_names: Vec<String> = fs
        .files()
        .keys()
        .filter(|p| p.starts_with("blobs/"))
        .cloned()
        .collect();
    assert_eq!(blob_names.len(), 1);
    let hex = blob_names[0].strip_prefix("blobs/").unwrap();
    assert_eq!(hex.len(), 64);
    assert_eq!(
        Hash::parse(hex).unwrap(),
        file_sha(&fs.files()[&blob_names[0]])
    );
    // Marking the lane dirty again writes neither the text nor the blob.
    fs.take_log();
    let mut dirty = Dirty::new();
    dirty.mark(Unit::Lane(t, l));
    let report = save(
        &mut fs,
        &project,
        &mut state,
        Some(&dirty),
        &SaveOptions::default(),
    )
    .unwrap();
    assert!(report.written.is_empty() && report.blobs.is_empty());
    assert_eq!(fs.take_log(), vec![]);
}

fn file_sha(bytes: &[u8]) -> Hash {
    use sha2::{Digest, Sha256};
    Hash(Sha256::digest(bytes).into())
}

// ------------------------------------------------------------------------ cache

#[test]
fn the_cache_rereads_only_the_files_that_changed() {
    let project = song();
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    save(&mut fs, &project, &mut state, None, &options).unwrap();
    let total = open_default(&fs).report.files_read;
    let (t, c) = first_midi_clip(&project).unwrap();
    let notes_path = format!("tracks/{t}/clips/{c}.notes.tsv");

    // Same size, other bytes (the velocity 100 -> 101 kind of edit, as `git checkout` does).
    let mut bytes = fs.read(&notes_path).unwrap();
    let last_digit = bytes.iter().rposition(u8::is_ascii_digit).unwrap();
    bytes[last_digit] = if bytes[last_digit] == b'1' {
        b'2'
    } else {
        b'1'
    };
    fs.insert(notes_path.clone(), bytes.clone());
    let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
    assert_eq!(
        cached.report.files_read, 2,
        "project.json and the edited file"
    );
    assert_eq!(cached.report.files_from_cache, total - 2);
    let direct = open_default(&fs);
    assert_eq!(cached.project, direct.project);
    assert_ne!(cached.project, project);

    // A file added by hand is noticed: it is not in the snapshot, so it is read as text.
    let (dev_t, dev_id) = project
        .tracks
        .iter()
        .find_map(|(t, tr)| {
            tr.devices
                .iter()
                .find(|(_, d)| d.role == DeviceRole::Effect)
                .map(|(d, _)| (t.clone(), d.clone()))
        })
        .unwrap();
    let copy = DeviceId::generate(&mut IdGenerator::new(77, 77));
    let source = fs
        .read(&format!("tracks/{dev_t}/devices/{dev_id}.json"))
        .unwrap();
    let text = String::from_utf8(source).unwrap();
    let added = format!("tracks/{dev_t}/devices/{copy}.json");
    fs.insert(added.clone(), text);
    let with_added = open(&fs, &OpenOptions { use_cache: true }).unwrap();
    assert_eq!(
        with_added.project.tracks[&dev_t].devices.len(),
        project.tracks[&dev_t].devices.len() + 1
    );
    assert_eq!(with_added.project, open_default(&fs).project);
    fs.remove(&added).unwrap();

    // A file removed: noticed.
    fs.remove(&notes_path).unwrap();
    let refused = open(&fs, &OpenOptions { use_cache: true }).unwrap_err();
    assert!(matches!(refused, FileError::Missing { .. }), "{refused:?}");
}

#[test]
fn a_bad_snapshot_is_ignored_never_an_error() {
    let project = song();
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    save(&mut fs, &project, &mut state, None, &options).unwrap();
    let snap = fs.read(".faulk/cache/project.snap").unwrap();
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("truncated", snap[..snap.len() / 2].to_vec()),
        ("garbage", vec![7; 100]),
    ];
    let mut flipped = snap.clone();
    flipped[snap.len() / 2] ^= 0x40;
    cases.push(("checksum", flipped));
    let mut other_format = snap.clone();
    other_format[8] = 99;
    cases.push(("format", other_format));
    for (what, bytes) in cases {
        let mut fs = fs.clone();
        fs.insert(".faulk/cache/project.snap", bytes);
        let opened = open(&fs, &OpenOptions { use_cache: true }).expect(what);
        assert!(
            matches!(opened.report.cache, CacheUse::Unusable(_)),
            "{what}"
        );
        assert_eq!(opened.project, project, "{what}");
        assert_eq!(opened.report.files_from_cache, 0);
    }
    fs.remove(".faulk/cache/project.snap").unwrap();
    let opened = open(&fs, &OpenOptions { use_cache: true }).unwrap();
    assert_eq!(opened.report.cache, CacheUse::Missing);
    assert_eq!(opened.project, project);
}

#[test]
fn a_snapshot_after_a_save_with_the_cache_off_is_rebuilt_in_full() {
    let project = song();
    let (mut fs, _) = save_all(&project);
    // Opened without a cache, then saved with the cache on: the state has no payloads, so
    // the whole project is emitted once more.
    let mut state = open_default(&fs).state;
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    let report = save(&mut fs, &project, &mut state, None, &options).unwrap();
    assert!(report.cache_written);
    let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
    assert_eq!(cached.report.files_read, 1);
    assert_eq!(cached.project, project);
}

#[test]
fn memfs_stamps_follow_the_content_unless_the_host_gives_its_own() {
    let mut fs = MemFs::new();
    fs.insert("a/x", "one");
    let a = fs.list("a").unwrap()[0].stamp;
    fs.insert("a/x", "two");
    assert_ne!(fs.list("a").unwrap()[0].stamp, a);
    fs.insert_with_stamp("a/x", "two", 42);
    assert_eq!(fs.list("a").unwrap()[0].stamp, 42);
}

// ------------------------------------------------------------------------ session

#[test]
fn the_session_is_lenient_and_forgets_missing_tracks() {
    let project = song();
    let (mut fs, _) = save_all(&project);
    let (s, how) = load_session(&fs, &project);
    assert_eq!((s, how), (SessionState::default(), SessionLoad::Missing));

    let t = project.tracks.keys().next().unwrap().clone();
    let gone = TrackId::generate(&mut IdGenerator::new(9, 9));
    let session = SessionState {
        selected_track: Some(gone.clone()),
        solo: [t.clone(), gone.clone()].into(),
        midi_input: Some("Keys".into()),
        armed: [t.clone()].into(),
        monitor: [gone].into(),
        record: project::RecordSettings { offset_ms: -12 },
    };
    assert!(save_session(&mut fs, &session).unwrap());
    assert!(
        !save_session(&mut fs, &session).unwrap(),
        "unchanged: not rewritten"
    );
    let (loaded, how) = load_session(&fs, &project);
    assert_eq!(how, SessionLoad::Loaded);
    assert_eq!(loaded.solo, [t.clone()].into());
    assert_eq!(loaded.selected_track, None);
    assert!(loaded.monitor.is_empty());
    assert_eq!(loaded.armed, [t].into());
    assert_eq!(loaded.midi_input.as_deref(), Some("Keys"));

    for garbage in [&b"{"[..], b"[]", b"\xff\xfe", b"{\"solo\": 3}"] {
        fs.insert(".faulk/session.json", garbage);
        let (s, how) = load_session(&fs, &project);
        assert_eq!((s, how), (SessionState::default(), SessionLoad::Reset));
    }
    // The session never goes in the project's files: opening it changes nothing.
    assert_eq!(open_default(&fs).project, project);
}

#[test]
fn std_fs_writes_a_real_folder_and_cleans_up() {
    let dir = std::env::temp_dir().join(format!("faulk-sv1-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = song();
    {
        let mut fs = StdFs::new(&dir);
        let mut state = SaveState::new();
        save(&mut fs, &project, &mut state, None, &SaveOptions::default()).unwrap();
        let opened = open(&fs, &OpenOptions::default()).unwrap();
        assert_eq!(opened.project, project);
        assert!(dir.join("project.json").is_file());
        // No temporary file or folder is left: `.faulk/` is not even created.
        assert!(!dir.join(".faulk").exists());
        assert!(walk(&fs).unwrap().iter().all(|(p, _)| !p.contains("tmp")));

        // Removing the last file of a track removes its (then empty) folders.
        let t = project.tracks.keys().next().unwrap().clone();
        let mut project2 = project.clone();
        project2.tracks.remove(&t);
        save(
            &mut fs,
            &project2,
            &mut state,
            None,
            &SaveOptions::default(),
        )
        .unwrap();
        assert!(!dir.join("tracks").join(t.to_string()).exists());
        assert_eq!(
            open(&fs, &OpenOptions::default()).unwrap().project,
            project2
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_outside_edit_is_not_vouched_for_by_a_later_cached_save() {
    // `git pull` changes a notes file while the project is open; Faulk then saves something
    // else with the cache on. The snapshot must not pair the pulled file's new stamp with
    // the notes Faulk still holds.
    let project = song();
    let (t, c) = first_midi_clip(&project).unwrap();
    let path = format!("tracks/{t}/clips/{c}.notes.tsv");
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    for open_with_cache in [false, true] {
        let mut fs = MemFs::new();
        save(&mut fs, &project, &mut SaveState::new(), None, &options).unwrap();
        let mut state = open(
            &fs,
            &OpenOptions {
                use_cache: open_with_cache,
            },
        )
        .unwrap()
        .state;
        // The outside edit: the same size, other bytes, a new stamp.
        let mut bytes = fs.read(&path).unwrap();
        let at = bytes.iter().rposition(u8::is_ascii_digit).unwrap();
        bytes[at] = if bytes[at] == b'1' { b'2' } else { b'1' };
        fs.insert(path.clone(), bytes.clone());
        // Faulk saves another unit, with the cache.
        let mut edited = project.clone();
        edited.name = "Renamed".into();
        let mut dirty = Dirty::new();
        dirty.mark(Unit::Project);
        save(&mut fs, &edited, &mut state, Some(&dirty), &options).unwrap();
        // The cached open sees the pulled notes, as the text does.
        let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
        let text = open_default(&fs);
        assert_eq!(
            cached.project, text.project,
            "cache on at open: {open_with_cache}"
        );
        assert_ne!(cached.project, edited);
        assert_eq!(
            fs.read(&path).unwrap(),
            bytes,
            "the pulled file is untouched"
        );
    }
}

#[test]
fn a_snapshot_whose_payload_does_not_decode_falls_back_to_the_text() {
    let project = song();
    let mut fs = MemFs::new();
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    save(&mut fs, &project, &mut SaveState::new(), None, &options).unwrap();
    // Break every entry's payload but keep the checksum right: byte after the header of
    // the first entry's payload would be hard to find, so rebuild the file from its entries.
    let snap = fs.read(".faulk/cache/project.snap").unwrap();
    let mut body = snap[..snap.len() - 8].to_vec();
    // Entry payloads are the tails of the entries: flip bits across the body after the
    // preamble (12 bytes) in many places, then fix the checksum.
    for i in (40..body.len()).step_by(37) {
        body[i] ^= 0x55;
    }
    let sum = fast_hash(&body);
    body.extend_from_slice(&sum.to_le_bytes());
    fs.insert(".faulk/cache/project.snap", body);
    let opened = open(&fs, &OpenOptions { use_cache: true }).expect("the text is fine");
    assert_eq!(opened.project, project);
}

#[test]
fn a_cached_save_ignores_the_users_folders_and_never_fails_for_them() {
    let project = song();
    let mut fs = MemFs::new();
    // A sample library nested deeper than any project folder.
    fs.insert("samples/a/b/c/d/e/f/g/h/i/j/k/kick.wav", vec![1, 2, 3]);
    fs.insert("notes.txt", "mine");
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    let report = save(&mut fs, &project, &mut SaveState::new(), None, &options).unwrap();
    assert!(report.cache_written);
    let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
    assert_eq!(cached.report.files_read, 1);
    assert_eq!(cached.project, project);
}

#[test]
fn std_fs_refuses_paths_that_lead_out_of_the_folder() {
    let dir = std::env::temp_dir().join(format!("faulk-sv1-unsafe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut fs = StdFs::new(&dir);
    for bad in [
        "../x.json",
        "tracks/../../x.json",
        "a\\b.json",
        "C:\\x.json",
        "C:x.json",
        "tracks/devices/..\\..\\x.json",
    ] {
        assert!(fs.write(bad, b"x").is_err(), "{bad}");
        assert!(fs.read(bad).is_err(), "{bad}");
        assert!(fs.remove(bad).is_err(), "{bad}");
        assert!(!fs.exists(bad), "{bad}");
    }
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A project saved before clips were named: opening names its MIDI clips in memory, in the
/// order a musician meets them, and writes nothing; the next save writes the names, and
/// the project then opens as it is.
#[test]
fn opening_names_the_unnamed_clips_and_the_next_save_keeps_them() {
    opening_colors_the_uncolored_tracks_by_position();
    names_the_unnamed_clips();
}

/// #479: a folder saved before track colours were stored opens with each uncoloured track
/// holding the hue of its position, writes nothing, and keeps it after the next save.
fn opening_colors_the_uncolored_tracks_by_position() {
    use project::colors::palette_hue;
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Old");
    let mut key = OrderKey::first();
    let mut tracks = Vec::new();
    for i in 0..3 {
        let mut track = Track::new(TrackKind::Midi, "T", key.clone());
        if i == 1 {
            track.color = Some(project::Rgb::new(1, 2, 3));
        }
        let id = TrackId::generate(&mut ids);
        project.tracks.insert(id.clone(), track);
        tracks.push(id);
        key = OrderKey::between(Some(&key), None).unwrap();
    }
    let (mut fs, _) = save_all(&project);
    let _ = fs.take_log();

    let opened = open_default(&fs);
    assert_eq!(opened.report.colored_tracks, 2);
    assert_eq!(fs.take_log(), vec![], "opening writes nothing");
    let color = |i: usize| opened.project.tracks[&tracks[i]].color;
    assert_eq!(color(0), Some(palette_hue(0)));
    assert_eq!(
        color(1),
        Some(project::Rgb::new(1, 2, 3)),
        "a stored colour is kept"
    );
    assert_eq!(color(2), Some(palette_hue(2)));

    let mut state = opened.state;
    save(
        &mut fs,
        &opened.project,
        &mut state,
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    let again = open_default(&fs);
    assert_eq!(again.report.colored_tracks, 0);
    assert_eq!(again.project, opened.project);
}

fn names_the_unnamed_clips() {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Old");
    let mut track = Track::new(TrackKind::Midi, "T", OrderKey::first());
    track.color = Some(project::Rgb::new(1, 2, 3));
    let clip = |name: &str| Clip {
        name: name.to_owned(),
        color: None,
        slot: None,
        launch_quantize: None,
        content: ClipContent::Midi {
            length: Tick(960),
            looping: None,
            notes: Default::default(),
        },
    };
    // Placed late, placed early, unplaced, and one the user named "T 1" himself (the default names take the track's name, #473).
    let (late, early, loose, kept) = (
        ClipId::generate(&mut ids),
        ClipId::generate(&mut ids),
        ClipId::generate(&mut ids),
        ClipId::generate(&mut ids),
    );
    for (id, name) in [(&late, ""), (&early, ""), (&loose, ""), (&kept, "T 1")] {
        track.clips.insert(id.clone(), clip(name));
    }
    for (id, start) in [(&late, 3840), (&early, 0)] {
        track.placements.insert(
            PlacementId::generate(&mut ids),
            Placement {
                clip: id.clone(),
                start: Tick(start),
                span: Span::Ticks {
                    length: Tick(960),
                    offset: Tick(0),
                },
            },
        );
    }
    project.tracks.insert(TrackId::generate(&mut ids), track);
    let (mut fs, _) = save_all(&project);
    let _ = fs.take_log();

    let opened = open_default(&fs);
    assert_eq!(opened.report.named_clips, 3);
    assert_eq!(fs.take_log(), vec![], "opening writes nothing");
    let track = opened.project.tracks.values().next().unwrap();
    let name = |id: &ClipId| track.clips[id].name.as_str();
    assert_eq!(
        [name(&early), name(&late), name(&loose), name(&kept)],
        ["T 2", "T 3", "T 4", "T 1"],
        "placed clips first by start, then the unplaced; the user's own name is kept"
    );

    // The names reach the folder with the next save, whatever it was for.
    let mut state = opened.state;
    let report = save(
        &mut fs,
        &opened.project,
        &mut state,
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    assert_eq!(report.written.len(), 3, "{report:?}");
    let again = open_default(&fs);
    assert_eq!(again.report.named_clips, 0);
    assert_eq!(again.project, opened.project);
}

#[test]
fn a_track_height_is_kept_with_the_folder_and_absent_when_unset() {
    let (mut project, tid, _) = lane_project(0);
    let track_json = |fs: &MemFs| {
        let path = format!("tracks/{tid}/track.json");
        String::from_utf8(fs.files()[&path].clone()).expect("utf8")
    };
    let (fs, _) = save_all(&project);
    assert!(!track_json(&fs).contains("height"), "unset: not written");

    project.tracks.get_mut(&tid).expect("track").height = Some(120);
    let (fs, _) = save_all(&project);
    assert!(track_json(&fs).contains("\"height\": 120"));
    let opened = open_default(&fs);
    assert_eq!(opened.project.tracks[&tid].height, Some(120));
    assert_eq!(opened.project, project);
}
