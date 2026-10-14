//! Offline render of the prepare/apply cycle (MO-1): [`scenario`] grows every runtime table
//! while a note plays, through the real `Preparer` and `Engine` and the stand-in render of
//! `engine::engine::cycle`, under the allocation detector on the audio side. Asserts on the
//! signal: no gap while the note is held, silence after the release, no NaN, no infinity,
//! no denormal; and on the tables: each grew as the scenario says, nothing was skipped.

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::engine::cycle::{
    Cycle, SCENARIO_BLOCK, SCENARIO_CLIP, SCENARIO_NOTES, SCENARIO_RATE, Step, scenario,
    scenario_held_blocks,
};
use engine::engine::{ClipContent, initial};
use engine::msg::{ClipSlot, LaneSlot, PAGE_SLOTS, SceneSlot, SourceSlot, TrackSlot};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// Peak and RMS of each rendered block (both channels).
fn run(cycle: &mut Cycle, steps: &[Step]) -> (Vec<f32>, Vec<(f32, f32)>) {
    let mut out = Vec::new();
    let mut blocks = Vec::new();
    for step in steps {
        match step {
            Step::Submit(bytes) => cycle.submit(bytes).expect("the scenario is accepted"),
            Step::Render(n) => {
                for _ in 0..*n {
                    let frames = assert_no_alloc(|| cycle.render(SCENARIO_BLOCK));
                    let block: Vec<f32> = cycle.left()[..frames]
                        .iter()
                        .chain(&cycle.right()[..frames])
                        .copied()
                        .collect();
                    let peak = block.iter().fold(0f32, |m, s| m.max(s.abs()));
                    let rms =
                        (block.iter().map(|s| s * s).sum::<f32>() / block.len() as f32).sqrt();
                    blocks.push((peak, rms));
                    out.extend_from_slice(&cycle.left()[..frames]);
                    out.extend_from_slice(&cycle.right()[..frames]);
                }
            }
        }
    }
    (out, blocks)
}

#[test]
fn every_table_grows_during_playback_without_a_gap() {
    let steps = scenario().unwrap();
    let mut cycle = Cycle::new(SCENARIO_RATE).unwrap();
    let (out, blocks) = run(&mut cycle, &steps);

    // The signal: finite, no denormal, sound in every held block, silence after release.
    for (i, s) in out.iter().enumerate() {
        assert!(s.is_finite(), "sample {i} is {s}");
        assert!(*s == 0.0 || s.abs() >= f32::MIN_POSITIVE, "denormal at {i}");
    }
    let held = scenario_held_blocks(&steps);
    assert_eq!(blocks.len(), held + 8);
    for (i, (peak, rms)) in blocks[..held].iter().enumerate() {
        assert!(*rms > 0.01, "block {i} has a gap: rms {rms}, peak {peak}");
        assert!(*peak <= 1.0, "block {i} clips: {peak}");
    }
    // The test tone is hard gated: the block after the release is silent.
    for (i, (peak, _)) in blocks[held..].iter().enumerate() {
        assert_eq!(*peak, 0.0, "block {} after the release is not silent", i);
    }

    // The tables: each grew, nothing was skipped, nothing leaked, nothing waits.
    let engine = &cycle.engine;
    let state = engine.state();
    assert_eq!(engine.skipped(), 0);
    assert_eq!(engine.leaked(), 0);
    assert_eq!(cycle.preparer.rejected(), 0);
    assert_eq!(cycle.preparer.pending(), 0);
    assert_eq!(state.tracks.capacity(), 64);
    assert_eq!(
        state.tracks.as_slice().iter().flatten().count(),
        48,
        "every track kept its slot"
    );
    let track0 = state.track(TrackSlot(0)).unwrap();
    assert_eq!(track0.devices.capacity(), 16);
    assert_eq!(track0.devices.as_slice().iter().flatten().count(), 10);
    assert_eq!(track0.scenes.capacity(), 32);
    assert_eq!(track0.scenes.as_slice()[20], Some(ClipSlot(SCENARIO_CLIP)));
    assert!(state.clips.directory_capacity() > initial::CLIP_PAGES);
    assert!(state.clips.capacity() > SCENARIO_CLIP as usize);
    let clip = state.clip(ClipSlot(SCENARIO_CLIP)).unwrap();
    let ClipContent::Notes(notes) = &clip.content else {
        panic!("a MIDI clip")
    };
    assert_eq!(notes.len(), SCENARIO_NOTES as usize);
    assert_eq!(notes.as_slice()[123_456].start.0, 123_456 * 120);
    assert!(state.lanes.capacity() > 3000 && state.lane(LaneSlot(3000)).is_some());
    assert_eq!(state.lanes.capacity() % PAGE_SLOTS, 0);
    assert_eq!(state.sources.capacity(), 128);
    assert!(state.source(SourceSlot(100)).is_some());
    assert_eq!(
        state.track(TrackSlot(0)).unwrap().scenes.as_slice()[21],
        None
    );
    let _ = SceneSlot(0);
}
