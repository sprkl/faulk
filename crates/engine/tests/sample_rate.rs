//! A sample rate change (#236) through the host's two sides: `Prepare::set_sample_rate` builds
//! every device again for the new rate off the audio thread, the audio side takes the rate
//! (clock, metronome, limiter, fades) and keeps the playhead. Asserts on the signal: a note's
//! pitch before and after the change, an echo's delay in frames at the new rate, the
//! settings and the bypass of every device kept; the audio side runs under the allocation
//! detector. A control shows the old behaviour (`Engine::set_sample_rate` alone) plays the
//! note an octave up, so the pitch test tells the two apart.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::devices::{compressor, echo, filter};
use engine::host::{Host, Prepare, channel};
use engine::msg::{
    ControlMsg as M, DeviceSlot, MsgError, ParamId, TrackKind, TrackSlot, encode_state,
    encode_stream,
};
use engine::time::{SampleRate, Tempo, Tick};
use engine::{OSC_SYNTH, Param};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const BLOCK: usize = 128;
const A4: u8 = 69;

struct Rig {
    prepare: Prepare,
    host: Host,
    seq: u32,
    rate: f32,
}

impl Rig {
    fn new(rate: f32) -> Rig {
        let (prepare, host) = channel(rate).unwrap();
        let mut rig = Rig {
            prepare,
            host,
            seq: 0,
            rate,
        };
        rig.send(&[
            M::Reset { epoch: 1 },
            M::TrackCreate {
                track: TrackSlot(0),
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: vec![],
            },
            // Set after the insert: the prepare side keeps what `ParamSet` says too.
            M::ParamSet {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                param: ParamId(Param::Waveform.id()),
                value: 0.0,
            },
            M::ParamSet {
                track: TrackSlot(0),
                device: DeviceSlot(0),
                param: ParamId(Param::Release.id()),
                value: 0.002,
            },
        ]);
        rig
    }

    fn send(&mut self, msgs: &[M]) {
        let bytes = encode_stream(1, self.seq, msgs).unwrap();
        self.seq = self.seq.wrapping_add(msgs.len() as u32);
        self.prepare.submit(&bytes).unwrap();
    }

    /// `secs` of audio at the rig's current rate (left channel).
    fn secs(&mut self, secs: f32) -> Vec<f32> {
        let mut todo = (secs * self.rate) as usize;
        let mut out = Vec::new();
        while todo > 0 {
            self.prepare.flush();
            let n = assert_no_alloc(|| self.host.render(BLOCK.min(todo)));
            out.extend_from_slice(&self.host.left()[..n]);
            todo -= n;
        }
        assert_eq!(self.host.engine().limiter_silenced(), 0);
        assert_eq!(self.prepare.preparer().rejected(), 0);
        assert_eq!(self.host.engine().skipped(), 0);
        out
    }

    fn note(&mut self, on: bool) {
        self.send(&[if on {
            M::NoteOn {
                track: TrackSlot(0),
                pitch: A4,
                velocity: 110,
                at: None,
            }
        } else {
            M::NoteOff {
                track: TrackSlot(0),
                pitch: A4,
                at: None,
            }
        }]);
    }

    fn change_rate(&mut self, hz: u32) {
        let rate = SampleRate::new(hz).unwrap();
        self.prepare.set_sample_rate(rate).unwrap();
        self.rate = hz as f32;
    }
}

/// Rising zero crossings per second at `rate`.
fn pitch(x: &[f32], rate: f32) -> f32 {
    let crossings: Vec<usize> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, _)| i)
        .collect();
    let (first, last) = (crossings[0], *crossings.last().unwrap());
    (crossings.len() - 1) as f32 * rate / (last - first) as f32
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, s| m.max(s.abs()))
}

