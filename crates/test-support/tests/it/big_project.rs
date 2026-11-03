//! A big project's snapshot (#245): more messages than the engine takes at once (its rt ring
//! plus its prepare queue), sent at the start, at `project.open` and at a mode switch (a
//! resync into an engine that does not render yet). The host's queue (`StreamQueue`, the
//! native twin of the web `PushQueue`) sends it in pieces as the engine consumes it: nothing
//! is refused, lost or reordered, the engine ends up holding the project, and the project
//! plays. The time to a playable state is printed; the release figures:
//! `cargo test -p test-support --release --test it big_project:: -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use commands::contract::Folder;
use commands::engine_msgs::Synced;
use commands::{Command, launch};
use engine::host::StreamQueue;
use engine::msg::limits;
use project::file::{MemFs, SaveOptions, SaveState, save, save_session};
use test_support::big::{BigProject, big_project};
use test_support::view::messages;
use test_support::{
    EngineView, QUANTUM, SAMPLE_RATE, Scenario, Session, Step, parity, render_native,
};

/// What the engine takes before it is busy.
const AT_ONCE: usize = limits::RT_RING_CAPACITY + limits::PENDING_QUEUE_CAPACITY;

/// 48 tracks of 64 grid clips and 64 arrangement clips: 6 144 clips.
fn project() -> BigProject {
    big_project(48, 64, 64)
}

fn folder(big: &BigProject) -> MemFs {
    let mut fs = MemFs::new();
    save(
        &mut fs,
        &big.state.project,
        &mut SaveState::new(),
        None,
        &SaveOptions::default(),
    )
    .expect("the project saves");
    save_session(&mut fs, &big.state.session).expect("the session saves");
    fs
}

/// Messages in one push.
fn count(bytes: &[u8]) -> usize {
    messages(&Scenario {
        steps: vec![Step::Push(bytes.to_vec())],
    })
    .len()
}

/// The last push of a session.
fn last_push(s: &Session) -> &[u8] {
    s.scenario()
        .steps
        .iter()
        .rev()
        .find_map(|step| match step {
            Step::Push(bytes) => Some(bytes.as_slice()),
            Step::Render(_) => None,
        })
        .expect("a push")
}

/// Blocks to render so that a stream of `n` messages is applied (128 per block), with room.
fn blocks_for(n: usize) -> u32 {
    (n.div_ceil(limits::MAX_APPLY_PER_BLOCK) + 64) as u32
}

fn launch_first_scene(s: &mut Session, big: &BigProject) {
    s.run(Command::Launch(launch::Command::Scene {
        scene: big.scenes[0].clone(),
    }));
}

fn assert_holds_the_project(s: &Session, big: &BigProject) {
    assert_eq!(
        EngineView::of_stream(s.scenario(), &s.ctx().engine),
        EngineView::of_project(&big.state.project, &big.state.session),
        "the engine holds the whole project"
    );
}

#[test]
fn the_setup_of_a_big_project_goes_in_pieces_and_the_project_plays() {
    let big = project();
    assert!(big.clips() >= 6_000, "{} clips", big.clips());
    let mut s = big.session();
    let setup = count(last_push(&s));
    assert!(setup > AT_ONCE, "{setup} messages");
    s.render_blocks(blocks_for(setup));
    assert_holds_the_project(&s, &big);
    launch_first_scene(&mut s, &big);
    s.render_secs(1.0);
    // `render_native` panics on a refused, skipped, pending or waiting message.
    let render = render_native(s.scenario());
    let from = blocks_for(setup) as f32 * QUANTUM as f32 / SAMPLE_RATE;
    render.assert_clean();
    render.assert_audible(from + 0.1..from + 0.9, 0.01);
}

#[test]
fn a_big_project_opened_from_its_folder_goes_in_pieces_over_a_playing_one() {
    let big = project();
    let mut s = Session::single_synth();
    s.note_on(60, 100).render_secs(0.1);
    s.stage(Folder::new("big", folder(&big)));
    s.open("big");
    let snapshot = count(last_push(&s));
    assert!(snapshot > AT_ONCE, "{snapshot} messages");
    assert_eq!(s.state().project, big.state.project);
    s.render_blocks(blocks_for(snapshot));
    assert_holds_the_project(&s, &big);
    launch_first_scene(&mut s, &big);
    s.render_secs(1.0);
    let render = render_native(s.scenario());
    render.assert_clean();
    let end = render.seconds();
    render.assert_audible(end - 0.9..end - 0.1, 0.01);
}

#[test]
fn a_switch_resyncs_into_an_engine_that_does_not_render_yet() {
    // The switch back to Web: the snapshot is pushed while the audio context is suspended,
    // here behind the setup, which waits too since nothing renders between them.
    let big = project();
    let mut s = big.session();
    s.resync();
    launch_first_scene(&mut s, &big);
    assert!(count(&first_push(&s)) > AT_ONCE);
    let total: usize = s
        .scenario()
        .steps
        .iter()
        .map(|step| match step {
            Step::Push(bytes) => count(bytes),
            Step::Render(_) => 0,
        })
        .sum();
    s.render_blocks(blocks_for(total));
    s.render_secs(1.0);
    assert_holds_the_project(&s, &big);
    let render = render_native(s.scenario());
    render.assert_clean();
    let end = render.seconds();
    render.assert_audible(end - 0.9..end - 0.1, 0.01);
}

