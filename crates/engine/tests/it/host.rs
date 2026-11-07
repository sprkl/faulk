//! The shipping host (MO-2, `engine::host`): the registered synth (DV-1) renders as its own
//! calls do,
//! the render never allocates, the report region decodes, the live overflow releases.

use assert_no_alloc::assert_no_alloc;
use engine::host::{Host, Prepare, channel, copy_feedback, feedback_voices, read_feedback};
use engine::msg::{
    ControlMsg as M, DeviceSlot, FEEDBACK_HEADER_BYTES as HEADER, ParamId, TrackKind, TrackSlot,
    TransportState, encode_stream, limits,
};
use engine::time::Tick;
use engine::{OSC_SYNTH, Param, Synth};

const RATE: f32 = 48_000.0;

struct Session {
    prepare: Prepare,
    host: Host,
    seq: u32,
}

impl Session {
    fn new() -> Self {
        let (prepare, host) = channel(RATE).unwrap();
        let mut s = Session {
            prepare,
            host,
            seq: 0,
        };
        s.send(&[
            M::Reset { epoch: 1 },
            M::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: Vec::new(),
            },
        ]);
        s
    }

    fn send(&mut self, msgs: &[M]) {
        let bytes = encode_stream(1, self.seq, msgs).unwrap();
        self.seq += msgs.len() as u32;
        self.prepare.submit(&bytes).unwrap();
    }
}

fn note_on(pitch: u8, velocity: u8) -> M {
    M::NoteOn {
        track: TrackSlot(0),
        pitch,
        velocity,
        at: None,
    }
}

fn param(param: Param, value: f32) -> M {
    M::ParamSet {
        track: TrackSlot(0),
        device: DeviceSlot(0),
        param: ParamId(param.id()),
        value,
    }
}

/// The host's routing (live notes at offset 0, parameter sets, the track's fade-in, the
/// master): the registered synth in the host renders as a lone synth driven by its own calls.
/// The synth itself is checked against the pre-DV-1 synth in `tests/it/osc_synth.rs`.
#[test]
fn the_host_renders_the_registered_synth_as_its_own_calls_do() {
    let mut session = Session::new();
    // The new track fades in over 5 ms (MO-3's graph): let it reach unity first.
    for _ in 0..4 {
        session.host.render(128);
    }
    let mut synth = Synth::new(RATE);
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    // (block, what happens at its start)
    let script: Vec<(usize, Vec<M>)> = vec![
        (0, vec![note_on(60, 100)]),
        (10, vec![param(Param::Waveform, 2.0), note_on(64, 80)]),
        (
            30,
            vec![param(Param::Gain, 0.3), param(Param::Release, 0.05)],
        ),
        (
            50,
            vec![M::NoteOff {
                track: TrackSlot(0),
                pitch: 60,
                at: None,
            }],
        ),
        (
            70,
            vec![M::AllNotesOff {
                track: TrackSlot(0),
            }],
        ),
    ];
    let mut peak = 0.0f32;
    for block in 0..120 {
        if let Some((_, msgs)) = script.iter().find(|(b, _)| *b == block) {
            session.send(msgs);
            for m in msgs {
                match *m {
                    M::NoteOn {
                        pitch, velocity, ..
                    } => synth.note_on(pitch, velocity),
                    M::NoteOff { pitch, .. } => synth.note_off(pitch),
                    M::AllNotesOff { .. } => synth.all_notes_off(),
                    M::ParamSet { param, value, .. } => {
                        synth.set_param(Param::from_id(param.0).unwrap(), value)
                    }
                    _ => {}
                }
            }
        }
        synth.render(&mut l, &mut r);
        let n = session.host.render(128);
        assert_eq!(n, 128);
        assert_eq!(&session.host.left()[..128], &l[..], "block {block}");
        assert_eq!(&session.host.right()[..128], &r[..], "block {block}");
        peak = l.iter().fold(peak, |p, s| p.max(s.abs()));
    }
    assert!(peak > 0.05);
    assert_eq!(session.host.engine().skipped(), 0);
    assert_eq!(session.prepare.preparer().rejected(), 0);
}

#[test]
fn reset_keeps_the_parameters_and_silences() {
    use engine::device::Device;
    let mut device = Synth::new(RATE);
    device.note_on(60, 100);
    Device::set_param(&mut device, ParamId(Param::Gain.id()), 0.25);
    device.reset();
    assert_eq!(device.param(Param::Gain), 0.25);
    assert_eq!(device.active_voices(), 0);
}

#[test]
fn render_never_allocates_with_notes_params_and_growth() {
    let mut session = Session::new();
    session.send(&[note_on(57, 100)]);
    for block in 0..2_000 {
        if block % 50 == 0 {
            let slot = block / 50 + 1;
            session.send(&[
                M::TrackCreate {
                    track: TrackSlot(slot),
                    kind: TrackKind::Midi,
                },
                M::DeviceInsert {
                    track: TrackSlot(slot),
                    device: DeviceSlot(0),
                    kind: OSC_SYNTH.kind_id,
                    state: Vec::new(),
                },
                note_on(60 + (block % 12) as u8, 50),
                param(Param::Gain, (block % 100) as f32 / 100.0),
            ]);
        }
        // Frames from 1 to the largest block, the report written on the way.
        let frames = 1 + (block * 37) % limits::MAX_BLOCK_FRAMES as u32;
        assert_no_alloc(|| {
            session.host.render(frames as usize);
        });
    }
    // The track table grew past its initial 16 slots while rendering.
    assert!(session.host.engine().state().tracks.capacity() >= 64);
    assert_eq!(session.host.engine().skipped(), 0);
}

