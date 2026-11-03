//! One helper per port: what a starting helper decides when the port is free, held by a
//! Faulk helper (ready, or still starting), or held by another program.

use helper::audio::{EngineSlot, Meter, audio_core};
use helper::instance::{Startup, claim};
use helper::midi::MidiManager;
use helper::security::Policy;
use helper::server::{Shared, serve};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ORIGIN: &str = "http://localhost:5173";

fn origins(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// Binds a port now and starts a Faulk server on it after `delay`, on its own thread,
/// like a helper that is still opening its audio output.
fn faulk_helper(allowed: &str, delay: Duration) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let allowed = allowed.to_owned();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            tokio::time::sleep(delay).await;
            let meter = Arc::new(Meter::default());
            let (_core, sender) = audio_core(48_000.0, meter.clone()).unwrap();
            let engine = Arc::new(EngineSlot::default());
            engine.install(sender, 48_000.0);
            let (midi_out, midi_in) = tokio::sync::mpsc::unbounded_channel();
            let midi = MidiManager::new(midi_out);
            let shared = Arc::new(Shared::new(
                Policy {
                    port,
                    allowed_origins: vec![allowed],
                    token: "0123456789abcdef0123456789abcdef".into(),
                },
                engine,
                midi,
                midi_in,
                meter,
                Box::new(|_| {}),
            ));
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            serve(listener, shared).await;
        });
    });
    port
}

#[test]
fn a_free_port_is_served() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port(); // released at once
    assert!(matches!(
        claim(port, &origins(&[ORIGIN]), Duration::from_secs(1)),
        Startup::Serve(_)
    ));
}

#[test]
fn a_running_faulk_helper_is_found() {
    let port = faulk_helper(ORIGIN, Duration::ZERO);
    assert!(matches!(
        claim(port, &origins(&[ORIGIN]), Duration::from_secs(5)),
        Startup::AlreadyRunning
    ));
}

#[test]
fn a_faulk_helper_still_starting_is_waited_for() {
    let port = faulk_helper(ORIGIN, Duration::from_millis(1500));
    assert!(matches!(
        claim(port, &origins(&[ORIGIN]), Duration::from_secs(6)),
        Startup::AlreadyRunning
    ));
}

#[test]
fn a_faulk_helper_with_other_origins_is_found_through_the_next_origin() {
    let port = faulk_helper(ORIGIN, Duration::ZERO);
    assert!(matches!(
        claim(
            port,
            &origins(&["http://elsewhere.test", ORIGIN]),
            Duration::from_secs(5)
        ),
        Startup::AlreadyRunning
    ));
}

#[test]
fn a_faulk_helper_that_allows_none_of_the_origins_is_still_found_at_once() {
    let port = faulk_helper("http://localhost:4173", Duration::ZERO);
    let start = Instant::now();
    let startup = claim(port, &origins(&[ORIGIN]), Duration::from_secs(5));
    assert!(matches!(startup, Startup::AlreadyRunning), "{startup:?}");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "no wait for the patience"
    );
}

#[test]
fn another_server_that_refuses_without_the_helper_header_blocks() {
    use std::io::{Read, Write};
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = other.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in other.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 403 Forbidden
content-length: 0

",
            );
        }
    });
    let startup = claim(port, &origins(&[ORIGIN]), Duration::from_secs(1));
    assert!(matches!(startup, Startup::Blocked(_)), "{startup:?}");
}

#[test]
fn another_program_on_the_port_blocks_after_the_patience() {
    // Accepts connections (the OS backlog does) but never answers.
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = other.local_addr().unwrap().port();
    let start = Instant::now();
    let startup = claim(port, &origins(&[ORIGIN]), Duration::from_secs(1));
    assert!(matches!(startup, Startup::Blocked(_)), "{startup:?}");
    assert!(start.elapsed() < Duration::from_secs(4), "gave up in time");
    drop(other);
}
