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

/// Where the helper's stdout and stderr go (beside the config folder, which the test removes
/// last), so that a failure shows them (#895: they were thrown away).
fn output_file(config: &Path, stream: &str) -> PathBuf {
    let mut name = config.file_name().unwrap().to_os_string();
    name.push(format!(".{stream}"));
    config.with_file_name(name)
}

/// What the helper printed, for a panic message.
fn printed(config: &Path) -> String {
    ["stdout", "stderr"]
        .iter()
        .map(|stream| {
            let text = std::fs::read_to_string(output_file(config, stream)).unwrap_or_default();
            format!("--- helper {stream} ---\n{text}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The journal and what the helper printed, for a failure's message: read before the folders go.
fn diagnostics(config: &Path) -> String {
    let journal = std::fs::read_to_string(config.join("logs").join("helper.log"))
        .unwrap_or_else(|e| format!("(no journal: {e})"));
    format!("journal:\n{journal}\n{}", printed(config))
}

/// The helper a test started: whatever way the test ends (an assert, a panic), the helper is
/// killed and its folder and output files removed.
struct Run {
    child: Child,
    config: PathBuf,
}

impl Run {
    fn start(command: &mut Command, config: &Path) -> Run {
        Run {
            child: command.spawn().unwrap(),
            config: config.to_path_buf(),
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.config);
        for stream in ["stdout", "stderr"] {
            let _ = std::fs::remove_file(output_file(&self.config, stream));
        }
    }
}

/// The port the helper holds, read from its journal: it is started on port 0, so that the
/// system gives it one that no other test can take between a lookup and the helper's bind
/// (#895: a port found free then released could be taken by a parallel test).
fn claimed_port(text: &str) -> u16 {
    let line = text
        .lines()
        .find_map(|line| line.split("claimed 127.0.0.1:").nth(1))
        .unwrap_or_else(|| panic!("no claimed port in the journal:\n{text}"));
    line.trim().parse().unwrap()
}

fn temp_config(name: &str) -> PathBuf {
    let config =
        std::env::temp_dir().join(format!("faulk-startup-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&config);
    config
}

fn helper(config: &Path) -> Command {
    let file = |stream| std::fs::File::create(output_file(config, stream)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_faulk-helper"));
    command
        .args(["--port", "0"])
        .env(helper::CONFIG_DIR_ENV, config)
        .env("FAULK_HELPER_TEST_NO_AUDIO", "1")
        .env("FAULK_HELPER_TEST_NO_TRAY", "1")
        .env_remove("FAULK_UPDATE_URL")
        .stdin(Stdio::null())
        .stdout(file("stdout"))
        .stderr(file("stderr"));
    command
}

/// Waits up to 60 s for the journal to hold `line`; fails if the helper stops meanwhile.
fn wait_for_line(child: &mut Child, config: &Path, line: &str) -> String {
    let journal = config.join("logs").join("helper.log");
    let start = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(&journal)
            && text.contains(line)
        {
            return text;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "the helper stopped ({status}) before writing {line:?}\n{}",
                diagnostics(config)
            );
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("no {line:?} in the journal\n{}", diagnostics(config));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The first message the helper sends on `path`, as the page would see it.
/// An error says what failed; the caller adds what the helper printed.
async fn first_message(port: u16, path: &str) -> Result<ServerMessage, String> {
    let mut request = format!("ws://127.0.0.1:{port}{path}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", ORIGIN.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| format!("cannot connect to {path}: {e}"))?;
    let message = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .map_err(|_| format!("no answer within 10 s on {path}"))?
        .ok_or_else(|| format!("{path} closed without a frame"))?
        .map_err(|e| format!("{path} failed: {e}"))?;
    serde_json::from_str(message.to_text().map_err(|e| e.to_string())?)
        .map_err(|e| format!("unreadable answer on {path}: {e}"))
}

/// #525: no audio output is no reason to stop. The helper starts its updater first (#531's
/// order, as the journal shows), serves the page, says it has no output, refuses a session with
/// `audio.no_output`, and keeps running.
#[test]
fn a_helper_without_audio_output_starts_and_serves_the_page() {
    let config = temp_config("no-output");
    let mut run = Run::start(&mut helper(&config), &config);
    let text = wait_for_line(&mut run.child, &config, "no audio output device");
    let port = claimed_port(&text);

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let probe = runtime
        .block_on(first_message(port, "/probe"))
        .unwrap_or_else(|e| panic!("{e}\n{}", diagnostics(&config)));
    let token = std::fs::read_to_string(config.join("session-token")).unwrap();
    let session = runtime
        .block_on(first_message(
            port,
            &format!("/session?token={}", token.trim()),
        ))
        .unwrap_or_else(|e| panic!("{e}\n{}", diagnostics(&config)));
    let still_up = run.child.try_wait().unwrap().is_none();
    let why = diagnostics(&config);

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
        "{probe:?}\n{why}"
    );
    assert!(
        matches!(&session, ServerMessage::Error { code, .. } if code == helper::server::NO_OUTPUT),
        "{session:?}\n{why}"
    );
    assert!(
        still_up,
        "the helper keeps running without an output\n{why}"
    );
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
    let mut run = Run::start(
        helper(&config).env("FAULK_HELPER_TEST_FATAL_PAUSE_MS", "20000"),
        &config,
    );
    let text = wait_for_line(&mut run.child, &config, "cannot store the session token");
    let port = claimed_port(&text);
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
    let still_up = run.child.try_wait().unwrap().is_none();
    let why = diagnostics(&config);
    assert!(
        bound.is_some(),
        "the port is free while the error is shown\n{why}"
    );
    assert!(
        alone,
        "the instance lock is free while the error is shown\n{why}"
    );
    assert!(
        still_up,
        "checked while the helper still showed its error (pause of 20 s)\n{why}"
    );
}
