//! Tests of the load bench's prototype (QA-7, `benches/load`): the generator is deterministic,
//! the folder round-trips, and the mechanisms the bench measures (lazy write, dirty-file
//! save, cache snapshot, blob switch) behave on each state transition. The bench itself is
//! `harness = false` and `cargo test` never runs it, so its modules are compiled here too.
#![allow(dead_code)]

#[path = "../benches/load/folder.rs"]
mod folder;
#[path = "../benches/load/synth.rs"]
mod synth;

use folder::*;
use project::*;
use std::path::{Path, PathBuf};
use synth::{Profile, SEED};

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("faulk-qa7-test-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn small() -> Project {
    synth::generate(&Profile::small(), 64 * 1024).0
}

fn digest(project: &Project, layout: Layout) -> String {
    let dir = Dir::new("digest");
    write_all(project, dir.path(), layout, false, None, 1).unwrap();
    folder_digest(dir.path(), true).unwrap()
}

#[test]
fn the_profile_counts_are_what_the_project_holds() {
    let profile = Profile::small();
    let p = small();
    assert_eq!(p.tracks.len(), profile.tracks());
    let mut notes = 0;
    let mut points = 0;
    for t in p.tracks.values() {
        for c in t.clips.values() {
            if let ClipContent::Midi { notes: n, .. } = &c.content {
                notes += n.len();
            }
        }
        points += t.automation.values().map(|l| l.points.len()).sum::<usize>();
    }
    assert_eq!(notes, profile.notes());
    assert_eq!(points, profile.total_points);
    assert_eq!(p.counts().lanes, profile.lanes());
    p.validate().expect("the generated project is valid");
}

#[test]
fn the_heavy_profile_arithmetic_matches_the_issue() {
    let h = Profile::heavy();
    assert_eq!(h.tracks(), 100);
    assert_eq!(h.notes(), 200_000);
    assert_eq!(h.total_points, 6_000_000);
    assert_eq!(h.seed, SEED);
    assert!(h.lanes() * limits::MAX_POINTS_PER_LANE > h.total_points);
    assert!(h.lanes() <= limits::MAX_LANES);
}

#[test]
fn the_same_seed_gives_the_same_bytes_and_another_seed_does_not() {
    let a = digest(&small(), Layout::TEXT);
    let b = digest(&small(), Layout::TEXT);
    assert_eq!(a, b);
    let other = Profile {
        seed: SEED + 1,
        ..Profile::small()
    };
    let c = digest(&synth::generate(&other, 64 * 1024).0, Layout::TEXT);
    assert_ne!(a, c);
}

#[test]
fn audio_files_are_deterministic_and_hash_what_they_write() {
    let f = &synth::audio_files(&Profile::small(), 40_000)[0];
    let mut a = Vec::new();
    let ha = f.write_to(&mut a).unwrap();
    let mut b = Vec::new();
    let hb = f.write_to(&mut b).unwrap();
    assert_eq!(a, b);
    assert_eq!(ha, hb);
    assert_eq!(a.len() as u64, f.bytes);
    use sha2::{Digest, Sha256};
    assert_eq!(ha.0, <[u8; 32]>::from(Sha256::digest(&a)));
}

#[test]
fn the_folder_round_trips_byte_for_byte_in_both_layouts() {
    let p = small();
    for layout in [Layout::TEXT, Layout::blob_above(50)] {
        let dir = Dir::new("rt");
        write_all(&p, dir.path(), layout, false, None, 1).unwrap();
        let (back, _) = read_project(&FsFiles(dir.path().to_owned()), 1).unwrap();
        assert_eq!(back, p);
        back.validate().unwrap();
        let first = folder_digest(dir.path(), true).unwrap();
        let again = Dir::new("rt2");
        write_all(&back, again.path(), layout, false, None, 1).unwrap();
        assert_eq!(folder_digest(again.path(), true).unwrap(), first);
        // The in-memory folder and the threaded reader and writer agree.
        let mem = MemFiles::from_project(&p, layout);
        assert_eq!(read_project(&mem, 1).unwrap().0, p);
        assert_eq!(
            read_project(&FsFiles(dir.path().to_owned()), 4).unwrap().0,
            p
        );
        let par = Dir::new("par");
        write_all(&p, par.path(), layout, false, None, 4).unwrap();
        assert_eq!(folder_digest(par.path(), true).unwrap(), first);
    }
}

