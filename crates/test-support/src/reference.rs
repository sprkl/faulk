//! Reference projects: scenarios built through commands and recorded in
//! `tests/fixtures/<name>.stream`, so a change of the command layer's mapping to the
//! engine shows as a diff of a fixture, and every engine target renders the same bytes.
//!
//! `FAULK_RECORD=1 cargo test -p test-support` rewrites the fixtures.

use std::path::PathBuf;

use crate::scenario::{Scenario, Session};

/// Silence before the first note of the references, in seconds (whole blocks).
pub const LEAD_IN: f32 = 0.25;

/// A reference project: its name (the fixture's), how it is built, and what it must do.
pub struct Reference {
    pub name: &'static str,
    pub build: fn() -> Scenario,
}

/// Every reference project.
pub const ALL: &[Reference] = &[
    Reference {
        name: "chord",
        build: chord,
    },
    Reference {
        name: "melody",
        build: melody,
    },
];

/// A sine A minor chord (A3, C4, E4) that starts [`LEAD_IN`] seconds in, is held one
/// second, then released (50 ms): silence, the chord, then exact silence again.
pub fn chord() -> Scenario {
    let mut s = Session::single_synth();
    s.set_param("waveform", 0.0)
        .set_param("release", 0.05)
        .render_secs(LEAD_IN)
        .note_on(57, 100)
        .note_on(60, 100)
        .note_on(64, 100)
        .render_secs(1.0)
        .note_off(57)
        .note_off(60)
        .note_off(64)
        .render_secs(0.5);
    s.into_scenario()
}

/// A saw melody of four notes (A3, C4, E4, A4), 0.25 s each with the next one starting as
/// the previous one is released, after [`LEAD_IN`] seconds of silence.
pub fn melody() -> Scenario {
    let mut s = Session::single_synth();
    s.set_param("waveform", 1.0)
        .set_param("gain", 0.5)
        .set_param("release", 0.02)
        .render_secs(LEAD_IN);
    for note in [57, 60, 64, 69] {
        s.note_on(note, 110).render_secs(0.25).note_off(note);
    }
    s.render_secs(0.25);
    s.into_scenario()
}

/// The path of a reference's fixture.
pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.stream"))
}

/// Rewrites a fixture when its bytes differ, through a temporary file and a rename so a
/// reader never sees a half-written file; one writer at a time inside a process.
fn record(path: &std::path::Path, bytes: &[u8]) {
    static WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITER.lock().unwrap_or_else(|e| e.into_inner());
    if std::fs::read(path).is_ok_and(|old| old == bytes) {
        return;
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

/// The recorded scenario of a reference. With `FAULK_RECORD` set, the fixture is
/// rewritten from the build first. Panics when the fixture is missing, or differs from
/// what the command layer produces today: the mapping changed, record again.
pub fn load(reference: &Reference) -> Scenario {
    let path = fixture_path(reference.name);
    let built = (reference.build)();
    if std::env::var_os("FAULK_RECORD").is_some() {
        record(&path, &built.encode());
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing: FAULK_RECORD=1 cargo test -p test-support",
            path.display()
        )
    });
    let recorded = Scenario::decode(&bytes)
        .unwrap_or_else(|| panic!("{} is truncated: FAULK_RECORD=1", path.display()));
    assert!(
        recorded == built,
        "fixture {} is stale: FAULK_RECORD=1 cargo test -p test-support",
        reference.name
    );
    recorded
}

/// The reference called `name`.
pub fn get(name: &str) -> &'static Reference {
    ALL.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no reference project called {name}"))
}
