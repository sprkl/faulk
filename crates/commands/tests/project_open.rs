//! `project.open` (SV-4): the staged folder replaces the project, its session and its
//! history; a folder that is not a valid project of this version is refused with a typed
//! reason and nothing changes.

use ::project::file::{FileError, FileSystem, MemFs, SaveOptions, SaveState, save, save_session};
use ::project::{FORMAT_VERSION, SessionState};
use commands::contract::{CommandError, Folder, FolderError, ProjectEvent, State};
use commands::testing::{OPENED_FOLDER, bare_context, context, fixture};
use commands::{Command, Output, project as cmd};

fn open(name: &str) -> Command {
    Command::Project(cmd::Command::Open {
        folder: name.to_owned(),
    })
}

/// The files `testing::opened_folder` stages, as a `MemFs` to break.
fn files() -> MemFs {
    let f = fixture();
    let mut project = f.state.project;
    project.name = "Opened song".into();
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    let mut session = SessionState {
        selected_track: Some(f.track.clone()),
        ..SessionState::default()
    };
    session.solo.insert(f.track);
    save_session(&mut fs, &session).unwrap();
    fs
}

fn staged(fs: MemFs) -> commands::Context {
    let mut ctx = bare_context();
    ctx.folder = Some(Folder::new("broken", fs));
    ctx
}

fn refusal(error: &CommandError) -> &FolderError {
    match error {
        CommandError::Folder { error } => error,
        other => panic!("not a folder refusal: {other:?}"),
    }
}

#[test]
fn opening_replaces_the_project_the_session_and_the_history() {
    let f = fixture();
    let mut state = f.state.clone();
    assert!(
        state.history.can_undo(),
        "the fixture has something to undo"
    );
    let mut ctx = context();
    ctx.group = Some(commands::GroupId(9));
    let outcome = commands::run(&open(OPENED_FOLDER), &mut state, &mut ctx).unwrap();

    assert_eq!(state.project.name, "Opened song");
    assert_eq!(state.session.selected_track, Some(f.track.clone()));
    assert!(
        state.session.solo.contains(&f.track),
        "the solo is restored"
    );
    assert!(!state.history.can_undo() && !state.history.can_redo());
    assert_eq!(state.history.open_group, None);
    assert_eq!(ctx.group, None);
    assert_eq!(ctx.folder, None, "the folder served once");
    assert!(
        ctx.engine.tracks.get(&f.track).is_none(),
        "the old engine's slots are gone"
    );
    assert!(outcome.control.is_empty());
    // One `history_changed`, not two: the old history was cleared by the opening itself.
    assert_eq!(
        outcome.events,
        [
            ProjectEvent::ProjectOpened {
                name: "Opened song".into()
            },
            ProjectEvent::HistoryChanged {
                can_undo: false,
                can_redo: false
            },
        ]
    );
    match outcome.output {
        Output::Project(cmd::Output::Open {
            name,
            migrated_from,
            session_reset,
            missing_blobs,
        }) => {
            assert_eq!(name, "Opened song");
            assert_eq!(migrated_from, None);
            assert!(!session_reset);
            assert_eq!(missing_blobs, 0);
        }
        other => panic!("{other:?}"),
    }
    // Undo has nothing of the old project left to undo.
    let undo = Command::History(commands::history::Command::Undo {});
    let undone = commands::run(&undo, &mut state, &mut ctx).unwrap();
    assert!(
        undone
            .events
            .iter()
            .all(|e| !matches!(e, ProjectEvent::ProjectRenamed { .. }))
    );
    assert_eq!(state.project.name, "Opened song");
}

#[test]
fn the_session_does_not_travel_in_the_versioned_files() {
    // The same project saved with and without a solo: only `.faulk/session.json` differs.
    let f = fixture();
    let mut with_solo = files();
    let mut without = files();
    save_session(&mut without, &SessionState::default()).unwrap();
    let mut session = SessionState::default();
    session.solo.insert(f.track);
    save_session(&mut with_solo, &session).unwrap();
    let differing: Vec<&String> = with_solo
        .files()
        .iter()
        .filter(|(path, bytes)| without.files().get(*path) != Some(*bytes))
        .map(|(path, _)| path)
        .collect();
    assert_eq!(differing, [".faulk/session.json"]);
}

#[test]
fn a_folder_not_staged_or_staged_under_another_name_is_refused_and_kept() {
    let mut state = fixture().state;
    let before = state.clone();

    let mut ctx = bare_context();
    let err = commands::run(&open("x"), &mut state, &mut ctx).unwrap_err();
    assert_eq!(err.code(), "folder");
    assert_eq!(
        refusal(&err),
        &FolderError::NotStaged { folder: "x".into() }
    );
    assert_eq!(state, before);

    // Another name: the folder staged for someone else stays staged.
    let mut ctx = context();
    let staged_before = ctx.folder.clone();
    let err = commands::run(&open("other"), &mut state, &mut ctx).unwrap_err();
    assert_eq!(
        refusal(&err),
        &FolderError::NotStaged {
            folder: "other".into()
        }
    );
    assert_eq!(ctx.folder, staged_before);
    assert_eq!(state, before);
    commands::testing::check_refusal(&open("other"), &state, &context()).unwrap();
}

