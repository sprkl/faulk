//! The reference projects: committed folders that are read and rewritten byte for byte, and
//! whose files are checked against the examples of the format's documentation
//! (`docs/architecture/02-modele-projet.md` § 2.3 to § 2.5), written by hand here, never
//! produced by the writer under test.

use project::file::*;
use project::*;
use std::path::{Path, PathBuf};

mod common;
use common::reference::{LAYOUT, all};

fn reference_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("reference")
        .join(name)
}

/// A folder on disk, as a map of files (blobs included).
fn load_dir(dir: &Path) -> MemFs {
    fn go(dir: &Path, rel: &str, fs: &mut MemFs) {
        for entry in std::fs::read_dir(dir).expect("reference folder") {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if entry.file_type().unwrap().is_dir() {
                go(&entry.path(), &path, fs);
            } else {
                fs.insert(path, std::fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut fs = MemFs::new();
    go(dir, "", &mut fs);
    fs
}

fn options() -> SaveOptions {
    SaveOptions {
        layout: LAYOUT,
        support_files: false,
        write_cache: false,
    }
}

fn write_dir(dir: &Path, fs: &MemFs) {
    let _ = std::fs::remove_dir_all(dir);
    for (path, bytes) in fs.files() {
        let target = dir.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, bytes).unwrap();
    }
}

/// `FAULK_BLESS=1` rewrites the committed folders from the project builders: review the
/// diff against the documentation before committing it.
#[test]
fn the_reference_folders_are_up_to_date() {
    for (name, project) in all() {
        let mut fs = MemFs::new();
        save(&mut fs, &project, &mut SaveState::new(), None, &options()).unwrap();
        if std::env::var_os("FAULK_BLESS").is_some() {
            write_dir(&reference_dir(name), &fs);
        }
        let committed = load_dir(&reference_dir(name));
        assert_eq!(
            fs.files().keys().collect::<Vec<_>>(),
            committed.files().keys().collect::<Vec<_>>(),
            "{name}: the files differ (FAULK_BLESS=1 to rewrite)"
        );
        for (path, bytes) in fs.files() {
            assert_eq!(
                String::from_utf8_lossy(bytes),
                String::from_utf8_lossy(&committed.files()[path]),
                "{name}: {path}"
            );
        }
    }
}

#[test]
fn reference_projects_are_read_and_rewritten_byte_for_byte() {
    for (name, project) in all() {
        let committed = load_dir(&reference_dir(name));
        let opened =
            open(&committed, &OpenOptions::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(opened.project, project, "{name}");
        assert!(opened.report.ignored.is_empty(), "{name}");
        // Rewritten over a fresh folder: the same bytes, blobs included.
        let mut rewritten = MemFs::new();
        save(
            &mut rewritten,
            &opened.project,
            &mut SaveState::new(),
            None,
            &options(),
        )
        .unwrap();
        assert_eq!(rewritten.files(), committed.files(), "{name}");
        // And over the folder itself, with the state of the open: nothing to write.
        let mut again = committed.clone();
        let mut state = opened.state;
        again.take_log();
        let report = save(&mut again, &opened.project, &mut state, None, &options()).unwrap();
        assert!(
            report.written.is_empty() && report.removed.is_empty() && report.blobs.is_empty(),
            "{name}: {report:?}"
        );
        assert_eq!(again.take_log(), vec![], "{name}");
    }
}

#[test]
fn the_audio_blob_the_song_names_is_reported_missing_not_an_error() {
    let committed = load_dir(&reference_dir("song"));
    let opened = open(&committed, &OpenOptions::default()).unwrap();
    assert_eq!(opened.report.missing_blobs, vec![Hash([0xab; 32])]);
}
