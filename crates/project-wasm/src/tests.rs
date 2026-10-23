//! The C ABI as the page drives it: JSON in through the inbox, JSON and engine bytes out.

use super::*;
use serde_json::{Value, json};

/// A live core freed on drop.
struct Handle(*mut Core);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a pointer from `pw_new`, freed once.
        unsafe { pw_free(self.0) };
    }
}

impl Handle {
    fn new(kind: u32) -> Handle {
        let core = pw_new(kind, 1, 1_760_000_000_000.0, 7, 8, 9);
        assert!(!core.is_null());
        Handle(core)
    }

    fn core(&self) -> &Core {
        // SAFETY: live until drop; no call in progress.
        unsafe { &*self.0 }
    }

    /// Calls `f` with the input in the inbox; returns the JSON and the engine bytes.
    fn call(
        &self,
        input: &str,
        f: unsafe extern "C" fn(*mut Core, u32) -> i32,
    ) -> (Value, Vec<u8>) {
        write_inbox(input);
        // SAFETY: a live core.
        let len = unsafe { f(self.0, input.len() as u32) };
        (out_json(len), engine_bytes())
    }

    fn call0(&self, f: unsafe extern "C" fn(*mut Core) -> i32) -> (Value, Vec<u8>) {
        // SAFETY: a live core.
        let len = unsafe { f(self.0) };
        (out_json(len), engine_bytes())
    }

    fn run(&self, command: Value) -> (Value, Vec<u8>) {
        self.call(&command.to_string(), pw_run)
    }

    /// Runs a command that must apply; returns its output.
    fn apply(&self, command: Value) -> Value {
        let (report, _) = self.run(command);
        assert_eq!(report["status"], "applied", "{report}");
        report["output"].clone()
    }

    fn view(&self) -> Value {
        self.call0(pw_project).0
    }

    /// The files to write ([`pw_save`] gives no removal for the interim files).
    fn save(&self) -> Value {
        let report = self.call0(pw_save).0;
        assert_eq!(report["status"], "changes", "{report}");
        assert_eq!(report["changes"]["remove"], json!([]));
        report["changes"]["write"].clone()
    }

    /// Reports `files` as written.
    fn saved(&self, files: &Value) -> i32 {
        self.saved_raw(&json!({"write": files, "remove": []}))
    }

    fn saved_raw(&self, changes: &Value) -> i32 {
        let text = changes.to_string();
        write_inbox(&text);
        // SAFETY: a live core.
        unsafe { pw_saved(self.0, text.len() as u32) }
    }
}

fn write_inbox(input: &str) {
    let ptr = pw_inbox(input.len() as u32);
    assert!(!ptr.is_null());
    // SAFETY: the inbox holds at least `input.len()` bytes.
    unsafe { core::ptr::copy_nonoverlapping(input.as_ptr(), ptr, input.len()) };
}

fn out_json(len: i32) -> Value {
    assert!(len > 0, "error code {len}");
    // SAFETY: `pw_out` holds the `len` bytes of the last call.
    let bytes = unsafe { core::slice::from_raw_parts(pw_out(), len as usize) };
    serde_json::from_slice(bytes).expect("the core returns JSON")
}

fn engine_bytes() -> Vec<u8> {
    let len = pw_engine_len() as usize;
    if len == 0 {
        return Vec::new();
    }
    // SAFETY: `pw_engine` holds `pw_engine_len` bytes.
    unsafe { core::slice::from_raw_parts(pw_engine(), len) }.to_vec()
}

const SYNTH_TRACK: &str = commands::engine_msgs::single_synth::TRACK;

