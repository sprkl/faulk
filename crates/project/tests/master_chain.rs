//! #364: the master's chain of effects is saved with the project and read back; a project
//! without one writes the same `project.json` as before; a chain with an instrument in it
//! is refused on read.

use project::file::*;
use project::*;
use std::collections::BTreeMap;

fn effect(order: OrderKey, kind: &str, enabled: bool) -> Device {
    Device {
        order,
        role: DeviceRole::Effect,
        enabled,
        source: DeviceSource::Builtin {
            id: kind.into(),
            version: 1,
        },
        params: BTreeMap::from([("threshold".into(), -30.0)]),
        state: None,
    }
}

fn project_with_chain() -> (Project, DeviceId, DeviceId) {
    let mut ids = IdGenerator::for_tests();
    let mut project = Project::new(ProjectId::generate(&mut ids), "Master");
    let (a, b) = (DeviceId::generate(&mut ids), DeviceId::generate(&mut ids));
    let first = OrderKey::first();
    let second = OrderKey::between(Some(&first), None).unwrap();
    project
        .master_devices
        .insert(a.clone(), effect(first, "faulk.compressor", true));
    project
        .master_devices
        .insert(b.clone(), effect(second, "faulk.echo", false));
    (project, a, b)
}

fn save_all(project: &Project) -> MemFs {
    let mut fs = MemFs::new();
    let mut state = SaveState::new();
    save(&mut fs, project, &mut state, None, &SaveOptions::default()).expect("save");
    fs
}

#[test]
fn the_master_chain_is_saved_and_read_back_in_order_with_its_bypass() {
    let (project, a, b) = project_with_chain();
    project.validate().unwrap();
    let fs = save_all(&project);
    let opened = open(&fs, &OpenOptions::default()).expect("open");
    assert_eq!(opened.project, project);
    let order: Vec<_> = opened
        .project
        .master_devices_in_order()
        .into_iter()
        .map(|(id, _)| id.clone())
        .collect();
    assert_eq!(order, [a.clone(), b.clone()]);
    assert!(!opened.project.master_devices[&b].enabled);
    // A second save of what was read writes nothing new.
    let again = save_all(&opened.project);
    assert_eq!(
        again.read("project.json").unwrap(),
        fs.read("project.json").unwrap()
    );
}

#[test]
fn a_project_without_a_master_chain_writes_no_master_field() {
    let mut ids = IdGenerator::for_tests();
    let project = Project::new(ProjectId::generate(&mut ids), "Plain");
    let fs = save_all(&project);
    let text = String::from_utf8(fs.read("project.json").unwrap()).unwrap();
    assert!(!text.contains("master_devices"), "{text}");
    // And an old file, without the field, still reads.
    let opened = open(&fs, &OpenOptions::default()).expect("open");
    assert!(opened.project.master_devices.is_empty());
}

#[test]
fn an_instrument_in_the_master_chain_is_refused() {
    let (mut project, a, _) = project_with_chain();
    project.master_devices.get_mut(&a).unwrap().role = DeviceRole::Instrument;
    let violations = project.validate().unwrap_err();
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::NotAnEffect),
        "{violations:?}"
    );
}
