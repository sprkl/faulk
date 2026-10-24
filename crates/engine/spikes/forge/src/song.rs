//! The demo every sketch plays, and the three sketches.
//!
//! One piece, the same notes in every sketch so the comparison is fair: 96 BPM, A minor,
//! nine bars (22.5 s) plus a tail. A bass line in eighths from the first bar; a chord pad
//! from bar 3 (Am7, Fmaj7, C, G); a lead phrase from bar 5 that resolves on bar 9. Each part
//! is its own [`Synth`] with its own patch; only the patches, the chorus and the mix levels
//! change between sketches.

use crate::chorus::ChorusSettings;
use crate::env::Adsr;
use crate::synth::{MAX_BLOCK, Oversample, Synth};
use crate::voice::{FilterKind, Patch};

pub const BPM: f32 = 96.0;
pub const BEATS: f32 = 36.0;
/// Seconds rendered after the last bar, for the releases.
pub const TAIL: f32 = 1.5;

/// A note of the score, in beats.
#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub start: f32,
    pub length: f32,
    pub key: u8,
    pub velocity: f32,
}

const ROOTS: [u8; 4] = [33, 29, 36, 31]; // A1, F1, C2, G1
const BASS_STEPS: [(u8, f32); 8] = [
    (0, 1.0),
    (0, 0.7),
    (12, 0.85),
    (0, 0.7),
    (0, 0.95),
    (7, 0.7),
    (12, 0.85),
    (7, 0.75),
];
const CHORDS: [[u8; 4]; 4] = [
    [57, 60, 64, 67], // Am7
    [53, 57, 60, 64], // Fmaj7
    [55, 60, 64, 67], // C (G in the bass of the voicing)
    [55, 59, 62, 67], // G
];
const LEAD: [(f32, f32, u8); 17] = [
    (16.0, 1.0, 76),
    (17.0, 0.5, 79),
    (17.5, 1.5, 81),
    (19.0, 1.0, 79),
    (20.0, 1.5, 77),
    (21.5, 0.5, 76),
    (22.0, 1.0, 72),
    (23.0, 1.0, 74),
    (24.0, 1.0, 76),
    (25.0, 0.5, 79),
    (25.5, 0.5, 76),
    (26.0, 2.0, 72),
    (28.0, 0.5, 74),
    (28.5, 0.5, 76),
    (29.0, 1.0, 79),
    (30.0, 2.0, 74),
    (32.0, 3.0, 76),
];

pub fn bass() -> Vec<Note> {
    let mut notes = Vec::new();
    for bar in 0..8 {
        let root = ROOTS[bar % 4];
        for (step, &(interval, velocity)) in BASS_STEPS.iter().enumerate() {
            notes.push(Note {
                start: bar as f32 * 4.0 + step as f32 * 0.5,
                length: 0.3,
                key: root + interval,
                velocity,
            });
        }
    }
    notes.push(Note {
        start: 32.0,
        length: 3.0,
        key: ROOTS[0],
        velocity: 1.0,
    });
    notes
}

pub fn pad() -> Vec<Note> {
    let mut notes = Vec::new();
    for bar in 2..8 {
        for &key in &CHORDS[bar % 4] {
            notes.push(Note {
                start: bar as f32 * 4.0,
                length: 3.9,
                key,
                velocity: 0.75,
            });
        }
    }
    for key in [57, 60, 64, 71] {
        notes.push(Note {
            start: 32.0,
            length: 3.0,
            key,
            velocity: 0.75,
        });
    }
    notes
}

pub fn lead() -> Vec<Note> {
    LEAD.iter()
        .enumerate()
        .map(|(i, &(start, length, key))| Note {
            start,
            // Overlap the next note a little: a mono legato patch glides between them.
            length: if i + 1 < LEAD.len() {
                length + 0.03
            } else {
                length
            },
            key,
            velocity: if start.fract() == 0.0 { 0.9 } else { 0.75 },
        })
        .collect()
}