#[test]
fn a_note_keeps_its_pitch_across_a_sample_rate_change() {
    for (from, to) in [(48_000, 96_000), (48_000, 44_100), (44_100, 48_000)] {
        let mut r = Rig::new(from as f32);
        r.note(true);
        let before = r.secs(0.3);
        let hz = pitch(&before[(0.05 * from as f32) as usize..], from as f32);
        assert!((hz - 440.0).abs() < 1.0, "{from} Hz: {hz}");
        r.note(false);
        r.change_rate(to);
        assert_eq!(r.prepare.preparer().sample_rate(), to as f32);
        r.secs(0.05);
        r.note(true);
        let after = r.secs(0.3);
        let hz = pitch(&after[(0.05 * to as f32) as usize..], to as f32);
        assert!((hz - 440.0).abs() < 1.0, "{from} → {to} Hz: {hz}");
        assert!(peak(&after) > 0.1, "it sounds");
    }

    // Control: the engine's rate alone (before #236) leaves the synth at its old rate, so at
    // twice the rate the same note plays an octave up.
    let mut r = Rig::new(48_000.0);
    r.secs(0.05);
    r.host.engine_mut().set_sample_rate(SampleRate::HZ_96000);
    r.rate = 96_000.0;
    r.note(true);
    let after = r.secs(0.3);
    let hz = pitch(&after[4_800..], 96_000.0);
    assert!((hz - 880.0).abs() < 2.0, "the control plays {hz}");
}

#[test]
fn the_playhead_goes_on_from_where_it_was() {
    let mut r = Rig::new(48_000.0);
    r.send(&[
        M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: Default::default(),
            launch_quantize: Default::default(),
        },
        M::Play { from: Tick(0) },
    ]);
    r.secs(1.0);
    let before = r.host.engine().clock().position();
    // One block at 48 kHz moves the playhead by this many ticks.
    let at48 = {
        let p = r.host.engine().clock().position();
        r.secs(BLOCK as f32 / 48_000.0);
        r.host.engine().clock().position() - p
    };
    let before = before + at48;
    r.change_rate(96_000);
    r.secs(BLOCK as f32 / 96_000.0);
    let after = r.host.engine().clock().position();
    assert!(before > 900.0, "it played: {before}");
    // The same block of frames is half the time at 96 kHz: half the ticks, from where it was.
    let moved = after - before;
    assert!(
        (moved - at48 / 2.0).abs() < 1e-6 * at48.max(1.0) + 1e-9,
        "moved {moved} ticks, {at48} per block at 48 kHz"
    );
    assert_eq!(r.host.engine().clock().sample_rate(), SampleRate::HZ_96000);
}

#[test]
fn every_device_is_built_again_with_its_settings_and_its_bypass() {
    let mut r = Rig::new(48_000.0);
    r.send(&[
        M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(1),
            kind: echo::DESCRIPTOR.kind_id,
            state: encode_state(&[(ParamId(echo::MODE), 0.0)]),
        },
        // Given after the insert: kept by the prepare side.
        M::ParamSet {
            track: TrackSlot(0),
            device: DeviceSlot(1),
            param: ParamId(echo::TIME),
            value: 0.1,
        },
        M::ParamSet {
            track: TrackSlot(0),
            device: DeviceSlot(1),
            param: ParamId(echo::MIX),
            value: 1.0,
        },
        M::ParamSet {
            track: TrackSlot(0),
            device: DeviceSlot(1),
            param: ParamId(echo::FEEDBACK),
            value: 0.0,
        },
        // An undeclared id changes nothing.
        M::ParamSet {
            track: TrackSlot(0),
            device: DeviceSlot(1),
            param: ParamId(999),
            value: 3.0,
        },
        M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(2),
            kind: filter::DESCRIPTOR.kind_id,
            state: vec![],
        },
        M::DeviceSetEnabled {
            track: TrackSlot(0),
            device: DeviceSlot(2),
            enabled: false,
        },
        // A device removed is not built again.
        M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(3),
            kind: filter::DESCRIPTOR.kind_id,
            state: vec![],
        },
        M::DeviceRemove {
            track: TrackSlot(0),
            device: DeviceSlot(3),
        },
        M::DeviceInsert {
            track: TrackSlot::MASTER,
            device: DeviceSlot(0),
            kind: compressor::DESCRIPTOR.kind_id,
            state: vec![],
        },
    ]);
    r.secs(0.1);
    let applied = r.host.engine().applied();
    r.change_rate(96_000);
    r.secs(0.05);
    // The rate, then the synth, the echo, the filter and the master's compressor.
    assert_eq!(r.host.engine().applied() - applied, 5);
    let state = r.host.engine().state();
    assert!(!state.device(TrackSlot(0), DeviceSlot(2)).unwrap().enabled);
    assert!(state.device(TrackSlot(0), DeviceSlot(3)).is_none());
    assert!(state.device(TrackSlot::MASTER, DeviceSlot(0)).is_some());
    // A short note: the echo (mix 1, no feedback) repeats it 100 ms later, 9 600 frames at
    // the new rate, and the dry note is gone (mix 1).
    r.note(true);
    let mut out = r.secs(0.01);
    r.note(false);
    out.extend(r.secs(0.3));
    let first = out.iter().position(|s| s.abs() > 0.01).unwrap();
    assert!(
        (9_550..=9_700).contains(&first),
        "the repeat starts at frame {first}"
    );
}

