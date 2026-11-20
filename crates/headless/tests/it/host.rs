//! The host on a folder: create, open, run, save, export, analyze, refuse.

use commands::State;
use commands::export::ExportSettings;
use headless::host::{Answer, Host};
use serde_json::{Map, json};
use test_support::export::export_native;

use crate::song::{AUTHOR, applied, build, scratch, snapshot};

#[test]
fn a_project_made_saved_and_opened_again_is_the_same_and_a_second_save_writes_nothing() {
    let root = scratch("round-trip");
    let mut host = Host::create(&root, Some("Round trip")).expect("the project is made");
    build(&mut host, 2);
    assert!(host.dirty());
    assert!(host.save().expect("the save"), "the edits are written");
    let saved = snapshot(&root);
    let before = host.view();

    let mut again = Host::open(&root).expect("the folder opens");
    assert_eq!(again.view().project, before.project);
    assert_eq!(again.view().session, before.session);
    assert!(!again.dirty());
    assert!(!again.save().expect("the save"), "nothing to write");
    assert_eq!(snapshot(&root), saved, "no file was rewritten");
}

#[test]
fn a_project_is_made_only_in_an_empty_folder() {
    let root = scratch("not-empty");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("notes.txt"), b"mine").unwrap();
    assert!(Host::create(&root, None).is_err());
    assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"mine");
    let bad_name = scratch("bad-name");
    assert!(Host::create(&bad_name, Some("   ")).is_err());
}

#[test]
fn a_folder_that_is_no_project_is_refused() {
    let root = scratch("no-project");
    std::fs::create_dir_all(&root).unwrap();
    assert!(Host::open(&root).is_err());
}

#[test]
fn a_refused_command_changes_nothing_in_the_folder() {
    let root = scratch("refused");
    let mut host = Host::create(&root, None).expect("made");
    build(&mut host, 1);
    host.save().expect("saved");
    let saved = snapshot(&root);
    let unknown_track = json!({
        "type": "clip.create",
        "track": "trk_0000000000000000",
        "at": { "at": "arrangement", "start": 0 },
        "length": 3840,
        "name": null
    });
    let answer = host.call(AUTHOR, &unknown_track);
    assert!(
        matches!(answer, Answer::Refused { .. } | Answer::Invalid { .. }),
        "{answer:?}"
    );
    assert!(!host.dirty());
    assert!(!host.save().expect("the save"));
    assert_eq!(snapshot(&root), saved);
}

#[test]
fn the_commands_that_need_a_page_are_refused_with_the_headless_code() {
    let root = scratch("unsupported");
    let mut host = Host::create(&root, None).expect("made");
    for command in [
        json!({ "type": "transport.play" }),
        json!({ "type": "project.open", "folder": "x" }),
        json!({ "type": "project.new" }),
        json!({ "type": "mixer.levels" }),
        json!({ "type": "project.list_stored" }),
    ] {
        let answer = host.call(AUTHOR, &command);
        assert!(
            matches!(answer, Answer::Unsupported { .. }),
            "{command}: {answer:?}"
        );
        assert_eq!(answer.to_json()["code"], "headless.unsupported");
    }
    assert!(matches!(
        host.call(AUTHOR, &json!({ "type": "no.such" })),
        Answer::Invalid { .. }
    ));
    assert!(matches!(
        host.call(AUTHOR, &json!({ "track": 1 })),
        Answer::Invalid { .. }
    ));
    assert!(matches!(
        host.call("user:nobody", &json!({ "type": "project.get" })),
        Answer::Invalid { .. }
    ));
}

#[test]
fn the_headless_export_is_the_native_export_byte_for_byte() {
    let root = scratch("export-parity");
    let mut host = Host::create(&root, Some("Parity")).expect("made");
    build(&mut host, 2);
    let out = root.join("parity.wav");
    let exported = host.export(&Map::new(), Some(&out)).expect("exported");
    let view = host.view();
    let mut state = State::new(view.project);
    state.session = view.session;
    let native = export_native(&state, ExportSettings::default());
    let wav = std::fs::read(&out).expect("the file");
    assert_eq!(wav.len(), native.wav.len());
    assert!(wav == native.wav, "the files differ");
    let r = native.render();
    r.assert_clean();
    assert_eq!(exported.silenced_frames, 0);
    assert!(
        exported.peak_db.is_some_and(|db| db > -30.0),
        "{exported:?}"
    );
    assert!(exported.missing.is_empty());
    std::fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).ok();
}

#[test]
fn an_agent_exports_into_the_folder_without_overwriting() {
    let root = scratch("export-agent");
    let mut host = Host::create(&root, Some("My song")).expect("made");
    build(&mut host, 1);
    host.save().expect("saved");
    let first = host.call(AUTHOR, &json!({ "type": "project.export_audio" }));
    let Answer::Applied {
        export: Some(export),
        output,
    } = first
    else {
        panic!("{first:?}")
    };
    assert_eq!(export.file, "exports/My song.wav");
    assert!(export.missing.is_empty() && export.silenced_frames == 0);
    assert!(export.peak_db.is_some());
    assert!(output["duration_secs"].as_f64().is_some_and(|s| s > 1.9));
    let second = host.call(AUTHOR, &json!({ "type": "project.export_audio" }));
    let Answer::Applied {
        export: Some(export),
        ..
    } = second
    else {
        panic!("{second:?}")
    };
    assert_eq!(export.file, "exports/My song 2.wav");
    assert!(root.join("exports/My song.wav").is_file());
    // An export is no edit: nothing to save, and the exports stay out of the project.
    assert!(!host.dirty());
    let refused = host.call(
        AUTHOR,
        &json!({ "type": "project.export_audio", "region": "loop" }),
    );
    assert!(matches!(refused, Answer::Refused { .. }), "{refused:?}");
}

#[test]
fn an_empty_project_is_not_exported() {
    let root = scratch("export-empty");
    let mut host = Host::create(&root, None).expect("made");
    let answer = host.call(AUTHOR, &json!({ "type": "project.export_audio" }));
    assert!(matches!(answer, Answer::Refused { .. }), "{answer:?}");
    assert!(!root.join("exports").exists());
}

#[test]
fn the_analysis_measures_the_song() {
    let root = scratch("analyze");
    let mut host = Host::create(&root, None).expect("made");
    build(&mut host, 2);
    let output = applied(host.call(AUTHOR, &json!({ "type": "project.analyze_render" })));
    assert_eq!(output["silenced_frames"], 0, "{output}");
    assert_eq!(
        output["tracks"].as_array().map(Vec::len),
        Some(2),
        "{output}"
    );
    assert!(
        output["master"]["peak_db"]
            .as_f64()
            .is_some_and(|db| db > -30.0),
        "{output}"
    );
    let bars = applied(host.call(
        AUTHOR,
        &json!({ "type": "project.analyze_render", "from_bar": 2, "to_bar": 2 }),
    ));
    assert!(
        bars["duration_secs"]
            .as_f64()
            .is_some_and(|s| (s - 2.0).abs() < 1e-9),
        "{bars}"
    );
}

#[test]
fn an_agent_turn_is_one_history_entry_stamped_with_its_author() {
    let root = scratch("turn");
    let mut host = Host::create(&root, None).expect("made");
    build(&mut host, 1);
    let history = serde_json::to_value(host.history()).expect("serializes");
    let text = history.to_string();
    assert!(text.contains(AUTHOR), "{history}");
    // The six edits of one agent, within its turn, are one undo.
    applied(host.call(AUTHOR, &json!({ "type": "history.undo" })));
    assert!(
        host.view().project.tracks.is_empty(),
        "one undo undid the turn"
    );
}