#[test]
fn the_report_holds_the_meters_and_the_counters() {
    let mut session = Session::new();
    session.send(&[note_on(69, 127)]);
    // A message of a past epoch: rejected and counted in the report.
    let stale = encode_stream(0, 99, &[note_on(1, 1)]).unwrap();
    session.prepare.submit(&stale).unwrap();
    for _ in 0..20 {
        session.host.render(128);
    }
    session.host.write_feedback();
    let fb = read_feedback(session.host.feedback()).expect("a whole report");
    assert_eq!(fb.epoch, 1);
    assert_eq!(fb.seq % 2, 0);
    assert_eq!(fb.rejected, 1);
    assert_eq!(fb.meters.len(), 1, "one track meter");
    assert_eq!(
        fb.master.peak_l, fb.meters[0].peak_l,
        "unity master: same signal"
    );
    let m = fb.meters[0];
    assert!(m.peak_l > 0.05 && m.peak_r > 0.05);
    assert!(m.rms_l > 0.0 && m.rms_l <= m.peak_l);
    assert_eq!(session.host.voices(), 1);
    assert_eq!(feedback_voices(session.host.feedback()), 1);
    // A torn write (odd seq) reads as nothing.
    let region = session.host.feedback();
    let seq = region[1].load(std::sync::atomic::Ordering::Relaxed);
    region[1].store(
        seq | u32::from_ne_bytes(1u32.to_le_bytes()),
        std::sync::atomic::Ordering::Relaxed,
    );
    assert!(read_feedback(region).is_none());
}

#[test]
fn a_full_live_queue_releases_every_note() {
    let mut session = Session::new();
    session.send(&[note_on(60, 100)]);
    session.host.render(128);
    // Fill the live queue: the note-off that follows is lost and counted.
    let mut msgs: Vec<M> = (0..limits::LIVE_QUEUE_CAPACITY)
        .map(|_| note_on(72, 100))
        .collect();
    msgs.push(M::NoteOff {
        track: TrackSlot(0),
        pitch: 60,
        at: None,
    });
    session.send(&msgs);
    // Release time 0.3 s: after a second, silence.
    for _ in 0..400 {
        session.host.render(128);
    }
    assert!(session.host.left()[..128].iter().all(|s| *s == 0.0));
    session.host.write_feedback();
    assert!(
        read_feedback(session.host.feedback())
            .unwrap()
            .live_overflows
            > 0
    );
}

/// The two tracks of the meter tests: a synth each, track 0 at unity and track 1 at `gain`.
fn two_tracks(gain: f32) -> Session {
    let mut session = Session::new();
    session.send(&[
        M::TrackCreate {
            track: TrackSlot(1),
            kind: TrackKind::Midi,
        },
        M::DeviceInsert {
            track: TrackSlot(1),
            device: DeviceSlot(0),
            kind: OSC_SYNTH.kind_id,
            state: Vec::new(),
        },
        M::TrackSetMix {
            track: TrackSlot(1),
            gain,
            pan: 0.0,
            mute: false,
        },
        note_on(69, 127),
        M::NoteOn {
            track: TrackSlot(1),
            pitch: 69,
            velocity: 127,
            at: None,
        },
    ]);
    // The fades settle (5 ms), then a few reporting periods.
    for _ in 0..60 {
        session.host.render(128);
    }
    session
}

/// A report of a whole period rendered after `session` settled.
fn report_of(session: &mut Session) -> engine::msg::Feedback {
    session.host.write_feedback();
    for _ in 0..12 {
        session.host.render(128);
    }
    session.host.write_feedback();
    read_feedback(session.host.feedback()).expect("a whole report")
}

#[test]
fn the_meters_follow_each_tracks_fader_and_the_master() {
    let mut session = two_tracks(0.5);
    let fb = report_of(&mut session);
    assert_eq!(fb.meters.len(), 2);
    let (a, b) = (fb.meters[0], fb.meters[1]);
    assert!(a.peak_l > 0.05, "track 0 sounds: {a:?}");
    // Same note and velocity on both: the second is the first through a 0.5 fader.
    assert!((b.peak_l - a.peak_l * 0.5).abs() < 1e-3, "{a:?} {b:?}");
    assert!((b.rms_l - a.rms_l * 0.5).abs() < 1e-3, "{a:?} {b:?}");
    for m in [a, b, fb.master] {
        assert!(m.peak_l.is_finite() && m.rms_l.is_finite());
        assert!(m.rms_l <= m.peak_l && m.rms_r <= m.peak_r);
        assert_eq!(m.peak_l, m.peak_r, "centered");
    }
    // The master is the sum of the tracks (identical waveforms, so the peaks add).
    assert!(
        (fb.master.peak_l - (a.peak_l + b.peak_l)).abs() < 1e-3,
        "{fb:?}"
    );
    assert_eq!(fb.voices, 2);
}

#[test]
fn a_muted_track_and_an_empty_slot_meter_zero_and_master_gain_applies() {
    let mut session = two_tracks(1.0);
    session.send(&[
        M::TrackSetMix {
            track: TrackSlot(1),
            gain: 1.0,
            pan: 0.0,
            mute: true,
        },
        M::SetMaster {
            gain: 0.25,
            pan: 0.0,
            mute: false,
        },
        // A track far beyond: the slots between are empty.
        M::TrackCreate {
            track: TrackSlot(5),
            kind: TrackKind::Midi,
        },
    ]);
    for _ in 0..60 {
        session.host.render(128);
    }
    let fb = report_of(&mut session);
    assert_eq!(fb.meters.len(), 6, "up to the last track");
    assert!(fb.meters[0].peak_l > 0.0);
    for slot in [1, 2, 3, 4, 5] {
        assert_eq!(fb.meters[slot], Default::default(), "slot {slot}");
    }
    // After the master: a quarter of the audible track.
    assert!(
        (fb.master.peak_l - fb.meters[0].peak_l * 0.25).abs() < 1e-3,
        "{fb:?}"
    );
}

