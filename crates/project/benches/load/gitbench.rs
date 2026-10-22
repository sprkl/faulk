//! git on the project folder: status, commit, diff and merge, with the blobs under Git LFS
//! (the layout of `docs/architecture/02-modele-projet.md` § 3.2) or, for contrast, as plain
//! git objects. The system `git` and `git lfs` run as a user would run them; the user's own
//! git configuration is left out (an empty global file, no system file) so that the numbers
//! do not depend on it.

use crate::folder::*;
use crate::stats::*;
use project::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Git {
    dir: PathBuf,
    empty_config: PathBuf,
}

impl Git {
    fn run(&self, args: &[&str]) -> (Output, Duration) {
        let t = Instant::now();
        let out = Command::new("git")
            .current_dir(&self.dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.empty_config)
            .env("GIT_AUTHOR_NAME", "bench")
            .env("GIT_AUTHOR_EMAIL", "bench@example.invalid")
            .env("GIT_COMMITTER_NAME", "bench")
            .env("GIT_COMMITTER_EMAIL", "bench@example.invalid")
            .args(["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("git runs");
        (out, t.elapsed())
    }

    /// Runs a command that must succeed; returns its stdout and its duration.
    fn ok(&self, args: &[&str]) -> (String, Duration) {
        let (out, d) = self.run(args);
        assert!(
            out.status.success(),
            "git {args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (String::from_utf8_lossy(&out.stdout).into_owned(), d)
    }
}

fn dir_size(path: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    rd.filter_map(Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

pub fn run(args: &[String]) {
    let get = |key: &str| {
        let flag = format!("--{key}");
        args.iter()
            .position(|a| *a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let dir = PathBuf::from(get("dir").expect("--dir <project folder>"));
    let lfs = get("lfs").as_deref() != Some("0");
    let layout = match get("blob-above") {
        Some(n) => Layout::blob_above(n.parse().unwrap()),
        None => Layout::TEXT,
    };
    let empty_config = dir.with_extension("gitconfig");
    std::fs::write(&empty_config, "").unwrap();
    let _ = std::fs::remove_dir_all(dir.join(".git"));
    let git = Git {
        dir: dir.clone(),
        empty_config,
    };
    let (project, _) = read_project(&FsFiles(dir.clone()), 8).expect("read");
    let blob_bytes = dir_size(&dir.join("blobs"));
    println!(
        "git {}, audio blobs {:.0} MiB {}",
        git.ok(&["--version"]).0.trim(),
        mib(blob_bytes),
        if lfs {
            "under Git LFS"
        } else {
            "as plain git objects"
        }
    );
    git.ok(&["init", "-q", "-b", "main"]);
    if lfs {
        git.ok(&["lfs", "install", "--local"]);
    }
    let mut attrs = String::new();
    if lfs {
        attrs.push_str("blobs/** filter=lfs diff=lfs merge=lfs -text\n");
    } else {
        attrs.push_str("blobs/** -text\n");
    }
    attrs.push_str(
        "*.json text eol=lf merge=faulk diff=faulk\n*.tsv text eol=lf merge=faulk diff=faulk\n",
    );
    std::fs::write(dir.join(".gitattributes"), attrs).unwrap();
    std::fs::write(dir.join(".gitignore"), ".faulk/\n").unwrap();

    let (_, d_add) = git.ok(&["add", "-A"]);
    println!("git add -A (first time, all files): {:.0} ms", ms(d_add));
    let (_, d_commit) = git.ok(&["commit", "-q", "-m", "first version"]);
    println!("git commit (first version): {:.0} ms", ms(d_commit));
    let files = git.ok(&["ls-files"]).0.lines().count();
    println!("tracked files: {files}");
    let git_dir = dir.join(".git");
    println!(
        ".git after the first commit: {:.1} MiB (of which LFS objects {:.1} MiB)",
        mib(dir_size(&git_dir)),
        mib(dir_size(&git_dir.join("lfs")))
    );

    let mut times = Vec::new();
    for _ in 0..5 {
        times.push(ms(git.ok(&["status", "--porcelain"]).1));
    }
    println!("git status, nothing changed: {}", describe(&times, "ms"));

    // One note changed.
    let mut project = project;
    let (tid, cid) = first_midi_clip(&project).expect("a clip");
    change_note(&mut project, &tid, &cid, 77);
    save_clip_notes(&project, &dir, &tid, &cid, false).unwrap();
    let mut times = Vec::new();
    for _ in 0..5 {
        times.push(ms(git.ok(&["status", "--porcelain"]).1));
    }
    println!("git status, one note changed: {}", describe(&times, "ms"));
    let (diff, d_diff) = git.ok(&["diff"]);
    println!(
        "git diff, one note changed: {:.0} ms, {} lines of output",
        ms(d_diff),
        diff.lines().count()
    );
    for l in diff
        .lines()
        .filter(|l| l.starts_with('+') || l.starts_with('-'))
        .take(4)
    {
        println!("  {l}");
    }
    let (_, d) = git.ok(&["add", "-A"]);
    println!("git add -A, one note changed: {:.0} ms", ms(d));
    let (_, d) = git.ok(&["commit", "-q", "-m", "one note"]);
    println!("git commit, one note changed: {:.0} ms", ms(d));
    let (_, d_show) = git.ok(&["diff", "--stat", "HEAD~1", "HEAD"]);
    println!(
        "git diff --stat between the two versions: {:.0} ms",
        ms(d_show)
    );

    // One lane point changed (the lane's text file is rewritten: a 7 143-line file).
    let (ltid, lid) = project
        .tracks
        .iter()
        .find_map(|(t, tr)| tr.automation.keys().next().map(|l| (t.clone(), l.clone())))
        .expect("a lane");
    {
        let lane = project
            .tracks
            .get_mut(&ltid)
            .unwrap()
            .automation
            .get_mut(&lid)
            .unwrap();
        let k = *lane.points.keys().nth(lane.points.len() / 2).unwrap();
        lane.points.get_mut(&k).unwrap().value += 0.001;
    }
    save_lane(&project, &dir, &ltid, &lid, layout, false).unwrap();
    let (diff, d_diff) = git.ok(&["diff"]);
    println!(
        "git diff, one automation point changed: {:.0} ms, {} lines",
        ms(d_diff),
        diff.lines().count()
    );
    git.ok(&["add", "-A"]);
    git.ok(&["commit", "-q", "-m", "one point"]);

    // Branches and merges.
    let base = git.ok(&["rev-parse", "HEAD"]).0.trim().to_owned();
    // a: another velocity on the first note of the first clip, one commit.
    git.ok(&["checkout", "-q", "-b", "a"]);
    change_note(&mut project, &tid, &cid, 55);
    save_clip_notes(&project, &dir, &tid, &cid, false).unwrap();
    git.ok(&["add", "-A"]);
    git.ok(&["commit", "-qm", "a: velocity"]);
    // b from the base: a different track gets a changed note, and the first track a new gain.
    git.ok(&["checkout", "-q", "-b", "b", &base]);
    let mut other = project.clone();
    // Back to the base state of the first clip, then edit elsewhere.
    change_note(&mut other, &tid, &cid, 77);
    let (t2, c2) = other
        .tracks
        .iter()
        .filter(|(t, _)| **t != tid)
        .find_map(|(t, tr)| {
            tr.clips
                .iter()
                .find(|(_, c)| c.content.is_midi())
                .map(|(c, _)| (t.clone(), c.clone()))
        })
        .expect("another clip");
    change_note(&mut other, &t2, &c2, 33);
    other.tracks.get_mut(&tid).unwrap().mixer.gain_db = -3.5;
    write_all(&other, &dir, layout, false, None, 1).unwrap();
    git.ok(&["add", "-A"]);
    git.ok(&["commit", "-qm", "b: elsewhere"]);
    // c from the base: the same note as a, another value.
    git.ok(&["checkout", "-q", "-b", "c", &base]);
    let mut third = project.clone();
    change_note(&mut third, &tid, &cid, 99);
    write_all(&third, &dir, layout, false, None, 1).unwrap();
    git.ok(&["add", "-A"]);
    git.ok(&["commit", "-qm", "c: same note"]);
    // d from the base: a note added in the same clip file as a (another line).
    git.ok(&["checkout", "-q", "-b", "d", &base]);
    let mut fourth = project.clone();
    change_note(&mut fourth, &tid, &cid, 77);
    if let ClipContent::Midi { notes, .. } = &mut fourth
        .tracks
        .get_mut(&tid)
        .unwrap()
        .clips
        .get_mut(&cid)
        .unwrap()
        .content
    {
        notes.insert(Note::new(Tick(60_000), 90, Tick(120), 64));
    }
    write_all(&fourth, &dir, layout, false, None, 1).unwrap();
    git.ok(&["add", "-A"]);
    git.ok(&["commit", "-qm", "d: a note added"]);

    git.ok(&["checkout", "-q", "a"]);
    let (_, d) = git.ok(&["merge", "--no-edit", "b"]);
    println!(
        "git merge b into a (other tracks, other files): {:.0} ms, no conflict",
        ms(d)
    );
    git.ok(&["reset", "-q", "--hard", "a"]);
    git.ok(&["checkout", "-q", "-B", "m", "a"]);
    let (_, d) = git.ok(&["merge", "--no-edit", "d"]);
    println!(
        "git merge d into a (same notes file, another line): {:.0} ms, no conflict",
        ms(d)
    );
    git.ok(&["checkout", "-q", "-B", "m2", "a"]);
    let (out, d) = git.run(&["merge", "--no-edit", "c"]);
    println!(
        "git merge c into a (same note changed on both sides): {:.0} ms, exit code {:?}, {}",
        ms(d),
        out.status.code(),
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .last()
            .unwrap_or("")
    );
    git.ok(&["merge", "--abort"]);

    // Storage: loose objects, then packed.
    let size_loose = dir_size(&git_dir);
    let (_, d_gc) = git.ok(&["gc", "-q"]);
    println!(
        ".git: {:.1} MiB before gc, {:.1} MiB after gc ({:.1} s), LFS objects {:.1} MiB",
        mib(size_loose),
        mib(dir_size(&git_dir)),
        d_gc.as_secs_f64(),
        mib(dir_size(&git_dir.join("lfs")))
    );
    let mut times = Vec::new();
    for _ in 0..5 {
        times.push(ms(git.ok(&["status", "--porcelain"]).1));
    }
    println!(
        "git status after gc, nothing changed: {}",
        describe(&times, "ms")
    );
    let (out, d) = git.ok(&["log", "--oneline", "--all"]);
    println!(
        "git log --all: {:.0} ms, {} commits",
        ms(d),
        out.lines().count()
    );
    let _ = std::fs::remove_dir_all(dir.join(".git"));
    let _ = std::fs::remove_file(git.empty_config);
    let _ = std::fs::remove_file(dir.join(".gitattributes"));
    let _ = std::fs::remove_file(dir.join(".gitignore"));
}
