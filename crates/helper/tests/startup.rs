//! The helper binary's start. The updater starts before anything that can stop the start
//! (#515, #531); a helper without an audio output starts anyway and serves the page (#525); a
//! start that does fail frees its port while its error is shown (#515).
//! Runs the debug binary with its config folder in a temporary directory
//! (`FAULK_HELPER_CONFIG_DIR`), no audio output (`FAULK_HELPER_TEST_NO_AUDIO`) and no tray icon
//! (`FAULK_HELPER_TEST_NO_TRAY`), all read by debug builds only. Each test kills only the helper
//! it started.

use futures_util::StreamExt;
use protocol::ServerMessage;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// An origin the helper allows by default (the dev server).
const ORIGIN: &str = "http://localhost:5173";

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn temp_config(name: &str) -> PathBuf {
    let config =
        std::env::temp_dir().join(format!("faulk-startup-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&config);
    config
}

fn helper(config: &Path, port: u16) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_faulk-helper"));
    command
        .args(["--port", &port.to_string()])
        .env(helper::CONFIG_DIR_ENV, config)
        .env("FAULK_HELPER_TEST_NO_AUDIO", "1")
        .env("FAULK_HELPER_TEST_NO_TRAY", "1")
        .env_remove("FAULK_UPDATE_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Waits up to 60 s for the journal to hold `line`; fails if the helper stops meanwhile.
fn wait_for_line(child: &mut Child, journal: &Path, line: &str) -> String {
    let start = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(journal)
            && text.contains(line)
        {
            return text;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the helper stopped ({status}) before writing {line:?}");
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("no {line:?} in the journal");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The first message the helper sends on `path`, as the page would see it.
async fn first_message(port: u16, path: &str) -> ServerMessage {
    let mut request = format!("ws://127.0.0.1:{port}{path}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", ORIGIN.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let message = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("an answer within 10 s")
        .expect("a frame")
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

/// #525: no audio output is no reason to stop. The helper starts its updater first (#531's
/// order, as the journal shows), serves the page, says it has no output, refuses a session with
/// `audio.no_output`, and keeps running.
#[test]
fn a_helper_without_audio_output_starts_and_serves_the_page() {
    let config = temp_config("no-output");
    let port = free_port();
    let mut child = helper(&config, port).spawn().unwrap();
    let journal = config.join("logs").join("helper.log");
    let text = wait_for_line(&mut child, &journal, "no audio output device");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let probe = runtime.block_on(first_message(port, "/probe"));
    let token = std::fs::read_to_string(config.join("session-token")).unwrap();
    let session = runtime.block_on(first_message(
        port,
        &format!("/session?token={}", token.trim()),
    ));
    let still_up = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&config);

    // Run from cargo, the updater is disabled: its line proves it was started.
    let update = text
        .find("update: disabled (")
        .unwrap_or_else(|| panic!("no update line in the journal:\n{text}"));
    let output = text.find("no audio output device").unwrap();
    assert!(
        update < output,
        "the updater starts before the audio output:\n{text}"
    );
    assert!(
        matches!(
            probe,
            ServerMessage::Probe {
                audio_output: false,
                ..
            }
        ),
        "{probe:?}"
    );
    assert!(
        matches!(&session, ServerMessage::Error { code, .. } if code == helper::server::NO_OUTPUT),
        "{session:?}"
    );
    assert!(still_up, "the helper keeps running without an output");
}

/// A failed start frees its port before its error dialog (stood for by a pause in a debug
/// build) and before it waits for the update check: a helper launched meanwhile finds the port
/// free (#515: a helper stuck behind its dialog held port 47800). The failure here: the session
/// token cannot be stored (a folder stands where its file goes), which happens after the port
/// is claimed and the updater started.
#[test]
fn a_failed_start_frees_its_port_while_its_error_is_shown() {
    let config = temp_config("pause");
    std::fs::create_dir_all(config.join("session-token")).unwrap();
    let port = free_port();
    let mut child = helper(&config, port)
        .env("FAULK_HELPER_TEST_FATAL_PAUSE_MS", "20000")
        .spawn()
        .unwrap();
    let journal = config.join("logs").join("helper.log");
    wait_for_line(&mut child, &journal, "cannot store the session token");
    let start = Instant::now();
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
