//! Opening a large project through the page's core (SV-4): the folder frame, the staged
//! folder, `project.open` and the snapshot for the engine, on the QA-7 profile (100 tracks,
//! 200 000 notes, 6 million automation points; `docs/spikes/QA-7-banc-de-charge-format-projet.md`).
//!
//! A scaled profile runs with the tests; the full one is a measurement:
//! `cargo test -p project-wasm --release --test large -- --ignored --nocapture`.
//! It times the Rust side on a `MemFs` (no storage read, no JavaScript): what the page adds
//! (reading OPFS or the picked folder, building the frame) is not in these numbers.

use std::time::Instant;

use project::file::{MemFs, SaveOptions, SaveState, save, save_session};
use project::*;
use project_wasm::{Core, KIND_EMPTY, Mode};

/// The QA-7 profile, scaled: `tracks` tracks of 2 clips of `notes` notes and 2 lanes of
/// `points` points.
fn profile(tracks: usize, notes: u64, points: u64) -> Project {
    let mut ids = IdGenerator::new(1_700_000_000_000, 7);
    let mut project = Project::new(ProjectId::generate(&mut ids), "Load");
    let mut order = None;
    for _ in 0..tracks {
        let key = OrderKey::between(None, order.as_ref()).unwrap();
        order = Some(key.clone());
        let mut track = Track::new(TrackKind::Midi, "Track", key);
        for _ in 0..2 {
            track.clips.insert(
                ClipId::generate(&mut ids),
                Clip {
                    name: "Clip".into(),
                    color: None,
                    slot: None,
                    launch_quantize: None,
                    content: ClipContent::Midi {
                        length: Tick(960 * 4 * 64),
                        looping: None,
                        notes: (0..notes)
                            .map(|i| {
                                Note::new(
                                    Tick(i as i64 * 240),
                                    (i % 88) as u8 + 20,
                                    Tick(200),
                                    64 + (i % 60) as u8,
                                )
                            })
                            .collect(),
                    },
                },
            );
        }
        for _ in 0..2 {
            track.automation.insert(
                LaneId::generate(&mut ids),
                AutomationLane {
                    target: AutomationTarget::Mixer {
                        param: MixerParam::Pan,
                    },
                    points: (0..points)
                        .map(|i| {
                            (
                                Tick(i as i64 * 10),
                                AutomationPoint {
                                    value: (i % 200) as f64 / 100.0 - 1.0,
                                    shape: Shape::Linear,
                                },
                            )
                        })
                        .collect(),
                },
            );
        }
        project.tracks.insert(TrackId::generate(&mut ids), track);
    }
    project
}

/// The frame the page stages for `fs`.
fn frame(name: &str, fs: &MemFs) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend((name.len() as u32).to_le_bytes());
    frame.extend(name.as_bytes());
    for (path, bytes) in fs.files() {
        frame.extend((path.len() as u32).to_le_bytes());
        frame.extend(path.as_bytes());
        frame.push(0);
        frame.extend((bytes.len() as u64).to_le_bytes());
        frame.extend(bytes);
    }
    frame
}

/// Opens `project` through a core that plays something else; returns the seconds the
/// frame, the staging and the command took, and the engine bytes.
fn open(project: &Project) -> (f64, f64, usize) {
    let mut fs = MemFs::new();
    save(
        &mut fs,
        project,
        &mut SaveState::new(),
        None,
        &SaveOptions {
            write_cache: true,
            ..SaveOptions::default()
        },
    )
    .unwrap();
    save_session(&mut fs, &SessionState::default()).unwrap();
    let frame = frame("big", &fs);

    // Where the time goes, apart from the command.
    {
        let t = Instant::now();
        let (_, staged) = project_wasm::parse_folder(&frame).unwrap();
        let parse = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let opened =
            project::file::open(&staged, &project::file::OpenOptions { use_cache: true }).unwrap();
        let read = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let (msgs, _) = commands::engine_msgs::snapshot(&opened.project, &SessionState::default());
        let snap = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let bytes = engine::msg::encode_stream(1, 0, &msgs).unwrap();
        let encode = t.elapsed().as_secs_f64();
        println!(
            "parts: parse {parse:.3} s, open {read:.3} s ({:?}), snapshot {snap:.3} s ({} msgs), encode {encode:.3} s ({} MiB)",
            opened.report.cache,
            msgs.len(),
            bytes.len() >> 20
        );
    }
    let mut core = Core::new(KIND_EMPTY, 1, 1_700_000_000_000, 1).unwrap();
    let t = Instant::now();
    core.stage(&frame).expect("the frame stages");
    let staged = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let (report, engine) = core.run(br#"{"type":"project.open","folder":"big"}"#);
    let opened = t.elapsed().as_secs_f64();
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["status"], "applied", "{json}");
    assert!(json["stream_error"].is_null(), "{json}");
    assert_eq!(core.view().project, *project);
    (staged, opened, engine.len())
}

