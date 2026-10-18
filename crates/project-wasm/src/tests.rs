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
        [".faulk/session.json", "project.json"],
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
        ["project.json"]
    );
    h.apply(json!({"type": "project.rename", "name": "Song 2"}));
    assert_eq!(h.saved(&pending), 0);
    let dirty = h.save();
    assert!(dirty["project.json"].as_str().unwrap().contains("Song 2"));

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

#[test]
fn load_replaces_the_project_or_changes_nothing() {
    let source = Handle::new(KIND_EMPTY);
    let track =
        source.apply(
            json!({"type": "track.create", "kind": "midi", "instrument": "faulk.osc_synth"}),
        )["track"]
            .clone();
    let files = source.save();

    let h = Handle::new(KIND_SINGLE_SYNTH);
    h.call0(pw_setup);
    h.apply(json!({"type": "project.rename", "name": "Before"}));
    let before = h.view();

    // Refusals leave the project, the history and the epoch.
    for bad in [
        "not json".to_owned(),
        json!({}).to_string(),
        json!({"project.json": "{}"}).to_string(),
        json!({"project.json": files["project.json"].as_str().unwrap().replace("\"format\": 1", "\"format\": 99")}).to_string(),
    ] {
        let (report, bytes) = h.call(&bad, pw_load);
        assert_eq!(report["status"], "refused", "{bad}");
        assert!(bytes.is_empty());
    }
    let (report, _) = h.call(&json!({}).to_string(), pw_load);
    assert_eq!(
        report["error"],
        json!({"code": "missing_file", "file": "project.json"})
    );
    assert_eq!(h.view(), before);
    assert_eq!(h.core().undo_depth(), 1);
    assert_eq!(h.core().epoch(), 1);

    let (report, bytes) = h.call(&files.to_string(), pw_load);
    assert_eq!(
        report,
        json!({"status": "loaded", "snapshot": {"status": "encoded", "epoch": 2}, "session_reset": false})
    );
    assert!(
        !bytes.is_empty(),
        "the engine is reset to the loaded project"
    );
    assert_eq!(h.view(), source.view());
    assert_eq!(
        h.core().undo_depth(),
        0,
        "the history is the loaded project's: empty"
    );
    let (undo, _) = h.run(json!({"type": "history.undo"}));
    assert_eq!(undo["events"], json!([]), "nothing to undo: {undo}");
    assert_eq!(h.view(), source.view());
    assert_eq!(h.save(), json!({}), "a loaded project is clean");
    // Its ids keep coming from the page's generator.
    let next = h.apply(json!({"type": "track.create", "kind": "midi"}))["track"].clone();
    assert_ne!(next, track);
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
        assert_eq!(pw_load(core::ptr::null_mut(), 0), ERR_NO_CORE);
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
    let (report, bytes) = h.call("{}", pw_load);
    assert_eq!(report["status"], "refused");
    assert!(bytes.is_empty());
    h.run(note);
    assert_eq!(h.saved(&json!({})), 0);
    assert_eq!(pw_engine_len(), 0);
}

#[test]
fn an_unreadable_session_loads_the_project_with_the_default_session() {
    let source = Handle::new(KIND_SINGLE_SYNTH);
    let mut files = source.save();
    files[".faulk/session.json"] = json!("{\"view\": {}}");
    let h = Handle::new(KIND_EMPTY);
    let (report, _) = h.call(&files.to_string(), pw_load);
    assert_eq!(report["status"], "loaded", "{report}");
    assert_eq!(report["session_reset"], true);
    assert_eq!(h.view(), source.view());
    // The unreadable file is left as it is until the session changes.
    assert_eq!(h.save(), json!({}));
    h.apply(json!({"type": "view.select_track", "track": SYNTH_TRACK}));
    assert_eq!(
        h.save().as_object().unwrap().keys().collect::<Vec<_>>(),
        [".faulk/session.json"]
    );
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
