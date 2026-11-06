//! The attack pass on the project folder's confinement (AU-7, #116): every form of path the
//! helper refuses, the links that lead out, a folder switched under a read or a write, two
//! folders of one project, the dialog gate, the remembered list.

use super::*;
use std::collections::BTreeMap;

const PROJECT: &str = "01J0000000000000000000000A";
const OTHER: &str = "01J0000000000000000000000B";

/// A fresh directory of its own under the system's temporary folder, removed on drop.
struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!(
            "faulk-116-{tag}-{}",
            crate::security::random_hex(6)
        ));
        fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        // Links first, so the removal never follows one out of the directory.
        let _ = remove_tree(&self.0);
    }
}

/// Removes a tree without following links (a junction or a symlink is removed itself).
fn remove_tree(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || is_junction(path) {
        return fs::remove_dir(path).or_else(|_| fs::remove_file(path));
    }
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            remove_tree(&entry?.path())?;
        }
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(windows)]
fn is_junction(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_REPARSE_POINT
    fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & 0x400 != 0)
}

#[cfg(not(windows))]
fn is_junction(_path: &Path) -> bool {
    false
}

/// A project folder holding `project.json` of `id`.
fn project_dir(parent: &Path, name: &str, id: &str) -> PathBuf {
    let dir = parent.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("project.json"),
        format!("{{\n  \"id\": \"{id}\",\n  \"name\": \"{name}\"\n}}\n"),
    )
    .unwrap();
    dir
}

/// Every file below `dir` with its bytes, links not followed: what "the disk untouched" compares.
fn listing(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if meta.file_type().is_symlink() || is_junction(&path) {
                out.insert(format!("{rel} -> link"), Vec::new());
            } else if meta.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(base, &path, out);
            } else {
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    walk(dir, dir, &mut out);
    out
}

/// A directory link at `link` to `target`: a symlink on Unix, a junction on Windows (no
/// privilege needed for one).
fn dir_link(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(out.status.success(), "mklink /J: {out:?}");
    }
}

/// A file symlink at `link` to `target`; `false` where the system refuses to make one (Windows
/// without the privilege or developer mode).
fn file_link(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).unwrap();
        true
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_file(target, link) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("skipped: this machine refuses file symlinks ({e})");
                false
            }
        }
    }
}

/// A root, an outside folder with a canary, and the project folder opened.
struct Setup {
    _temp: Temp,
    root: PathBuf,
    outside: PathBuf,
    folder: ProjectFolder,
}

fn setup(tag: &str) -> Setup {
    let temp = Temp::new(tag);
    let root = project_dir(temp.path(), "Song", PROJECT);
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("canary.txt"), b"canary").unwrap();
    let folder = ProjectFolder::open(&root, Some(PROJECT)).unwrap();
    Setup {
        _temp: temp,
        root,
        outside,
        folder,
    }
}

impl Setup {
    fn snapshot(&self) -> (BTreeMap<String, Vec<u8>>, BTreeMap<String, Vec<u8>>) {
        (listing(&self.root), listing(&self.outside))
    }
}

// ---------------------------------------------------------------- T1: paths as text

#[test]
fn every_traversal_form_is_refused_by_its_typed_error_before_the_disk() {
    let s = setup("text");
    let before = s.snapshot();
    let cases: &[(&str, PathError)] = &[
        ("", PathError::Empty),
        ("a//b", PathError::Empty),
        ("a/", PathError::Empty),
        ("../x", PathError::Dot),
        ("a/../../x", PathError::Dot),
        ("./x", PathError::Dot),
        ("a/./b", PathError::Dot),
        ("..", PathError::Dot),
        ("/etc/passwd", PathError::Absolute),
        ("/x", PathError::Absolute),
        ("C:/Windows/x", PathError::Colon),
        ("C:x", PathError::Colon),
        ("c:", PathError::Colon),
        ("x.wav:stream", PathError::Colon),
        ("//server/share/x", PathError::Absolute),
        ("\\\\server\\share\\x", PathError::Backslash),
        ("a\\..\\x", PathError::Backslash),
        ("\\x", PathError::Backslash),
        ("a\0b", PathError::Control),
        ("a\nb", PathError::Control),
        ("a./b", PathError::Trailing),
        ("a /b", PathError::Trailing),
        ("x.", PathError::Trailing),
        ("nul", PathError::Device),
        ("NUL", PathError::Device),
        ("con.txt", PathError::Device),
        ("exports/COM1.wav", PathError::Device),
        ("lpt9", PathError::Device),
        ("aux .txt", PathError::Device),
        ("conin$", PathError::Device),
    ];
    for (path, expected) in cases {
        assert_eq!(check_relative(path).unwrap_err(), *expected, "{path:?}");
        assert_eq!(
            s.folder.write(path, b"attack").unwrap_err(),
            *expected,
            "write {path:?}"
        );
    }
    let long = "a/".repeat(MAX_COMPONENTS) + "x";
    assert_eq!(s.folder.write(&long, b"x").unwrap_err(), PathError::TooLong);
    let wide = "a".repeat(MAX_PATH_BYTES + 1);
    assert_eq!(s.folder.write(&wide, b"x").unwrap_err(), PathError::TooLong);
    assert_eq!(s.snapshot(), before, "a refused path touched the disk");
}

