//! The shipping host (MO-2, `engine::host`): the registered synth (DV-1) renders as its own
//! calls do,
//! the render never allocates, the report region decodes, the live overflow releases.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::host::{Host, Prepare, channel, copy_feedback, feedback_voices, read_feedback};
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, TransportState, encode_stream,
    limits,
};
use engine::time::Tick;
use engine::{OSC_SYNTH, Param, Synth};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

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
/// The synth itself is checked against the pre-DV-1 synth in `tests/osc_synth.rs`.
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

#[test]
fn the_cpu_load_is_the_timed_render_time_over_the_period() {
    let mut session = Session::new();
    // Untimed: not measured.
    assert_eq!(report_of(&mut session).cpu_load, engine::host::NOT_MEASURED);
    // 1600 frames at 48 kHz are 1/30 s: 5 ms of render time is 15 %.
    for _ in 0..12 {
        session.host.render(128);
    }
    session.host.add_render_time(0.005);
    session.host.add_render_time(f32::NAN);
    session.host.add_render_time(-1.0);
    session.host.render(128);
    let load = read_feedback(session.host.feedback()).unwrap().cpu_load;
    assert!((0.1..0.2).contains(&load), "{load}");
    // Over budget saturates at 1.
    session.host.add_render_time(10.0);
    session.host.render(1600);
    assert_eq!(
        read_feedback(session.host.feedback()).unwrap().cpu_load,
        1.0
    );
}

#[test]
fn a_report_is_copied_as_bytes_and_decodes_alike() {
    let mut session = two_tracks(0.5);
    session.host.write_feedback();
    let mut bytes = vec![0xAA; 3];
    assert!(copy_feedback(session.host.feedback(), &mut bytes));
    assert_eq!(bytes.len(), 56 + 3 * 16, "header, two tracks, the output");
    assert_eq!(bytes[0], engine::msg::FEEDBACK_VERSION);
    let decoded = engine::host::decode_feedback(&bytes).unwrap();
    assert_eq!(decoded, read_feedback(session.host.feedback()).unwrap());
    // Truncated or of another version: nothing.
    assert!(engine::host::decode_feedback(&bytes[..bytes.len() - 1]).is_none());
    bytes[0] = 9;
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

#[test]
fn rendering_and_reporting_with_tracks_never_allocates() {
    let mut session = two_tracks(0.7);
    session.send(&[M::Play { from: Tick(0) }]);
    session.host.render(128);
    let mut bytes = Vec::with_capacity(limits::MAX_TRACKS as usize * 16 + 64);
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
        56 + 2 * 16 + 16,
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
        let at = 56 + 2 * 16 + 8;
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
    let state_at = 56 + 2 * 16 + 6;
    for state in [0u8, 5, 255] {
        unknown[state_at] = state;
        assert!(engine::host::decode_feedback(&unknown).is_none(), "{state}");
    }
    // A report with no cell (what an older engine writes) decodes with none.
    let mut none = bytes[..56 + 2 * 16].to_vec();
    none[44..48].copy_from_slice(&0u32.to_le_bytes());
    assert!(
        engine::host::decode_feedback(&none)
            .unwrap()
            .active_clips
            .is_empty()
    );
}
