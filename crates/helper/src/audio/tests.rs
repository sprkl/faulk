//! The helper's engine fed with a page's streams (BO-1): what the page's project core encodes
//! for its own engine, submitted unchanged.

use super::*;
use assert_no_alloc::assert_no_alloc;
use commands::engine_msgs::Synced;
use commands::{Command, device, live, transport};
use engine::msg::TransportState;

/// The page's side: its project and the stream it encodes (the page's core, `Synced`).
struct Page(Synced);

impl Page {
    /// The page's one-synth project, set up in `epoch`.
    fn single_synth(epoch: u16) -> Page {
        Page(Synced::single_synth(epoch))
    }

    fn setup(&mut self) -> Vec<u8> {
        self.0.setup().unwrap()
    }

    fn run(&mut self, command: Command) -> Vec<u8> {
        self.0.run(&command).unwrap().1
    }

    fn note_on(&mut self, note: u8, velocity: u8) -> Vec<u8> {
        self.run(Command::Live(live::Command::NoteOn {
            track: commands::engine_msgs::single_synth::track(),
            note,
            velocity,
        }))
    }
}

fn core() -> (AudioCore, EventSender, Arc<Meter>) {
    let meter = Arc::new(Meter::default());
    let (core, sender) = audio_core(48_000.0, meter.clone()).unwrap();
    (core, sender, meter)
}

/// A core holding the page's one-synth project, and that page.
fn with_page() -> (AudioCore, EventSender, Arc<Meter>, Page) {
    let (core, mut sender, meter) = core();
    let mut page = Page::single_synth(1);
    sender.submit(&page.setup()).unwrap();
    (core, sender, meter, page)
}

#[test]
fn a_fresh_engine_is_silent_in_epoch_0_until_a_page_sends_its_snapshot() {
    let (mut core, mut sender, meter) = core();
    assert_eq!(sender.epoch(), 0);
    let mut out = vec![0.0; 960];
    core.process(&mut out, 2);
    assert_eq!(meter.take(), (0.0, 0));
    let mut page = Page::single_synth(1);
    sender.submit(&page.setup()).unwrap();
    assert_eq!(sender.epoch(), 1);
    sender.submit(&page.note_on(69, 127)).unwrap();
    core.process(&mut out, 2);
    assert_eq!(meter.take().1, 1, "the page's track plays");
    assert_eq!(sender.rejected(), 0);
    assert_eq!(sender.received(), 2);
}

#[test]
fn process_never_allocates_for_any_buffer_shape() {
    let (mut core, mut sender, _, mut page) = with_page();
    // Buffers allocated before the check; the check covers only `process`.
    let mut buffers: Vec<(Vec<f32>, usize)> =
        [(441, 2), (480, 2), (5_000, 2), (128, 1), (256, 6), (0, 2)]
            .iter()
            .map(|&(frames, ch)| (vec![0.0; frames * ch + 1], ch))
            .collect();
    sender.submit(&page.note_on(60, 100)).unwrap();
    assert_no_alloc(|| {
        for _ in 0..20 {
            for (buf, ch) in buffers.iter_mut() {
                core.process(buf, *ch);
            }
        }
    });
}

#[test]
fn notes_reach_both_channels_and_extra_channels_stay_silent() {
    let (mut core, mut sender, meter, mut page) = with_page();
    sender.submit(&page.note_on(69, 127)).unwrap();
    let mut out = vec![0.0; 4_800 * 4];
    core.process(&mut out, 4);
    let frame = |i: usize| &out[i * 4..i * 4 + 4];
    assert!(out.chunks(4).any(|f| f[0].abs() > 0.05));
    for i in 0..4_800 {
        assert_eq!(frame(i)[0], frame(i)[1]);
        assert_eq!(frame(i)[2], 0.0);
        assert_eq!(frame(i)[3], 0.0);
    }
    let (peak, voices) = meter.take();
    assert!(peak > 0.05);
    assert_eq!(voices, 1);
    assert_eq!(meter.callback_frames(), Some(4_800));
    assert_eq!(meter.take().0, 0.0, "the peak resets after a read");
    assert_eq!(sender.rejected(), 0, "the setup and the note were taken");
}

