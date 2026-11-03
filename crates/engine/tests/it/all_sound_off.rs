//! `AllSoundOff` (#331): a synth note with a long release ringing into an echo with a high
//! feedback, then the cut. Asserts on the signal: under -90 dBFS within 10 ms of the cut and
//! silent after, no sample step above a click threshold, no NaN, no denormal, nothing
//! allocated on the audio path (every block under the allocation detector, the cut's
//! included), the sound back after the cut (a new note plays), and a cut with nothing
//! ringing changes nothing. The native/WASM parity is
//! `crates/engine-wasm/tests/all_sound_off_parity.rs`.

use assert_no_alloc::assert_no_alloc;
use engine::OSC_SYNTH;
use engine::devices::echo;
use engine::host::channel;
use engine::msg::{
    ControlMsg as M, DeviceSlot, ParamId, TrackKind, TrackSlot, encode_state, encode_stream,
};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
/// -90 dBFS.
const FLOOR: f32 = 3.162_277_7e-5;
/// 10 ms in frames.
const TEN_MS: usize = 480;
/// The largest step between two consecutive samples the test accepts: a 330 Hz sine of full
/// scale steps by 0.043, and the synth plays at a gain of 0.8 and the echo adds to it.
const CLICK: f32 = 0.1;

struct Rig {
    prepare: engine::host::Prepare,
    host: engine::host::Host,
    seq: u32,
    left: Vec<f32>,
    right: Vec<f32>,
    /// Frame at which the last `cut` message was applied (start of the block after it).
    cut_at: usize,
}

impl Rig {
    fn new() -> Self {
        let (prepare, host) = channel(RATE as f32).unwrap();
        let mut rig = Rig {
            prepare,
            host,
            seq: 0,
            left: vec![],
            right: vec![],
            cut_at: 0,
        };
        let t = TrackSlot(0);
        let release = (ParamId(4), 6.0);
        rig.send(&[
            M::Reset { epoch: 1 },
            M::TrackCreate {
                track: t,
                kind: TrackKind::Midi,
            },
            M::DeviceInsert {
                track: t,
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: encode_state(&[release]),
            },
            M::DeviceInsert {
                track: t,
                device: DeviceSlot(1),
                kind: echo::DESCRIPTOR.kind_id,
                state: encode_state(&[
                    (ParamId(echo::MODE), 0.0),
                    (ParamId(echo::TIME), 0.25),
                    (ParamId(echo::FEEDBACK), echo::FEEDBACK_MAX),
                    (ParamId(echo::MIX), 0.6),
                ]),
            },
        ]);
        rig
    }

    fn send(&mut self, msgs: &[M]) {
        let bytes = encode_stream(1, self.seq, msgs).unwrap();
        self.seq = self.seq.wrapping_add(msgs.len() as u32);
        self.prepare.submit(&bytes).unwrap();
        self.prepare.flush();
    }

    fn render_blocks(&mut self, frames: usize, block: usize) {
        let mut done = 0;
        while done < frames {
            self.prepare.flush();
            let n = assert_no_alloc(|| self.host.render(block.min(frames - done)));
            self.left.extend_from_slice(&self.host.left()[..n]);
            self.right.extend_from_slice(&self.host.right()[..n]);
            done += n;
        }
    }

    fn secs(&mut self, s: f32) {
        self.render_blocks((s * RATE as f32) as usize, BLOCK);
    }

    fn note_on(&mut self, pitch: u8) {
        self.send(&[M::NoteOn {
            track: TrackSlot(0),
            pitch,
            velocity: 110,
            at: None,
        }]);
    }

    fn cut(&mut self) {
        self.send(&[
            M::AllNotesOff {
                track: TrackSlot(0),
            },
            M::AllSoundOff,
        ]);
        self.cut_at = self.left.len();
    }
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, s| m.max(s.abs()))
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0, |m, w| m.max((w[1] - w[0]).abs()))
}

/// A note held, released, with the echo feeding back and the release ringing, then the cut.
fn ringing() -> Rig {
    let mut rig = Rig::new();
    rig.note_on(60);
    rig.secs(0.6);
    rig.send(&[M::NoteOff {
        track: TrackSlot(0),
        pitch: 60,
        at: None,
    }]);
    rig.secs(0.4);
    rig
}