#[test]
fn plain_names_pass_the_text_check() {
    for path in [
        "mix.wav",
        "exports/Song 1.wav",
        "a/b/c.txt",
        "été/ça.wav",
        ".hidden",
        "a..b",
        "console.txt",
        "comet",
    ] {
        assert!(check_relative(path).is_ok(), "{path:?}");
    }
}

// ---------------------------------------------------------------- zones

#[test]
fn the_projects_own_files_and_the_folders_data_are_never_written_through_a_path() {
    let s = setup("zones");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    fs::create_dir_all(s.root.join(".faulk").join("cache").join("pcm")).unwrap();
    let before = s.snapshot();
    for path in [
        "project.json",
        "Project.JSON",
        "tracks/01J0000000000000000000000C/track.json",
        "master/automation/x.tsv",
        "presets/x.json",
        "blobs/abc",
        "BLOBS/abc",
        "Blobs/abc",
        ".faulk/cache/pcm/abc.pcm",
        ".FAULK/cache/x",
        ".faulk/session.json",
        ".faulk/tmp/x",
        ".git/config",
        ".gitignore",
        ".gitattributes",
        ".github/x",
    ] {
        assert_eq!(
            s.folder.write(path, b"attack").unwrap_err(),
            PathError::Reserved,
            "{path:?}"
        );
    }
    assert_eq!(s.snapshot(), before);
}

#[test]
fn a_link_inside_the_folder_to_its_blobs_is_still_a_reserved_zone() {
    let s = setup("zone-link");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    dir_link(&s.root.join("blobs"), &s.root.join("exports"));
    let before = s.snapshot();
    assert_eq!(
        s.folder.write("exports/abc", b"attack").unwrap_err(),
        PathError::Reserved
    );
    assert_eq!(s.snapshot(), before);
}

/// Windows gives `.faulk` an 8.3 name (`FAULK~1`) on volumes that make short names: the zone is
/// checked again on the canonical path, which spells the long name.
#[cfg(windows)]
#[test]
fn a_short_name_of_the_faulk_folder_is_still_a_reserved_zone() {
    let s = setup("short");
    fs::create_dir_all(s.root.join(".faulk").join("cache")).unwrap();
    // Whether the volume makes short names is the volume's setting: when `FAULK~1` does not
    // resolve, there is no alias to attack.
    if !s.root.join("FAULK~1").exists() {
        eprintln!("skipped: this volume makes no 8.3 names");
        return;
    }
    let before = s.snapshot();
    assert_eq!(
        s.folder.write("FAULK~1/cache/x", b"attack").unwrap_err(),
        PathError::Reserved
    );
    assert_eq!(s.snapshot(), before);
}

// ---------------------------------------------------------------- writes

#[test]
fn a_write_lands_whole_in_the_folder_and_leaves_no_temporary_file() {
    let s = setup("write");
    s.folder.write("exports/Song.wav", b"RIFF....").unwrap();
    assert_eq!(
        fs::read(s.root.join("exports").join("Song.wav")).unwrap(),
        b"RIFF...."
    );
    s.folder.write("exports/Song.wav", b"second").unwrap();
    assert_eq!(
        fs::read(s.root.join("exports").join("Song.wav")).unwrap(),
        b"second"
    );
    let tmp: Vec<_> = fs::read_dir(s.root.join(".faulk").join("tmp"))
        .unwrap()
        .collect();
    assert!(tmp.is_empty(), "temporary files left: {tmp:?}");
}

#[test]
fn a_directory_where_the_file_goes_is_refused() {
    let s = setup("isdir");
    fs::create_dir_all(s.root.join("exports").join("Song.wav")).unwrap();
    assert_eq!(
        s.folder.write("exports/Song.wav", b"x").unwrap_err(),
        PathError::IsDirectory
    );
}

// ---------------------------------------------------------------- T2: links leading out

#[test]
fn a_directory_link_leading_out_is_refused_for_a_write() {
    let s = setup("dirlink");
    dir_link(&s.outside, &s.root.join("exports"));
    let before = s.snapshot();
    assert_eq!(
        s.folder.write("exports/canary.txt", b"attack").unwrap_err(),
        PathError::Escapes
    );
    assert_eq!(
        s.folder.write("exports/new/x.wav", b"attack").unwrap_err(),
        PathError::Escapes
    );
    assert_eq!(s.snapshot(), before);
    assert_eq!(fs::read(s.outside.join("canary.txt")).unwrap(), b"canary");
}

#[test]
fn a_link_in_the_middle_of_a_path_leading_out_is_refused() {
    let s = setup("midlink");
    fs::create_dir_all(s.root.join("a")).unwrap();
    dir_link(&s.outside, &s.root.join("a").join("b"));
    let before = s.snapshot();
    assert_eq!(
        s.folder.write("a/b/c/x.wav", b"attack").unwrap_err(),
        PathError::Escapes
    );
    assert_eq!(s.snapshot(), before);
}