#[test]
fn an_empty_project_gets_tracks_clips_and_notes_by_commands() {
    let h = Handle::new(KIND_EMPTY);
    let (setup, bytes) = h.call0(pw_setup);
    assert_eq!(setup, json!({"status": "encoded", "epoch": 1}));
    assert!(!bytes.is_empty(), "a reset and the snapshot");

    let (report, bytes) = h.run(json!({"type": "track.create", "kind": "midi", "instrument": "faulk.osc_synth", "name": "Bass"}));
    assert_eq!(report["status"], "applied");
    assert_eq!(report["output"]["type"], "track.create");
    assert_eq!(report["stream_error"], Value::Null);
    let track = report["output"]["track"].as_str().unwrap().to_owned();
    assert_eq!(
        report["events"][0],
        json!({"type": "track_created", "track": track})
    );
    assert!(!bytes.is_empty(), "the new track reaches the engine");
    let other = h.apply(json!({"type": "track.create", "kind": "midi"}))["track"].clone();
    assert_ne!(other, json!(track));

    let clip = h.apply(json!({"type": "clip.create", "track": track, "at": {"at": "arrangement", "start": 0}, "length": 3840}))["clip"]
        .as_str()
        .unwrap()
        .to_owned();
    let notes = json!([
        {"start": 0, "pitch": 60, "length": 480, "velocity": 100},
        {"start": 960, "pitch": 64, "length": 480, "velocity": 90},
    ]);
    let (report, bytes) = h.run(json!({"type": "clip.notes.add", "clip": clip, "notes": notes}));
    assert_eq!(report["status"], "applied", "{report}");
    assert!(!bytes.is_empty(), "the notes reach the engine");

    let view = h.view();
    let tracks = view["project"]["tracks"].as_object().unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[&track]["name"], "Bass");
    let content = &tracks[&track]["clips"][&clip]["content"]["midi"];
    assert_eq!(content["notes"], notes, "{content}");
    assert_eq!(h.core().undo_depth(), 4);

    // Undo and redo go through the same path and reach the engine.
    let (report, bytes) = h.run(json!({"type": "history.undo"}));
    assert_eq!(report["status"], "applied");
    assert!(!bytes.is_empty());
    assert_eq!(
        h.view()["project"]["tracks"][&track]["clips"][&clip]["content"]["midi"]["notes"],
        json!([])
    );
    h.apply(json!({"type": "history.redo"}));
    assert_eq!(
        h.view()["project"]["tracks"][&track]["clips"][&clip]["content"]["midi"]["notes"],
        notes
    );
}

#[test]
fn refused_and_invalid_commands_change_nothing() {
    let h = Handle::new(KIND_SINGLE_SYNTH);
    h.call0(pw_setup);
    let before = h.view();
    let (report, bytes) =
        h.run(json!({"type": "track.rename", "track": "00000000000000000000000009", "name": "x"}));
    assert_eq!(report["status"], "refused");
    assert_eq!(report["error"]["code"], "not_found");
    assert!(bytes.is_empty());
    let (report, bytes) = h.call("not json", pw_run);
    assert_eq!(report["status"], "invalid");
    assert!(bytes.is_empty());
    let (report, _) = h.run(json!({"type": "nope"}));
    assert_eq!(report["status"], "invalid");
    assert_eq!(h.view(), before);
    assert_eq!(h.core().undo_depth(), 0);
}

#[test]
fn live_commands_reach_the_engine_and_the_session_is_seen() {
    let h = Handle::new(KIND_SINGLE_SYNTH);
    h.call0(pw_setup);
    let (report, bytes) =
        h.run(json!({"type": "live.note_on", "track": SYNTH_TRACK, "note": 60, "velocity": 100}));
    assert_eq!(report["status"], "applied");
    assert!(!bytes.is_empty());
    let (report, bytes) = h.run(json!({"type": "midi.list_inputs"}));
    assert_eq!(report["status"], "applied");
    assert!(
        bytes.is_empty(),
        "not an engine command: the previous bytes are not returned again"
    );
    h.apply(json!({"type": "view.select_track", "track": SYNTH_TRACK}));
    assert_eq!(h.view()["session"]["selected_track"], SYNTH_TRACK);
}