#[test]
fn parameters_reach_the_synth() {
    let (mut core, mut sender, meter, mut page) = with_page();
    sender.submit(&page.note_on(69, 127)).unwrap();
    let mut out = vec![0.0; 9_600 * 2];
    core.process(&mut out, 2);
    let loud = meter.take().0;
    let gain = page.run(Command::Device(device::Command::SetParam {
        device: commands::engine_msgs::single_synth::device(),
        param: "gain".into(),
        value: 0.1,
    }));
    sender.submit(&gain).unwrap();
    for _ in 0..4 {
        core.process(&mut out, 2);
    }
    meter.take();
    core.process(&mut out, 2);
    let quiet = meter.take().0;
    assert!(quiet < loud * 0.5, "gain 0.1: {quiet} against {loud}");
}

#[test]
fn latency_estimate_adds_the_driver_delay_to_one_buffer() {
    let meter = Meter::default();
    assert_eq!(
        meter.estimated_latency_ms(48_000.0),
        None,
        "unknown before the first callback"
    );
    meter.callback_frames.store(480, Ordering::Relaxed);
    assert_eq!(meter.estimated_latency_ms(48_000.0), Some(10.0));
    meter.set_output_delay(Duration::from_micros(12_500));
    assert_eq!(meter.estimated_latency_ms(48_000.0), Some(22.5));
}

/// The load and peak of the last report, after `callbacks` callbacks of 480 frames. With a
/// `cost`, each chunk's render lasts that many seconds on a synthetic clock; without, the real
/// monotonic clock times it.
fn load_after(core: &mut AudioCore, callbacks: usize, cost: Option<f64>) -> (f32, f32) {
    let mut out = vec![0.0; 480 * 2];
    let base = Instant::now();
    let mut t = 0.0f64;
    for _ in 0..callbacks {
        match cost {
            Some(cost) => core.process_with(&mut out, 2, &mut || {
                // Each read of the clock is `cost` after the one before: whatever the number of reads, the time
                // between the two around a render is `cost`.
                t += cost;
                base + Duration::from_secs_f64(t)
            }),
            None => core.process(&mut out, 2),
        }
    }
    let report = engine::host::read_feedback(core.host.feedback()).unwrap();
    (report.cpu_load, report.cpu_peak)
}

/// The same with the real clock: the comparison depends on the machine's load, so it is not
/// part of the default run (`cargo test -p helper -- --ignored`).
#[test]
#[ignore = "compares measured times: depends on the machine's load"]
fn the_real_clock_times_the_render_and_more_voices_weigh_more() {
    let (mut core, mut sender, _, mut page) = with_page();
    let (idle, _) = load_after(&mut core, 200, None);
    for note in 0..16 {
        sender.submit(&page.note_on(40 + note * 3, 100)).unwrap();
    }
    let (busy, busy_peak) = load_after(&mut core, 200, None);
    assert!(busy > 0.0 && busy_peak >= busy && busy_peak <= 1.0);
    assert!(
        busy > idle,
        "16 voices weigh more than none: {busy} against {idle}"
    );
}

