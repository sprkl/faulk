//! Every allocation of the prepare side is fallible (MO-1): an allocator that refuses the
//! n-th allocation of the calling thread is armed around one `handle`, for n = 1, 2, …
//! until the call needs fewer allocations. Each refusal must come back as
//! `MsgError::OutOfMemory` (an infallible allocation would abort the test process instead)
//! and leave the prepare side as it was: the same message, sent again with the memory
//! back, is applied as if the refusal had never happened.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use engine::device::DeviceSetup;
use engine::device::testing::{GAIN, TONE, factory, tone};
use engine::engine::{ClipContent, Engine, Preparer, channel};
use engine::msg::{
    AutomationPoint, AutomationTarget, ClipSlot, ControlMsg as M, DeviceSlot, LaneSlot, MsgError,
    Note, Placement, SceneSlot, SourceSlot, Span, TrackKind, TrackSlot, encode_state,
    encode_stream,
};
use engine::time::{Frame, Tick};

thread_local! {
    /// 0: off; n: refuse the n-th allocation from now on this thread.
    static FAIL_AT: Cell<usize> = const { Cell::new(0) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

struct Failing;

// SAFETY: forwards to the system allocator, or returns null (a refusal the `GlobalAlloc`
// contract allows) when the thread armed a failure for this allocation.
unsafe impl GlobalAlloc for Failing {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if refuse() {
            return core::ptr::null_mut();
        }
        // SAFETY: forwarded from this function's contract.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded from this function's contract.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if refuse() {
            return core::ptr::null_mut();
        }
        // SAFETY: forwarded from this function's contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn refuse() -> bool {
    let fail_at = FAIL_AT.try_with(Cell::get).unwrap_or(0);
    if fail_at == 0 {
        return false;
    }
    let n = COUNT.try_with(|c| {
        let n = c.get() + 1;
        c.set(n);
        n
    });
    n == Ok(fail_at)
}

#[global_allocator]
static ALLOCATOR: Failing = Failing;

fn armed<R>(fail_at: usize, f: impl FnOnce() -> R) -> R {
    COUNT.with(|c| c.set(0));
    FAIL_AT.with(|c| c.set(fail_at));
    let r = f();
    FAIL_AT.with(|c| c.set(0));
    r
}

fn settle(p: &mut Preparer, e: &mut Engine) {
    for _ in 0..4 {
        e.begin_block();
        p.flush();
    }
}

/// A base project: track 0 with a tone, a chunk of a staged upload of clip 1.
fn base() -> (Preparer, Engine) {
    let (mut p, mut e) = channel(DeviceSetup::default(), factory).unwrap();
    for msg in [
        M::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Midi,
        },
        M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(0),
            kind: TONE.kind_id,
            state: vec![],
        },
        upload(1, 0, 4, 2),
    ] {
        p.handle(msg).unwrap();
    }
    settle(&mut p, &mut e);
    (p, e)
}

fn upload(clip: u32, first: u32, total: u32, n: u32) -> M {
    M::ClipNotesUpload {
        clip: ClipSlot(clip),
        length: Tick(960),
        looping: None,
        first,
        total,
        notes: (first..first + n)
            .map(|i| Note {
                start: Tick(i64::from(i)),
                length: Tick(1),
                pitch: 60,
                velocity: 100,
                ..Note::default()
            })
            .collect(),
    }
}