#[test]
fn a_snapshot_moves_to_the_next_epoch() {
    let h = Handle::new(KIND_SINGLE_SYNTH);
    let (setup, a) = h.call0(pw_setup);
    assert_eq!(setup["epoch"], 1);
    let (snapshot, b) = h.call0(pw_snapshot);
    assert_eq!(snapshot, json!({"status": "encoded", "epoch": 2}));
    assert_eq!(a.len(), b.len(), "the same project, another epoch");
    assert_ne!(a, b);
}

#[test]
fn ids_come_from_the_seed() {
    let a = Handle::new(KIND_EMPTY);
    let b = Handle::new(KIND_EMPTY);
    assert_eq!(
        a.view()["project"]["id"],
        b.view()["project"]["id"],
        "same seed, same ids"
    );
    let other = Handle(pw_new(KIND_EMPTY, 1, 1_760_000_000_000.0, 7, 8, 10));
    assert_ne!(a.view()["project"]["id"], other.view()["project"]["id"]);
    assert!(pw_new(2, 1, 0.0, 0, 0, 0).is_null(), "unknown kind");
    // A clock that is not finite or before 1970 still gives a core.
    let _ = Handle(pw_new(KIND_EMPTY, 1, f64::NAN, 0, 0, 0));
    let _ = Handle(pw_new(KIND_EMPTY, 1, -5.0, 0, 0, 0));
}

#[test]
fn save_gives_the_dirty_files_until_they_are_written() {
    let h = Handle::new(KIND_SINGLE_SYNTH);
    let first = h.save();
    let paths: Vec<&String> = first.as_object().unwrap().keys().collect();
    assert_eq!(
        paths,
        [".faulk/interim-project.json", ".faulk/session.json"],
        "nothing written yet"
    );
    // The write failed: the host never calls `saved`, the files stay dirty.
    assert_eq!(h.save(), first);
    assert_eq!(h.saved(&first), 0);
    assert_eq!(h.save(), json!({}), "clean once written");

    // An edit made between `save` and `saved` stays dirty.
    h.apply(json!({"type": "project.rename", "name": "Song"}));
    let pending = h.save();
    assert_eq!(
        pending.as_object().unwrap().keys().collect::<Vec<_>>(),
        [".faulk/interim-project.json"]
    );
    h.apply(json!({"type": "project.rename", "name": "Song 2"}));
    assert_eq!(h.saved(&pending), 0);
    let dirty = h.save();
    assert!(
        dirty[".faulk/interim-project.json"]
            .as_str()
            .unwrap()
            .contains("Song 2")
    );

    // A session change dirties the session file only.
    assert_eq!(h.saved(&dirty), 0);
    h.apply(json!({"type": "view.select_track", "track": SYNTH_TRACK}));
    assert_eq!(
        h.save().as_object().unwrap().keys().collect::<Vec<_>>(),
        [".faulk/session.json"]
    );

    // Unknown paths are ignored; a bad input is refused.
    assert_eq!(h.saved(&json!({"other.json": "{}"})), 0);
    assert_eq!(h.save().as_object().unwrap().len(), 1);
    assert_eq!(h.saved_raw(&json!(["project.json"])), ERR_BAD_INPUT);
    assert_eq!(h.saved(&json!(["project.json"])), ERR_BAD_INPUT);

    // A removal recorded for a file the project still has makes it dirty again.
    let session = h.save();
    assert_eq!(h.saved(&session), 0);
    assert_eq!(h.save(), json!({}));
    assert_eq!(
        h.saved_raw(&json!({"write": {}, "remove": [".faulk/session.json"]})),
        0
    );
    assert_eq!(
        h.save().as_object().unwrap().keys().collect::<Vec<_>>(),
        [".faulk/session.json"]
    );
}

/// The folder `state`'s project and session make on disk (`project::file`, as SV-2 will
/// write it), and the frame the page stages for it.
fn folder_of(state: &State, name: &str) -> (MemFs, Vec<u8>) {
    use project::file::{SaveOptions, SaveState, save, save_session};
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &state.project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .expect("a valid project saves");
    save_session(&mut fs, &state.session).expect("a session saves");
    let frame = frame_of(name, &fs);
    (fs, frame)
}