#[test]
fn a_temporary_folder_that_is_a_link_is_refused() {
    let s = setup("tmplink");
    fs::create_dir_all(s.root.join(".faulk")).unwrap();
    dir_link(&s.outside, &s.root.join(".faulk").join("tmp"));
    let before = s.snapshot();
    assert_eq!(
        s.folder.write("mix.wav", b"attack").unwrap_err(),
        PathError::Escapes
    );
    assert_eq!(s.snapshot(), before);
    // Even to inside the folder: the temporary file would not be where `.faulk/tmp/` is cleaned.
    let s = setup("tmplink-in");
    fs::create_dir_all(s.root.join(".faulk")).unwrap();
    fs::create_dir_all(s.root.join("elsewhere")).unwrap();
    dir_link(
        &s.root.join("elsewhere"),
        &s.root.join(".faulk").join("tmp"),
    );
    assert_eq!(
        s.folder.write("mix.wav", b"x").unwrap_err(),
        PathError::Escapes
    );
}

#[test]
fn a_file_link_at_the_target_is_replaced_never_written_through() {
    let s = setup("filelink");
    if !file_link(&s.outside.join("canary.txt"), &s.root.join("mix.wav")) {
        return;
    }
    s.folder.write("mix.wav", b"new").unwrap();
    assert_eq!(fs::read(s.outside.join("canary.txt")).unwrap(), b"canary");
    let meta = fs::symlink_metadata(s.root.join("mix.wav")).unwrap();
    assert!(!meta.file_type().is_symlink() && meta.is_file());
    assert_eq!(fs::read(s.root.join("mix.wav")).unwrap(), b"new");
}

#[test]
fn a_link_that_stays_inside_the_folder_is_followed() {
    let s = setup("inlink");
    fs::create_dir_all(s.root.join("renders")).unwrap();
    dir_link(&s.root.join("renders"), &s.root.join("exports"));
    s.folder.write("exports/x.wav", b"ok").unwrap();
    assert_eq!(
        fs::read(s.root.join("renders").join("x.wav")).unwrap(),
        b"ok"
    );
}

// ---------------------------------------------------------------- reads

const MEDIA: [u8; 32] = [0xab; 32];

/// A media read as (bytes, file length).
fn range(
    f: &ProjectFolder,
    kind: FileKind,
    media: &[u8; 32],
    offset: u64,
    len: u32,
) -> Result<(Vec<u8>, u64), Failure> {
    f.read_media(kind, media, offset, len)
        .map(|r| (r.bytes, r.file_len))
}