/// Refuses each allocation of `handle(msg)` in turn; after each refusal, the same message
/// with the memory back must give exactly the state `check` expects.
fn every_site(name: &str, msg: M, check: impl Fn(&Engine)) {
    let mut refused = 0;
    for n in 1.. {
        let (mut p, mut e) = base();
        let rejected = p.rejected();
        let attempt = msg.clone();
        let result = armed(n, || p.handle(attempt));
        match result {
            Ok(()) => {
                settle(&mut p, &mut e);
                check(&e);
                assert_eq!(e.skipped(), 0, "{name}");
                break;
            }
            Err(MsgError::OutOfMemory { .. }) => {
                refused += 1;
                assert_eq!(
                    p.rejected(),
                    rejected,
                    "{name}: a refusal is not a rejection"
                );
                // Nothing committed: the retry behaves as a first attempt.
                p.handle(msg.clone())
                    .unwrap_or_else(|e| panic!("{name}, retry after refusal {n}: {e:?}"));
                settle(&mut p, &mut e);
                check(&e);
                assert_eq!(e.skipped(), 0, "{name}, after refusal {n}");
                assert_eq!(p.rejected(), rejected, "{name}, after refusal {n}");
            }
            Err(other) => panic!("{name}: allocation {n} refused gave {other:?}"),
        }
    }
    assert!(refused > 0, "{name}: no allocation site was reached");
    eprintln!("{name}: {refused} allocation sites, each refused cleanly");
}

#[test]
fn a_track_that_grows_the_table() {
    every_site(
        "TrackCreate",
        M::TrackCreate {
            track: TrackSlot(100),
            kind: TrackKind::Audio,
        },
        |e| {
            assert!(e.state().track(TrackSlot(100)).is_some());
            assert_eq!(e.state().tracks.capacity(), 128);
            assert!(e.state().track(TrackSlot(0)).is_some());
        },
    );
}

#[test]
fn a_device_with_a_state_that_grows_the_row() {
    every_site(
        "DeviceInsert",
        M::DeviceInsert {
            track: TrackSlot(0),
            device: DeviceSlot(9),
            kind: GAIN.kind_id,
            state: encode_state(&[(tone::GAIN, 0.5)]),
        },
        |e| {
            let t = e.state().track(TrackSlot(0)).unwrap();
            assert_eq!(t.devices.capacity(), 16);
            assert!(t.devices.as_slice()[0].is_some() && t.devices.as_slice()[9].is_some());
        },
    );
}

#[test]
fn the_last_chunk_of_an_upload_that_grows_the_clip_directory() {
    // Clip 1 is staged with 2 of 4 notes in the base; clip 9000 is a fresh upload in a
    // page beyond the directory.
    every_site("ClipNotesUpload (last chunk)", upload(1, 2, 4, 2), |e| {
        let ClipContent::Notes(n) = &e.state().clip(ClipSlot(1)).unwrap().content else {
            panic!()
        };
        assert_eq!(n.notes(), 4);
    });
    every_site("ClipNotesUpload (directory)", upload(9000, 0, 3, 3), |e| {
        assert!(e.state().clip(ClipSlot(9000)).is_some());
    });
}

#[test]
fn the_other_puts() {
    every_site(
        "AudioSourceDeclare",
        M::AudioSourceDeclare {
            source: SourceSlot(40),
            sample_rate: 48_000,
            channels: 2,
            frames: Frame(1),
            media: [0; 32],
        },
        |e| assert!(e.state().source(SourceSlot(40)).is_some()),
    );
    every_site(
        "ClipAudioSet",
        M::ClipAudioSet {
            clip: ClipSlot(5000),
            source: SourceSlot(0),
            start: Frame(0),
            length: Frame(1),
            gain: 1.0,
            looping: None,
        },
        |e| assert!(e.state().clip(ClipSlot(5000)).is_some()),
    );
    every_site(
        "AutomationLaneUpload",
        M::AutomationLaneUpload {
            lane: LaneSlot(2000),
            track: TrackSlot(0),
            target: AutomationTarget::MixerGain,
            first: 0,
            total: 1,
            points: vec![AutomationPoint::default()],
        },
        |e| assert!(e.state().lane(LaneSlot(2000)).is_some()),
    );
    every_site(
        "TrackPlacementsUpload",
        M::TrackPlacementsUpload {
            track: TrackSlot(0),
            first: 0,
            total: 2,
            placements: vec![
                Placement {
                    span: Span::Ticks {
                        offset: Tick(0),
                        length: Tick(960),
                    },
                    ..Placement::default()
                };
                2
            ],
        },
        |e| assert_eq!(e.state().track(TrackSlot(0)).unwrap().placements.len(), 2),
    );
    every_site(
        "SceneSetSlot",
        M::SceneSetSlot {
            track: TrackSlot(0),
            scene: SceneSlot(100),
            clip: Some(ClipSlot(1)),
        },
        |e| {
            let t = e.state().track(TrackSlot(0)).unwrap();
            assert_eq!(t.scenes.as_slice()[100], Some(ClipSlot(1)));
        },
    );
}

