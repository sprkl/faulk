//! The shipping host (MO-2, `engine::host`): the registered synth (DV-1) renders as its own
//! calls do,
//! the render never allocates, the report region decodes, the live overflow releases.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::host::{Host, Prepare, channel, feedback_voices, read_feedback};
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_stream, limits,
};
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

#[test]
fn the_adapter_renders_the_synth_sample_for_sample() {
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
    assert_eq!(fb.meters.len(), 1, "one meter: the output's");
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