fn hex(media: &[u8; 32]) -> String {
    media.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_media_read_gives_its_range_and_the_files_length_and_a_short_read_past_the_end() {
    let s = setup("read");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    fs::write(s.root.join("blobs").join(hex(&MEDIA)), b"0123456789").unwrap();
    let pcm = s.root.join(".faulk").join("cache").join("pcm");
    fs::create_dir_all(&pcm).unwrap();
    fs::write(pcm.join(format!("{}.pcm", hex(&MEDIA))), b"FKPCM001").unwrap();
    let f = &s.folder;
    assert_eq!(
        range(f, FileKind::Blob, &MEDIA, 2, 3).unwrap(),
        (b"234".to_vec(), 10)
    );
    assert_eq!(
        range(f, FileKind::Blob, &MEDIA, 8, 100).unwrap(),
        (b"89".to_vec(), 10)
    );
    assert_eq!(
        range(f, FileKind::Blob, &MEDIA, 50, 4).unwrap(),
        (Vec::new(), 10)
    );
    assert_eq!(
        range(f, FileKind::Cache, &MEDIA, 0, 8).unwrap(),
        (b"FKPCM001".to_vec(), 8)
    );
    assert_eq!(
        range(f, FileKind::Blob, &[0; 32], 0, 4).unwrap_err(),
        Failure::NotFound
    );
}

#[test]
fn a_read_is_capped() {
    let s = setup("cap");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    let big = vec![7u8; MAX_READ_BYTES as usize + 10];
    fs::write(s.root.join("blobs").join(hex(&MEDIA)), &big).unwrap();
    let (bytes, len) = s
        .folder
        .read_media(FileKind::Blob, &MEDIA, 0, u32::MAX)
        .map(|r| (r.bytes, r.file_len))
        .unwrap();
    assert_eq!(bytes.len(), MAX_READ_BYTES as usize);
    assert_eq!(len, big.len() as u64);
}

#[test]
fn a_blobs_folder_leading_out_is_never_read() {
    let s = setup("readlink");
    fs::write(s.outside.join(hex(&MEDIA)), b"secret").unwrap();
    dir_link(&s.outside, &s.root.join("blobs"));
    assert_eq!(
        s.folder
            .read_media(FileKind::Blob, &MEDIA, 0, 6)
            .unwrap_err(),
        Failure::Unreadable
    );
}

#[test]
fn a_blob_that_is_a_link_leading_out_is_never_read() {
    let s = setup("readfilelink");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    fs::write(s.outside.join("secret"), b"secret").unwrap();
    if !file_link(
        &s.outside.join("secret"),
        &s.root.join("blobs").join(hex(&MEDIA)),
    ) {
        return;
    }
    assert_eq!(
        s.folder
            .read_media(FileKind::Blob, &MEDIA, 0, 6)
            .unwrap_err(),
        Failure::Unreadable
    );
}

// ---------------------------------------------------------------- opening

#[test]
fn a_folder_opens_only_with_a_project_of_the_expected_id() {
    let temp = Temp::new("open");
    let song = project_dir(temp.path(), "Song", PROJECT);
    assert_eq!(
        ProjectFolder::open(&song, Some(PROJECT)).unwrap().name(),
        "Song"
    );
    assert_eq!(
        ProjectFolder::open(&song, Some(OTHER)).unwrap_err(),
        OpenError::OtherProject
    );
    let empty = temp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    assert_eq!(
        ProjectFolder::open(&empty, None).unwrap_err(),
        OpenError::NoProject
    );
    fs::write(empty.join("project.json"), b"{\"id\": \"not a ulid\"}").unwrap();
    assert_eq!(
        ProjectFolder::open(&empty, None).unwrap_err(),
        OpenError::NoProject
    );
    assert_eq!(
        ProjectFolder::open(&temp.path().join("missing"), None).unwrap_err(),
        OpenError::Unreadable
    );
}

// ---------------------------------------------------------------- T4: switched mid-write, mid-read

#[test]
fn a_folder_replaced_by_another_project_before_the_commit_gets_nothing() {
    let s = setup("swap");
    let parent = s.root.parent().unwrap().to_path_buf();
    let moved = parent.join("Song-moved");
    let result = s.folder.write_with("mix.wav", b"attack", || {
        fs::rename(&s.root, &moved).unwrap();
        project_dir(&parent, "Song", OTHER);
    });
    assert_eq!(result.unwrap_err(), PathError::FolderChanged);
    assert!(!s.root.join("mix.wav").exists());
    assert!(!moved.join("mix.wav").exists());
    // The temporary file went with the folder it was made in, where the helper cannot find it
    // any more: it stays in that folder's `.faulk/tmp/`, never as a project file, and the page's
    // sweep of `.faulk/tmp/` removes it at the folder's next attach (`FolderBackend.cleanUp`).
    let tmp: Vec<String> = fs::read_dir(moved.join(".faulk").join("tmp"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        tmp.len() == 1 && tmp[0].starts_with("helper-") && tmp[0].ends_with(".tmp"),
        "{tmp:?}"
    );
    assert!(!s.root.join(".faulk").join("tmp").exists());
}

#[test]
fn a_folder_replaced_by_a_link_leading_out_before_the_commit_gets_nothing() {
    let s = setup("swaplink");
    let parent = s.root.parent().unwrap().to_path_buf();
    let moved = parent.join("Song-moved");
    // Outside, a folder that even holds the same project.
    let decoy = project_dir(&parent, "decoy", PROJECT);
    let result = s.folder.write_with("mix.wav", b"attack", || {
        fs::rename(&s.root, &moved).unwrap();
        dir_link(&decoy, &s.root);
    });
    assert_eq!(result.unwrap_err(), PathError::FolderChanged);
    assert!(!decoy.join("mix.wav").exists());
    assert!(!moved.join("mix.wav").exists());
}

#[test]
fn a_parent_replaced_by_a_link_before_the_commit_gets_nothing() {
    let s = setup("swapparent");
    fs::create_dir_all(s.root.join("exports")).unwrap();
    let result = s.folder.write_with("exports/mix.wav", b"attack", || {
        fs::rename(s.root.join("exports"), s.root.join("old")).unwrap();
        dir_link(&s.outside, &s.root.join("exports"));
    });
    assert_eq!(result.unwrap_err(), PathError::FolderChanged);
    assert_eq!(listing(&s.outside).len(), 1, "only the canary outside");
    let tmp: Vec<_> = fs::read_dir(s.root.join(".faulk").join("tmp"))
        .unwrap()
        .collect();
    assert!(tmp.is_empty());
}

#[test]
fn a_write_stays_in_the_folder_it_began_in_when_the_session_switched_meanwhile() {
    let s = setup("switch");
    let b = project_dir(s.root.parent().unwrap(), "Other", OTHER);
    let open = OpenFolder::default();
    let a = Arc::new(ProjectFolder::open(&s.root, Some(PROJECT)).unwrap());
    open.set(Some(a.clone()));
    a.write_with("mix.wav", b"for A", || {
        open.set(Some(Arc::new(
            ProjectFolder::open(&b, Some(OTHER)).unwrap(),
        )));
    })
    .unwrap();
    assert_eq!(fs::read(s.root.join("mix.wav")).unwrap(), b"for A");
    assert!(!b.join("mix.wav").exists());
}

fn job(file: FileKind, media: [u8; 32]) -> Job {
    Job {
        id: 7,
        file,
        media,
        offset: 0,
        len: 4,
    }
}

#[test]
fn a_read_finished_after_the_folder_switched_is_answered_not_found() {
    let s = setup("readswitch");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    fs::write(s.root.join("blobs").join(hex(&MEDIA)), b"AAAA").unwrap();
    let open = Arc::new(OpenFolder::default());
    open.set(Some(Arc::new(ProjectFolder::open(&s.root, None).unwrap())));
    let mut reader = DiskReader::new(open.clone());
    let j = job(FileKind::Blob, MEDIA);
    assert_eq!(
        reader.run(&j, || {}),
        Outcome::Done {
            bytes: b"AAAA".to_vec(),
            file_len: 4
        }
    );
    // Switched between the read and its answer: to no folder, then to the same one again.
    assert_eq!(
        reader.run(&j, || open.set(None)),
        Outcome::Failed(Failure::NotFound)
    );
    let again = Arc::new(ProjectFolder::open(&s.root, None).unwrap());
    assert_eq!(
        reader.run(&j, || open.set(Some(again.clone()))),
        Outcome::Failed(Failure::NotFound)
    );
    // With no folder at all: not found, so the planner keeps the source offline.
    open.set(None);
    assert_eq!(reader.run(&j, || {}), Outcome::Failed(Failure::NotFound));
}

#[test]
fn a_file_replaced_between_two_reads_is_answered_changed_then_read_again() {
    let s = setup("changed");
    let pcm = s.root.join(".faulk").join("cache").join("pcm");
    fs::create_dir_all(&pcm).unwrap();
    let file = pcm.join(format!("{}.pcm", hex(&MEDIA)));
    fs::write(&file, b"FKPCM001").unwrap();
    let open = Arc::new(OpenFolder::default());
    open.set(Some(Arc::new(ProjectFolder::open(&s.root, None).unwrap())));
    let mut reader = DiskReader::new(open.clone());
    let j = job(FileKind::Cache, MEDIA);
    assert!(matches!(reader.run(&j, || {}), Outcome::Done { .. }));
    assert!(matches!(reader.run(&j, || {}), Outcome::Done { .. }));
    // The page made the cache again (another length): the planner probes it again.
    fs::write(&file, b"FKPCM001 and more").unwrap();
    assert_eq!(reader.run(&j, || {}), Outcome::Failed(Failure::Changed));
    assert!(matches!(
        reader.run(&j, || {}),
        Outcome::Done { file_len: 17, .. }
    ));
    // A file gone and back is a new snapshot, not a change.
    fs::remove_file(&file).unwrap();
    assert_eq!(reader.run(&j, || {}), Outcome::Failed(Failure::NotFound));
    fs::write(&file, b"FKPCM001").unwrap();
    assert!(matches!(reader.run(&j, || {}), Outcome::Done { .. }));
}

/// A sender whose engine declared the impulses' source: its planner has probe jobs to give.
fn sender_with_a_source() -> crate::audio::EventSender {
    let meter = Arc::new(crate::audio::Meter::default());
    let (_core, mut sender) = crate::audio::audio_core(48_000.0, meter).unwrap();
    let (setup, _, _) = test_support::clip_start::impulse_streams(48_000, &[]);
    sender.submit(&setup).unwrap();
    sender.flush();
    sender
}

#[test]
fn an_answer_never_reaches_an_engine_installed_after_its_poll() {
    let s = setup("engines");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    let open = Arc::new(OpenFolder::default());
    open.set(Some(Arc::new(ProjectFolder::open(&s.root, None).unwrap())));
    let mut reader = DiskReader::new(open);
    // The next output's engine: its planner numbers its jobs from 1 too, and has them in flight.
    let mut next = sender_with_a_source();
    let ids: Vec<u32> = next.disk_poll(0).iter().map(|j| j.id).collect();
    assert!(!ids.is_empty());
    let in_flight = next.disk_in_flight();
    let slot = crate::audio::EngineSlot::default();
    slot.install(sender_with_a_source(), 48_000.0);
    let mut jobs = Vec::new();
    disk_round(&slot, &mut reader, 0, &mut jobs, || {
        slot.install(next, 48_000.0);
    });
    assert_eq!(
        jobs.iter().map(|j| j.id).collect::<Vec<_>>(),
        ids,
        "both engines numbered the same jobs"
    );
    // The next engine's jobs are still waiting for their own answers.
    assert_eq!(slot.with(|s| s.disk_in_flight()), Some(in_flight));
    // Without a change of engine, a round answers the jobs it polled.
    let slot = crate::audio::EngineSlot::default();
    slot.install(sender_with_a_source(), 48_000.0);
    disk_round(&slot, &mut reader, 0, &mut jobs, || {});
    assert!(!jobs.is_empty());
    assert_eq!(slot.with(|s| s.disk_in_flight()), Some(0));
}

// ---------------------------------------------------------------- T5: remembered folders

const NONCE: &str = "0123456789abcdef0123456789abcdef";
const OTHER_NONCE: &str = "fedcba9876543210fedcba9876543210";

/// The page's link marker in `dir`.
fn mark(dir: &Path, nonce: &str) {
    fs::create_dir_all(dir.join(".faulk")).unwrap();
    fs::write(
        dir.join(".faulk").join(LINK_FILE),
        format!("{{\"nonce\": \"{nonce}\"}}"),
    )
    .unwrap();
}

#[test]
fn two_folders_of_one_project_are_told_apart_by_the_pages_marker_never_by_a_guess() {
    let temp = Temp::new("twins");
    let a = project_dir(temp.path(), "Song", PROJECT);
    let copies = temp.path().join("backup");
    fs::create_dir_all(&copies).unwrap();
    let b = project_dir(&copies, "Song", PROJECT);
    let both = Remembered::in_memory(vec![
        fs::canonicalize(&a).unwrap(),
        fs::canonicalize(&b).unwrap(),
    ]);
    // Neither holds the page's marker: a question.
    assert!(matches!(both.lookup(PROJECT, NONCE), Lookup::Unknown));
    // The page has `b` open: its marker is there only.
    mark(&b, NONCE);
    assert!(
        matches!(both.lookup(PROJECT, NONCE), Lookup::Found(f) if f.root() == fs::canonicalize(&b).unwrap())
    );
    // Only the copy is remembered while the page has the other one open: never taken for it.
    let only_a = Remembered::in_memory(vec![fs::canonicalize(&a).unwrap()]);
    assert!(matches!(only_a.lookup(PROJECT, NONCE), Lookup::Unknown));
    // A copy made after the marker holds it too: both, so the user chooses.
    mark(&a, NONCE);
    assert!(matches!(both.lookup(PROJECT, NONCE), Lookup::Ambiguous(2)));
    // The same folder by two paths is one folder.
    let same = Remembered::in_memory(vec![b.clone(), fs::canonicalize(&b).unwrap()]);
    assert!(matches!(same.lookup(PROJECT, NONCE), Lookup::Found(_)));
}

#[test]
fn a_remembered_folder_of_another_project_or_marker_is_not_opened() {
    let temp = Temp::new("names");
    let a = project_dir(temp.path(), "Song", PROJECT);
    mark(&a, NONCE);
    let list = Remembered::in_memory(vec![a.clone()]);
    assert!(matches!(list.lookup(PROJECT, NONCE), Lookup::Found(_)));
    assert!(matches!(list.lookup(PROJECT, OTHER_NONCE), Lookup::Unknown));
    assert!(matches!(list.lookup(OTHER, NONCE), Lookup::Unknown));
    // Its project changed since it was picked (another project saved there).
    fs::write(a.join("project.json"), format!("{{\"id\": \"{OTHER}\"}}")).unwrap();
    assert!(matches!(list.lookup(PROJECT, NONCE), Lookup::Unknown));
    fs::remove_dir_all(&a).unwrap();
    assert!(matches!(list.lookup(OTHER, NONCE), Lookup::Unknown));
}

#[test]
fn a_marker_is_read_only_when_valid_and_inside_the_folder() {
    let s = setup("marker");
    assert_eq!(s.folder.link_nonce(), None);
    mark(&s.root, "NOT HEX");
    assert_eq!(s.folder.link_nonce(), None);
    mark(&s.root, &NONCE.to_uppercase());
    assert_eq!(s.folder.link_nonce(), None);
    mark(&s.root, NONCE);
    assert!(s.folder.holds_link(NONCE));
    assert!(!s.folder.holds_link(OTHER_NONCE));
    // A `.faulk` leading out, to a folder holding the marker: not the folder's marker.
    let s = setup("marker-out");
    mark(&s.outside, NONCE);
    dir_link(&s.outside.join(".faulk"), &s.root.join(".faulk"));
    assert!(!s.folder.holds_link(NONCE));
}

// ---------------------------------------------------------------- T7: the remembered file

#[test]
fn the_remembered_file_is_read_leniently_bounded_and_kept_most_recent_first() {
    let temp = Temp::new("file");
    let file = temp.path().join(REMEMBERED_FILE);
    fs::write(&file, b"not json").unwrap();
    assert!(Remembered::load(file.clone()).paths().is_empty());
    let many: Vec<String> = (0..100).map(|i| format!("/p/{i}")).collect();
    fs::write(&file, serde_json::to_vec(&many).unwrap()).unwrap();
    assert_eq!(Remembered::load(file.clone()).paths().len(), MAX_REMEMBERED);
    let mut list = Remembered::load(file.clone());
    list.remember(Path::new("/p/5"));
    list.remember(Path::new("/q"));
    let again = Remembered::load(file.clone());
    assert_eq!(again.paths()[0], PathBuf::from("/q"));
    assert_eq!(again.paths()[1], PathBuf::from("/p/5"));
    assert_eq!(again.paths().len(), MAX_REMEMBERED);
    assert_eq!(
        again
            .paths()
            .iter()
            .filter(|p| p.as_path() == Path::new("/p/5"))
            .count(),
        1
    );
}

// ---------------------------------------------------------------- T6: one dialog at a time

#[test]
fn the_dialog_gate_lets_one_dialog_through_at_a_time_and_opens_again_however_it_ends() {
    let gate = Arc::new(DialogGate::default());
    let taken = gate.take().unwrap();
    assert!(gate.take().is_none());
    drop(taken);
    let taken = gate.take().unwrap();
    // A picker that panics with the gate held: the gate opens all the same.
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _held = taken;
        panic!("the dialog failed");
    }));
    assert!(panicked.is_err());
    assert!(gate.take().is_some());
}

