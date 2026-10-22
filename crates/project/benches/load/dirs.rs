//! Blobs directory fan-out: how a flat `blobs/` folder behaves as it fills, against one
//! level of 256 sub-folders named by the first byte of the hash (`blobs/ab/abcdef...`),
//! the way git names its objects. Native file system only; the browser's is in `web/tests/bench`.

use crate::stats::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn name(i: u64) -> String {
    let h = Sha256::digest(i.to_le_bytes());
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn path(root: &Path, name: &str, fan: bool) -> PathBuf {
    if fan {
        root.join(&name[..2]).join(name)
    } else {
        root.join(name)
    }
}

fn count(root: &Path, fan: bool) -> usize {
    if fan {
        std::fs::read_dir(root)
            .unwrap()
            .map(|d| std::fs::read_dir(d.unwrap().path()).unwrap().count())
            .sum()
    } else {
        std::fs::read_dir(root).unwrap().count()
    }
}

pub fn run(args: &[String]) {
    let base = PathBuf::from(
        args.iter()
            .position(|a| a == "--dir")
            .and_then(|i| args.get(i + 1))
            .expect("--dir <scratch folder>"),
    );
    println!(
        "blobs/ directory, files of 64 bytes named by a SHA-256, native file system (warm cache)"
    );
    println!(
        "layout | files | create total ms | create µs/file | list ms | 1000 random lookups ms | delete ms"
    );
    for n in [1_000u64, 10_000, 100_000] {
        for fan in [false, true] {
            let root = base.join(format!("dirs-{n}-{fan}"));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let names: Vec<String> = (0..n).map(name).collect();
            if fan {
                for b in 0..256 {
                    std::fs::create_dir_all(root.join(format!("{b:02x}"))).unwrap();
                }
            }
            let payload = [7u8; 64];
            let ((), d_create) = time(|| {
                for nm in &names {
                    std::fs::write(path(&root, nm, fan), payload).unwrap();
                }
            });
            let (listed, d_list) = time(|| count(&root, fan));
            assert_eq!(listed, n as usize);
            let ((), d_look) = time(|| {
                for i in 0..1000u64 {
                    let nm = &names[((i * 7919) % n) as usize];
                    let bytes = std::fs::read(path(&root, nm, fan)).unwrap();
                    assert_eq!(bytes.len(), 64);
                }
            });
            let ((), d_del) = time(|| std::fs::remove_dir_all(&root).unwrap());
            println!(
                "{} | {} | {:.0} | {:.1} | {:.1} | {:.1} | {:.0}",
                if fan { "256 sub-folders" } else { "flat" },
                n,
                ms(d_create),
                us(d_create) / n as f64,
                ms(d_list),
                ms(d_look),
                ms(d_del)
            );
        }
    }
}