#[test]
fn silence_meters_zero_and_the_period_restarts() {
    let mut session = Session::new();
    for _ in 0..20 {
        session.host.render(128);
    }
    let fb = report_of(&mut session);
    assert_eq!(fb.meters[0], Default::default());
    assert_eq!(fb.master, Default::default());
    assert_eq!(fb.voices, 0);
    // A note, then its period passes: the next report starts from a fresh period.
    session.send(&[note_on(69, 127)]);
    let loud = report_of(&mut session);
    assert!(loud.master.peak_l > 0.05);
    session.send(&[
        M::AllNotesOff {
            track: TrackSlot(0),
        },
        M::AllNotesOff {
            track: TrackSlot(1),
        },
    ]);
    for _ in 0..400 {
        session.host.render(128);
    }
    let quiet = report_of(&mut session);
    assert_eq!(quiet.master, Default::default());
}

#[test]
fn a_stop_followed_by_a_seek_leaves_the_playhead_on_the_cue() {
    // What `transport.stop` sends (#360): the head is on the cue, whichever blocks the two land in.
    let mut session = Session::new();
    session.send(&[M::Play { from: Tick(3840) }]);
    for _ in 0..40 {
        session.host.render(128);
    }
    assert!(report_of(&mut session).playhead_tick > Tick(3840));
    session.send(&[M::Stop, M::Seek { tick: Tick(3840) }]);
    session.host.render(128);
    let stopped = report_of(&mut session);
    assert_eq!(stopped.transport, TransportState::Stopped);
    assert_eq!(stopped.playhead_tick, Tick(3840));
    session.host.render(128);
    assert_eq!(
        report_of(&mut session).playhead_tick,
        Tick(3840),
        "it stays"
    );
    // A second stop goes to the start.
    session.send(&[M::Stop, M::Seek { tick: Tick(0) }]);
    session.host.render(128);
    assert_eq!(report_of(&mut session).playhead_tick, Tick(0));
    // Play restarts from where the head is shown.
    session.send(&[M::Play { from: Tick(0) }]);
    session.host.render(128);
    let playing = report_of(&mut session);
    assert_eq!(playing.transport, TransportState::Playing);
    assert!(playing.playhead_tick >= Tick(0) && playing.playhead_tick < Tick(960));
}

#[test]
fn the_report_carries_the_transport_and_the_playhead() {
    let mut session = Session::new();
    let fb = report_of(&mut session);
    assert_eq!(fb.transport, TransportState::Stopped);
    assert_eq!(fb.playhead_tick, Tick(0));
    session.send(&[M::Play { from: Tick(0) }]);
    let a = report_of(&mut session);
    assert_eq!(a.transport, TransportState::Playing);
    let b = report_of(&mut session);
    assert!(b.playhead_tick > a.playhead_tick, "{a:?} {b:?}");
    assert!(b.playhead_frame.0 > a.playhead_frame.0);
    session.send(&[M::Stop]);
    session.host.render(128);
    let stopped = report_of(&mut session);
    assert_eq!(stopped.transport, TransportState::Stopped);
}

/// One 128-frame block timed as `seconds`, the way the helper and the worklet do it: the
/// render (which may write a report), then its time.
fn timed_block(session: &mut Session, seconds: f32) {
    let frames = session.host.render(128);
    session.host.add_render_time(seconds, frames as u32);
}

/// The load and peak of the report written now.
fn loads(session: &mut Session) -> (f32, f32) {
    session.host.write_feedback();
    let report = read_feedback(session.host.feedback()).unwrap();
    (report.cpu_load, report.cpu_peak)
}

/// 128 frames at 48 kHz last 1/375 s.
const BLOCK: f32 = 128.0 / RATE;

#[test]
fn the_cpu_load_is_the_timed_render_time_over_the_timed_frames() {
    let mut session = Session::new();
    // Never timed: not measured, load and peak.
    let report = report_of(&mut session);
    assert_eq!(report.cpu_load, engine::host::NOT_MEASURED);
    assert_eq!(report.cpu_peak, engine::host::NOT_MEASURED);
    // Ten blocks at 25 % of their duration: 25 %, the peak too.
    session.host.write_feedback();
    for _ in 0..10 {
        timed_block(&mut session, BLOCK / 4.0);
    }
    let (load, peak) = loads(&mut session);
    assert!((load - 0.25).abs() < 1e-4, "{load}");
    assert!((peak - 0.25).abs() < 1e-4, "{peak}");
    // A period with no timed block after that: load 0, not "not measured"; the peak is the
    // last window closed.
    assert_eq!(loads(&mut session), (0.0, 0.25));
    // Not finite or negative times count as 0 with their frames; over budget saturates.
    session.host.add_render_time(f32::NAN, 128);
    session.host.add_render_time(-1.0, 128);
    session.host.add_render_time(f32::INFINITY, 128);
    session.host.add_render_time(BLOCK * 3.0 / 4.0, 128);
    let (load, peak) = loads(&mut session);
    assert!((load - 0.75 / 4.0).abs() < 1e-4, "{load}");
    assert!((peak - 0.75).abs() < 1e-4, "{peak}");
    session.host.add_render_time(10.0, 128);
    assert_eq!(loads(&mut session), (1.0, 1.0));
}

#[test]
fn a_block_timed_after_the_render_that_reported_counts_in_the_next_report() {
    let mut session = Session::new();
    session.host.write_feedback();
    // 1 600 frames make a report: 12 blocks of 128 do not, the 13th writes it inside its
    // render, before its own time is added.
    for _ in 0..12 {
        timed_block(&mut session, BLOCK / 10.0);
    }
    let seq = read_feedback(session.host.feedback()).unwrap().seq;
    timed_block(&mut session, BLOCK);
    let report = read_feedback(session.host.feedback()).unwrap();
    assert_ne!(report.seq, seq, "the 13th block wrote a report");
    assert!((report.cpu_load - 0.1).abs() < 1e-4, "{}", report.cpu_load);
    assert!((report.cpu_peak - 0.1).abs() < 1e-4, "{}", report.cpu_peak);
    // The 13th block's time is in the next report, with its own frames.
    let (load, peak) = loads(&mut session);
    assert!((load - 1.0).abs() < 1e-4, "{load}");
    assert!((peak - 1.0).abs() < 1e-4, "{peak}");
}