#[test]
fn the_webassembly_engine_takes_a_big_setup_in_pieces_and_plays_the_same() {
    // The page's path: the module's `fk_push` stops on a busy engine and the rest goes in
    // again at the next flush (`render_wasm` does what `push-queue.ts` does).
    let big = project();
    let mut s = big.session();
    let setup = count(last_push(&s));
    s.render_blocks(blocks_for(setup));
    launch_first_scene(&mut s, &big);
    s.render_secs(0.25);
    let render = parity(s.scenario());
    let end = render.seconds();
    render.assert_audible(end - 0.2..end, 0.01);
}

fn first_push(s: &Session) -> Vec<u8> {
    match &s.scenario().steps[0] {
        Step::Push(bytes) => bytes.clone(),
        Step::Render(_) => panic!("a push first"),
    }
}

/// What it takes, natively, from the moment a stream is handed to the host's queue to the
/// block after which the engine has applied all of it (the project is playable): the blocks
/// rendered meanwhile (each applies at most 128 messages) and the wall time of preparing and
/// rendering them as fast as the machine goes.
fn time_to_playable(bytes: &[u8]) -> (usize, Duration) {
    let (prepare, mut host) = engine::host::channel(SAMPLE_RATE).unwrap();
    let mut queue = StreamQueue::new(prepare);
    let start = Instant::now();
    queue.send(bytes).unwrap();
    let mut blocks = 0;
    loop {
        let before = host.engine().applied();
        host.render(QUANTUM as usize);
        queue.flush().unwrap();
        if host.engine().applied() == before
            && queue.waiting() == 0
            && queue.prepare().preparer().pending() == 0
        {
            break;
        }
        blocks += 1;
    }
    let wall = start.elapsed();
    assert_eq!(queue.prepare().preparer().rejected(), 0);
    assert_eq!(host.engine().skipped(), 0);
    (blocks, wall)
}

fn report(what: &str, encode: Duration, bytes: &[u8]) {
    let (blocks, wall) = time_to_playable(bytes);
    let audio_ms = blocks as f64 * f64::from(QUANTUM) / f64::from(SAMPLE_RATE) * 1000.0;
    println!(
        "{what}: {} messages, {} bytes; encode {:.1} ms; applied after {blocks} blocks of {QUANTUM} frames = {audio_ms:.0} ms of audio at {SAMPLE_RATE} Hz; prepare + render as fast as possible {:.1} ms",
        count(bytes),
        bytes.len(),
        encode.as_secs_f64() * 1000.0,
        wall.as_secs_f64() * 1000.0,
    );
}

#[test]
fn time_to_a_playable_state_is_bounded_by_the_engines_pace() {
    let big = project();
    let mut synced = Synced::new(big.state.clone(), big.ctx.clone(), 1);
    let start = Instant::now();
    let setup = synced.setup().unwrap();
    report("setup", start.elapsed(), &setup);
    let (blocks, _) = time_to_playable(&setup);
    // The engine applies 128 messages per block: the pace is the floor, and the queue adds
    // no idle block (one flush per block).
    let floor = count(&setup).div_ceil(limits::MAX_APPLY_PER_BLOCK);
    assert!(
        blocks >= floor - 1 && blocks <= floor + 2,
        "{blocks} blocks for a floor of {floor}"
    );
}

/// The figures of the pull request: the setup, `project.open` from a folder in memory and
/// the resync of a switch, on the test project and on one four times larger.
#[test]
#[ignore = "a measurement: run with --release --ignored --nocapture"]
fn measure_time_to_playable() {
    for (tracks, scenes, bars) in [(48, 64, 64), (96, 128, 128)] {
        let big = big_project(tracks, scenes, bars);
        println!("project: {tracks} tracks, {} clips", big.clips());
        let mut synced = Synced::new(big.state.clone(), big.ctx.clone(), 1);
        let start = Instant::now();
        let setup = synced.setup().unwrap();
        report("  setup", start.elapsed(), &setup);
        let start = Instant::now();
        let resync = synced.resync().unwrap();
        report("  switch (resync)", start.elapsed(), &resync);
        let mut opener = Synced::new(big.state.clone(), big.ctx.clone(), 1);
        opener.ctx.folder = Some(Folder::new("big", folder(&big)));
        let start = Instant::now();
        let (_, open) = opener
            .run(&Command::Project(commands::project::Command::Open {
                folder: "big".into(),
            }))
            .unwrap();
        report("  project.open (read + snapshot)", start.elapsed(), &open);
    }
}
