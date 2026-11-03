//! The cost of the folder format on the QA-7 profile (100 tracks, 200 000 notes, 6 million
//! automation points): `cargo test -p project --release --test it folder_load:: -- --ignored
//! --nocapture`. In memory (`MemFs`): this measures the format and the writer, not a disk.

use project::file::*;
use project::*;
use std::time::Instant;

fn profile() -> Project {
    let mut ids = IdGenerator::new(1_700_000_000_000, 7);
    let mut project = Project::new(ProjectId::generate(&mut ids), "Load");
    let mut order = None;
    for _ in 0..100 {
        let key = OrderKey::between(None, order.as_ref()).unwrap();
        order = Some(key.clone());
        let mut track = Track::new(TrackKind::Midi, "Track", key);
        for c in 0..2 {
            let _ = c;
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
                        notes: (0..1000)
                            .map(|i| {
                                Note::new(
                                    Tick(i * 240),
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
        // 6 million points over 100 tracks: 60 000 a track, in 2 lanes of 30 000.
        for _ in 0..2 {
            track.automation.insert(
                LaneId::generate(&mut ids),
                AutomationLane {
                    target: AutomationTarget::Mixer {
                        param: MixerParam::Pan,
                    },
                    points: (0..30_000)
                        .map(|i| {
                            (
                                Tick(i * 10),
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

#[test]
#[ignore = "a measurement, not a test"]
fn the_qa7_profile() {
    let project = profile();
    let counts = project.counts();
    let notes = 200_000;
    let points: usize = project
        .tracks
        .values()
        .flat_map(|t| t.automation.values())
        .map(|l| l.points.len())
        .sum();
    println!("{} clips, {notes} notes, {points} points", counts.clips);
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    let options = SaveOptions {
        write_cache: true,
        ..SaveOptions::default()
    };
    let t = Instant::now();
    let report = save(&mut fs, &project, &mut state, None, &options).unwrap();
    println!(
        "full save with cache: {:?}, {} files written",
        t.elapsed(),
        report.written.len()
    );
    let bytes: usize = fs.files().values().map(Vec::len).sum();
    println!(
        "folder: {} files, {} MB, snapshot {} MB",
        fs.len(),
        bytes >> 20,
        fs.files()[".faulk/cache/project.snap"].len() >> 20
    );

    let t = Instant::now();
    let text = open(&fs, &OpenOptions { use_cache: false }).unwrap();
    println!("open from text: {:?}", t.elapsed());
    let t = Instant::now();
    let cached = open(&fs, &OpenOptions { use_cache: true }).unwrap();
    println!(
        "open from snapshot: {:?} ({} files read)",
        t.elapsed(),
        cached.report.files_read
    );
    assert_eq!(text.project, cached.project);

    // One note edited: dirty save without and with the snapshot.
    let mut edited = project.clone();
    let (t_id, c_id) = {
        let (t, tr) = edited.tracks.iter().next().unwrap();
        (t.clone(), tr.clips.keys().next().unwrap().clone())
    };
    if let ClipContent::Midi { notes, .. } = &mut edited
        .tracks
        .get_mut(&t_id)
        .unwrap()
        .clips
        .get_mut(&c_id)
        .unwrap()
        .content
    {
        let old = *notes.iter().next().unwrap();
        notes.remove(&old);
        notes.insert(Note {
            velocity: old.velocity ^ 1,
            ..old
        });
    }
    let mut dirty = Dirty::new();
    dirty.mark(Unit::Notes(t_id, c_id));
    let mut s2 = state.clone();
    let mut fs2 = fs.clone();
    let t = Instant::now();
    save(
        &mut fs2,
        &edited,
        &mut s2,
        Some(&dirty),
        &SaveOptions {
            write_cache: false,
            ..options
        },
    )
    .unwrap();
    println!("dirty save of one note, no snapshot: {:?}", t.elapsed());
    let t = Instant::now();
    save(&mut fs, &edited, &mut state, Some(&dirty), &options).unwrap();
    println!(
        "dirty save of one note, snapshot updated: {:?}",
        t.elapsed()
    );
}