#[test]
fn with_a_fine_clock_the_peak_is_the_worst_block() {
    let mut session = Session::new();
    session.host.write_feedback();
    // Eleven light blocks and one at 90 %: the mean hides it, the peak shows it.
    for i in 0..12 {
        timed_block(&mut session, if i == 5 { BLOCK * 0.9 } else { BLOCK * 0.1 });
    }
    let (load, peak) = loads(&mut session);
    assert!((load - (11.0 * 0.1 + 0.9) / 12.0).abs() < 1e-4, "{load}");
    assert!((peak - 0.9).abs() < 1e-4, "{peak}");
    // The peak starts again with the period.
    for _ in 0..3 {
        timed_block(&mut session, BLOCK * 0.2);
    }
    let (_, peak) = loads(&mut session);
    assert!((peak - 0.2).abs() < 1e-4, "{peak}");
}

#[test]
fn with_a_coarse_clock_the_peak_is_taken_over_windows_of_its_steps() {
    let mut session = Session::new();
    // 1 ms steps: a window lasts at least 100 ms, 38 blocks of 2.67 ms.
    session.host.set_clock_resolution(0.001);
    session.host.write_feedback();
    // Before any window closed, the peak is the period's load.
    for _ in 0..20 {
        session.host.add_render_time(0.001, 128);
    }
    let (load, peak) = loads(&mut session);
    assert!((load - 0.001 / BLOCK).abs() < 1e-4, "{load}");
    assert_eq!(peak, load);
    // The open window carries over the report and closes at its 38th block: 20 ms of
    // render over 38 blocks, not one block's 37 %.
    for _ in 0..17 {
        session.host.add_render_time(0.0, 128);
    }
    let (load, peak) = loads(&mut session);
    assert_eq!(load, 0.0);
    assert_eq!(
        peak, 0.0,
        "37 blocks: no window closed yet, the peak is the load"
    );
    session.host.add_render_time(0.0, 128);
    let window = 0.020 / (38.0 * BLOCK);
    let (load, peak) = loads(&mut session);
    assert_eq!(load, 0.0);
    assert!((peak - window).abs() < 1e-4, "{peak}");
    // A period in which no window closes reports the last window, not its own noisy load.
    session.host.add_render_time(0.002, 128);
    let (load, peak) = loads(&mut session);
    assert!((load - 0.002 / BLOCK).abs() < 1e-4, "{load}");
    assert!((peak - window).abs() < 1e-4, "{peak}");
}

#[test]
fn a_resolution_that_is_not_finite_or_negative_is_a_fine_clock() {
    for resolution in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        let mut session = Session::new();
        session.host.set_clock_resolution(resolution);
        session.host.write_feedback();
        session.host.add_render_time(BLOCK * 0.1, 128);
        session.host.add_render_time(BLOCK * 0.5, 128);
        let (load, peak) = loads(&mut session);
        assert!((load - 0.3).abs() < 1e-4, "{resolution}: {load}");
        assert!((peak - 0.5).abs() < 1e-4, "{resolution}: {peak}");
    }
}

#[test]
fn timing_rendering_and_reporting_never_allocates() {
    let mut session = two_tracks(0.7);
    session.host.set_clock_resolution(0.001);
    session.send(&[M::Play { from: Tick(0) }]);
    session.host.render(128);
    assert_no_alloc(|| {
        for block in 0..2_000u32 {
            let frames = session.host.render(1 + (block as usize * 37) % 512);
            session
                .host
                .add_render_time(block as f32 * 1e-6, frames as u32);
        }
        session.host.write_feedback();
    });
    let report = read_feedback(session.host.feedback()).unwrap();
    assert!((0.0..=1.0).contains(&report.cpu_load));
    assert!((0.0..=1.0).contains(&report.cpu_peak));
}

/// The cost of the measurement itself (#486), the helper's way: two `Instant` reads and
/// `add_render_time` per block, against the render of the blocks it times (two tracks of
/// eight voices). Prints the numbers; run optimised (debug assertions and the allocation
/// detector stay on, as in the compressor's `cpu_cost`):
/// `cargo test -p engine --test it host::measurement_overhead --config "profile.dev.opt-level=3" -- --ignored --nocapture`
#[test]
#[ignore]
fn measurement_overhead() {
    use std::time::Instant;
    let mut session = two_tracks(0.7);
    session.send(
        &(0..14u8)
            .map(|i| M::NoteOn {
                track: TrackSlot(u32::from(i % 2)),
                pitch: 40 + i * 3,
                velocity: 100,
                at: None,
            })
            .collect::<Vec<_>>(),
    );
    session.host.render(128);
    const BLOCKS: u32 = 200_000;
    let mut best = (f64::MAX, f64::MAX);
    for _ in 0..5 {
        let start = Instant::now();
        for _ in 0..BLOCKS {
            session.host.render(128);
        }
        let plain = start.elapsed().as_secs_f64() / f64::from(BLOCKS);
        let start = Instant::now();
        for _ in 0..BLOCKS {
            let t = Instant::now();
            let n = session.host.render(128);
            session
                .host
                .add_render_time(t.elapsed().as_secs_f32(), n as u32);
        }
        let timed = start.elapsed().as_secs_f64() / f64::from(BLOCKS);
        best = (best.0.min(plain), best.1.min(timed));
    }
    let start = Instant::now();
    for _ in 0..BLOCKS {
        let t = Instant::now();
        session.host.add_render_time(t.elapsed().as_secs_f32(), 128);
    }
    let alone = start.elapsed().as_secs_f64() / f64::from(BLOCKS);
    let budget = 128.0 / f64::from(RATE);
    println!(
        "voices {}; render {:.0} ns/block, timed {:.0} ns/block (difference {:.0} ns); \
         two clock reads + add_render_time alone {:.1} ns/block = {:.4} % of the 2.67 ms budget",
        session.host.voices(),
        best.0 * 1e9,
        best.1 * 1e9,
        (best.1 - best.0) * 1e9,
        alone * 1e9,
        alone / budget * 100.0
    );
}