#[test]
fn each_way_a_folder_is_not_a_project_is_a_typed_reason_and_nothing_changes() {
    let good = files();
    let text = String::from_utf8(good.read("project.json").unwrap()).unwrap();
    let with = |path: &str, bytes: &[u8]| {
        let mut fs = good.clone();
        fs.insert(path, bytes.to_vec());
        fs
    };
    let renamed = |from: &str, to: &str| {
        assert!(text.contains(from), "{from} in {text}");
        text.replacen(from, to, 1)
    };
    let newer = renamed(
        &format!("\"format\": {FORMAT_VERSION}"),
        &format!("\"format\": {}", FORMAT_VERSION + 1),
    );
    let empty_name = renamed("\"name\": \"Opened song\"", "\"name\": \"\"");

    type Case = (&'static str, MemFs, fn(&FolderError) -> bool);
    let cases: Vec<Case> = vec![
        ("an empty folder", MemFs::new(), |e| {
            matches!(e, FolderError::NotAProject)
        }),
        (
            "a newer format",
            with("project.json", newer.as_bytes()),
            |e| matches!(e, FolderError::NewerFormat { found, supported } if *found == FORMAT_VERSION + 1 && *supported == FORMAT_VERSION),
        ),
        (
            "format zero",
            with(
                "project.json",
                renamed(&format!("\"format\": {FORMAT_VERSION}"), "\"format\": 0").as_bytes(),
            ),
            |e| matches!(e, FolderError::UnsupportedFormat { found: 0 }),
        ),
        (
            "not json",
            with("project.json", b"not json"),
            |e| matches!(e, FolderError::Corrupt { path, .. } if path == "project.json"),
        ),
        (
            "a project that breaks its invariants",
            with("project.json", empty_name.as_bytes()),
            |e| matches!(e, FolderError::Invalid { total, violations } if *total >= 1 && !violations.is_empty()),
        ),
    ];
    for (what, fs, expected) in cases {
        let mut state = fixture().state;
        let before = state.clone();
        let mut ctx = staged(fs);
        let err = commands::run(&open("broken"), &mut state, &mut ctx).unwrap_err();
        assert!(expected(refusal(&err)), "{what}: {err:?}");
        assert_eq!(state, before, "{what}: the current project is intact");
        assert_eq!(ctx.group, None);
        // The reason is on the wire with its stable code.
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "folder", "{what}");
        assert!(json["error"]["reason"].is_string(), "{what}: {json}");
        let back: CommandError = serde_json::from_value(json).unwrap();
        assert_eq!(back, err);
    }
}

#[test]
fn a_damaged_file_is_refused_or_read_never_a_panic_and_a_refusal_changes_nothing() {
    let good = files();
    let paths: Vec<String> = good.files().keys().cloned().collect();
    assert!(paths.iter().any(|p| p.starts_with("tracks/")));
    let mut refused = 0;
    let mut opened = 0;
    for path in &paths {
        let original = good.read(path).unwrap();
        let mut damages: Vec<Option<Vec<u8>>> = vec![None]; // removed
        for len in [0, 1, original.len() / 2, original.len().saturating_sub(1)] {
            damages.push(Some(original[..len.min(original.len())].to_vec()));
        }
        let mut flipped = original.clone();
        if let Some(byte) = flipped.get_mut(original.len() / 3) {
            *byte ^= 0xFF;
        }
        damages.push(Some(flipped));
        damages.push(Some(vec![0xFF; 64]));
        for damage in damages {
            let mut fs = good.clone();
            match damage {
                Some(bytes) => fs.insert(path.clone(), bytes),
                None => {
                    FileSystem::remove(&mut fs, path).unwrap();
                }
            }
            let mut state: State = fixture().state;
            let before = state.clone();
            let mut ctx = staged(fs);
            match commands::run(&open("broken"), &mut state, &mut ctx) {
                Ok(_) => opened += 1,
                Err(err) => {
                    refused += 1;
                    assert_eq!(err.code(), "folder");
                    assert_eq!(state, before, "{path}");
                }
            }
        }
    }
    assert!(
        refused > 0 && opened > 0,
        "refused {refused}, opened {opened}"
    );
}

#[test]
fn every_file_error_maps_to_a_folder_reason() {
    use ::project::Violation;
    let violation: Violation = {
        // A project with an empty name breaks an invariant.
        let mut p = fixture().state.project;
        p.name = String::new();
        p.validate().unwrap_err().remove(0)
    };
    let errors = vec![
        FileError::NotAProject,
        FileError::NewerFormat {
            found: 9,
            supported: 1,
        },
        FileError::UnsupportedFormat { found: 0 },
        FileError::Missing { path: "a".into() },
        FileError::Corrupt {
            path: "a".into(),
            message: "m".into(),
        },
        FileError::TooLarge {
            path: "a".into(),
            size: 2,
            max: 1,
        },
        FileError::Invalid {
            violations: vec![violation.clone(); 30],
        },
        FileError::Unwritable {
            path: "a".into(),
            message: "m".into(),
        },
        FileError::Io {
            path: "a".into(),
            message: "m".into(),
        },
        FileError::Migration {
            from: 0,
            message: "m".into(),
        },
        FileError::BadPack {
            message: "m".into(),
        },
        FileError::DestinationNotEmpty,
    ];
    for error in errors {
        let label = format!("{error:?}");
        let mapped = FolderError::from(error);
        let json = serde_json::to_value(&mapped).unwrap();
        assert!(json["reason"].is_string(), "{label}: {json}");
        if let FolderError::Invalid { violations, total } = &mapped {
            assert_eq!(*total, 30);
            assert_eq!(violations.len(), 20, "the list is capped, the total is not");
        }
    }
}
