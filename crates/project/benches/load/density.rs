//! Density sweep: one automation lane of N points, as a text file (TSV) and as a packed blob,
//! for N from 100 to 1 000 000. What the writer, the reader and git pay per layout, to find
//! the density where the blob starts to be worth its costs.

use crate::folder::*;
use crate::stats::*;
use crate::synth::{Rng, SEED, lane_points};
use project::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, cfg: &Path, args: &[&str]) -> (String, f64) {
    let t = std::time::Instant::now();
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", cfg)
        .env("GIT_AUTHOR_NAME", "bench")
        .env("GIT_AUTHOR_EMAIL", "bench@example.invalid")
        .env("GIT_COMMITTER_NAME", "bench")
        .env("GIT_COMMITTER_EMAIL", "bench@example.invalid")
        .args(["-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .expect("git");
    let d = ms(t.elapsed());
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (String::from_utf8_lossy(&out.stdout).into_owned(), d)
}

/// `size` (loose, KiB) and `size-pack` (KiB) of `git count-objects -v`.
fn object_sizes(dir: &Path, cfg: &Path) -> (u64, u64) {
    let (out, _) = git(dir, cfg, &["count-objects", "-v"]);
    let field = |k: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}: ")))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    };
    (field("size"), field("size-pack"))
}

pub fn run(args: &[String]) {
    let base = PathBuf::from(
        args.iter()
            .position(|a| a == "--dir")
            .and_then(|i| args.get(i + 1))
            .expect("--dir <scratch folder>"),
    );
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let cfg = base.join("empty.gitconfig");
    std::fs::write(&cfg, "").unwrap();
    let target = AutomationTarget::Device {
        device: DeviceId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").unwrap(),
        param: "sustain".into(),
    };
    println!(
        "one lane of N points (a recorded gesture: a tick every 5 to 35, values to 0.001), TSV against packed blob (17 bytes a point)"
    );
    println!(
        "N | TSV KiB | blob KiB | TSV serialise ms | blob serialise+sha ms | TSV parse ms (memory) | blob parse ms (memory) | TSV read+parse from disk ms | blob read+parse from disk ms | save 1 point TSV ms | save 1 point blob ms"
    );
    let mut detail = Vec::new();
    for n in [100usize, 1_000, 10_000, 100_000, 1_000_000] {
        let runs = if n <= 10_000 {
            25
        } else if n <= 100_000 {
            9
        } else {
            5
        };
        let mut rng = Rng(SEED ^ n as u64);
        let mut points = lane_points(&mut rng, n, &target);
        let tsv = points_tsv(&target, &points);
        let blob = points_blob(&points);
        let t_ser = series(runs, || {
            std::hint::black_box(points_tsv(&target, &points));
        });
        let b_ser = series(runs, || {
            let b = points_blob(&points);
            std::hint::black_box(Sha256::digest(&b));
        });
        // Parse from memory: through the folder reader's lane parser, via a one-track MemFiles
        // would add noise, so call the same functions the reader calls.
        let t_par = series(runs, || {
            std::hint::black_box(parse_lane_text(&tsv));
        });
        let b_par = series(runs, || {
            std::hint::black_box(parse_lane_blob(&blob));
        });
        let dir = base.join(format!("lane-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        let tsv_path = dir.join("lane.tsv");
        let blob_path = dir.join("lane.blob");
        atomic_write(&tsv_path, &tsv, false).unwrap();
        atomic_write(&blob_path, &blob, false).unwrap();
        let t_disk = series(runs, || {
            let b = std::fs::read(&tsv_path).unwrap();
            std::hint::black_box(parse_lane_text(&b));
        });
        let b_disk = series(runs, || {
            let b = std::fs::read(&blob_path).unwrap();
            std::hint::black_box(parse_lane_blob(&b));
        });
        // Save after one point changed.
        let mid = *points.keys().nth(n / 2).unwrap();
        let mut i = 0;
        let t_save = series(runs, || {
            i += 1;
            points.get_mut(&mid).unwrap().value = (i % 9) as f64 / 10.0;
            let bytes = points_tsv(&target, &points);
            atomic_write(&tsv_path, &bytes, false).unwrap();
        });
        let b_save = series(runs, || {
            i += 1;
            points.get_mut(&mid).unwrap().value = (i % 9) as f64 / 10.0;
            let bytes = points_blob(&points);
            let name = Hash(Sha256::digest(&bytes).into()).to_string();
            let p = dir.join(&name);
            atomic_write(&p, &bytes, false).unwrap();
            // The header that names it.
            atomic_write(&dir.join("lane.header"), name.as_bytes(), false).unwrap();
        });
        println!(
            "{n} | {:.1} | {:.1} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2}",
            tsv.len() as f64 / 1024.0,
            blob.len() as f64 / 1024.0,
            median(&t_ser),
            median(&b_ser),
            median(&t_par),
            median(&b_par),
            median(&t_disk),
            median(&b_disk),
            median(&t_save),
            median(&b_save)
        );
        detail.push(n);
    }

    println!();
    println!(
        "git, one lane file, then one point changed (plain git objects for both; Git LFS would keep a full copy of the blob for each version)"
    );
    println!(
        "N | layout | commit 1 ms | commit 2 (one point) ms | diff ms | diff lines | loose KiB after 2 commits | pack KiB after gc"
    );
    for n in [100usize, 1_000, 10_000, 100_000, 1_000_000] {
        for layout in ["tsv", "blob"] {
            let repo = base.join(format!("git-{n}-{layout}"));
            std::fs::create_dir_all(&repo).unwrap();
            git(&repo, &cfg, &["init", "-q", "-b", "main"]);
            let mut rng = Rng(SEED ^ n as u64);
            let mut points = lane_points(&mut rng, n, &target);
            let mid = *points.keys().nth(n / 2).unwrap();
            let write = |points: &std::collections::BTreeMap<Tick, AutomationPoint>| {
                let bytes = if layout == "tsv" {
                    points_tsv(&target, points)
                } else {
                    points_blob(points)
                };
                std::fs::write(repo.join("lane"), bytes).unwrap();
            };
            write(&points);
            git(&repo, &cfg, &["add", "-A"]);
            let (_, c1) = git(&repo, &cfg, &["commit", "-qm", "v1"]);
            points.get_mut(&mid).unwrap().value += 0.001;
            write(&points);
            let (diff, d_diff) = git(&repo, &cfg, &["diff"]);
            git(&repo, &cfg, &["add", "-A"]);
            let (_, c2) = git(&repo, &cfg, &["commit", "-qm", "v2"]);
            let (loose, _) = object_sizes(&repo, &cfg);
            git(&repo, &cfg, &["gc", "-q"]);
            let (_, pack) = object_sizes(&repo, &cfg);
            let deltas = |repo: &Path| {
                let idx = std::fs::read_dir(repo.join(".git/objects/pack"))
                    .unwrap()
                    .filter_map(Result::ok)
                    .find(|e| e.path().extension().is_some_and(|x| x == "idx"))
                    .map(|e| e.path());
                idx.map(|idx| {
                    let (out, _) = git(repo, &cfg, &["verify-pack", "-v", idx.to_str().unwrap()]);
                    // A delta line has the chain depth and the base after the offset.
                    out.lines()
                        .filter(|l| l.split_whitespace().count() >= 7)
                        .count()
                })
                .unwrap_or(0)
            };
            let after_gc = deltas(&repo);
            git(
                &repo,
                &cfg,
                &["repack", "-a", "-d", "-f", "-q", "--window=50"],
            );
            let (_, forced) = object_sizes(&repo, &cfg);
            println!(
                "{n} | {layout} | {c1:.0} | {c2:.0} | {d_diff:.0} | {} | {loose} | {pack} ({after_gc} delta objects) | {forced} after repack -f",
                if layout == "tsv" {
                    diff.lines().count().to_string()
                } else {
                    "binary".into()
                }
            );
        }
    }
    let _ = std::fs::remove_dir_all(&base);
}