#[test]
fn the_callback_times_its_render_and_a_slower_render_weighs_more() {
    // A synthetic clock (#611): the loads depend on the injected times only.
    let (mut core, mut sender, _, mut page) = with_page();
    let (idle, idle_peak) = load_after(&mut core, 20, Some(0.000_5));
    assert!((0.0..=1.0).contains(&idle), "{idle}");
    assert!(idle_peak >= idle && idle_peak <= 1.0, "{idle_peak}");
    for note in 0..16 {
        sender.submit(&page.note_on(40 + note * 3, 100)).unwrap();
    }
    let (busy, busy_peak) = load_after(&mut core, 20, Some(0.004));
    assert!(busy > 0.0, "a measured load: {busy}");
    assert!(busy_peak >= busy && busy_peak <= 1.0, "{busy_peak}");
    assert!(
        busy > idle,
        "a slower render weighs more: {busy} against {idle}"
    );
    // 480 frames are 10 ms of audio: a render of 0.5 ms is 5 %, of 4 ms is 40 %. This pins that `process`
    // times the render itself, and nothing else.
    assert!((idle - 0.05).abs() < 0.005, "{idle}");
    assert!((busy - 0.4).abs() < 0.005, "{busy}");
}

#[test]
fn full_live_queue_releases_every_note() {
    let (mut core, mut sender, _, mut page) = with_page();
    sender.submit(&page.note_on(60, 100)).unwrap();
    let mut out = vec![0.0; 256];
    core.process(&mut out, 2);
    // Fill the live queue with note-ons: the note-off that follows is lost.
    for _ in 0..engine::msg::limits::LIVE_QUEUE_CAPACITY {
        sender.submit(&page.note_on(72, 100)).unwrap();
    }
    let off = page.run(Command::Live(live::Command::NoteOff {
        track: commands::engine_msgs::single_synth::track(),
        note: 60,
    }));
    sender.submit(&off).unwrap();
    // The overflow releases everything, so after the release time all is silent.
    for _ in 0..200 {
        core.process(&mut out, 2);
    }
    assert!(out.iter().all(|s| *s == 0.0));
}

/// The page's big project (`tracks * 128` clips): its snapshot is longer than the engine
/// takes at once.
fn big_page(tracks: usize) -> (Page, Vec<u8>) {
    let big = test_support::big::big_project(tracks, 64, 64);
    let mut page = Page(Synced::new(big.state, big.ctx, 1));
    let setup = page.setup();
    (page, setup)
}

/// The last track of a page's project.
fn last_track(page: &Page) -> project::TrackId {
    page.0.state.project.tracks.keys().last().unwrap().clone()
}

/// Callbacks of 256 frames, each followed by a flush of the control thread, until the
/// engine has applied everything (nothing waits, nothing is pending, a callback applies
/// nothing): how many.
fn until_applied(core: &mut AudioCore, sender: &mut EventSender) -> usize {
    let mut out = vec![0.0; 256 * 2];
    let mut callbacks = 0;
    loop {
        let before = core.host.engine().applied();
        assert_no_alloc(|| core.process(&mut out, 2));
        sender.flush();
        if core.host.engine().applied() == before
            && sender.waiting() == 0
            && sender.queue.prepare().preparer().pending() == 0
        {
            return callbacks;
        }
        callbacks += 1;
        assert!(callbacks < 10_000, "the setup does not drain");
    }
}

#[test]
fn a_big_project_is_set_up_in_pieces_and_the_audio_thread_never_allocates() {
    let (mut core, mut sender, meter) = core();
    let (mut page, setup) = big_page(48);
    sender.submit(&setup).unwrap();
    assert_eq!(sender.waiting(), 1, "the setup waits for the engine");
    assert_eq!(sender.epoch(), 1, "the epoch of the Reset that waits");
    let callbacks = until_applied(&mut core, &mut sender);
    assert!(callbacks > 1, "{callbacks} callbacks");
    assert_eq!(sender.rejected(), 0);
    assert_eq!(core.host.engine().skipped(), 0);
    // The page plays its last track.
    let track = last_track(&page);
    let note = page.run(Command::Live(live::Command::NoteOn {
        track,
        note: 69,
        velocity: 100,
    }));
    sender.submit(&note).unwrap();
    let mut out = vec![0.0; 4_800 * 2];
    assert_no_alloc(|| core.process(&mut out, 2));
    assert!(out.iter().any(|s| s.abs() > 0.01), "the last track plays");
    assert_eq!(meter.take().1, 1);
}