#[test]
fn a_report_is_copied_as_bytes_and_decodes_alike() {
    let mut session = two_tracks(0.5);
    session.host.write_feedback();
    let mut bytes = vec![0xAA; 3];
    assert!(copy_feedback(session.host.feedback(), &mut bytes));
    assert_eq!(
        bytes.len(),
        HEADER + 3 * 20,
        "header, two tracks, the output"
    );
    assert_eq!(bytes[0], engine::msg::FEEDBACK_VERSION);
    let decoded = engine::host::decode_feedback(&bytes).unwrap();
    assert_eq!(decoded, read_feedback(session.host.feedback()).unwrap());
    // Truncated or of another version: nothing.
    assert!(engine::host::decode_feedback(&bytes[..bytes.len() - 1]).is_none());
    bytes[0] = 10;
    assert!(engine::host::decode_feedback(&bytes).is_none());
    // Torn (odd seq): `false`, the buffer cleared of a half copy.
    let region = session.host.feedback();
    let seq = region[1].load(std::sync::atomic::Ordering::Relaxed);
    region[1].store(
        seq | u32::from_ne_bytes(1u32.to_le_bytes()),
        std::sync::atomic::Ordering::Relaxed,
    );
    assert!(!copy_feedback(region, &mut bytes));
    // No report yet.
    let (_prepare, fresh) = channel(RATE).unwrap();
    assert!(!copy_feedback(fresh.feedback(), &mut bytes));
}

/// The prepare side names the tracks it holds (BO-1: the helper releases their notes without
/// knowing the project): added, removed, and none after a `Reset`.
#[test]
fn the_prepare_side_names_the_tracks_it_holds() {
    let mut session = two_tracks(1.0);
    let slots = |s: &Session| s.prepare.preparer().track_slots().collect::<Vec<_>>();
    assert_eq!(slots(&session), [TrackSlot(0), TrackSlot(1)]);
    session.send(&[
        M::TrackCreate {
            track: TrackSlot(5),
            kind: TrackKind::Midi,
        },
        M::TrackRemove {
            track: TrackSlot(0),
        },
    ]);
    assert_eq!(slots(&session), [TrackSlot(1), TrackSlot(5)]);
    let bytes = encode_stream(2, 0, &[M::Reset { epoch: 2 }]).unwrap();
    session.prepare.submit(&bytes).unwrap();
    assert_eq!(slots(&session), []);
}

#[test]
fn rendering_and_reporting_with_tracks_never_allocates() {
    let mut session = two_tracks(0.7);
    session.send(&[M::Play { from: Tick(0) }]);
    session.host.render(128);
    let mut bytes = Vec::with_capacity(limits::MAX_TRACKS as usize * 20 + 64);
    // 2 000 blocks: many reports (every 1 600 frames), with the clock running.
    assert_no_alloc(|| {
        for block in 0..2_000u32 {
            session.host.render(1 + (block as usize * 37) % 512);
            assert!(session.host.voices() <= 2);
        }
        session.host.write_feedback();
        assert!(copy_feedback(session.host.feedback(), &mut bytes));
    });
}

#[test]
fn the_output_meter_includes_the_metronome_and_the_track_meters_do_not() {
    let mut session = Session::new();
    session.send(&[
        M::SetMetronome {
            enabled: true,
            volume: 1.0,
            count_in_bars: 0,
        },
        M::Play { from: Tick(0) },
    ]);
    for _ in 0..8 {
        session.host.render(128);
    }
    // A beat at the default tempo is 0.5 s (40 reports of 32 ms): a click passes.
    let mut master = 0.0f32;
    for _ in 0..40 {
        let fb = report_of(&mut session);
        assert_eq!(fb.meters[0], Default::default(), "no track sounds");
        master = master.max(fb.master.peak_l);
    }
    assert!(
        master > 0.01,
        "the click reaches the output meter: {master}"
    );
}

#[test]
fn a_fader_ramp_is_metered_as_it_plays_not_at_its_target() {
    let mut session = two_tracks(1.0);
    let settled = report_of(&mut session);
    let steady = settled.meters[1].peak_l;
    assert!(steady > 0.05);
    // Track 1 to silence: its fade lasts 5 ms (240 frames); 64 frames are rendered.
    session.host.write_feedback();
    session.send(&[M::TrackSetMix {
        track: TrackSlot(1),
        gain: 0.0,
        pan: 0.0,
        mute: false,
    }]);
    session.host.render(64);
    session.host.write_feedback();
    let ramp = read_feedback(session.host.feedback()).unwrap().meters[1];
    assert!(
        ramp.peak_l > 0.0,
        "still audible at the ramp's start: {ramp:?}"
    );
    assert!(
        ramp.peak_l < steady * 1.001,
        "never louder than before: {ramp:?}"
    );
    assert!(ramp.rms_l < steady, "the ramp lowers the level: {ramp:?}");
    // Past the ramp the track meters silence.
    for _ in 0..4 {
        session.host.render(128);
    }
    session.host.write_feedback();
    for _ in 0..12 {
        session.host.render(128);
    }
    session.host.write_feedback();
    assert_eq!(
        read_feedback(session.host.feedback()).unwrap().meters[1].peak_l,
        0.0
    );
}

