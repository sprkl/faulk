//! `project.new`, `project.save` and `project.save_as` (SV-2), and what `project.open`
//! leaves for the host that keeps saving into the folder it read.

use commands::contract::{Folder, ProjectEvent};
use commands::testing::{OPENED_FOLDER, bare_context, context, fixture};
use commands::{Command, Output, project as cmd};

fn project(command: cmd::Command) -> Command {
    Command::Project(command)
}

#[test]
fn a_new_project_replaces_everything_with_its_own_identity() {
    let f = fixture();
    let mut state = f.state.clone();
    state.session.selected_track = Some(f.track.clone());
    state.session.solo.insert(f.track.clone());
    state.session.midi_input = Some("in-1".into());
    assert!(state.history.can_undo());
    let mut ctx = context();
    ctx.group = Some(commands::GroupId(4));
    ctx.folder = Some(Folder::new("left over", ::project::file::MemFs::new()));
    let old_id = state.project.id.clone();

    let outcome = commands::run(&project(cmd::Command::New {}), &mut state, &mut ctx).unwrap();

    assert_eq!(state.project.name, "Untitled");
    assert_ne!(state.project.id, old_id, "a new project is a new identity");
    assert_eq!(state.project.tracks.len(), 1, "the page's one synth");
    assert!(state.session.solo.is_empty() && state.session.selected_track.is_none());
    assert_eq!(
        state.session.midi_input.as_deref(),
        Some("in-1"),
        "the player's MIDI input stays chosen"
    );
    assert!(!state.history.can_undo() && !state.history.can_redo());
    assert_eq!(ctx.group, None);
    assert_eq!(ctx.folder, None);
    assert_eq!(ctx.opened, None, "no folder yet");
    assert_eq!(
        outcome.output,
        Output::Project(cmd::Output::New {
            name: "Untitled".into()
        })
    );
    assert!(matches!(
        outcome.events.first(),
        Some(ProjectEvent::ProjectOpened { name }) if name == "Untitled"
    ));
    assert!(commands::engine_msgs::replaces_project(&project(
        cmd::Command::New {}
    )));

    // Two new projects are two identities.
    let mut other = fixture().state;
    commands::run(&project(cmd::Command::New {}), &mut other, &mut ctx).unwrap();
    assert_ne!(other.project.id, state.project.id);
}

#[test]
fn saving_changes_nothing_in_the_project_and_is_never_recorded() {
    for command in [
        cmd::Command::Save {},
        cmd::Command::SaveAs {},
        cmd::Command::ExportPack {},
        cmd::Command::ImportPack {},
        cmd::Command::OpenUrl {
            url: "https://example.org/song.faulkpack".into(),
        },
        cmd::Command::ReopenLast {},
        cmd::Command::OpenFolder {},
        cmd::Command::OpenRecent { id: "a".into() },
        cmd::Command::OpenBrowserProject { dir: "a".into() },
        cmd::Command::DeleteBrowserProject { dir: "a".into() },
        cmd::Command::KeepMine {},
        cmd::Command::LoadFolderVersion {},
        cmd::Command::AllowAccess {},
    ] {
        let mut state = fixture().state;
        let before = state.clone();
        let mut ctx = context();
        let outcome = commands::run(&project(command.clone()), &mut state, &mut ctx).unwrap();
        assert_eq!(state, before, "{command:?}");
        assert!(outcome.events.is_empty(), "{command:?}");
        assert_eq!(
            Command::Project(command.clone()).meta().kind,
            commands::Kind::Live
        );
        assert!(!commands::engine_msgs::replaces_project(&project(command)));
    }
}

#[test]
fn opening_leaves_what_a_later_save_needs_and_a_refusal_leaves_nothing() {
    let mut state = fixture().state;
    let mut ctx = context();
    commands::run(
        &project(cmd::Command::Open {
            folder: OPENED_FOLDER.into(),
        }),
        &mut state,
        &mut ctx,
    )
    .unwrap();
    let opened = ctx.opened.take().expect("the folder's facts");
    assert!(
        opened.state.known_files().any(|p| p == "project.json"),
        "the hash of each file as read"
    );
    assert!(opened.root.contains("project.json"));

    // A folder that is refused changes nothing, the facts of the folder before included.
    let mut ctx = bare_context();
    ctx.folder = Some(Folder::new("empty", ::project::file::MemFs::new()));
    let refused = commands::run(
        &project(cmd::Command::Open {
            folder: "empty".into(),
        }),
        &mut state,
        &mut ctx,
    );
    assert!(refused.is_err());
    assert_eq!(ctx.opened, None);
}

#[test]
fn listing_answers_with_what_the_host_staged_once_and_changes_nothing() {
    use commands::contract::{StoredBrowserProject, StoredFolder, StoredProjects};
    let mut state = fixture().state;
    let before = state.clone();
    let mut ctx = context();
    ctx.stored = Some(StoredProjects {
        recent: vec![StoredFolder {
            id: "r1".into(),
            name: "Song".into(),
        }],
        browser: vec![StoredBrowserProject {
            dir: "d1".into(),
            name: "Draft".into(),
        }],
    });
    let ask = || project(cmd::Command::ListStored {});
    let first = commands::run(&ask(), &mut state, &mut ctx).unwrap();
    assert_eq!(
        first.output,
        Output::Project(cmd::Output::ListStored {
            recent: vec![StoredFolder {
                id: "r1".into(),
                name: "Song".into()
            }],
            browser: vec![StoredBrowserProject {
                dir: "d1".into(),
                name: "Draft".into()
            }],
        })
    );
    assert_eq!(state, before);
    assert!(first.events.is_empty());
    // The list served one answer: asking again without a new staging gives nothing, never an old list.
    let second = commands::run(&ask(), &mut state, &mut ctx).unwrap();
    assert_eq!(
        second.output,
        Output::Project(cmd::Output::ListStored {
            recent: vec![],
            browser: vec![]
        })
    );
    assert_eq!(ctx.stored, None);
}