/// One part of a sketch: its patch, its chorus and its level in the mix.
#[derive(Clone, Copy, Debug)]
pub struct Part {
    pub patch: Patch,
    pub chorus: ChorusSettings,
    pub gain: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Sketch {
    pub name: &'static str,
    pub bass: Part,
    pub pad: Part,
    pub lead: Part,
}

const fn adsr(attack: f32, decay: f32, sustain: f32, release: f32) -> Adsr {
    Adsr {
        attack,
        decay,
        sustain,
        release,
    }
}

/// (a) Warm and fat: ladder, strong drive, thick mono unison bass and lead.
pub const SKETCH_A: Sketch = Sketch {
    name: "a",
    bass: Part {
        patch: Patch {
            level_a: 0.6,
            shape_b: 0.0,
            level_b: 0.45,
            detune_b: -12.07,
            level_sub: 0.4,
            filter: FilterKind::Ladder,
            cutoff_hz: 170.0,
            resonance: 0.45,
            env_amount: 3.3,
            key_track: 0.3,
            velocity_to_cutoff: 0.6,
            drive: 4.0,
            out_drive: 1.8,
            snap: 1.2,
            amp: adsr(0.002, 0.4, 0.8, 0.08),
            filt: adsr(0.002, 0.25, 0.15, 0.15),
            mono: true,
            unison: 3,
            unison_detune: 10.0,
            spread: 0.3,
            age: 0.5,
            level: 0.55,
            ..Patch::INIT
        },
        chorus: ChorusSettings::OFF,
        gain: 1.0,
    },
    pad: Part {
        patch: Patch {
            level_a: 0.55,
            level_b: 0.5,
            detune_b: 0.08,
            filter: FilterKind::Ladder,
            cutoff_hz: 650.0,
            resonance: 0.3,
            env_amount: 1.6,
            key_track: 0.4,
            drive: 2.5,
            out_drive: 1.3,
            amp: adsr(0.35, 1.0, 0.8, 1.2),
            filt: adsr(0.6, 1.5, 0.4, 1.0),
            unison: 2,
            unison_detune: 9.0,
            spread: 0.7,
            age: 0.5,
            level: 0.3,
            ..Patch::INIT
        },
        chorus: ChorusSettings::OFF,
        gain: 0.72,
    },
    lead: Part {
        patch: Patch {
            level_a: 0.6,
            shape_b: 1.0,
            pw_b: 0.4,
            level_b: 0.3,
            detune_b: 12.03,
            filter: FilterKind::Ladder,
            cutoff_hz: 850.0,
            resonance: 0.55,
            env_amount: 2.2,
            key_track: 0.5,
            drive: 5.0,
            out_drive: 2.2,
            snap: 0.6,
            amp: adsr(0.005, 0.3, 0.9, 0.25),
            filt: adsr(0.005, 0.5, 0.35, 0.3),
            mono: true,
            legato: true,
            glide: 0.06,
            unison: 2,
            unison_detune: 7.0,
            spread: 0.4,
            vibrato: 12.0,
            lfo_rate: 5.2,
            age: 0.5,
            level: 0.36,
            ..Patch::INIT
        },
        chorus: ChorusSettings::OFF,
        gain: 1.45,
    },
};

/// (b) Soft and polyphonic: state-variable filter, light drive, chorus; lush pad and brass.
pub const SKETCH_B: Sketch = Sketch {
    name: "b",
    bass: Part {
        patch: Patch {
            shape_a: 0.6,
            pw_a: 0.35,
            level_a: 0.65,
            level_sub: 0.5,
            filter: FilterKind::Svf,
            cutoff_hz: 380.0,
            resonance: 0.25,
            env_amount: 2.0,
            key_track: 0.3,
            velocity_to_cutoff: 0.4,
            drive: 1.2,
            amp: adsr(0.003, 0.5, 0.7, 0.15),
            filt: adsr(0.003, 0.3, 0.2, 0.2),
            age: 0.3,
            level: 0.6,
            ..Patch::INIT
        },
        chorus: ChorusSettings {
            mix: 0.3,
            rate: 0.5,
            depth_ms: 1.5,
            delay_ms: 6.0,
        },
        gain: 1.5,
    },
    pad: Part {
        patch: Patch {
            shape_a: 1.0,
            pw_a: 0.5,
            level_a: 0.5,
            level_b: 0.5,
            detune_b: 0.06,
            pwm: 0.25,
            filter: FilterKind::Svf,
            cutoff_hz: 1_200.0,
            resonance: 0.2,
            env_amount: 0.8,
            key_track: 0.5,
            drive: 1.0,
            amp: adsr(0.6, 1.0, 0.85, 1.6),
            filt: adsr(0.8, 2.0, 0.6, 1.5),
            unison: 2,
            unison_detune: 6.0,
            spread: 0.5,
            lfo_rate: 0.35,
            age: 0.3,
            level: 0.32,
            ..Patch::INIT
        },
        chorus: ChorusSettings {
            mix: 0.7,
            rate: 0.45,
            depth_ms: 2.5,
            delay_ms: 8.0,
        },
        gain: 0.65,
    },
    lead: Part {
        patch: Patch {
            level_a: 0.55,
            level_b: 0.5,
            detune_b: 0.1,
            filter: FilterKind::Svf,
            cutoff_hz: 500.0,
            resonance: 0.15,
            env_amount: 2.5,
            key_track: 0.6,
            velocity_to_cutoff: 0.5,
            drive: 1.3,
            amp: adsr(0.05, 0.4, 0.85, 0.3),
            filt: adsr(0.08, 0.6, 0.45, 0.3),
            unison: 2,
            unison_detune: 5.0,
            spread: 0.4,
            vibrato: 8.0,
            lfo_rate: 4.5,
            age: 0.3,
            level: 0.4,
            ..Patch::INIT
        },
        chorus: ChorusSettings {
            mix: 0.35,
            rate: 0.6,
            depth_ms: 1.8,
            delay_ms: 7.0,
        },
        gain: 1.55,
    },
};

/// (c) Glass and steel: hard sync swept by the envelope, high resonance, short envelopes,
/// almost no drift (age 0.05), wide unison instead of chorus. Bright and precise.
pub const SKETCH_C: Sketch = Sketch {
    name: "c",
    bass: Part {
        patch: Patch {
            level_a: 0.7,
            level_b: 0.0,
            level_sub: 0.45,
            sync: true,
            sync_sweep: 14.0,
            filter: FilterKind::Ladder,
            cutoff_hz: 600.0,
            resonance: 0.7,
            env_amount: 2.5,
            key_track: 0.3,
            drive: 2.5,
            out_drive: 1.5,
            snap: 2.0,
            amp: adsr(0.001, 0.25, 0.7, 0.06),
            filt: adsr(0.001, 0.18, 0.1, 0.1),
            mono: true,
            age: 0.05,
            level: 0.55,
            ..Patch::INIT
        },
        chorus: ChorusSettings::OFF,
        gain: 1.65,
    },
    pad: Part {
        patch: Patch {
            shape_a: 1.0,
            pw_a: 0.2,
            level_a: 0.55,
            shape_b: 1.0,
            pw_b: 0.5,
            level_b: 0.35,
            detune_b: 12.0,
            pwm: 0.3,
            filter: FilterKind::Svf,
            cutoff_hz: 2_600.0,
            resonance: 0.6,
            env_amount: 1.0,
            key_track: 1.0,
            drive: 1.5,
            amp: adsr(0.02, 1.5, 0.5, 0.9),
            filt: adsr(0.005, 0.8, 0.3, 0.8),
            unison: 2,
            unison_detune: 14.0,
            spread: 1.0,
            lfo_rate: 0.25,
            age: 0.05,
            level: 0.3,
            ..Patch::INIT
        },
        chorus: ChorusSettings::OFF,
        gain: 0.9,
    },
    lead: Part {
        patch: Patch {
            level_a: 0.65,
            level_b: 0.25,
            sync: true,
            sync_sweep: 9.0,
            filter: FilterKind::Ladder,
            cutoff_hz: 2_500.0,
            resonance: 0.65,
            env_amount: 1.0,
            key_track: 0.5,
            drive: 3.0,
            out_drive: 2.5,
            amp: adsr(0.002, 0.3, 0.85, 0.15),
            filt: adsr(0.02, 0.35, 0.4, 0.2),
            mono: true,
            legato: true,
            glide: 0.03,
            vibrato: 6.0,
            lfo_rate: 5.5,
            age: 0.05,
            level: 0.36,
            ..Patch::INIT
        },
        chorus: ChorusSettings::OFF,
        gain: 1.5,
    },
};

pub const SKETCHES: [Sketch; 3] = [SKETCH_A, SKETCH_B, SKETCH_C];

/// A note event at a frame, for one part.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub frame: usize,
    pub part: usize,
    pub key: u8,
    pub velocity: f32,
    pub on: bool,
}