/// The cells of the launch grid that are not idle (GR-1) travel in the report after the
/// meters: written, copied as bytes and decoded alike, without any allocation on the audio
/// side; a report that is cut short, claims more cells than the room or names an unknown
/// state does not decode.
#[test]
fn the_launch_grid_states_travel_in_the_report() {
    use engine::msg::{ActiveClip, ClipSlot, ClipState, Note, SceneSlot};
    use engine::time::LoopRegion;
    let mut session = Session::new();
    session.send(&[
        M::ClipNotesUpload {
            clip: ClipSlot(0),
            length: Tick(3840),
            looping: LoopRegion::new(Tick(0), Tick(3840)),
            first: 0,
            total: 1,
            notes: vec![Note {
                start: Tick(0),
                length: Tick(960),
                pitch: 60,
                velocity: 100,
                ..Note::default()
            }],
        },
        M::SceneSetSlot {
            track: TrackSlot(0),
            scene: SceneSlot(3),
            clip: Some(ClipSlot(0)),
        },
        M::LaunchClip {
            track: TrackSlot(0),
            scene: SceneSlot(3),
        },
    ]);
    assert_no_alloc(|| {
        for _ in 0..13 {
            session.host.render(128);
        }
        session.host.write_feedback();
    });
    let cell = |state, anchor| ActiveClip {
        track: TrackSlot(0),
        scene: SceneSlot(3),
        state,
        anchor: Tick(anchor),
    };
    let armed = read_feedback(session.host.feedback()).unwrap();
    assert_eq!(armed.active_clips, vec![cell(ClipState::Armed, 0)]);
    let mut bytes = Vec::new();
    assert!(copy_feedback(session.host.feedback(), &mut bytes));
    assert_eq!(
        bytes.len(),
        HEADER + 2 * 20 + 16,
        "header, a track and the output, a cell"
    );
    assert_eq!(&bytes[44..48], &1u32.to_le_bytes());
    assert_eq!(engine::host::decode_feedback(&bytes).unwrap(), armed);

    // Playing from the middle of a bar: the clip starts on the next bar (tick 3840), which the
    // report names as the clip's position 0. Rendered and reported under the detector.
    session.send(&[M::Play { from: Tick(3000) }]);
    session.host.render(128);
    let mut bytes = Vec::with_capacity(4096);
    assert_no_alloc(|| {
        for _ in 0..300 {
            session.host.render(128);
        }
        session.host.write_feedback();
        assert!(copy_feedback(session.host.feedback(), &mut bytes));
    });
    assert_eq!(
        engine::host::decode_feedback(&bytes).unwrap().active_clips,
        vec![cell(ClipState::Playing, 3840)]
    );
    session.send(&[M::StopAll]);
    assert_no_alloc(|| {
        session.host.render(128);
        session.host.write_feedback();
    });
    // A stop waits for its boundary (the next bar).
    assert_eq!(
        read_feedback(session.host.feedback()).unwrap().active_clips,
        vec![cell(ClipState::Stopping, 3840)]
    );

    // Refused: cut short, more cells than two per track, an unknown state.
    assert!(copy_feedback(session.host.feedback(), &mut bytes));
    // The anchor travels as a little-endian i64 after the cell's first eight bytes.
    let mut anchored = bytes.clone();
    for anchor in [3840i64, -960, i64::MAX, i64::MIN] {
        let at = HEADER + 2 * 20 + 8;
        anchored[at..at + 8].copy_from_slice(&anchor.to_le_bytes());
        let cells = engine::host::decode_feedback(&anchored)
            .unwrap()
            .active_clips;
        assert_eq!(cells[0].anchor, Tick(anchor));
    }
    assert!(engine::host::decode_feedback(&bytes[..bytes.len() - 1]).is_none());
    let mut many = bytes.clone();
    many[44..48].copy_from_slice(&3u32.to_le_bytes());
    assert!(engine::host::decode_feedback(&many).is_none());
    let mut unknown = bytes.clone();
    let state_at = HEADER + 2 * 20 + 6;
    for state in [0u8, 6, 255] {
        unknown[state_at] = state;
        assert!(engine::host::decode_feedback(&unknown).is_none(), "{state}");
    }
    // Past the stop's boundary the grid holds the track with no clip: a Held cell in no scene,
    // through the writer and the decoder.
    for _ in 0..1200 {
        session.host.render(128);
    }
    session.host.write_feedback();
    let mut held = Vec::new();
    assert!(copy_feedback(session.host.feedback(), &mut held));
    assert_eq!(
        engine::host::decode_feedback(&held).unwrap().active_clips,
        vec![ActiveClip {
            track: engine::msg::TrackSlot(0),
            scene: engine::msg::HELD_SCENE,
            state: ClipState::Held,
            anchor: Tick(0),
        }]
    );
    assert_eq!(held[HEADER + 2 * 20 + 6], ClipState::Held as u8);
    // A report with no cell (what an older engine writes) decodes with none.
    let mut none = bytes[..HEADER + 2 * 20].to_vec();
    none[44..48].copy_from_slice(&0u32.to_le_bytes());
    assert!(
        engine::host::decode_feedback(&none)
            .unwrap()
            .active_clips
            .is_empty()
    );
}

// ------------------------------------------------------------ gain reduction (#469)

/// The slot of the compressor in a strip: after the synth on the track, first on the master.
fn compressor_slot(track: TrackSlot) -> DeviceSlot {
    DeviceSlot(if track == TrackSlot::MASTER { 0 } else { 1 })
}