#[test]
fn the_cut_is_silent_within_10_ms_and_stays_so_without_a_click() {
    let mut rig = ringing();
    let before = peak(&rig.left[rig.left.len() - 4_800..]);
    assert!(before > 0.05, "the release and the echo ring: {before}");
    rig.cut();
    rig.secs(4.5);
    let at = rig.cut_at;
    for channel in [&rig.left, &rig.right] {
        assert!(channel.iter().all(|s| s.is_finite()), "no NaN");
        let after = &channel[at..];
        let silent_from = after
            .iter()
            .rposition(|s| s.abs() > FLOOR)
            .map_or(0, |i| i + 1);
        assert!(
            silent_from <= TEN_MS,
            "under -90 dBFS from frame {silent_from} after the cut"
        );
        assert!(
            after[TEN_MS..].iter().all(|s| *s == 0.0),
            "exact silence after the cut: no denormal, no tail"
        );
        let step = max_step(&channel[at - 1..at + TEN_MS + 2]);
        assert!(step < CLICK, "largest step across the cut: {step}");
        // The fade only shrinks the signal: no step is larger than the ringing's own.
        let own = max_step(&channel[at - 4_800..at]);
        assert!(
            step <= own * 1.001,
            "the cut steps {step}, the ringing {own}"
        );
        println!(
            "silent {silent_from} frames ({:.2} ms) after the cut, largest step {step} (ringing: {own})",
            silent_from as f32 * 1000.0 / RATE as f32
        );
    }
    assert_eq!(rig.host.engine().skipped(), 0);
    // The is_finite checks above are vacuous since the output protection (#402) silences
    // non-finite samples; this is the real no-NaN guard across the all-sound-off cut.
    assert_eq!(
        rig.host.engine().limiter_silenced(),
        0,
        "the all-sound-off cut produced a non-finite sample"
    );
}

#[test]
fn the_sound_comes_back_after_the_cut() {
    let mut rig = ringing();
    rig.cut();
    rig.secs(0.1);
    let at = rig.left.len();
    rig.note_on(64);
    rig.secs(0.3);
    assert!(peak(&rig.left[at..]) > 0.1, "a new note plays after a cut");
    // `is_finite` on the output is vacuous (the output protection of #402 zeroes non-finite
    // samples): the limiter's silenced-frame counter is the real no-NaN guard.
    assert_eq!(
        rig.host.engine().limiter_silenced(),
        0,
        "a note after the cut produced a non-finite sample"
    );
}

#[test]
fn a_cut_over_silence_changes_nothing() {
    let mut rig = Rig::new();
    rig.secs(0.1);
    rig.cut();
    rig.secs(0.2);
    assert!(rig.left.iter().chain(&rig.right).all(|s| *s == 0.0));
}

#[test]
fn a_second_cut_while_one_fades_does_not_restart_it() {
    let mut rig = ringing();
    rig.cut();
    // Three blocks of 16 frames into the 240-frame fade, a second order arrives. A restart
    // would stretch the fade to 48 + 240 frames; the first fade's end (frame 240) stands.
    rig.render_blocks(48, 16);
    rig.send(&[M::AllSoundOff]);
    rig.render_blocks(2_000, 16);
    let at = rig.cut_at;
    assert!(rig.left[at + 240..].iter().all(|s| *s == 0.0));
}

#[test]
fn a_cut_in_small_blocks_fades_across_them() {
    let mut rig = ringing();
    rig.cut();
    // Blocks of 16 frames: the 240-frame fade spans 15 of them.
    rig.render_blocks(2_000, 16);
    let at = rig.cut_at;
    assert!(rig.left[at + TEN_MS..].iter().all(|s| *s == 0.0));
    assert!(max_step(&rig.left[at - 1..at + TEN_MS]) < CLICK);
}

#[test]
fn a_fade_that_ends_on_the_last_frame_of_a_block_is_silent_after_it() {
    let mut rig = ringing();
    rig.cut();
    // Blocks of exactly the fade (5 ms at 48 kHz = 240 frames).
    rig.render_blocks(2_400, 240);
    let at = rig.cut_at;
    assert!(rig.left[at + 240..].iter().all(|s| *s == 0.0));
    assert!(max_step(&rig.left[at - 1..at + 241]) < CLICK);
    // And a note after it still plays.
    let from = rig.left.len();
    rig.note_on(64);
    rig.render_blocks(14_400, 240);
    assert!(peak(&rig.left[from..]) > 0.1);
}