/// The page's folder frame (see [`parse_folder`]) of `fs`.
fn frame_of(name: &str, fs: &MemFs) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend((name.len() as u32).to_le_bytes());
    frame.extend(name.as_bytes());
    for (path, bytes) in fs.files() {
        frame.extend((path.len() as u32).to_le_bytes());
        frame.extend(path.as_bytes());
        frame.push(ENTRY_FILE);
        frame.extend((bytes.len() as u64).to_le_bytes());
        frame.extend(bytes);
    }
    frame
}

impl Handle {
    fn stage_raw(&self, frame: &[u8]) -> i32 {
        let ptr = pw_inbox(frame.len() as u32);
        assert!(!ptr.is_null());
        // SAFETY: the inbox holds at least `frame.len()` bytes.
        unsafe { core::ptr::copy_nonoverlapping(frame.as_ptr(), ptr, frame.len()) };
        // SAFETY: a live core.
        unsafe { pw_stage(self.0, frame.len() as u32) }
    }

    fn open(&self, name: &str) -> (Value, Vec<u8>) {
        self.run(json!({"type": "project.open", "folder": name}))
    }

    fn state(&self) -> &State {
        &self.core().synced.state
    }
}

#[test]
fn open_replaces_the_project_or_changes_nothing() {
    let source = Handle::new(KIND_EMPTY);
    let track =
        source.apply(
            json!({"type": "track.create", "kind": "midi", "instrument": "faulk.osc_synth"}),
        )["track"]
            .clone();
    source.apply(json!({"type": "view.select_track", "track": track}));
    let (fs, frame) = folder_of(source.state(), "song");

    let h = Handle::new(KIND_SINGLE_SYNTH);
    h.call0(pw_setup);
    h.apply(json!({"type": "project.rename", "name": "Before"}));
    let before = h.view();

    // Nothing staged, or another name staged: refused, nothing changes.
    let (report, bytes) = h.open("song");
    assert_eq!(
        report["error"],
        json!({"code": "folder", "error": {"reason": "not_staged", "folder": "song"}}),
        "{report}"
    );
    assert!(bytes.is_empty());
    assert_eq!(h.stage_raw(&frame), 0);
    let (report, _) = h.open("other");
    assert_eq!(report["status"], "refused", "{report}");
    assert_eq!(h.view(), before);

    // Every refusal leaves the project, the history and the epoch.
    let mut broken = fs.clone();
    broken.insert("project.json", b"{}".to_vec());
    let mut other = MemFs::new();
    other.insert("x.txt", b"hi".to_vec());
    for (what, bad) in [
        ("empty", MemFs::new()),
        ("no project", other),
        ("broken", broken),
    ] {
        assert_eq!(h.stage_raw(&frame_of("bad", &bad)), 0, "{what}");
        let (report, bytes) = h.open("bad");
        assert_eq!(report["status"], "refused", "{what}: {report}");
        assert!(bytes.is_empty(), "{what}");
        assert_eq!(h.view(), before, "{what}");
        assert_eq!(h.core().undo_depth(), 1, "{what}");
        assert_eq!(h.core().epoch(), 1, "{what}");
    }

    assert_eq!(h.stage_raw(&frame), 0);
    let (report, bytes) = h.open("song");
    assert_eq!(report["status"], "applied", "{report}");
    assert_eq!(report["stream_error"], Value::Null);
    assert_eq!(report["output"]["name"], "Untitled");
    assert!(!bytes.is_empty(), "the engine starts over from the project");
    assert_eq!(h.view()["project"], source.view()["project"]);
    assert_eq!(h.view()["session"], source.view()["session"]);
    assert_eq!(
        h.view()["slots"]["epoch"],
        2,
        "the opening moved the engine to the next epoch"
    );
    assert_eq!(
        h.core().undo_depth(),
        0,
        "the history is the opened project's"
    );
    let (undo, _) = h.run(json!({"type": "history.undo"}));
    assert_eq!(undo["events"], json!([]), "nothing to undo: {undo}");
    assert_eq!(h.view()["project"], source.view()["project"]);
    // The interim save forgets what it wrote for the previous project.
    assert_eq!(h.save().as_object().unwrap().len(), 2);
    // Its ids keep coming from the page's generator.
    let next = h.apply(json!({"type": "track.create", "kind": "midi"}))["track"].clone();
    assert_ne!(next, track);
    // The folder served once.
    let (report, _) = h.open("song");
    assert_eq!(report["status"], "refused");
}