#[test]
fn a_change_sent_while_the_setup_drains_waits_behind_it_and_is_taken() {
    // In order, as on the web: the note waits behind the snapshot of its track, in the epoch
    // of the Reset that waits (not refused as stale).
    let (mut core, mut sender, _) = core();
    let (mut page, setup) = big_page(48);
    sender.submit(&setup).unwrap();
    let track = last_track(&page);
    let note = page.run(Command::Live(live::Command::NoteOn {
        track,
        note: 69,
        velocity: 100,
    }));
    assert_eq!(sender.submit(&note), Ok(()));
    assert_eq!(sender.waiting(), 2, "the note waits behind the setup");
    let mut out = vec![0.0f32; 256 * 2];
    let mut callbacks = 0;
    while !out.iter().any(|s| s.abs() > 0.01) {
        assert_no_alloc(|| core.process(&mut out, 2));
        sender.flush();
        callbacks += 1;
        assert!(callbacks < 10_000, "the note never sounds");
    }
    assert_eq!(sender.rejected(), 0);
    assert_eq!(core.host.engine().skipped(), 0);
    assert_eq!(sender.take_refusal(), None);
}

#[test]
fn a_corrupt_stream_is_refused_once_and_the_engine_is_not_resynced_by_the_helper() {
    let (mut core, mut sender, meter, mut page) = with_page();
    // A message header with reserved bits set: corrupt.
    let mut corrupt = page.note_on(60, 100);
    corrupt[1] = 1;
    let refusal = sender.submit(&corrupt).unwrap_err();
    assert_eq!(refusal.code, MsgError::Corrupt.code());
    assert_eq!(refusal.at, 0);
    assert_eq!(refusal.epoch, 1);
    assert_eq!(
        sender.take_refusal(),
        None,
        "returned to the caller, which tells the page: not told twice"
    );
    // Too short for a header: refused too, the session's engine goes on.
    assert!(sender.submit(&[1, 2, 3]).is_err());
    assert_eq!(sender.epoch(), 1, "no resync of its own");
    // The page's next change still plays; its snapshot after the refusal is taken.
    sender.submit(&page.note_on(69, 127)).unwrap();
    let mut out = vec![0.0; 960];
    core.process(&mut out, 2);
    assert_eq!(meter.take().1, 1);
    sender
        .submit(&page.0.resync_after(refusal.epoch).unwrap())
        .unwrap();
    assert_eq!(sender.epoch(), 2);
}

#[test]
fn a_waiting_stream_the_engine_refuses_at_a_flush_is_a_refusal_too() {
    let (mut core, mut sender, _) = core();
    let (_, setup) = big_page(48);
    sender.submit(&setup).unwrap();
    // Behind the setup that waits, in its epoch, but cut short: refused when its turn comes.
    let mut other = Page::single_synth(1);
    other.setup();
    // Past the padding (a message's frame is padded to 8 bytes): into its payload.
    let mut cut = other.note_on(60, 100);
    cut.truncate(cut.len() - 4);
    assert_eq!(sender.submit(&cut), Ok(()));
    assert_eq!(sender.waiting(), 2);
    until_applied(&mut core, &mut sender);
    let refusal = sender.take_refusal().expect("refused at a flush");
    assert_eq!(
        refusal.code,
        MsgError::Truncated {
            needed: 0,
            available: 0
        }
        .code()
    );
    assert_eq!(refusal.epoch, 1);
}