#[test]
fn the_blob_layout_moves_exactly_the_lanes_above_the_threshold() {
    let p = small();
    let lanes: Vec<usize> = p
        .tracks
        .values()
        .flat_map(|t| t.automation.values().map(|l| l.points.len()))
        .collect();
    let threshold = lanes.iter().copied().min().unwrap();
    let above = lanes.iter().filter(|n| **n > threshold).count();
    let dir = Dir::new("blobs");
    write_all(
        &p,
        dir.path(),
        Layout::blob_above(threshold),
        false,
        None,
        1,
    )
    .unwrap();
    let blobs = std::fs::read_dir(dir.path().join("blobs")).unwrap().count();
    // Distinct contents only: a blob is named by its bytes.
    assert!(blobs > 0 && blobs <= above + p.counts().clips);
    let (back, _) = read_project(&FsFiles(dir.path().to_owned()), 1).unwrap();
    assert_eq!(back, p);
}

#[test]
fn crossing_the_threshold_both_ways_reads_back_the_same_and_leaves_no_new_blob_without_an_edit() {
    let p = small();
    let dir = Dir::new("flip");
    let blobs = |d: &Path| std::fs::read_dir(d.join("blobs")).unwrap().count();
    write_all(&p, dir.path(), Layout::blob_above(10), false, None, 1).unwrap();
    let after_first = blobs(dir.path());
    write_all(&p, dir.path(), Layout::TEXT, false, None, 1).unwrap();
    assert_eq!(
        read_project(&FsFiles(dir.path().to_owned()), 1).unwrap().0,
        p
    );
    // The blobs the lanes left behind are orphans: nothing deletes them (§ 3.4: on request only).
    assert_eq!(blobs(dir.path()), after_first);
    for _ in 0..3 {
        write_all(&p, dir.path(), Layout::blob_above(10), false, None, 1).unwrap();
        write_all(&p, dir.path(), Layout::TEXT, false, None, 1).unwrap();
    }
    // Flipping again without editing a lane gives the same content, hence the same blob.
    assert_eq!(blobs(dir.path()), after_first);
    write_all(&p, dir.path(), Layout::blob_above(10), false, None, 1).unwrap();
    assert_eq!(
        read_project(&FsFiles(dir.path().to_owned()), 1).unwrap().0,
        p
    );
    // A lane that changes gets a new blob: the old one is an orphan until a sweep.
    let mut edited = p.clone();
    let lane = edited
        .tracks
        .values_mut()
        .flat_map(|t| t.automation.values_mut())
        .max_by_key(|l| l.points.len())
        .unwrap();
    let first = *lane.points.keys().next().unwrap();
    lane.points.get_mut(&first).unwrap().value += 0.5;
    write_all(&edited, dir.path(), Layout::blob_above(10), false, None, 1).unwrap();
    assert_eq!(blobs(dir.path()), after_first + 1);
}

