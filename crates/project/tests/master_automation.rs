//! #378: the master's automation lanes are saved with the project in
//! `master/automation/<lane>.tsv` and read back; a project without any writes the same files as
//! before (the format stays at version 1) and an older folder reads with none; they go through
//! the text, the blob form of a dense lane, the cache snapshot, the dirty-file save, the full
//! save that removes what is gone, and the pack; the project refuses a master lane that
//! targets a device the master's chain does not hold.

use project::file::*;
use project::*;
use std::collections::BTreeMap;
use std::io::Cursor;

mod common;

fn effect(order: OrderKey) -> Device {
    Device {
        order,
        role: DeviceRole::Effect,
        enabled: true,
        source: DeviceSource::Builtin {
            id: "faulk.compressor".into(),
            version: 1,
        },
        params: BTreeMap::from([("threshold".into(), -30.0)]),
        state: None,
    }
}

fn points(n: usize) -> BTreeMap<Tick, AutomationPoint> {
    (0..n)
        .map(|i| {
            (
                Tick(i as i64 * 10),
                AutomationPoint {
                    value: -((i % 90) as f64),
                    shape: if i % 5 == 0 {
                        Shape::Step
                    } else {
                        Shape::Linear
                    },
                },
            )
        })
        .collect()
}

struct Song {
    project: Project,
    device: DeviceId,
    volume: LaneId,
    threshold: LaneId,
}

fn song(volume_points: usize) -> Song {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Master lanes");
    let device = DeviceId::generate(&mut ids);
    project
        .master_devices
        .insert(device.clone(), effect(OrderKey::first()));
    let (volume, threshold) = (LaneId::generate(&mut ids), LaneId::generate(&mut ids));
    project.master_automation.insert(
        volume.clone(),
        AutomationLane {
            target: AutomationTarget::Mixer {
                param: MixerParam::Gain,
            },
            points: points(volume_points),
        },
    );
    project.master_automation.insert(
        threshold.clone(),
        AutomationLane {
            target: AutomationTarget::Device {
                device: device.clone(),
                param: "threshold".into(),
            },
            points: points(3),
        },
    );
    // A track with a lane of its own, apart from the master's.
    let track = TrackId::generate(&mut ids);
    let mut t = Track::new(TrackKind::Midi, "T", OrderKey::first());
    t.automation.insert(
        LaneId::generate(&mut ids),
        AutomationLane {
            target: AutomationTarget::Mixer {
                param: MixerParam::Pan,
            },
            points: points(2),
        },
    );
    project.tracks.insert(track, t);
    project.validate().unwrap();
    Song {
        project,
        device,
        volume,
        threshold,
    }
}

fn save_all(project: &Project, options: &SaveOptions) -> (MemFs, SaveState) {
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    save(&mut fs, project, &mut state, None, options).expect("save");
    (fs, state)
}

fn open_default(fs: &MemFs) -> Opened {
    open(fs, &OpenOptions::default()).expect("open")
}

fn master_files(fs: &MemFs) -> Vec<String> {
    fs.files()
        .keys()
        .filter(|p| p.starts_with("master/"))
        .cloned()
        .collect()
}