/// The demo prepared for rendering: its synths and its sorted events (all allocated here).
pub struct Performance {
    pub synths: [Synth; 3],
    pub gains: [f32; 3],
    pub events: Vec<Event>,
    pub frames: usize,
}

impl Performance {
    pub fn new(sketch: &Sketch, fs: f32, os: Oversample, seed: u64) -> Self {
        let frames_per_beat = fs * 60.0 / BPM;
        let parts = [sketch.bass, sketch.pad, sketch.lead];
        let synths = core::array::from_fn(|i| {
            Synth::new(
                fs,
                os,
                parts[i].patch,
                parts[i].chorus,
                seed + i as u64 * 1000,
            )
        });
        let mut events = Vec::new();
        for (part, notes) in [bass(), pad(), lead()].into_iter().enumerate() {
            for n in notes {
                let on = (n.start * frames_per_beat).round() as usize;
                let off = ((n.start + n.length) * frames_per_beat).round() as usize;
                events.push(Event {
                    frame: on,
                    part,
                    key: n.key,
                    velocity: n.velocity,
                    on: true,
                });
                events.push(Event {
                    frame: off,
                    part,
                    key: n.key,
                    velocity: 0.0,
                    on: false,
                });
            }
        }
        // At the same frame, note-offs first (a repeated key restarts cleanly).
        events.sort_by_key(|e| (e.frame, e.on));
        let frames = ((BEATS * 60.0 / BPM + TAIL) * fs) as usize;
        Performance {
            synths,
            gains: parts.map(|p| p.gain),
            events,
            frames,
        }
    }