// ---------------------------------------------------------------- refused before anything is made

#[test]
fn a_refused_write_makes_no_directory_not_even_its_parents() {
    // A missing parent below a link inside the folder to `blobs/`: a reserved zone once the link
    // is followed, refused before `blobs/new/` is made.
    let s = setup("noparents");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    dir_link(&s.root.join("blobs"), &s.root.join("renders"));
    let before = s.snapshot();
    assert_eq!(
        s.folder
            .write("renders/2026/mix.wav", b"attack")
            .unwrap_err(),
        PathError::Reserved
    );
    assert_eq!(s.snapshot(), before);
    assert!(!s.root.join("blobs").join("2026").exists());
    // A missing parent while `.faulk/tmp` leads out: refused before `exports/new/` is made.
    let s = setup("noparents-tmp");
    fs::create_dir_all(s.root.join(".faulk")).unwrap();
    dir_link(&s.outside, &s.root.join(".faulk").join("tmp"));
    let before = s.snapshot();
    assert_eq!(
        s.folder
            .write("exports/new/mix.wav", b"attack")
            .unwrap_err(),
        PathError::Escapes
    );
    assert_eq!(s.snapshot(), before);
    assert!(!s.root.join("exports").exists());
    // A missing parent below a directory link leading out: nothing made outside either.
    let s = setup("noparents-out");
    dir_link(&s.outside, &s.root.join("exports"));
    let before = s.snapshot();
    assert_eq!(
        s.folder
            .write("exports/new/mix.wav", b"attack")
            .unwrap_err(),
        PathError::Escapes
    );
    assert_eq!(s.snapshot(), before);
    assert!(!s.outside.join("new").exists());
}

