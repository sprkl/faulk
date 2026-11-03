//! A song written before format 2 (#473) sounds the same after the migration: the export of
//! the migrated project, with today's engine, against the WAV the old build exported. The
//! folder and its WAV live outside the repository (a PO run's Journey pack), so the test is
//! ignored and takes their paths from the environment:
//!
//! ```text
//! FAULK_JOURNEY_PACK=<project folder> FAULK_JOURNEY_WAV=<its export> \
//!   cargo test -p test-support --test journey_pack -- --ignored --nocapture
//! ```

use commands::State;
use commands::export::ExportSettings;
use project::file::{FileSystem, Migration, OpenOptions, StdFs, open, open_with};
use test_support::Session;
use test_support::export::export_native;

/// The format alone, without the cut: the song as the current format reads it unmigrated.
fn bump_only(fs: &mut project::file::MemFs) -> Result<(), String> {
    let text = String::from_utf8(fs.read("project.json").map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs.insert(
        "project.json",
        text.replacen(
            "\"format\": 1",
            &format!("\"format\": {}", project::FORMAT_VERSION),
            1,
        ),
    );
    Ok(())
}

/// The samples of a 24-bit stereo WAV, as floats, left then right.
fn read_wav24(path: &str) -> (Vec<f32>, Vec<f32>) {
    let w = std::fs::read(path).expect("the WAV");
    let mut at = 12;
    let (mut left, mut right) = (Vec::new(), Vec::new());
    while at + 8 <= w.len() {
        let id = &w[at..at + 4];
        let size = u32::from_le_bytes(w[at + 4..at + 8].try_into().unwrap()) as usize;
        if id == b"data" {
            let data = &w[at + 8..(at + 8 + size).min(w.len())];
            for frame in data.as_chunks::<6>().0 {
                let s = |b: &[u8]| {
                    (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0
                };
                left.push(s(&frame[0..3]));
                right.push(s(&frame[3..6]));
            }
            break;
        }
        at += 8 + size + (size & 1);
    }
    (left, right)
}

#[test]
#[ignore = "needs a Journey pack outside the repository (FAULK_JOURNEY_PACK, FAULK_JOURNEY_WAV)"]
fn a_journey_pack_sounds_as_its_old_export_after_the_migration() {
    let (Ok(dir), Ok(wav)) = (
        std::env::var("FAULK_JOURNEY_PACK"),
        std::env::var("FAULK_JOURNEY_WAV"),
    ) else {
        eprintln!("FAULK_JOURNEY_PACK and FAULK_JOURNEY_WAV are not set: nothing to check");
        return;
    };
    let fs = StdFs::new(&dir);
    let migrated = open(&fs, &OpenOptions::default()).expect("the pack opens");
    eprintln!("migrated from format {:?}", migrated.report.migrated_from);
    let raw = open_with(
        &fs,
        &OpenOptions::default(),
        &[Migration {
            from: 1,
            apply: bump_only,
        }],
    )
    .expect("the pack opens unmigrated");
    let cut: usize = migrated
        .project
        .tracks
        .iter()
        .map(|(id, t)| {
            t.placements
                .iter()
                .filter(|(p, pl)| raw.project.tracks[id].placements.get(*p) != Some(*pl))
                .count()
                + raw.project.tracks[id].placements.len()
                - t.placements.len()
        })
        .sum();
    eprintln!("placements cut by the migration: {cut}");

    let settings = ExportSettings::of(None, None, Some(48_000), None, None).unwrap();
    let now = export_native(&State::new(migrated.project.clone()), settings).render();
    let (left, right) = read_wav24(&wav);
    let frames = left.len().min(now.left.len());
    let diff = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max)
    };
    let worst = diff(&left[..frames], &now.left[..frames])
        .max(diff(&right[..frames], &now.right[..frames]));
    eprintln!(
        "old export {} frames, migrated render {} frames, largest sample difference {worst:e}",
        left.len(),
        now.left.len()
    );
    if cut > 0 {
        let repeated = export_native(&State::new(raw.project.clone()), settings).render();
        let frames_r = left.len().min(repeated.left.len());
        let worst_r = diff(&left[..frames_r], &repeated.left[..frames_r]);
        eprintln!("unmigrated (repeating) render: largest sample difference {worst_r:e}");
    }
    // The same song: no sample further than a few 24-bit steps (rounding of the 24-bit
    // export, about -132 dBFS for two steps) from the old export.
    assert!(
        worst <= 4.0 / 8_388_608.0,
        "the migrated song does not sound as its old export: {worst}"
    );
}

/// A format-1 song with a one-bar clip without a loop stretched over four bars: it played
/// once then fell silent. Migrated, it still does (the placement is cut), while the same
/// song written in format 2 repeats the clip.
#[test]
fn a_stretched_one_shot_of_a_format_1_song_still_plays_once() {
    use commands::{Command, Output, arrangement, clip};
    use project::{Note, Tick};
    let mut s = Session::single_synth();
    let track = s.state().project.tracks.keys().next().unwrap().clone();
    let Output::Clip(clip::Output::Create { clip, placement }) =
        s.exec(Command::Clip(clip::Command::Create {
            track,
            at: clip::ClipTarget::Arrangement { start: Tick(0) },
            length: Some(Tick(3840)),
            name: None,
        }))
    else {
        panic!("a clip")
    };
    s.run(Command::Clip(clip::Command::NotesAdd {
        clip,
        notes: vec![Note::new(Tick(0), 60, Tick(480), 100)],
    }));
    s.run(Command::Arrangement(arrangement::Command::Resize {
        placement: placement.unwrap(),
        edge: arrangement::Edge::End,
        to: Tick(15360),
    }));
    let mut fs = project::file::MemFs::new();
    project::file::save(
        &mut fs,
        &s.state().project,
        &mut project::file::SaveState::new(),
        None,
        &project::file::SaveOptions::default(),
    )
    .unwrap();
    let text = String::from_utf8(fs.read("project.json").unwrap()).unwrap();
    fs.insert(
        "project.json",
        text.replacen(
            &format!("\"format\": {}", project::FORMAT_VERSION),
            "\"format\": 1",
            1,
        ),
    );
    let old = open(&fs, &OpenOptions::default()).unwrap();
    let settings = ExportSettings::of(None, None, Some(48_000), None, None).unwrap();
    // At 120 BPM a bar is 2 s: the note at 0 s, nothing at 2, 4 or 6 s.
    let once = export_native(&State::new(old.project), settings).render();
    once.assert_audible(0.05..0.2, 0.05);
    // The song ends with the clip (the export renders to the end of the last placement and
    // its tail): what followed was silence, and it is gone.
    assert!(
        once.left.len() < 3 * 48_000,
        "the song ends with its clip: {} frames",
        once.left.len()
    );
    // The same song in format 2: the clip repeats.
    let now = export_native(&State::new(s.state().project.clone()), settings).render();
    now.assert_audible(2.05..2.2, 0.05);
    now.assert_audible(6.05..6.2, 0.05);
}
