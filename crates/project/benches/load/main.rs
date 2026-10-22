//! Load bench of the project format (QA-7, #130).
//!
//! `cargo bench -p project --bench load -- <command> [--flag value]...` (always release: the
//! numbers of a debug build mean nothing). Commands: `gen`, `open`, `save`, `mem`, `swap`,
//! `vcs`, `density`, `dirs`. Method, machine and numbers: `docs/spikes/QA-7-banc-de-charge-format-projet.md`.
//!
//! The folder writer and reader of this bench are a prototype (`folder.rs`); SV-1 takes them
//! over or replaces them. Nothing here is the product's writer.

mod alloc;
mod density;
mod dirs;
#[allow(dead_code)]
mod folder;
mod gitbench;
#[allow(dead_code)]
mod stats;
mod swap;
mod synth;

use folder::*;
use project::*;
use stats::*;
use std::path::PathBuf;
use synth::Profile;

#[global_allocator]
static ALLOC: alloc::Counting = alloc::Counting;

struct Args(Vec<String>);

impl Args {
    fn get(&self, key: &str) -> Option<&str> {
        let flag = format!("--{key}");
        self.0
            .iter()
            .position(|a| *a == flag)
            .and_then(|i| self.0.get(i + 1))
            .map(String::as_str)
    }

