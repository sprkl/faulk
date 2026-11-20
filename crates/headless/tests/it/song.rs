//! A short song built through commands, as an agent builds it: two MIDI tracks with the
//! built-in synth, one clip each in the arrangement, notes in both. The commands are typed
//! values serialized to JSON, so their shape is the contract's, not written by hand here.

use std::path::{Path, PathBuf};

use commands::{Command, clip, track};
use headless::host::{Answer, Host};
use project::{Note, Tick, TrackId, TrackKind};
use serde_json::Value;

pub const AUTHOR: &str = "agent:it-test";

/// A fresh, empty folder under the target directory, removed first if a run left it.
pub fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("headless-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

pub fn json(command: Command) -> Value {
    serde_json::to_value(command).expect("a command serializes")
}

/// The output of an applied answer; panics on anything else.
pub fn applied(answer: Answer) -> Value {
    match answer {
        Answer::Applied { output, .. } => output,
        other => panic!("not applied: {other:?}"),
    }
}

fn note(start: i64, pitch: u8, length: i64, velocity: u8) -> Note {
    Note {
        start: Tick(start),
        pitch,
        length: Tick(length),
        velocity,
        cents: 0,
        chance: Note::DEFAULT_CHANCE,
        repeat: Note::DEFAULT_REPEAT,
    }
}

/// The commands of the song, in order, given the tracks the first two make.
pub fn track_commands() -> Vec<Value> {
    ["Bass", "Lead"]
        .into_iter()
        .map(|name| {
            json(Command::Track(track::Command::Create {
                kind: TrackKind::Midi,
                instrument: Some(commands::device::SYNTH_KIND.into()),
                name: Some(name.into()),
                color: None,
            }))
        })
        .collect()
}

pub fn clip_command(track: &TrackId, bars: i64) -> Value {
    json(Command::Clip(clip::Command::Create {
        track: track.clone(),
        at: clip::ClipTarget::Arrangement { start: Tick(0) },
        length: Some(Tick(bars * 3840)),
        name: None,
    }))
}

pub fn notes_command(clip: Value, bars: i64, lead: bool) -> Value {
    let clip = serde_json::from_value(clip).expect("a clip id");
    let roots = [36u8, 41, 43, 38];
    let mut notes = Vec::new();
    for bar in 0..bars {
        let root = roots[(bar % 4) as usize];
        for beat in 0..4 {
            let at = bar * 3840 + beat * 960;
            if lead {
                for (k, interval) in [12u8, 16, 19, 24].iter().enumerate() {
                    notes.push(note(at + k as i64 * 240, root + interval + 12, 200, 80));
                }
            } else {
                notes.push(note(at, root, 720, 100));
            }
        }
    }
    json(Command::Clip(clip::Command::NotesAdd { clip, notes }))
}

/// Builds the song of `bars` bars in the host's project, as `AUTHOR`.
pub fn build(host: &mut Host, bars: i64) {
    for (index, create) in track_commands().into_iter().enumerate() {
        let made = applied(host.call(AUTHOR, &create));
        let track: TrackId = serde_json::from_value(made["track"].clone()).expect("a track id");
        let clip = applied(host.call(AUTHOR, &clip_command(&track, bars)));
        applied(host.call(
            AUTHOR,
            &notes_command(clip["clip"].clone(), bars, index == 1),
        ));
    }
}

/// Every file of the folder with its bytes, sorted by path (`.faulk/tmp/` left out).
pub fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("the folder lists") {
            let entry = entry.expect("an entry");
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("inside")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((rel, std::fs::read(&path).expect("the file reads")));
            }
        }
    }
    files.sort();
    files
}