/// A track with the synth and a Compressor on it, and a Compressor on the master, both at a
/// threshold the synth's note exceeds; a compressor can be left disabled.
fn compressed(track_on: bool, master_on: bool) -> Session {
    use engine::devices::compressor::{DESCRIPTOR as COMPRESSOR, RATIO, THRESHOLD};
    let mut session = two_tracks(1.0);
    let mut msgs = Vec::new();
    for (track, on) in [(TrackSlot(1), track_on), (TrackSlot::MASTER, master_on)] {
        let device = compressor_slot(track);
        msgs.push(M::DeviceInsert {
            track,
            device,
            kind: COMPRESSOR.kind_id,
            state: Vec::new(),
        });
        for (param, value) in [(THRESHOLD, -60.0), (RATIO, 20.0)] {
            msgs.push(M::ParamSet {
                track,
                device,
                param: ParamId(param),
                value,
            });
        }
        if !on {
            msgs.push(M::DeviceSetEnabled {
                track,
                device,
                enabled: false,
            });
        }
    }
    session.send(&msgs);
    for _ in 0..200 {
        session.host.render(128);
    }
    session
}

#[test]
fn a_compressor_over_its_threshold_reports_its_gain_reduction_on_its_strip() {
    // On the track only: the track reports a reduction, the master (no enabled compressor) none.
    let mut session = compressed(true, false);
    let report = report_of(&mut session);
    assert!(
        report.meters[1].gain_reduction_db > 6.0,
        "{:?}",
        report.meters[1]
    );
    assert!(report.meters[1].gain_reduction_db <= 60.0);
    assert_eq!(report.master.gain_reduction_db, 0.0);
    assert_eq!(report.meters[0].gain_reduction_db, 0.0, "an empty slot");
    // On the master only: the disabled compressor of the track reports nothing.
    let mut session = compressed(false, true);
    let report = report_of(&mut session);
    assert_eq!(report.meters[1].gain_reduction_db, 0.0);
    assert!(report.master.gain_reduction_db > 6.0, "{:?}", report.master);
    // No compressor: no reduction.
    let mut plain = two_tracks(1.0);
    let report = report_of(&mut plain);
    assert_eq!(report.meters[1].gain_reduction_db, 0.0);
    assert_eq!(report.master.gain_reduction_db, 0.0);
}

#[test]
fn the_reduction_travels_in_the_report_bytes() {
    let mut session = compressed(true, true);
    session.host.write_feedback();
    for _ in 0..12 {
        session.host.render(128);
    }
    session.host.write_feedback();
    let mut bytes = Vec::new();
    assert!(copy_feedback(session.host.feedback(), &mut bytes));
    let decoded = engine::host::decode_feedback(&bytes).unwrap();
    assert!(decoded.meters[1].gain_reduction_db > 6.0);
    assert!(decoded.master.gain_reduction_db > 6.0);
    // The track's reduction is the fifth float of its meter (the slot 0 meter comes first).
    let at = HEADER + 20 + 16;
    let wire = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(wire, decoded.meters[1].gain_reduction_db);
    // A report of the previous version is refused by its first byte.
    bytes[0] = 3;
    assert!(engine::host::decode_feedback(&bytes).is_none());
}

/// #667: the live report carries the frames the output protection silenced because a sample
/// was non-finite, cumulative; the output stays finite; nothing on that path allocates.
#[test]
fn the_report_counts_the_frames_silenced_for_a_non_finite_sample() {
    let mut session = Session::new();
    session.send(&[M::Play { from: Tick(0) }, note_on(60, 100)]);
    session.host.render(128);
    session.host.write_feedback();
    assert_eq!(
        read_feedback(session.host.feedback())
            .unwrap()
            .silenced_frames,
        0
    );

    // 3 frames turned NaN before the protection: 3 silenced, the output finite.
    session.host.engine_mut().poison_next_frames(3);
    assert_no_alloc(|| {
        session.host.render(128);
        session.host.write_feedback();
    });
    assert!(
        session.host.left()[..128]
            .iter()
            .chain(&session.host.right()[..128])
            .all(|s| s.is_finite())
    );
    assert_eq!(
        session.host.left()[..3],
        [0.0; 3],
        "the NaN frames are silent"
    );
    assert_eq!(
        read_feedback(session.host.feedback())
            .unwrap()
            .silenced_frames,
        3
    );

    // Armed over two blocks: 200 more, split 128 + 72; cumulative.
    session.host.engine_mut().poison_next_frames(200);
    session.host.render(128);
    session.host.render(128);
    session.host.write_feedback();
    assert_eq!(
        read_feedback(session.host.feedback())
            .unwrap()
            .silenced_frames,
        203
    );

    // A clean block, a report period and an all-sound-off keep the count: it is cumulative.
    session.send(&[M::AllSoundOff]);
    for _ in 0..40 {
        session.host.render(128);
    }
    session.host.write_feedback();
    assert_eq!(
        read_feedback(session.host.feedback())
            .unwrap()
            .silenced_frames,
        203
    );
}

/// The count saturates in the report's 32-bit word.
#[test]
fn the_silenced_word_saturates() {
    use engine::host::silenced_word;
    assert_eq!(silenced_word(0), 0);
    assert_eq!(silenced_word(131), 131);
    assert_eq!(silenced_word(u64::from(u32::MAX)), u32::MAX);
    assert_eq!(silenced_word(u64::MAX), u32::MAX);
}

/// The report carries the host's frame at the playhead (#733, version 9): the clock given to `set_host_frame`
/// carried to the playhead's sample; the engine's own counter when none was given.
#[test]
fn the_report_carries_the_host_frame_at_the_playhead() {
    let mut session = Session::new();
    session.host.render(128);
    session.host.write_feedback();
    let own = read_feedback(session.host.feedback()).unwrap();
    assert_eq!(own.host_frame, 128, "no host clock: the engine's counter");
    // A host clock far from the engine's counter, advancing a block at a time like the worklet's currentFrame.
    const ORIGIN: u64 = 1_000_000;
    let mut last = 0;
    for block in 0..4u64 {
        session.host.set_host_frame(ORIGIN + 128 * (block + 1));
        session.host.render(128);
        session.host.write_feedback();
        let fb = read_feedback(session.host.feedback()).unwrap();
        // The report sits at the start of the next block: one block after the one rendered.
        assert_eq!(fb.host_frame, ORIGIN + 128 * (block + 2));
        assert!(fb.host_frame > last);
        last = fb.host_frame;
    }
}