    fn num<T: std::str::FromStr>(&self, key: &str, default: T) -> T {
        self.get(key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    fn dir(&self) -> PathBuf {
        PathBuf::from(self.get("dir").expect("--dir <folder>"))
    }

    fn layout(&self) -> Layout {
        match self.get("blob-above") {
            Some(n) => Layout::blob_above(n.parse().expect("--blob-above <points>")),
            None => Layout::TEXT,
        }
    }

    fn profile(&self) -> Profile {
        match self.get("profile").unwrap_or("heavy") {
            "heavy" => Profile::heavy(),
            "small" => Profile::small(),
            other => panic!("unknown profile {other}"),
        }
    }
}

fn main() {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    // `cargo bench` appends `--bench`.
    argv.retain(|a| a != "--bench");
    let Some(command) = argv.first().cloned() else {
        eprintln!("usage: load <gen|open|save|mem|swap|stream|vcs|density|dirs> [--flag value]...");
        std::process::exit(2);
    };
    let args = Args(argv);
    println!("command: {command}");
    println!(
        "build: {}",
        if cfg!(debug_assertions) {
            "DEBUG (numbers are meaningless)"
        } else {
            "release"
        }
    );
    match command.as_str() {
        "gen" => gen_cmd(&args),
        "open" => open_cmd(&args),
        "save" => save_cmd(&args),
        "mem" => mem_cmd(&args),
        "swap" => swap_cmd(&args),
        "stream" => swap::dump_stream(args.get("out").expect("--out <file>")),
        "vcs" => gitbench::run(&args.0),
        "density" => density::run(&args.0),
        "dirs" => dirs::run(&args.0),
        other => {
            eprintln!("unknown command {other}");
            std::process::exit(2);
        }
    }
}

fn read(dir: &std::path::Path, threads: usize) -> (Project, ReadStats) {
    read_project(&FsFiles(dir.to_owned()), threads).expect("read the folder")
}

/// Generates the project and writes it as a folder (audio blobs, text, cache snapshot).
fn gen_cmd(a: &Args) {
    let profile = a.profile();
    let dir = a.dir();
    let audio_mb: u64 = a.num("audio-mb", 256);
    let layout = a.layout();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    println!(
        "seed {}, {} tracks ({} MIDI, {} audio), {} notes, {} points over {} lanes, {} audio files, {} MiB of audio, blob above {:?}",
        profile.seed,
        profile.tracks(),
        profile.midi_tracks,
        profile.audio_tracks,
        profile.notes(),
        profile.total_points,
        profile.lanes(),
        profile.audio_files(),
        audio_mb,
        layout.blob_above
    );
    let files = synth::audio_files(&profile, audio_mb * 1024 * 1024);
    let (audio, d_audio) = time(|| write_audio(&files, &dir).expect("audio"));
    let bytes: u64 = files.iter().map(|f| f.bytes).sum();
    println!(
        "audio: {} files, {:.1} MiB written and hashed in {:.0} ms ({:.0} MiB/s)",
        files.len(),
        mib(bytes),
        ms(d_audio),
        mib(bytes) / d_audio.as_secs_f64()
    );
    let (project, d_gen) = time(|| synth::build(&profile, &audio));
    println!("project generated in memory: {:.0} ms", ms(d_gen));
    let (valid, d_val) = time(|| project.validate());
    println!("validate: {:?} in {:.0} ms", valid.is_ok(), ms(d_val));
    valid.expect("the generated project is valid");
    let counts = project.counts();
    println!(
        "clips {}, placements {}, lanes {}",
        counts.clips, counts.placements, counts.lanes
    );
    let (stats, d_write) =
        time(|| write_all(&project, &dir, layout, false, None, 1).expect("write"));
    println!(
        "text written: {} files, {:.1} MiB, {:.0} ms",
        stats.written,
        mib(stats.bytes),
        ms(d_write)
    );
    let (snap, d_snap) = time(|| write_snapshot(&project, &dir).expect("snapshot"));
    println!("cache snapshot: {:.1} MiB, {:.0} ms", mib(snap), ms(d_snap));
    let digest = folder_digest(&dir, false).expect("digest");
    println!("folder digest (text, without blobs): {digest}");
}

fn open_cmd(a: &Args) {
    let dir = a.dir();
    let runs: usize = a.num("runs", 5);
    // A first read warms the file cache: the numbers below are warm-cache.
    let (warm, _) = read(&dir, 8);
    drop(warm);
    for threads in [1usize, 8] {
        let mut times = Vec::new();
        let mut last = None;
        for _ in 0..runs {
            let ((p, st), d) = time(|| read(&dir, threads));
            times.push(ms(d));
            last = Some(st);
            drop(p);
        }
        let st = last.unwrap();
        println!(
            "open from the text folder, {threads} thread(s): {}",
            describe(&times, "ms")
        );
        println!(
            "  {} files, {:.1} MiB; time in file reads {:.0} ms and in parsing {:.0} ms, summed over threads",
            st.files.load(std::sync::atomic::Ordering::Relaxed),
            mib(st.bytes.load(std::sync::atomic::Ordering::Relaxed)),
            st.io_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6,
            st.parse_ns.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6
        );
    }
    let (project, _) = read(&dir, 8);
    let v = series(runs, || {
        project.validate().expect("valid");
    });
    println!("validate: {}", describe(&v, "ms"));
    let ((), d_drop) = time(|| drop(project));
    println!(
        "drop of the project (free of its memory): {:.0} ms",
        ms(d_drop)
    );

    let scan = series(runs, || {
        scan_manifest(&dir).expect("scan");
    });
    let n = scan_manifest(&dir).expect("scan").len();
    println!(
        "stat walk of the {n} source files (what the cache snapshot is checked with): {}",
        describe(&scan, "ms")
    );
    match std::fs::metadata(snapshot_path(&dir)) {
        Ok(m) => {
            println!("cache snapshot: {:.1} MiB", mib(m.len()));
            let snap = series(runs, || {
                read_snapshot(&dir).expect("snapshot is current");
            });
            println!(
                "open from the cache snapshot (stat walk, read, load): {}",
                describe(&snap, "ms")
            );
        }
        Err(_) => println!("no cache snapshot in the folder"),
    }
    // The cost of refreshing one changed file on top of the snapshot: a clip's notes.
    let (project, _) = read(&dir, 8);
    let (tid, cid) = project
        .tracks
        .iter()
        .find_map(|(t, tr)| {
            tr.clips
                .iter()
                .find(|(_, c)| c.content.is_midi())
                .map(|(c, _)| (t.clone(), c.clone()))
        })
        .expect("a MIDI clip");
    let path = format!("tracks/{tid}/clips/{cid}.notes.tsv");
    let files = FsFiles(dir.clone());
    let one = series(50, || {
        let bytes = files.read(&path).expect("read");
        parse_notes(&bytes).expect("parse");
    });
    println!(
        "re-reading one clip's notes file ({}): {}",
        path.len(),
        describe(&one, "ms")
    );
    println!(
        "peak working set of this process: {:.0} MiB",
        mib(alloc::peak_working_set().unwrap_or(0))
    );
}

fn save_cmd(a: &Args) {
    let dir = a.dir();
    let layout = a.layout();
    let runs: usize = a.num("runs", 3);
    let (mut project, _) = read(&dir, 8);
    let scratch = PathBuf::from(format!("{}.save", dir.display()));
    println!(
        "whole project, cold folder (every file created), atomic write by temporary file and rename:"
    );
    for (threads, durable) in [(1usize, false), (8, false), (1, true), (8, true)] {
        let mut times = Vec::new();
        let mut st = WriteStats::default();
        for _ in 0..runs {
            let _ = std::fs::remove_dir_all(&scratch);
            let (s, d) = time(|| {
                write_all(&project, &scratch, layout, durable, None, threads).expect("write")
            });
            times.push(ms(d));
            st = s;
        }
        println!(
            "  {threads} thread(s), fsync {}: {} ({} files, {:.1} MiB)",
            if durable { "yes" } else { "no" },
            describe(&times, "ms"),
            st.written,
            mib(st.bytes)
        );
    }
    // Overwrite of every file in a populated folder (the shape of a save without dirty tracking
    // and without lazy write).
    let mut times = Vec::new();
    for _ in 0..runs {
        let (_, d) = time(|| write_all(&project, &scratch, layout, false, None, 1).expect("write"));
        times.push(ms(d));
    }
    println!(
        "  overwrite of every file in a populated folder, 1 thread, no fsync: {}",
        describe(&times, "ms")
    );

    // The floor of the per-file cost: plain `fs::write`, no temporary file, no rename.
    let mut times = Vec::new();
    for _ in 0..runs {
        let _ = std::fs::remove_dir_all(&scratch);
        let (_, d) = time(|| {
            let mut made = std::collections::HashSet::new();
            emit_all(&project, layout, &mut |o| {
                let path = scratch.join(&o.path);
                let parent = path.parent().expect("parent").to_owned();
                if made.insert(parent.clone()) {
                    std::fs::create_dir_all(&parent).expect("dir");
                }
                std::fs::write(&path, &o.bytes).expect("write");
            });
        });
        times.push(ms(d));
    }
    println!(
        "  direct `fs::write` of every file (no temporary file, no rename), 1 thread: {}",
        describe(&times, "ms")
    );

    // Lazy write: what it costs to find out that nothing changed, then that one note did.
    let mut known = Known::default();
    let _ = std::fs::remove_dir_all(&scratch);
    write_all(&project, &scratch, layout, false, Some(&mut known), 1).expect("first lazy save");
    let mut times = Vec::new();
    for _ in 0..runs {
        let (s, d) = time(|| {
            write_all(&project, &scratch, layout, false, Some(&mut known), 1).expect("lazy")
        });
        assert_eq!(s.written, 0);
        times.push(ms(d));
    }
    println!(
        "lazy write with nothing changed (serialise every file, hash it, write none; no dirty tracking): {}",
        describe(&times, "ms")
    );
    let (tid, cid) = project
        .tracks
        .iter()
        .find_map(|(t, tr)| {
            tr.clips
                .iter()
                .find(|(_, c)| c.content.is_midi())
                .map(|(c, _)| (t.clone(), c.clone()))
        })
        .expect("a MIDI clip");
    let mut times = Vec::new();
    for i in 0..runs {
        change_note(&mut project, &tid, &cid, 20 + (i % 100) as u8);
        let (s, d) = time(|| {
            write_all(&project, &scratch, layout, false, Some(&mut known), 1).expect("lazy")
        });
        assert_eq!(s.written, 1);
        times.push(ms(d));
    }
    println!(
        "lazy write after one note changed (1 file written, the rest found identical): {}",
        describe(&times, "ms")
    );

    // Dirty-file save: the edited clip's file only.
    for durable in [false, true] {
        let mut times = Vec::new();
        for i in 0..30 {
            change_note(&mut project, &tid, &cid, 60 + (i % 60) as u8);
            let (s, d) =
                time(|| save_clip_notes(&project, &scratch, &tid, &cid, durable).expect("dirty"));
            assert_eq!(s.written, 1);
            times.push(ms(d));
        }
        println!(
            "one note changed, dirty-file save of its clip (250 notes), fsync {}: {}",
            if durable { "yes" } else { "no" },
            describe(&times, "ms")
        );
    }
    // One point of a lane changed.
    let (ltid, lid) = project
        .tracks
        .iter()
        .find_map(|(t, tr)| tr.automation.keys().next().map(|l| (t.clone(), l.clone())))
        .expect("a lane");
    let lane_len = project.tracks[&ltid].automation[&lid].points.len();
    for durable in [false, true] {
        let mut times = Vec::new();
        for i in 0..30 {
            let lane = project
                .tracks
                .get_mut(&ltid)
                .unwrap()
                .automation
                .get_mut(&lid)
                .unwrap();
            let key = *lane.points.keys().nth(lane_len / 2).unwrap();
            lane.points.get_mut(&key).unwrap().value = (i % 7) as f64 / 10.0;
            let (_, d) =
                time(|| save_lane(&project, &scratch, &ltid, &lid, layout, durable).expect("lane"));
            times.push(ms(d));
        }
        println!(
            "one point of a {lane_len}-point lane changed, dirty-file save of the lane, fsync {}: {}",
            if durable { "yes" } else { "no" },
            describe(&times, "ms")
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

fn mem_cmd(a: &Args) {
    use std::collections::{BTreeMap, BTreeSet};
    println!(
        "size_of Note {} B, AutomationPoint {} B, Tick {} B",
        size_of::<Note>(),
        size_of::<AutomationPoint>(),
        size_of::<Tick>()
    );
    // Per item, in isolation.
    let mut rng = synth::Rng(1);
    let target = AutomationTarget::Mixer {
        param: MixerParam::Pan,
    };
    alloc::start();
    let lanes: Vec<_> = (0..50)
        .map(|_| synth::lane_points(&mut rng, 10_000, &target))
        .collect();
    let live = alloc::live();
    alloc::stop();
    println!(
        "BTreeMap<Tick, AutomationPoint>, 50 lanes of 10 000 points: {:.1} B per point (heap)",
        live as f64 / 500_000.0
    );
    drop(lanes);
    alloc::start();
    let sets: Vec<BTreeSet<Note>> = (0..800)
        .map(|_| {
            (0..250)
                .map(|i| Note::new(Tick(i * 120 + (rng.below(100) as i64)), 40, Tick(120), 100))
                .collect()
        })
        .collect();
    let live = alloc::live();
    alloc::stop();
    println!(
        "BTreeSet<Note>, 800 clips of 250 notes: {:.1} B per note (heap)",
        live as f64 / 200_000.0
    );
    drop(sets);
    let _ = BTreeMap::<u8, u8>::new();

    // The whole project: read from the folder.
    let dir = a.dir();
    alloc::start();
    let (project, _) = read(&dir, 1);
    let live = alloc::live();
    let peak = alloc::peak();
    alloc::stop();
    let (notes, points) = swap::counts(&project);
    println!(
        "project read from the folder (1 thread): heap held {:.1} MiB, peak during the read {:.1} MiB; {} notes, {} points",
        mib(live as u64),
        mib(peak as u64),
        notes,
        points
    );
    println!(
        "  that is {:.1} B per point counting everything the project holds",
        live as f64 / points as f64
    );
    println!(
        "peak working set of this process so far: {:.0} MiB",
        mib(alloc::peak_working_set().unwrap_or(0))
    );
    drop(project);
}

fn swap_cmd(a: &Args) {
    match a.get("profile") {
        Some("limits-notes") => {
            let p = Profile {
                seed: synth::SEED,
                midi_tracks: 17,
                audio_tracks: 0,
                clips_per_midi_track: 1,
                effects: 0,
                notes_per_clip: 986_895,
                lanes_per_midi_track: 1,
                lanes_per_audio_track: 0,
                total_points: 17,
                clips_per_audio_track: 0,
                scenes: 1,
            };
            let (project, _) = synth::generate(&p, 0);
            project.validate().expect("valid at the limit");
            swap::run(
                &project,
                "notes at the safety limit (17 clips of 986 895 notes = 16 777 215)",
            );
        }
        Some("limits-lane") => {
            let p = Profile {
                seed: synth::SEED,
                midi_tracks: 1,
                audio_tracks: 0,
                clips_per_midi_track: 1,
                effects: 0,
                notes_per_clip: 1,
                lanes_per_midi_track: 1,
                lanes_per_audio_track: 0,
                total_points: limits::MAX_POINTS_PER_LANE,
                clips_per_audio_track: 0,
                scenes: 1,
            };
            let (project, _) = synth::generate(&p, 0);
            project.validate().expect("valid at the limit");
            swap::run(&project, "one lane at the safety limit (16 777 216 points)");
        }
        Some("heavy-builtin") => {
            let (project, _) = synth::generate(&Profile::heavy_builtin(), 0);
            swap::run(
                &project,
                "heavy project with the built-in devices only (7 lanes a MIDI track), in memory",
            );
            swap::blocks(&project);
        }
        _ => {
            let dir = a.dir();
            let (project, _) = read(&dir, 8);
            swap::run(&project, "heavy project, read from the folder");
            swap::blocks(&project);
        }
    }
    println!(
        "peak working set of this process: {:.0} MiB",
        mib(alloc::peak_working_set().unwrap_or(0))
    );
}