#[test]
fn the_lazy_write_skips_unchanged_files_and_writes_only_what_changed() {
    let mut p = small();
    let dir = Dir::new("lazy");
    let mut known = Known::default();
    let first = write_all(&p, dir.path(), Layout::TEXT, false, Some(&mut known), 1).unwrap();
    assert!(
        first.written > 0 && first.skipped == 0,
        "a first save writes every file"
    );
    let again = write_all(&p, dir.path(), Layout::TEXT, false, Some(&mut known), 1).unwrap();
    assert_eq!((again.written, again.skipped), (0, first.written));
    // One note changes: one file is written.
    let (tid, cid) = first_midi_clip(&p);
    change_one_note(&mut p, &tid, &cid);
    let one = write_all(&p, dir.path(), Layout::TEXT, false, Some(&mut known), 1).unwrap();
    assert_eq!(one.written, 1);
    let (back, _) = read_project(&FsFiles(dir.path().to_owned()), 1).unwrap();
    assert_eq!(back, p);
    // An edit made outside the writer is invisible to it (its memory is of what it wrote):
    // the limit of the lazy write, which a product writer must cover (re-read on external change).
    let notes = dir
        .path()
        .join(format!("tracks/{tid}/clips/{cid}.notes.tsv"));
    std::fs::write(&notes, b"# faulk-notes 1\tstart\tlength\tpitch\tvelocity\n").unwrap();
    let blind = write_all(&p, dir.path(), Layout::TEXT, false, Some(&mut known), 1).unwrap();
    assert_eq!(blind.written, 0);
    // A writer with no memory (a new session) rewrites the file.
    let cold = write_all(
        &p,
        dir.path(),
        Layout::TEXT,
        false,
        Some(&mut Known::default()),
        1,
    )
    .unwrap();
    assert_eq!(cold.written, first.written);
    assert_eq!(
        read_project(&FsFiles(dir.path().to_owned()), 1).unwrap().0,
        p
    );
}

fn first_midi_clip(p: &Project) -> (TrackId, ClipId) {
    p.tracks
        .iter()
        .find_map(|(t, track)| {
            track
                .clips
                .iter()
                .find(|(_, c)| c.content.is_midi())
                .map(|(c, _)| (t.clone(), c.clone()))
        })
        .unwrap()
}

fn change_one_note(p: &mut Project, t: &TrackId, c: &ClipId) {
    let ClipContent::Midi { notes, .. } = &mut p
        .tracks
        .get_mut(t)
        .unwrap()
        .clips
        .get_mut(c)
        .unwrap()
        .content
    else {
        unreachable!()
    };
    let old = *notes.iter().next().unwrap();
    notes.remove(&old);
    notes.insert(Note {
        velocity: if old.velocity == 100 { 90 } else { 100 },
        ..old
    });
}

#[test]
fn the_dirty_file_save_writes_the_one_file_of_the_edited_clip() {
    let mut p = small();
    let dir = Dir::new("dirty");
    write_all(&p, dir.path(), Layout::TEXT, false, None, 1).unwrap();
    let (tid, cid) = first_midi_clip(&p);
    change_one_note(&mut p, &tid, &cid);
    let s = save_clip_notes(&p, dir.path(), &tid, &cid, true).unwrap();
    assert_eq!(s.written, 1);
    assert_eq!(
        read_project(&FsFiles(dir.path().to_owned()), 1).unwrap().0,
        p
    );
    // An audio clip has no notes file.
    let audio = p
        .tracks
        .iter()
        .find_map(|(t, tr)| {
            tr.clips
                .iter()
                .find(|(_, c)| !c.content.is_midi())
                .map(|(c, _)| (t.clone(), c.clone()))
        })
        .unwrap();
    assert!(save_clip_notes(&p, dir.path(), &audio.0, &audio.1, false).is_err());
}

#[test]
fn a_written_file_is_never_left_half_written() {
    let p = small();
    let dir = Dir::new("atomic");
    write_all(&p, dir.path(), Layout::TEXT, true, None, 1).unwrap();
    let stray = scan_manifest(dir.path())
        .unwrap()
        .into_iter()
        .filter(|(path, _, _)| path.ends_with(".tmp"))
        .count();
    assert_eq!(stray, 0);
}

