//! The `faulk` binary: new, run, render, and its exit codes.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use headless::host::Host;
use serde_json::Value;

use crate::song::{scratch, track_commands};

fn faulk(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_faulk"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("faulk starts");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(input.as_bytes())
            .expect("written");
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().expect("faulk ends")
}

fn lines(out: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect()
}

#[test]
fn new_run_save_and_render_from_the_command_line() {
    let root = scratch("cli");
    let dir = root.to_str().expect("utf-8");
    let made = faulk(&["new", dir, "--name", "Cli song"], None);
    assert!(made.status.success(), "{made:?}");
    assert_eq!(lines(&made)[0]["name"], "Cli song");

    let script: String = track_commands()
        .iter()
        .map(|c| format!("{c}\n"))
        .chain(["# a comment\n\n".to_owned()])
        .collect();
    let ran = faulk(
        &["run", dir, "-", "--save", "--author", "script"],
        Some(&script),
    );
    assert!(ran.status.success(), "{ran:?}");
    let answers = lines(&ran);
    assert_eq!(answers.len(), 2);
    assert!(
        answers.iter().all(|a| a["status"] == "applied"),
        "{answers:?}"
    );
    assert_eq!(
        Host::open(&root)
            .expect("opens")
            .view()
            .project
            .tracks
            .len(),
        2
    );

    // No clip: the song is empty, so there is nothing to render.
    let render = faulk(&["render", dir, "-o", &format!("{dir}/x.wav")], None);
    assert_eq!(render.status.code(), Some(1), "{render:?}");
}

#[test]
fn a_run_stops_at_the_first_command_not_applied_unless_told_to_go_on() {
    let root = scratch("cli-stop");
    let dir = root.to_str().expect("utf-8");
    assert!(faulk(&["new", dir], None).status.success());
    let first = &track_commands()[0];
    let script = format!("{{\"type\":\"transport.play\"}}\nnot json\n{first}\n");

    let stopped = faulk(&["run", dir, "-", "--save"], Some(&script));
    assert_eq!(stopped.status.code(), Some(1));
    let answers = lines(&stopped);
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert_eq!(answers[0]["code"], "headless.unsupported");
    assert!(
        Host::open(&root)
            .expect("opens")
            .view()
            .project
            .tracks
            .is_empty()
    );

    let went_on = faulk(&["run", dir, "-", "--save", "--keep-going"], Some(&script));
    assert_eq!(went_on.status.code(), Some(1));
    let answers = lines(&went_on);
    assert_eq!(
        answers
            .iter()
            .map(|a| a["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["unsupported", "invalid", "applied"]
    );
    assert_eq!(
        Host::open(&root)
            .expect("opens")
            .view()
            .project
            .tracks
            .len(),
        1
    );
}

#[test]
fn a_bad_command_line_is_a_usage_error() {
    for args in [
        vec![],
        vec!["nope"],
        vec!["new"],
        vec!["render", "x", "--format", "mp3"],
        vec!["run", "x"],
        vec!["mcp", "x"],
        vec!["new", "x", "--bogus", "1"],
    ] {
        let out = faulk(&args, None);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
    }
}