/// A report of version 7 (the header of 88 bytes, without the silenced frames) is refused, and so is one of version 8.
#[test]
fn a_report_of_version_7_is_refused() {
    let mut session = Session::new();
    session.host.render(128);
    session.host.write_feedback();
    let mut bytes = Vec::new();
    assert!(copy_feedback(session.host.feedback(), &mut bytes));
    assert!(engine::host::decode_feedback(&bytes).is_some());
    bytes[0] = 7;
    assert!(engine::host::decode_feedback(&bytes).is_none());
    bytes[0] = 8;
    assert!(engine::host::decode_feedback(&bytes).is_none());
}

// ------------------------------------------------------------ device readouts (#686)

/// A Filter after the synth of track 1 whose LFO swings the cutoff, and a still one on the
/// master; the track's filter can be left disabled.
fn filtered(track_on: bool) -> Session {
    use engine::devices::filter::{CUTOFF, DESCRIPTOR as FILTER, LFO_DEPTH};
    let mut session = two_tracks(1.0);
    let mut msgs = Vec::new();
    for (track, device, depth) in [
        (TrackSlot(1), DeviceSlot(1), 2.0),
        (TrackSlot::MASTER, DeviceSlot(0), 0.0),
    ] {
        msgs.push(M::DeviceInsert {
            track,
            device,
            kind: FILTER.kind_id,
            state: Vec::new(),
        });
        for (param, value) in [(CUTOFF, 500.0), (LFO_DEPTH, depth)] {
            msgs.push(M::ParamSet {
                track,
                device,
                param: ParamId(param),
                value,
            });
        }
    }
    if !track_on {
        msgs.push(M::DeviceSetEnabled {
            track: TrackSlot(1),
            device: DeviceSlot(1),
            enabled: false,
        });
    }
    session.send(&msgs);
    for _ in 0..50 {
        session.host.render(128);
    }
    session
}

#[test]
fn a_filters_effective_cutoff_travels_in_the_report_with_its_slots() {
    use engine::devices::filter::CUTOFF;
    let mut session = filtered(true);
    session.host.write_feedback();
    let mut bytes = Vec::new();
    assert!(copy_feedback(session.host.feedback(), &mut bytes));
    let decoded = engine::host::decode_feedback(&bytes).unwrap();
    assert_eq!(decoded.readouts.len(), 2, "{:?}", decoded.readouts);
    let track = decoded.readouts[0];
    assert_eq!(
        (track.track, track.device, track.param),
        (TrackSlot(1), DeviceSlot(1), ParamId(CUTOFF))
    );
    // The synced triangle has moved the cutoff off its centre (the transport is stopped at
    // tick 0, where the triangle is 0: the swing is what the LFO holds there).
    assert!(track.value.is_finite() && track.value >= 20.0, "{track:?}");
    let master = decoded.readouts[1];
    assert_eq!(
        (master.track, master.device),
        (TrackSlot::MASTER, DeviceSlot(0))
    );
    assert_eq!(master.value, 500.0, "no movement: the centre");
    // The count word and the bytes after the notes.
    assert_eq!(u32::from_le_bytes(bytes[100..104].try_into().unwrap()), 2);
    // A disabled device reports nothing.
    let mut off = filtered(false);
    off.host.write_feedback();
    let report = engine::host::read_feedback(off.host.feedback()).unwrap();
    assert_eq!(report.readouts.len(), 1);
    assert_eq!(report.readouts[0].track, TrackSlot::MASTER);
    // A report cut before its readouts is refused.
    bytes.truncate(bytes.len() - 1);
    assert!(engine::host::decode_feedback(&bytes).is_none());
}

/// #686: the master's chain runs cut at the clock's segments, so a Filter on the master whose
/// LFO follows the transport sees a loop's wrap at its frame: the render does not depend on the
/// block size, with the wrap in the middle of a block.
#[test]
fn a_master_filter_sees_a_loop_wrap_inside_a_block_whatever_the_block_size() {
    use engine::devices::filter::{CUTOFF, DESCRIPTOR as FILTER, LFO_DEPTH, RESONANCE};
    use engine::time::LoopRegion;
    let render = |block: usize| {
        let mut session = Session::new();
        let mut msgs = vec![M::DeviceInsert {
            track: TrackSlot::MASTER,
            device: DeviceSlot(0),
            kind: FILTER.kind_id,
            state: Vec::new(),
        }];
        for (p, value) in [(CUTOFF, 600.0), (LFO_DEPTH, 3.0), (RESONANCE, 0.7)] {
            msgs.push(M::ParamSet {
                track: TrackSlot::MASTER,
                device: DeviceSlot(0),
                param: ParamId(p),
                value,
            });
        }
        msgs.push(param(Param::Waveform, 1.0));
        msgs.push(note_on(45, 127));
        // A loop of 4081 ticks: 102 025 frames at 120 BPM (the triangle at its top when it wraps to
        // 0), its wrap inside a block of any of the
        // sizes below.
        msgs.push(M::SetLoop {
            region: Some(LoopRegion::new(Tick(0), Tick(4081)).unwrap()),
        });
        msgs.push(M::Play { from: Tick(0) });
        session.send(&msgs);
        let mut out = Vec::new();
        let total = 2 * 102_025 + 20_000;
        while out.len() < 2 * total {
            let n = session.host.render(block);
            for i in 0..n {
                out.push(session.host.left()[i]);
                out.push(session.host.right()[i]);
            }
        }
        out.truncate(2 * total);
        out
    };
    let reference = render(128);
    for block in [100, 1000] {
        let other = render(block);
        let worst = reference
            .iter()
            .zip(&other)
            .fold(0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 1e-5, "blocks of {block}: differs by {worst}");
    }
}