#[test]
fn the_snapshot_is_used_when_current_and_refused_on_every_staleness() {
    let p = small();
    let dir = Dir::new("snap");
    write_all(&p, dir.path(), Layout::TEXT, false, None, 1).unwrap();
    assert_eq!(read_snapshot(dir.path()).unwrap_err(), Stale::Missing);
    write_snapshot(&p, dir.path()).unwrap();
    assert_eq!(read_snapshot(dir.path()).unwrap(), p);

    // A file changed (same size, new modification time): refused.
    let (tid, cid) = first_midi_clip(&p);
    let notes = dir
        .path()
        .join(format!("tracks/{tid}/clips/{cid}.notes.tsv"));
    let original = std::fs::read(&notes).unwrap();
    std::fs::write(&notes, &original).unwrap();
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&notes)
        .unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    drop(f);
    assert!(matches!(read_snapshot(dir.path()), Err(Stale::Changed(_))));
    // The text is rewritten and the snapshot too: current again.
    write_snapshot(&p, dir.path()).unwrap();
    assert_eq!(read_snapshot(dir.path()).unwrap(), p);

    // A file added (what a merge brings): refused.
    let extra = dir.path().join(format!("tracks/{tid}/clips/extra.json"));
    std::fs::write(&extra, b"{}").unwrap();
    assert!(matches!(read_snapshot(dir.path()), Err(Stale::Changed(_))));
    std::fs::remove_file(&extra).unwrap();
    assert_eq!(read_snapshot(dir.path()).unwrap(), p);

    // A file removed: refused.
    std::fs::remove_file(&notes).unwrap();
    assert!(matches!(read_snapshot(dir.path()), Err(Stale::Changed(_))));
    std::fs::write(&notes, &original).unwrap();
    write_snapshot(&p, dir.path()).unwrap();

    // A snapshot older than the text (a crash between the text and the snapshot): refused.
    let mut edited = p.clone();
    change_one_note(&mut edited, &tid, &cid);
    save_clip_notes(&edited, dir.path(), &tid, &cid, false).unwrap();
    assert!(matches!(read_snapshot(dir.path()), Err(Stale::Changed(_))));

    // Truncated, wrong magic, empty: refused, never a panic.
    write_snapshot(&edited, dir.path()).unwrap();
    let path = snapshot_path(dir.path());
    let bytes = std::fs::read(&path).unwrap();
    for cut in [0, 4, 8, 20, bytes.len() / 2, bytes.len() - 1] {
        std::fs::write(&path, &bytes[..cut]).unwrap();
        assert!(
            matches!(read_snapshot(dir.path()), Err(Stale::Corrupt(_))),
            "cut at {cut}"
        );
    }
    let mut wrong = bytes.clone();
    wrong[7] = b'9';
    std::fs::write(&path, &wrong).unwrap();
    assert!(matches!(read_snapshot(dir.path()), Err(Stale::Corrupt(_))));
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(read_snapshot(dir.path()).unwrap(), edited);
}

#[test]
fn the_reader_refuses_a_broken_folder_with_an_error() {
    let p = small();
    let mut mem = MemFiles::from_project(&p, Layout::TEXT);
    let notes = mem
        .0
        .keys()
        .find(|k| k.ends_with(".notes.tsv"))
        .unwrap()
        .clone();
    mem.0
        .insert(notes.clone(), b"# faulk-notes 1\tstart\n1\t2\n".to_vec());
    assert!(read_project(&mem, 1).is_err());
    mem.0.insert(
        notes,
        b"# faulk-notes 1\tstart\tlength\tpitch\tvelocity\n1\t2\tx\t4\n".to_vec(),
    );
    assert!(read_project(&mem, 1).is_err());
    let lane = mem
        .0
        .keys()
        .find(|k| k.contains("/automation/"))
        .unwrap()
        .clone();
    mem.0.insert(
        lane,
        b"# faulk-automation 1\ttarget={}\n0\t1\tlinear\n".to_vec(),
    );
    assert!(read_project(&mem, 1).is_err());
    let mut empty = MemFiles::default();
    assert!(read_project(&empty, 1).is_err());
    empty.0.insert("project.json".into(), b"nope".to_vec());
    assert!(read_project(&empty, 1).is_err());
}