#[test]
fn a_reset_refused_keeps_its_epoch_for_the_retry() {
    for n in 1.. {
        let (mut p, mut e) = base();
        let stream = encode_stream(0, 0, &[M::Reset { epoch: 4 }]).unwrap();
        match armed(n, || p.submit(&stream)) {
            Ok(()) => break,
            Err(MsgError::OutOfMemory { .. }) => {
                assert_eq!(p.epoch(), 0, "refusal {n} adopted the epoch");
                p.submit(&stream).unwrap();
                assert_eq!(p.epoch(), 4);
                settle(&mut p, &mut e);
                assert!(e.state().track(TrackSlot(0)).is_none());
            }
            Err(other) => panic!("{other:?}"),
        }
    }
}

#[test]
fn decoding_a_stream_refuses_cleanly_too() {
    // The decoder's own allocations (the notes of a chunk, a device state).
    let stream = encode_stream(
        0,
        0,
        &[
            upload(7, 0, 3, 3),
            M::DeviceInsert {
                track: TrackSlot(0),
                device: DeviceSlot(1),
                kind: GAIN.kind_id,
                state: encode_state(&[(tone::GAIN, 0.5); 4]),
            },
        ],
    )
    .unwrap();
    let mut refused = 0;
    for n in 1.. {
        let (mut p, mut e) = base();
        match armed(n, || p.submit(&stream)) {
            Ok(()) => break,
            Err(MsgError::OutOfMemory { .. }) => refused += 1,
            Err(other) => panic!("{other:?}"),
        }
        // A refused stream is resent whole by the project; a message already applied is
        // applied again (a put replaces), so the result is the same.
        p.submit(&stream).unwrap();
        settle(&mut p, &mut e);
        assert!(e.state().clip(ClipSlot(7)).is_some());
        assert!(e.state().device(TrackSlot(0), DeviceSlot(1)).is_some());
    }
    assert!(refused > 0);
}

/// `Preparer::set_sample_rate` (#236) builds every device again: each allocation refused in
/// turn comes back as `OutOfMemory`, the rate and the devices stay as they were, and the
/// same call with the memory back goes through.
#[test]
fn a_sample_rate_change_refused_at_each_allocation_changes_nothing() {
    use engine::time::SampleRate;
    let mut refused = 0;
    for n in 1.. {
        let (mut p, mut e) = base();
        let applied = e.applied();
        match armed(n, || p.set_sample_rate(SampleRate::HZ_96000)) {
            Ok(()) => break,
            Err(MsgError::OutOfMemory { .. }) => {
                refused += 1;
                settle(&mut p, &mut e);
                assert_eq!(e.applied(), applied, "refusal {n}: nothing sent");
                assert_eq!(p.sample_rate(), DeviceSetup::default().sample_rate);
                p.set_sample_rate(SampleRate::HZ_96000).unwrap();
                settle(&mut p, &mut e);
                assert_eq!(e.clock().sample_rate(), SampleRate::HZ_96000);
                assert_eq!(e.applied(), applied + 2, "the rate and the tone");
            }
            Err(other) => panic!("allocation {n} refused gave {other:?}"),
        }
    }
    assert!(refused > 0, "no allocation site was reached");
}