    /// Renders the whole demo into `left` and `right` (at least `self.frames` long), block
    /// by block, splitting the blocks at the events. Never allocates.
    pub fn render_into(&mut self, left: &mut [f32], right: &mut [f32]) {
        let mut tmp_l = [0.0f32; MAX_BLOCK];
        let mut tmp_r = [0.0f32; MAX_BLOCK];
        let total = self.frames.min(left.len()).min(right.len());
        left[..total].fill(0.0);
        right[..total].fill(0.0);
        let mut next = 0;
        let mut frame = 0;
        while frame < total {
            while next < self.events.len() && self.events[next].frame <= frame {
                let e = self.events[next];
                let synth = &mut self.synths[e.part];
                if e.on {
                    synth.note_on(e.key, e.velocity);
                } else {
                    synth.note_off(e.key);
                }
                next += 1;
            }
            let until = self.events.get(next).map_or(total, |e| e.frame.min(total));
            let n = (until - frame).clamp(1, MAX_BLOCK).min(total - frame);
            for (synth, gain) in self.synths.iter_mut().zip(self.gains) {
                synth.render(&mut tmp_l[..n], &mut tmp_r[..n]);
                for i in 0..n {
                    left[frame + i] += tmp_l[i] * gain;
                    right[frame + i] += tmp_r[i] * gain;
                }
            }
            frame += n;
        }
    }

    /// Renders the whole demo into new buffers.
    pub fn render(mut self) -> (Vec<f32>, Vec<f32>) {
        let mut left = vec![0.0; self.frames];
        let mut right = vec![0.0; self.frames];
        self.render_into(&mut left, &mut right);
        (left, right)
    }
}