#[test]
fn a_stream_in_another_epoch_is_refused_whole_with_the_engines_epoch() {
    let (mut core, mut sender, meter, mut page) = with_page();
    let stale = MsgError::StaleEpoch {
        message: 0,
        current: 0,
    }
    .code();
    // A Reset that is not newer (another page's core, started at the same epoch): refused
    // before the engine sees it, which would skip it silently.
    let mut other = Page::single_synth(1);
    let refusal = sender.submit(&other.setup()).unwrap_err();
    assert_eq!((refusal.code, refusal.at, refusal.epoch), (stale, 0, 1));
    // A change of the other page (epoch 1 is the engine's, but its next snapshot is in 2):
    // a change in an epoch that is not the engine's is refused the same way.
    let mut ahead = Page::single_synth(1);
    ahead.0.resync().unwrap();
    let refusal = sender.submit(&ahead.note_on(60, 100)).unwrap_err();
    assert_eq!((refusal.code, refusal.epoch), (stale, 1));
    assert_eq!(sender.rejected(), 0, "the engine never saw them");
    // The page answers with a snapshot newer than the refusal's epoch: taken.
    sender
        .submit(&other.0.resync_after(refusal.epoch).unwrap())
        .unwrap();
    assert_eq!(sender.epoch(), 2);
    sender.submit(&other.note_on(69, 127)).unwrap();
    let mut out = vec![0.0; 960];
    core.process(&mut out, 2);
    assert_eq!(meter.take().1, 1);
    // The first page, now behind, is refused.
    assert!(sender.submit(&page.note_on(60, 100)).is_err());
}

/// The transport state of the engine's latest report.
fn transport_of(core: &mut AudioCore, meter: &Meter) -> TransportState {
    let mut out = vec![0.0; 4_800 * 2];
    core.process(&mut out, 2);
    let mut bytes = Vec::new();
    assert!(meter.copy_report(&mut bytes));
    engine::host::decode_feedback(&bytes).unwrap().transport
}

#[test]
fn releasing_all_stops_the_transport_and_ends_every_note_of_every_track() {
    let (mut core, mut sender, meter, mut page) = with_page();
    // A second track, both playing a note, and the transport running.
    sender
        .submit(&page.run(Command::Track(commands::track::Command::Create {
            kind: project::TrackKind::Midi,
            name: None,
            color: None,
            instrument: None,
        })))
        .unwrap();
    for track in page
        .0
        .state
        .project
        .tracks
        .keys()
        .cloned()
        .collect::<Vec<_>>()
    {
        let note = page.run(Command::Live(live::Command::NoteOn {
            track,
            note: 69,
            velocity: 100,
        }));
        sender.submit(&note).unwrap();
    }
    sender
        .submit(&page.run(Command::Transport(transport::Command::Play {})))
        .unwrap();
    assert_eq!(transport_of(&mut core, &meter), TransportState::Playing);
    assert_eq!(meter.take().1, 2);
    sender.release_all();
    assert_eq!(transport_of(&mut core, &meter), TransportState::Stopped);
    // The voices end their release.
    let mut out = vec![0.0; 4_800 * 2];
    for _ in 0..40 {
        core.process(&mut out, 2);
    }
    assert_eq!(meter.take().1, 0);
    assert_eq!(sender.rejected(), 0);
    assert_eq!(sender.epoch(), 1, "the project stays, in its epoch");
}

#[test]
fn releasing_all_drops_what_waits_and_goes_in_at_once_in_the_engines_epoch() {
    let (mut core, mut sender, meter) = core();
    let (_, setup) = big_page(4);
    sender.submit(&setup).unwrap();
    until_applied(&mut core, &mut sender);
    // A page's newer snapshot waits (a switch, say) when the page leaves.
    let (mut page, _) = big_page(48);
    sender.submit(&page.0.resync_after(1).unwrap()).unwrap();
    assert!(sender.waiting() > 0);
    assert_eq!(sender.epoch(), 2, "the waiting Reset's epoch");
    sender.release_all();
    // The snapshot's rest that waited is gone; only the release may wait (its stop, behind
    // what the engine still prepares): its notes off went in at once, as live events.
    assert!(sender.waiting() <= 1, "{}", sender.waiting());
    let engine = sender.queue.prepare().preparer().epoch();
    assert_eq!(sender.epoch(), engine, "back to the engine's own epoch");
    until_applied(&mut core, &mut sender);
    assert_eq!(
        sender.rejected(),
        0,
        "the release was in the engine's epoch"
    );
    let _ = page;
    assert_eq!(transport_of(&mut core, &meter), TransportState::Stopped);
}