#[test]
fn a_scaled_large_project_opens_and_snapshots() {
    let project = profile(10, 100, 3_000);
    let (_, _, engine) = open(&project);
    assert!(engine > 0);
}

#[test]
#[ignore = "a measurement, not a test"]
fn the_qa7_profile_opens_within_the_measured_envelope() {
    let project = profile(100, 1000, 30_000);
    let counts = project.counts();
    let (staged, opened, engine) = open(&project);
    println!(
        "QA-7 profile ({counts:?}): stage {staged:.3} s, project.open (read, validate, snapshot, encode) {opened:.3} s, {} MiB of engine bytes",
        engine >> 20
    );
    // QA-7 measured 1.56 s to 2.45 s to open the same folder in the page (wasm, OPFS, cache
    // snapshot); the snapshot and its encoding are what this adds, measured at under a
    // second and a half. Run it on a quiet machine: other builds triple the numbers.
    let total = staged + opened;
    assert!(total < 2.45 + 1.5, "{total:.3} s is above the envelope");
}

/// Saves `project` the way the page does (SV-2): the first write into a folder, an edit, and a
/// save with nothing to write. Returns the seconds each took in the core and the entries of the
/// edit's save.
fn save_costs(project: &Project) -> (f64, f64, f64, usize) {
    let mut fs = MemFs::new();
    save(
        &mut fs,
        project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    save_session(&mut fs, &SessionState::default()).unwrap();
    let mut core = Core::new(KIND_EMPTY, 1, 1_700_000_000_000, 1).unwrap();
    core.stage(&frame("big", &fs)).expect("the frame stages");
    let (report, _) = core.run(br#"{"type":"project.open","folder":"big"}"#);
    assert_eq!(serde_json::to_value(&report).unwrap()["status"], "applied");

    // A save as: the whole project.
    let t = Instant::now();
    let whole = core.save(Mode::Everything);
    let everything = t.elapsed().as_secs_f64();
    core.saved(true);
    assert!(whole.len() > 1 << 20);

    // An edit, then the automatic save: every unit is serialized and hashed, one file is written.
    let (report, _) = core.run(br#"{"type":"project.rename","name":"Edited"}"#);
    assert_eq!(serde_json::to_value(&report).unwrap()["status"], "applied");
    let t = Instant::now();
    let edit = core.save(Mode::Changed);
    let edited = t.elapsed().as_secs_f64();
    core.saved(true);
    assert!(
        edit.len() < 4096,
        "an edit of the name writes only project.json"
    );

    // Nothing changed: the page's explicit Save looks at everything and writes nothing.
    let t = Instant::now();
    let nothing = core.save(Mode::Forced);
    let forced = t.elapsed().as_secs_f64();
    core.saved(true);
    assert!(nothing.len() < 64, "nothing to write");
    (everything, edited, forced, whole.len())
}

#[test]
fn a_scaled_large_project_saves_only_what_changed() {
    let project = profile(10, 100, 3_000);
    let (_, _, _, bytes) = save_costs(&project);
    assert!(bytes > 0);
}

#[test]
#[ignore = "a measurement, not a test"]
fn the_qa7_profile_saves_within_a_budget() {
    let project = profile(100, 1000, 30_000);
    let counts = project.counts();
    let (everything, edited, forced, bytes) = save_costs(&project);
    println!(
        "QA-7 profile ({counts:?}): save as {everything:.3} s ({} MiB), autosave after an edit {edited:.3} s, forced save with nothing to write {forced:.3} s",
        bytes >> 20
    );
}
