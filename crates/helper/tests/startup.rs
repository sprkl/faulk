//! The helper binary's start order (#515): the updater starts before anything that can stop
//! the start, so that a helper that cannot open its audio output still checks for an update.
//! Runs the debug binary with its config folder in a temporary directory
//! (`FAULK_HELPER_CONFIG_DIR`) and no audio output (`FAULK_HELPER_TEST_NO_AUDIO`), both read
//! by debug builds only.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_helper_without_audio_output_starts_its_updater_before_it_stops() {
    let config = std::env::temp_dir().join(format!("faulk-startup-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&config);
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_faulk-helper"))
        .args(["--port", &port.to_string()])
        .env(helper::CONFIG_DIR_ENV, &config)
        .env("FAULK_HELPER_TEST_NO_AUDIO", "1")
        .env_remove("FAULK_UPDATE_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("the helper did not stop");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stderr = String::new();
    use std::io::Read;
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let journal = std::fs::read_to_string(config.join("logs").join("helper.log")).unwrap();
    let _ = std::fs::remove_dir_all(&config);

    assert_eq!(status.code(), Some(1), "{stderr}");
    for (name, text) in [("stderr", &stderr), ("journal", &journal)] {
        // Run from cargo, the updater is disabled: its line proves it was started.
        let update = text
            .find("update: disabled (")
            .unwrap_or_else(|| panic!("no update line in the {name}:\n{text}"));
        let fatal = text
            .find("no audio output device")
            .unwrap_or_else(|| panic!("no fatal line in the {name}:\n{text}"));
        assert!(
            update < fatal,
            "the updater starts before the audio output ({name}):\n{text}"
        );
    }
}

/// A failed start frees its port before its error dialog (stood for by a pause in a debug
/// build) and before it waits for the update check: a helper launched meanwhile finds the port
/// free (#515: a helper stuck behind its dialog held port 47800).
#[test]
fn a_failed_start_frees_its_port_while_its_error_is_shown() {
    let config =
        std::env::temp_dir().join(format!("faulk-startup-test-pause-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&config);
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_faulk-helper"))
        .args(["--port", &port.to_string()])
        .env(helper::CONFIG_DIR_ENV, &config)
        .env("FAULK_HELPER_TEST_NO_AUDIO", "1")
        .env("FAULK_HELPER_TEST_FATAL_PAUSE_MS", "20000")
        .env_remove("FAULK_UPDATE_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let journal = config.join("logs").join("helper.log");
    let start = Instant::now();
    while !std::fs::read_to_string(&journal).is_ok_and(|t| t.contains("no audio output device")) {
        assert!(
            child.try_wait().unwrap().is_none(),
            "the helper stopped early"
        );
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("no fatal line");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // The fatal line is written before the port is freed: give the drop a moment.
    let mut bound = None;
    while bound.is_none() && start.elapsed() < Duration::from_secs(60) {
        bound = std::net::TcpListener::bind(("127.0.0.1", port)).ok();
        if bound.is_none() {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    // Its instance lock is let go too: a start now would be alone and could apply an update.
    let mut alone = false;
    while !alone && start.elapsed() < Duration::from_secs(60) {
        alone = helper::updater::InstanceLock::take(
            &config.join("helper.lock"),
            Duration::from_millis(200),
        )
        .is_ok_and(|lock| lock.alone());
        if !alone {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let still_up = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&config);
    assert!(bound.is_some(), "the port is free while the error is shown");
    assert!(alone, "the instance lock is free while the error is shown");
    assert!(
        still_up,
        "checked while the helper still showed its error (pause of 20 s)"
    );
}