#[test]
fn the_slot_feeds_only_the_engine_a_session_was_opened_on() {
    let slot = EngineSlot::default();
    assert_eq!(slot.current(), None);
    assert_eq!(slot.submit(1, &[]), None);
    let (mut core, sender, meter) = core();
    slot.install(sender, 48_000.0);
    let (generation, epoch) = slot.current().unwrap();
    assert_eq!(epoch, 0);
    let mut page = Page::single_synth(1);
    assert_eq!(slot.submit(generation, &page.setup()), Some(Ok(())));
    assert_eq!(slot.current(), Some((generation, 1)));
    assert_eq!(slot.submit(generation + 1, &page.note_on(69, 127)), None);
    // A new output's engine: the old generation's streams go nowhere.
    let (_core2, sender2, _) = super::tests::core();
    slot.install(sender2, 44_100.0);
    let (newer, epoch) = slot.current().unwrap();
    assert_ne!(newer, generation);
    assert_eq!(epoch, 0);
    assert_eq!(slot.submit(generation, &page.note_on(69, 127)), None);
    let mut out = vec![0.0; 960];
    core.process(&mut out, 2);
    assert_eq!(meter.take().1, 0);
    slot.remove();
    assert_eq!(slot.current(), None);
}

#[test]
fn the_control_thread_flushes_until_the_sender_is_dropped() {
    let (_core, sender, _) = core();
    let slot = Arc::new(EngineSlot::default());
    slot.install(sender, 48_000.0);
    let flusher = spawn_flusher(&slot);
    std::thread::sleep(FLUSH_INTERVAL * 3);
    drop(slot);
    flusher.join().unwrap();
}

#[test]
fn an_empty_slot_takes_nothing_and_reports_no_rate() {
    let slot = EngineSlot::default();
    assert!(!slot.is_open());
    assert_eq!(slot.sample_rate(), None);
    slot.release_all();
    slot.flush();
    assert_eq!(slot.with(|s| s.rejected()), None);
    let (mut core, sender, meter) = core();
    slot.install(sender, 44_100.0);
    assert_eq!(slot.sample_rate(), Some(44_100.0));
    let (generation, _) = slot.current().unwrap();
    let mut page = Page::single_synth(1);
    slot.submit(generation, &page.setup()).unwrap().unwrap();
    slot.submit(generation, &page.note_on(69, 127))
        .unwrap()
        .unwrap();
    let mut out = vec![0.0; 960];
    core.process(&mut out, 2);
    assert_eq!(meter.take().1, 1, "the installed engine plays");
    slot.remove();
    assert!(!slot.is_open());
}

#[test]
fn a_reset_meter_forgets_the_output_that_went_away() {
    let (mut core, mut sender, meter, mut page) = with_page();
    sender.submit(&page.note_on(69, 127)).unwrap();
    let mut out = vec![0.0; 960];
    core.process(&mut out, 2);
    meter.set_output_delay(Duration::from_millis(5));
    assert!(meter.callback_frames().is_some());
    let mut report = Vec::new();
    // The engine writes its report every few blocks.
    for _ in 0..100 {
        if meter.copy_report(&mut report) {
            break;
        }
        core.process(&mut out, 2);
    }
    assert!(meter.copy_report(&mut report), "a report before the reset");
    meter.reset();
    assert_eq!(meter.callback_frames(), None);
    assert_eq!(meter.estimated_latency_ms(48_000.0), None);
    assert_eq!(meter.take(), (0.0, 0));
    assert!(!meter.copy_report(&mut report), "no stale report");
}