/// The short name of a reserved file at the root (`PROJEC~1.JSO` for `project.json`) is the file
/// itself: refused like it, once the disk spells it.
#[cfg(windows)]
#[test]
fn a_short_name_of_a_reserved_file_is_refused() {
    let s = setup("shortfile");
    fs::write(s.root.join(".gitignore"), b"keep").unwrap();
    let mut tried = 0;
    for (short, long) in [("PROJEC~1.JSO", "project.json"), ("GITIGN~1", ".gitignore")] {
        if !s.root.join(short).exists() {
            eprintln!("skipped {short}: this volume makes no 8.3 names");
            continue;
        }
        tried += 1;
        let kept = fs::read(s.root.join(long)).unwrap();
        assert_eq!(
            s.folder.write(short, b"attack").unwrap_err(),
            PathError::Reserved,
            "{short}"
        );
        assert_eq!(fs::read(s.root.join(long)).unwrap(), kept);
    }
    eprintln!("{tried} short names tried");
}

#[test]
fn a_faulk_folder_that_is_a_link_inside_gets_no_temporary_folder_at_its_target() {
    let s = setup("faulklink");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    dir_link(&s.root.join("blobs"), &s.root.join(".faulk"));
    let before = s.snapshot();
    assert_eq!(
        s.folder.write("mix.wav", b"attack").unwrap_err(),
        PathError::Escapes
    );
    assert_eq!(s.snapshot(), before);
    assert!(!s.root.join("blobs").join("tmp").exists());
}