#[test]
fn the_master_lanes_are_files_of_the_master_and_read_back_equal() {
    let s = song(40);
    let (fs, state) = save_all(&s.project, &SaveOptions::default());
    common::rules::check(&fs);
    assert_eq!(
        master_files(&fs),
        [
            format!("master/automation/{}.tsv", s.volume),
            format!("master/automation/{}.tsv", s.threshold)
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
    );
    let opened = open_default(&fs);
    assert_eq!(opened.project, s.project);
    assert_eq!(opened.report.ignored, Vec::<String>::new());
    // The state of the open is the state of the write, and a save of what was read writes
    // nothing.
    assert_eq!(opened.state, state);
    let mut fs2 = fs.clone();
    let mut state = opened.state;
    fs2.take_log();
    let report = save(
        &mut fs2,
        &opened.project,
        &mut state,
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    assert!(
        report.written.is_empty() && report.removed.is_empty(),
        "{report:?}"
    );
    // The same bytes in a fresh folder.
    let (again, _) = save_all(&opened.project, &SaveOptions::default());
    assert_eq!(again.files(), fs.files());
}

#[test]
fn a_project_without_master_lanes_writes_what_it_always_wrote_and_an_older_folder_reads() {
    let mut s = song(10);
    s.project.master_automation.clear();
    let (fs, _) = save_all(&s.project, &SaveOptions::default());
    assert!(master_files(&fs).is_empty(), "no master folder");
    // `project.json` has no key for the lanes: the same file as before #378.
    let json = String::from_utf8(fs.read("project.json").unwrap()).unwrap();
    assert!(!json.contains("master_automation"), "{json}");
    let opened = open_default(&fs);
    assert!(opened.project.master_automation.is_empty());
    assert_eq!(opened.project, s.project);
    assert_eq!(opened.project.format, 1);
}

#[test]
fn a_dense_master_lane_is_a_blob_and_comes_back_a_table() {
    let s = song(150);
    let options = SaveOptions {
        layout: Layout {
            blob_above: 100,
            text_below: 50,
        },
        ..SaveOptions::default()
    };
    let (fs, state) = save_all(&s.project, &options);
    let text = String::from_utf8(
        fs.read(&format!("master/automation/{}.tsv", s.volume))
            .unwrap(),
    )
    .unwrap();
    assert!(text.contains("\tblob="), "{text}");
    assert!(state.is_blob_master_lane(&s.volume));
    assert!(!state.is_blob_master_lane(&s.threshold));
    let opened = open_default(&fs);
    assert_eq!(opened.project, s.project);
    assert!(opened.state.is_blob_master_lane(&s.volume));
    // Shrunk below the lower threshold, the lane goes back to text and forgets its blob.
    let mut project = opened.project;
    project.master_automation.get_mut(&s.volume).unwrap().points = points(10);
    let mut fs = fs;
    let mut state = opened.state;
    let dirty = {
        let mut d = Dirty::default();
        d.mark(Unit::MasterLane(s.volume.clone()));
        d
    };
    save(&mut fs, &project, &mut state, Some(&dirty), &options).unwrap();
    assert!(!state.is_blob_master_lane(&s.volume));
    assert_eq!(open_default(&fs).project, project);
}

#[test]
fn the_cache_gives_the_master_lanes_the_text_gives() {
    for density in [40, 150] {
        let s = song(density);
        let options = SaveOptions {
            layout: Layout {
                blob_above: 100,
                text_below: 50,
            },
            write_cache: true,
            ..SaveOptions::default()
        };
        let (fs, _) = save_all(&s.project, &options);
        let text = open_default(&fs);
        let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
        assert_eq!(cached.report.cache, CacheUse::Used, "{density}");
        assert_eq!(cached.project, text.project, "{density}");
        assert_eq!(cached.project, s.project, "{density}");
        assert_eq!(cached.state, text.state, "{density}");
        assert_eq!(cached.report.files_read, 1, "only project.json is read");
    }
}

#[test]
fn a_dirty_save_writes_one_master_lane_and_removes_the_one_that_is_gone() {
    let s = song(30);
    let (mut fs, mut state) = save_all(&s.project, &SaveOptions::default());
    let mut project = s.project.clone();
    // Edit one lane: one file.
    project
        .master_automation
        .get_mut(&s.threshold)
        .unwrap()
        .points
        .insert(
            Tick(999),
            AutomationPoint {
                value: -3.0,
                shape: Shape::Step,
            },
        );
    let mut dirty = Dirty::default();
    dirty.mark(Unit::MasterLane(s.threshold.clone()));
    fs.take_log();
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
        [format!("master/automation/{}.tsv", s.threshold)]
    );
    assert_eq!(open_default(&fs).project, project);
    // Delete the other: its file goes, and only its.
    project.master_automation.remove(&s.volume);
    let mut dirty = Dirty::default();
    dirty.mark(Unit::MasterLane(s.volume.clone()));
    let report = save(
        &mut fs,
        &project,
        &mut state,
        Some(&dirty),
        &SaveOptions::default(),
    )
    .unwrap();
    assert_eq!(
        report.removed,
        [format!("master/automation/{}.tsv", s.volume)]
    );
    assert_eq!(open_default(&fs).project, project);
    // A full save removes the file of a lane that is gone, too.
    let (mut fs, mut state) = save_all(&s.project, &SaveOptions::default());
    let report = save(&mut fs, &project, &mut state, None, &SaveOptions::default()).unwrap();
    assert_eq!(
        report.removed,
        [format!("master/automation/{}.tsv", s.volume)]
    );
    assert_eq!(open_default(&fs).project, project);
}

#[test]
fn a_pack_carries_the_master_lanes() {
    let s = song(150);
    let options = SaveOptions {
        layout: Layout {
            blob_above: 100,
            text_below: 50,
        },
        ..SaveOptions::default()
    };
    let (fs, _) = save_all(&s.project, &options);
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
    for path in master_files(&fs) {
        assert_eq!(dest.files().get(&path), fs.files().get(&path), "{path}");
    }
}

#[test]
fn a_stray_file_in_the_master_folder_is_ignored_and_a_corrupt_lane_is_refused() {
    let s = song(5);
    let (mut fs, _) = save_all(&s.project, &SaveOptions::default());
    fs.insert("master/automation/notes.txt", "mine");
    let opened = open_default(&fs);
    assert_eq!(opened.project, s.project);
    assert_eq!(opened.report.ignored, ["master/automation/notes.txt"]);
    let path = format!("master/automation/{}.tsv", s.volume);
    fs.insert(&path, "not a lane");
    assert!(matches!(
        open(&fs, &OpenOptions::default()),
        Err(FileError::Corrupt { .. })
    ));
}

#[test]
fn the_project_refuses_a_master_lane_that_no_master_device_can_hold() {
    let mut s = song(5);
    // A device the master's chain does not hold (a track's would not do either).
    let mut ids = IdGenerator::for_tests();
    let stranger = DeviceId::generate(&mut ids);
    s.project.master_automation.insert(
        LaneId::generate(&mut ids),
        AutomationLane {
            target: AutomationTarget::Device {
                device: stranger,
                param: "gain".into(),
            },
            points: BTreeMap::new(),
        },
    );
    let violations = s.project.validate().unwrap_err();
    assert!(
        violations
            .iter()
            .any(|v| matches!(v.kind, ViolationKind::UnknownDevice)
                && v.path.starts_with("master/automation/")),
        "{violations:?}"
    );
    // A gain out of range on the master's lane is as refused as on a track's.
    let mut s = song(5);
    s.project
        .master_automation
        .get_mut(&s.volume)
        .unwrap()
        .points
        .insert(
            Tick(7),
            AutomationPoint {
                value: 40.0,
                shape: Shape::Step,
            },
        );
    let violations = s.project.validate().unwrap_err();
    assert!(
        violations
            .iter()
            .any(|v| matches!(v.kind, ViolationKind::GainOutOfRange))
    );
    // The ids stay unique across the master's lanes and everything else.
    let s = song(1);
    assert!(s.project.contains_id(&s.volume.0));
    assert!(s.project.contains_id(&s.device.0));
    assert_eq!(s.project.counts().lanes, 3);
}