#[test]
fn a_malformed_folder_frame_stages_nothing() {
    let h = Handle::new(KIND_EMPTY);
    let (_, frame) = folder_of(h.state(), "p");
    // Cut anywhere before its end: refused or, at a file boundary, a shorter folder; never
    // a panic.
    for len in 0..frame.len() {
        let _ = h.stage_raw(&frame[..len]);
    }
    let mut fs = MemFs::new();
    fs.insert("a.txt", b"x".to_vec());
    let ok = frame_of("n", &fs);
    assert_eq!(h.stage_raw(&ok), 0);
    for bad in ["", "/a", "a//b", "a/./b", "../a", "a/..", "a\\b", "a\0b"] {
        let mut raw = Vec::new();
        raw.extend(1u32.to_le_bytes());
        raw.extend(b"n");
        raw.extend((bad.len() as u32).to_le_bytes());
        raw.extend(bad.as_bytes());
        raw.push(ENTRY_FILE);
        raw.extend(0u64.to_le_bytes());
        assert_eq!(h.stage_raw(&raw), ERR_BAD_FOLDER, "{bad:?}");
    }
    // A path twice, a size that runs past the frame, a name that is not UTF-8, huge lengths.
    let twice = [ok.clone(), ok[5..].to_vec()].concat();
    assert_eq!(h.stage_raw(&twice), ERR_BAD_FOLDER);
    let mut long = ok.clone();
    let at = long.len() - 1 - 8;
    long[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(h.stage_raw(&long), ERR_BAD_FOLDER);
    assert_eq!(h.stage_raw(&[1, 0, 0, 0, 0xFF]), ERR_BAD_FOLDER);
    assert_eq!(h.stage_raw(&u32::MAX.to_le_bytes()), ERR_BAD_FOLDER);
    assert_eq!(h.stage_raw(&[]), ERR_BAD_FOLDER);
}

#[test]
fn a_null_core_is_an_error_and_clears_the_engine_bytes() {
    let h = Handle::new(KIND_SINGLE_SYNTH);
    h.call0(pw_setup);
    assert!(pw_engine_len() > 0);
    // SAFETY: null is allowed.
    assert_eq!(unsafe { pw_run(core::ptr::null_mut(), 0) }, ERR_NO_CORE);
    assert_eq!(pw_engine_len(), 0);
    // SAFETY: null is allowed.
    unsafe {
        assert_eq!(pw_setup(core::ptr::null_mut()), ERR_NO_CORE);
        assert_eq!(pw_snapshot(core::ptr::null_mut()), ERR_NO_CORE);
        assert_eq!(pw_project(core::ptr::null_mut()), ERR_NO_CORE);
        assert_eq!(pw_stage(core::ptr::null_mut(), 0), ERR_NO_CORE);
        assert_eq!(pw_save(core::ptr::null_mut()), ERR_NO_CORE);
        assert_eq!(pw_saved(core::ptr::null_mut(), 0), ERR_NO_CORE);
        pw_free(core::ptr::null_mut());
    }
}

#[test]
fn a_call_that_produces_no_bytes_never_returns_the_previous_ones() {
    let h = Handle::new(KIND_SINGLE_SYNTH);
    h.call0(pw_setup);
    let note = json!({"type": "live.note_on", "track": SYNTH_TRACK, "note": 60, "velocity": 100});
    for next in [
        json!({"type": "track.rename", "track": "00000000000000000000000009", "name": "x"}),
        json!({"type": "nope"}),
        json!({"type": "midi.list_inputs"}),
    ] {
        let (_, bytes) = h.run(note.clone());
        assert!(!bytes.is_empty());
        let (report, bytes) = h.run(next.clone());
        assert!(bytes.is_empty(), "{report}");
    }
    for call in [pw_project, pw_save] {
        h.run(note.clone());
        assert!(pw_engine_len() > 0);
        h.call0(call);
        assert_eq!(pw_engine_len(), 0);
    }
    h.run(note.clone());
    let (report, bytes) = h.open("nothing staged");
    assert_eq!(report["status"], "refused");
    assert!(bytes.is_empty());
    h.run(note);
    assert_eq!(h.saved(&json!({})), 0);
    assert_eq!(pw_engine_len(), 0);
}

#[test]
fn an_unreadable_session_opens_the_project_with_the_default_session() {
    let source = Handle::new(KIND_SINGLE_SYNTH);
    let (mut fs, _) = folder_of(source.state(), "s");
    fs.insert(".faulk/session.json", b"{\"view\": {}}".to_vec());
    let h = Handle::new(KIND_EMPTY);
    assert_eq!(h.stage_raw(&frame_of("s", &fs)), 0);
    let (report, _) = h.open("s");
    assert_eq!(report["status"], "applied", "{report}");
    assert_eq!(report["output"]["session_reset"], true);
    assert_eq!(h.view()["project"], source.view()["project"]);
    assert_eq!(h.view()["session"], json!(project::SessionState::default()));
}

#[test]
fn a_project_that_does_not_serialize_is_reported_not_a_trap() {
    let h = Handle::new(KIND_SINGLE_SYNTH);
    // SAFETY: live until drop; no call in progress.
    unsafe { (*h.0).synced.state.project.master.pan = f64::NAN };
    let (report, _) = h.call0(pw_save);
    assert_eq!(report["status"], "failed", "{report}");
    // SAFETY: a live core.
    assert_eq!(unsafe { pw_project(h.0) }, ERR_SERIALIZE);
    // The core keeps running commands.
    // SAFETY: as above.
    unsafe { (*h.0).synced.state.project.master.pan = 0.0 };
    h.apply(json!({"type": "project.rename", "name": "Kept"}));
    assert_eq!(h.view()["project"]["name"], "Kept");
}

#[test]
fn the_view_gives_the_engine_slots_of_tracks_and_scenes() {
    let h = Handle::new(KIND_EMPTY);
    h.call0(pw_setup);
    assert_eq!(
        h.view()["slots"],
        json!({"epoch": 1, "tracks": {}, "scenes": {}})
    );

    let first = h.apply(json!({"type": "track.create", "kind": "midi"}))["track"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = h.apply(json!({"type": "track.create", "kind": "midi"}))["track"]
        .as_str()
        .unwrap()
        .to_owned();
    let scene = h.apply(json!({"type": "scene.create"}))["scene"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        h.view()["slots"],
        json!({"epoch": 1, "tracks": {first: 0, second: 1}, "scenes": {scene.clone(): 0}})
    );

    // A deleted scene gives its slot back, and the next one takes it.
    h.apply(json!({"type": "scene.delete", "scene": scene}));
    assert_eq!(h.view()["slots"]["scenes"], json!({}));
    let next = h.apply(json!({"type": "scene.create"}))["scene"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(h.view()["slots"]["scenes"], json!({next: 0}));

    // A snapshot moves to the next epoch and gives the slots again, in the same view.
    h.call0(pw_snapshot);
    assert_eq!(h.view()["slots"]["epoch"], 2);
    assert_eq!(h.view()["slots"]["tracks"].as_object().unwrap().len(), 2);
}

/// A project with one lane dense enough to be a blob in the folder.
fn project_with_a_blob_lane() -> project::Project {
    use project::*;
    let mut ids = IdGenerator::new(1_700_000_000_000, 7);
    let mut project = Project::new(ProjectId::generate(&mut ids), "Dense");
    let mut track = Track::new(
        TrackKind::Midi,
        "Track",
        OrderKey::between(None, None).unwrap(),
    );
    track.automation.insert(
        LaneId::generate(&mut ids),
        AutomationLane {
            target: AutomationTarget::Mixer {
                param: MixerParam::Pan,
            },
            points: (0..100_100)
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
    project.tracks.insert(TrackId::generate(&mut ids), track);
    project
}

/// `fs` as a frame in which every blob is listed by size instead of carried.
fn frame_with_blobs_listed(name: &str, fs: &MemFs) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend((name.len() as u32).to_le_bytes());
    frame.extend(name.as_bytes());
    for (path, bytes) in fs.files() {
        frame.extend((path.len() as u32).to_le_bytes());
        frame.extend(path.as_bytes());
        let listed = path.starts_with("blobs/");
        frame.push(if listed { ENTRY_LISTED } else { ENTRY_FILE });
        frame.extend((bytes.len() as u64).to_le_bytes());
        if !listed {
            frame.extend(bytes);
        }
    }
    frame
}

#[test]
fn blobs_are_carried_when_opening_reads_them_and_listed_when_it_only_counts_them() {
    use project::file::{SaveOptions, SaveState, save};
    let project = project_with_a_blob_lane();
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    let blobs: Vec<&String> = fs
        .files()
        .keys()
        .filter(|p| p.starts_with("blobs/"))
        .collect();
    assert_eq!(blobs.len(), 1, "the dense lane is a blob");

    // Carried whole: opens.
    let h = Handle::new(KIND_EMPTY);
    assert_eq!(h.stage_raw(&frame_of("dense", &fs)), 0);
    let (report, _) = h.open("dense");
    assert_eq!(report["status"], "applied", "{report}");
    assert_eq!(h.view()["project"]["name"], "Dense");

    // Listed by size only: a lane blob the project needs is missing, so it is refused and
    // the project in the core stays.
    let h = Handle::new(KIND_EMPTY);
    assert_eq!(h.stage_raw(&frame_with_blobs_listed("dense", &fs)), 0);
    let (report, bytes) = h.open("dense");
    assert_eq!(report["status"], "refused", "{report}");
    assert_eq!(report["error"]["error"]["reason"], "missing", "{report}");
    assert!(bytes.is_empty());
    assert_eq!(h.view()["project"]["name"], "Untitled");

    // Listed blobs behave as a read-only folder.
    let (_, staged) = parse_folder(&frame_with_blobs_listed("dense", &fs)).unwrap();
    let blob = blobs[0].as_str();
    assert!(staged.exists(blob));
    assert_eq!(staged.read(blob), Err(FsError::NotFound));
    assert!(
        staged
            .list("")
            .unwrap()
            .iter()
            .any(|e| e.name == "blobs" && e.is_dir)
    );
    let listed = staged.list("blobs").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].size, fs.files()[blob].len() as u64);
    let mut staged = staged;
    assert!(staged.write("x", b"").is_err());
    assert!(staged.remove("x").is_err());

    // Listing is for blobs only: anywhere else it is refused.
    let mut raw = Vec::new();
    raw.extend(1u32.to_le_bytes());
    raw.extend(b"n");
    for path in ["project.json", "blobs/a/b", "blobs"] {
        let mut one = raw.clone();
        one.extend((path.len() as u32).to_le_bytes());
        one.extend(path.as_bytes());
        one.push(ENTRY_LISTED);
        one.extend(5u64.to_le_bytes());
        assert!(parse_folder(&one).is_err(), "{path}");
    }
    let mut unknown = raw;
    unknown.extend(1u32.to_le_bytes());
    unknown.extend(b"a");
    unknown.push(7);
    unknown.extend(0u64.to_le_bytes());
    assert!(parse_folder(&unknown).is_err());
}