#[test]
fn a_project_file_that_is_a_link_leading_out_is_no_project() {
    let s = setup("projectlink");
    let decoy = project_dir(&s.outside, "Decoy", PROJECT);
    let dir = s.root.parent().unwrap().join("Linked");
    fs::create_dir_all(&dir).unwrap();
    if !file_link(&decoy.join("project.json"), &dir.join("project.json")) {
        return;
    }
    assert_eq!(project_id_of(&dir), None);
    assert_eq!(
        ProjectFolder::open(&dir, Some(PROJECT)).unwrap_err(),
        OpenError::NoProject
    );
}

// ---------------------------------------------------------------- the export's write (#319)

#[test]
fn every_hostile_export_name_is_refused_by_its_typed_error_before_the_disk() {
    let long = "a".repeat(MAX_EXPORT_NAME + 1);
    let cases: &[(&str, PathError)] = &[
        ("", PathError::Empty),
        ("..", PathError::Dot),
        (".", PathError::Dot),
        ("../x", PathError::Forbidden),
        ("a/b", PathError::Forbidden),
        ("/abs", PathError::Forbidden),
        (r"a\b", PathError::Backslash),
        (r"..\..\x", PathError::Backslash),
        ("C:x", PathError::Colon),
        ("x:stream", PathError::Colon),
        ("CON", PathError::Device),
        ("nul.txt", PathError::Device),
        ("com1", PathError::Device),
        ("Song.", PathError::Trailing),
        ("Song ", PathError::Trailing),
        ("a*b", PathError::Forbidden),
        ("a?b", PathError::Forbidden),
        ("a<b", PathError::Forbidden),
        ("a>b", PathError::Forbidden),
        ("a|b", PathError::Forbidden),
        ("a\"b", PathError::Forbidden),
        ("a\u{0}b", PathError::Control),
        ("a\nb", PathError::Control),
        (long.as_str(), PathError::TooLong),
    ];
    for (name, error) in cases {
        assert_eq!(export_path(name).unwrap_err(), *error, "{name:?}");
    }
    assert_eq!(export_path("Song").unwrap(), "exports/Song.wav");
    assert_eq!(
        export_path("Été — démo (v2)").unwrap(),
        "exports/Été — démo (v2).wav"
    );
}

#[test]
fn a_pending_write_dropped_mid_write_leaves_no_file_and_no_temporary_file() {
    let s = setup("pending-drop");
    let mut pending = s.folder.begin("exports/Song.wav").unwrap();
    pending.write(b"half a file").unwrap();
    let tmp = s.root.join(".faulk").join("tmp");
    assert_eq!(fs::read_dir(&tmp).unwrap().count(), 1, "the temporary file");
    drop(pending);
    assert_eq!(fs::read_dir(&tmp).unwrap().count(), 0);
    assert!(!s.root.join("exports").join("Song.wav").exists());
}

