//! The section markers in the document and its folder (#473): saved in `project.json` only
//! when there are some (a project without writes the same file as before), read back
//! unchanged, and checked by `validate`.

use project::file::*;
use project::*;

fn project_with(markers: &[(i64, &str)]) -> Project {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Song");
    for (tick, name) in markers {
        project.markers.insert(
            MarkerId::generate(&mut ids),
            Marker {
                tick: Tick(*tick),
                name: (*name).to_owned(),
            },
        );
    }
    project
}

fn saved(project: &Project) -> MemFs {
    let mut fs = MemFs::new();
    save(
        &mut fs,
        project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .unwrap();
    fs
}

#[test]
fn markers_round_trip_and_an_empty_list_writes_nothing() {
    let none = saved(&project_with(&[]));
    let text = String::from_utf8(none.read("project.json").unwrap()).unwrap();
    assert!(!text.contains("markers"), "{text}");

    let song = project_with(&[(0, "Intro"), (3840 * 8, "Verse"), (3840 * 8, "Fill")]);
    song.validate().unwrap();
    let fs = saved(&song);
    let text = String::from_utf8(fs.read("project.json").unwrap()).unwrap();
    assert!(text.contains("\"markers\""), "{text}");
    let opened = open(&fs, &OpenOptions::default()).unwrap();
    assert_eq!(opened.project.markers, song.markers);
    // In timeline order, two on one tick by id.
    let order: Vec<_> = opened
        .project
        .markers_in_order()
        .into_iter()
        .map(|(_, m)| m.tick.get())
        .collect();
    assert_eq!(order, [0, 3840 * 8, 3840 * 8]);
}

#[test]
fn validate_refuses_a_bad_marker() {
    for (tick, name) in [(-1, "A"), ((1 << 40) + 1, "A"), (0, ""), (0, " A")] {
        let p = project_with(&[(tick, name)]);
        assert!(p.validate().is_err(), "{tick} {name:?}");
    }
    let long = "x".repeat(201);
    assert!(project_with(&[(0, &long)]).validate().is_err());
}