#[test]
fn a_rate_the_devices_cannot_build_for_is_refused() {
    let mut r = Rig::new(48_000.0);
    for hz in [1, 4_000, 7_999, 192_001, 384_000] {
        let rate = SampleRate::new(hz).unwrap();
        assert!(
            matches!(
                r.prepare.set_sample_rate(rate),
                Err(MsgError::LimitExceeded { .. })
            ),
            "{hz} Hz"
        );
        assert_eq!(r.prepare.preparer().sample_rate(), 48_000.0);
    }
    for hz in [8_000, 192_000] {
        r.change_rate(hz);
        r.secs(0.01);
        assert_eq!(r.host.engine().clock().sample_rate().hz(), hz);
    }
}

#[test]
fn the_same_rate_rebuilds_nothing_and_the_reports_follow_a_new_one() {
    let mut r = Rig::new(48_000.0);
    r.secs(0.05);
    assert_eq!(r.host.report_period(), 1_600);
    let applied = r.host.engine().applied();
    r.change_rate(48_000);
    r.secs(0.05);
    assert_eq!(r.host.engine().applied(), applied, "nothing sent");
    assert_eq!(r.host.report_period(), 1_600);
    // 30 reports a second at the new rate.
    r.change_rate(96_000);
    r.secs(0.05);
    assert_eq!(r.host.report_period(), 3_200);
    // A host built for a rate the clock rounds keeps the rate it was given until a change.
    let (_, mut odd) = channel(44_100.4).unwrap();
    assert_no_alloc(|| odd.render(BLOCK));
    assert_eq!(odd.report_period(), 1_470);
}

#[test]
fn with_no_room_in_the_queue_nothing_changes() {
    let mut r = Rig::new(48_000.0);
    // The engine does not run: the ring (4 096) then the queue (4 096) fill up.
    let set = M::ParamSet {
        track: TrackSlot(0),
        device: DeviceSlot(0),
        param: ParamId(Param::Gain.id()),
        value: 0.5,
    };
    let mut sent = 0;
    loop {
        let bytes = encode_stream(1, r.seq, std::slice::from_ref(&set)).unwrap();
        r.seq += 1;
        match r.prepare.submit(&bytes) {
            Ok(()) => sent += 1,
            Err(MsgError::EngineBusy) => break,
            Err(e) => panic!("{e:?}"),
        }
        assert!(sent < 20_000);
    }
    let pending = r.prepare.preparer().pending();
    assert_eq!(
        r.prepare.set_sample_rate(SampleRate::HZ_96000),
        Err(MsgError::EngineBusy)
    );
    assert_eq!(r.prepare.preparer().sample_rate(), 48_000.0);
    assert_eq!(r.prepare.preparer().pending(), pending, "nothing sent");
    // Once the engine drains, it goes through.
    r.secs(0.5);
    r.change_rate(96_000);
    r.secs(0.05);
    assert_eq!(r.host.engine().clock().sample_rate(), SampleRate::HZ_96000);
}