#[test]
fn a_stream_write_puts_the_header_at_the_start_and_lands_whole() {
    let s = setup("pending-stream");
    let mut pending = s.folder.begin("exports/Song.wav").unwrap();
    pending.write(&[0; 4]).unwrap();
    pending.write(b"body").unwrap();
    pending.write_at_start(b"HEAD").unwrap();
    pending.write(b"tail").unwrap();
    assert_eq!(
        pending.commit(Collision::Unique).unwrap(),
        "exports/Song.wav"
    );
    assert_eq!(
        fs::read(s.root.join("exports").join("Song.wav")).unwrap(),
        b"HEADbodytail"
    );
    assert_eq!(
        fs::read_dir(s.root.join(".faulk").join("tmp"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn a_unique_commit_never_replaces_a_file_and_takes_the_next_free_name() {
    let s = setup("unique");
    let exports = s.root.join("exports");
    fs::create_dir_all(&exports).unwrap();
    fs::write(exports.join("Song.wav"), b"the user's").unwrap();
    // A folder at the second name counts as taken too.
    fs::create_dir_all(exports.join("Song (2).wav")).unwrap();
    for expected in ["exports/Song (3).wav", "exports/Song (4).wav"] {
        let mut pending = s.folder.begin("exports/Song.wav").unwrap();
        pending.write(b"new").unwrap();
        assert_eq!(pending.commit(Collision::Unique).unwrap(), expected);
    }
    assert_eq!(fs::read(exports.join("Song.wav")).unwrap(), b"the user's");
    assert_eq!(fs::read(exports.join("Song (3).wav")).unwrap(), b"new");
    assert_eq!(
        fs::read_dir(s.root.join(".faulk").join("tmp"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn a_file_made_between_the_start_and_the_commit_is_kept() {
    let s = setup("unique-race");
    let mut pending = s.folder.begin("exports/Song.wav").unwrap();
    pending.write(b"ours").unwrap();
    // Another writer (a sync client) makes the file while the export renders.
    fs::write(s.root.join("exports").join("Song.wav"), b"theirs").unwrap();
    assert_eq!(
        pending.commit(Collision::Unique).unwrap(),
        "exports/Song (2).wav"
    );
    assert_eq!(
        fs::read(s.root.join("exports").join("Song.wav")).unwrap(),
        b"theirs"
    );
}

#[test]
fn the_no_replace_move_refuses_an_existing_target() {
    let temp = Temp::new("noreplace");
    let (from, to) = (temp.path().join("a"), temp.path().join("b"));
    fs::write(&from, b"a").unwrap();
    fs::write(&to, b"b").unwrap();
    assert_eq!(
        move_no_replace(&from, &to).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&to).unwrap(), b"b");
    assert_eq!(fs::read(&from).unwrap(), b"a");
    fs::remove_file(&to).unwrap();
    move_no_replace(&from, &to).unwrap();
    assert!(!from.exists());
    assert_eq!(fs::read(&to).unwrap(), b"a");
}

#[test]
fn a_folder_replaced_between_the_start_and_the_commit_of_a_stream_gets_nothing() {
    let s = setup("pending-swap");
    let parent = s.root.parent().unwrap().to_path_buf();
    let moved = parent.join("Song-moved");
    let mut pending = s.folder.begin("exports/Song.wav").unwrap();
    pending.write(b"attack").unwrap();
    // Closed (as at the end of a render: Windows moves no folder holding an open file), then
    // the folder is swapped before the move.
    pending.finish().unwrap();
    fs::rename(&s.root, &moved).unwrap();
    project_dir(&parent, "Song", OTHER);
    assert_eq!(
        pending.commit(Collision::Unique).unwrap_err(),
        PathError::FolderChanged
    );
    assert!(!s.root.join("exports").join("Song.wav").exists());
    assert!(!moved.join("exports").join("Song.wav").exists());
}

#[test]
fn an_exports_folder_leading_out_or_to_the_blobs_is_refused_for_a_stream() {
    let s = setup("exports-out");
    dir_link(&s.outside, &s.root.join("exports"));
    let before = s.snapshot();
    assert_eq!(
        s.folder.begin("exports/Song.wav").unwrap_err(),
        PathError::Escapes
    );
    assert_eq!(s.snapshot(), before);
    let s = setup("exports-blobs");
    fs::create_dir_all(s.root.join("blobs")).unwrap();
    dir_link(&s.root.join("blobs"), &s.root.join("exports"));
    let before = s.snapshot();
    assert_eq!(
        s.folder.begin("exports/Song.wav").unwrap_err(),
        PathError::Reserved
    );
    assert_eq!(s.snapshot(), before);
}

#[test]
fn the_helper_removes_only_a_plain_file_it_names_outside_the_reserved_zones() {
    let s = setup("remove-own");
    s.folder.write("exports/Song.wav", b"x").unwrap();
    s.folder.remove_own("exports/Song.wav").unwrap();
    assert!(!s.root.join("exports").join("Song.wav").exists());
    assert_eq!(
        s.folder.remove_own("project.json").unwrap_err(),
        PathError::Reserved
    );
    assert_eq!(
        s.folder.remove_own("../outside/canary.txt").unwrap_err(),
        PathError::Dot
    );
    dir_link(&s.outside, &s.root.join("out"));
    assert_eq!(
        s.folder.remove_own("out/canary.txt").unwrap_err(),
        PathError::Escapes
    );
    assert_eq!(fs::read(s.outside.join("canary.txt")).unwrap(), b"canary");
}
