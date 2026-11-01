//! The prepare/apply cycle (MO-1) through every state transition, with `apply` under the
//! allocation detector (which also catches a free): every `RtMsg` variant, the swap of a
//! full table for a larger one, the contract violations, the chunked uploads, the epochs,
//! the backpressure, the garbage invariant under a concurrent audio thread, the live
//! overflow.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use engine::device::DeviceSetup;
use engine::device::testing::{GAIN, TONE, factory, gain, tone};
use engine::engine::{ClipContent, Engine, LaunchRequest, Preparer, channel, channel_with};
use engine::msg::{
    AutomationPoint, AutomationTarget, ClipSlot, ControlMsg as M, Curve, DeviceKind, DeviceSlot,
    LaneSlot, LiveEvent, MsgError, Note, PAGE_SLOTS, ParamId, Placement, Put, RtMsg, SceneSlot,
    SourceSlot, Span, Table, TrackKind, TrackSlot, encode_state, encode_stream, limits,
};
use engine::time::{Frame, LoopRegion, Quantize, Tempo, Tick, TimeSignature};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// A placement the arrangement keeps (one with no length is dropped by `prepare`).
fn placement() -> Placement {
    Placement {
        span: Span::Ticks {
            offset: Tick(0),
            length: Tick(960),
        },
        ..Placement::default()
    }
}

fn sides() -> (Preparer, Engine) {
    channel(DeviceSetup::default(), factory).unwrap()
}

fn small(ring: usize) -> (Preparer, Engine) {
    channel_with(DeviceSetup::default(), factory, ring).unwrap()
}

/// Prepares `msg` (it must reach the audio side) and applies it under the detector.
fn cycle(p: &mut Preparer, e: &mut Engine, msg: M) -> &'static str {
    let rt = p.prepare(msg).unwrap().expect("an rt message");
    let name = variant(&rt);
    assert_no_alloc(|| e.apply(rt));
    p.collect();
    name
}

/// The name of an rt message: an exhaustive match, so that a new variant does not compile
/// until this test covers it.
fn variant(rt: &RtMsg) -> &'static str {
    match rt {
        RtMsg::Reset(_) => "Reset",
        RtMsg::SetTransport { .. } => "SetTransport",
        RtMsg::SetMetronome { .. } => "SetMetronome",
        RtMsg::SetMaster { .. } => "SetMaster",
        RtMsg::TrackPut(_) => "TrackPut",
        RtMsg::TrackRemove(_) => "TrackRemove",
        RtMsg::TrackSetMix { .. } => "TrackSetMix",
        RtMsg::HoldMix { .. } => "HoldMix",
        RtMsg::TrackSetSolo { .. } => "TrackSetSolo",
        RtMsg::TrackArm { .. } => "TrackArm",
        RtMsg::TrackSetMonitor { .. } => "TrackSetMonitor",
        RtMsg::SelectTrack { .. } => "SelectTrack",
        RtMsg::DevicePut { .. } => "DevicePut",
        RtMsg::DeviceRemove { .. } => "DeviceRemove",
        RtMsg::DeviceSetEnabled { .. } => "DeviceSetEnabled",
        RtMsg::ParamSet { .. } => "ParamSet",
        RtMsg::ClipPut(_) => "ClipPut",
        RtMsg::ClipRemove(_) => "ClipRemove",
        RtMsg::SourcePut(_) => "SourcePut",
        RtMsg::SourceRemove(_) => "SourceRemove",
        RtMsg::PlacementsSet { .. } => "PlacementsSet",
        RtMsg::LanePut(_) => "LanePut",
        RtMsg::LaneRemove(_) => "LaneRemove",
        RtMsg::SceneSet { .. } => "SceneSet",
        RtMsg::Transport(_) => "Transport",
        RtMsg::Launch(_) => "Launch",
        RtMsg::AllSoundOff => "AllSoundOff",
    }
}

/// Every name [`variant`] returns (27 variants).
const ALL_VARIANTS: [&str; 27] = [
    "Reset",
    "SetTransport",
    "SetMetronome",
    "SetMaster",
    "TrackPut",
    "TrackRemove",
    "TrackSetMix",
    "HoldMix",
    "TrackSetSolo",
    "TrackArm",
    "TrackSetMonitor",
    "SelectTrack",
    "DevicePut",
    "DeviceRemove",
    "DeviceSetEnabled",
    "ParamSet",
    "ClipPut",
    "ClipRemove",
    "SourcePut",
    "SourceRemove",
    "PlacementsSet",
    "LanePut",
    "LaneRemove",
    "SceneSet",
    "Transport",
    "Launch",
    "AllSoundOff",
];

const T0: TrackSlot = TrackSlot(0);

fn note(i: u32) -> Note {
    Note {
        start: Tick(i64::from(i) * 240),
        length: Tick(240),
        pitch: (i % 128) as u8,
        velocity: 100,
        ..Note::default()
    }
}

fn create(track: u32) -> M {
    M::TrackCreate {
        track: TrackSlot(track),
        kind: TrackKind::Midi,
    }
}

fn insert(track: u32, device: u16, kind: DeviceKind) -> M {
    M::DeviceInsert {
        track: TrackSlot(track),
        device: DeviceSlot(device),
        kind,
        state: vec![],
    }
}

fn upload(clip: u32, first: u32, total: u32, n: u32) -> M {
    M::ClipNotesUpload {
        clip: ClipSlot(clip),
        length: Tick(3840),
        looping: None,
        first,
        total,
        notes: (first..first + n).map(note).collect(),
    }
}

#[test]
fn every_rt_message_applies_without_allocating() {
    let (mut p, mut e) = sides();
    let msgs = vec![
        M::Reset { epoch: 1 },
        M::SetTransport {
            tempo: Tempo::clamped(90.0),
            signature: TimeSignature::new(3, 4).unwrap(),
            launch_quantize: Quantize::Beat,
        },
        M::SetMetronome {
            enabled: true,
            volume: 0.3,
            count_in_bars: 2,
        },
        M::SetMaster {
            gain: 0.7,
            pan: 0.0,
            mute: true,
        },
        create(0),
        create(1),
        M::TrackRemove {
            track: TrackSlot(1),
        },
        M::TrackSetMix {
            track: T0,
            gain: 0.5,
            pan: -0.25,
            mute: true,
        },
        M::HoldMix {
            track: T0,
            gain: Some(0.5),
            pan: None,
        },
        M::TrackSetSolo {
            track: T0,
            solo: true,
        },
        M::TrackArm {
            track: T0,
            armed: true,
        },
        M::TrackSetMonitor {
            track: T0,
            monitor: true,
        },
        M::SelectTrack { track: Some(T0) },
        insert(0, 0, TONE.kind_id),
        insert(0, 1, GAIN.kind_id),
        M::DeviceRemove {
            track: T0,
            device: DeviceSlot(1),
        },
        M::DeviceSetEnabled {
            track: T0,
            device: DeviceSlot(0),
            enabled: false,
        },
        M::ParamSet {
            track: T0,
            device: DeviceSlot(0),
            param: tone::GAIN,
            value: 0.25,
        },
        upload(2, 0, 3, 3),
        M::ClipRemove { clip: ClipSlot(2) },
        M::AudioSourceDeclare {
            source: SourceSlot(1),
            sample_rate: 44_100,
            channels: 1,
            frames: Frame(100),
        },
        M::AudioSourceRelease {
            source: SourceSlot(1),
        },
        M::ClipAudioSet {
            clip: ClipSlot(3),
            source: SourceSlot(1),
            start: Frame(10),
            length: Frame(20),
        },
        M::TrackPlacementsUpload {
            track: T0,
            first: 0,
            total: 1,
            placements: vec![placement()],
        },
        M::AutomationLaneUpload {
            lane: LaneSlot(0),
            track: T0,
            target: AutomationTarget::MixerPan,
            first: 0,
            total: 1,
            points: vec![AutomationPoint {
                tick: Tick(0),
                value: 0.1,
                curve: Curve::Step,
            }],
        },
        M::AutomationLaneRemove { lane: LaneSlot(0) },
        M::SceneSetSlot {
            track: T0,
            scene: SceneSlot(1),
            clip: Some(ClipSlot(3)),
        },
        M::Play { from: Tick(960) },
        M::LaunchClip {
            track: T0,
            scene: SceneSlot(1),
        },
        M::LaunchScene {
            scene: SceneSlot(1),
        },
        M::AllSoundOff,
    ];
    let mut names: Vec<&str> = msgs
        .into_iter()
        .map(|msg| cycle(&mut p, &mut e, msg))
        .collect();
    names.sort_unstable();
    names.dedup();
    let mut all = ALL_VARIANTS.to_vec();
    all.sort_unstable();
    assert_eq!(names, all, "every RtMsg variant was applied");

    // What apply wrote.
    let s = e.state();
    assert_eq!(s.transport.launch_quantize, Quantize::Beat);
    assert!(s.metronome.enabled && s.master.mute);
    let t = s.track(T0).unwrap();
    assert_eq!(
        (t.gain, t.pan, t.mute, t.solo, t.armed, t.monitor),
        (0.5, -0.25, true, true, true, true)
    );
    assert_eq!(s.selected, Some(T0));
    assert!(s.track(TrackSlot(1)).is_none());
    assert!(!s.device(T0, DeviceSlot(0)).unwrap().enabled);
    assert!(s.device(T0, DeviceSlot(1)).is_none());
    assert!(s.clip(ClipSlot(2)).is_none());
    assert!(matches!(
        s.clip(ClipSlot(3)).unwrap().content,
        ClipContent::Audio { .. }
    ));
    assert!(s.source(SourceSlot(1)).is_none());
    assert_eq!(t.placements.len(), 1);
    assert!(s.lane(LaneSlot(0)).is_none());
    assert_eq!(t.scenes.as_slice()[1], Some(ClipSlot(3)));
    assert_eq!(t.launch, Some(LaunchRequest::Clip(SceneSlot(1))));
    assert!(s.playback.playing);
    assert_eq!(s.playback.cue, Some(Tick(960)));
    assert_eq!(e.skipped(), 0);
    assert_eq!(e.leaked(), 0);
}

#[test]
fn removes_of_slots_never_put_send_nothing_and_count_nothing() {
    let (mut p, mut e) = sides();
    for msg in [
        M::ClipRemove {
            clip: ClipSlot(5000),
        },
        M::AutomationLaneRemove {
            lane: LaneSlot(3000),
        },
        M::AudioSourceRelease {
            source: SourceSlot(100),
        },
    ] {
        assert!(p.prepare(msg).unwrap().is_none());
    }
    assert_eq!(e.skipped(), 0);
    // A remove of a slot whose page exists does reach the engine.
    cycle(&mut p, &mut e, upload(5000, 0, 1, 1));
    cycle(
        &mut p,
        &mut e,
        M::ClipRemove {
            clip: ClipSlot(5000),
        },
    );
    assert!(e.state().clip(ClipSlot(5000)).is_none());
    assert_eq!(e.skipped(), 0);
}

#[test]
fn a_removed_track_is_no_longer_selected() {
    let (mut p, mut e) = sides();
    cycle(&mut p, &mut e, create(3));
    cycle(
        &mut p,
        &mut e,
        M::SelectTrack {
            track: Some(TrackSlot(3)),
        },
    );
    cycle(
        &mut p,
        &mut e,
        M::TrackRemove {
            track: TrackSlot(3),
        },
    );
    cycle(&mut p, &mut e, create(3));
    assert_eq!(e.state().selected, None);
}

#[test]
fn only_the_last_chunk_allocates_the_put() {
    let (mut p, mut e) = sides();
    // A first chunk of an upload into a page never used: nothing for the audio side, and
    // the page is sent with the last chunk only.
    assert!(p.prepare(upload(9000, 0, 4, 2)).unwrap().is_none());
    let rt = p.prepare(upload(9000, 2, 4, 2)).unwrap().unwrap();
    assert!(
        matches!(&rt, RtMsg::ClipPut(put) if put.growth.as_ref().is_some_and(|g| g.page.is_some()))
    );
    assert_no_alloc(|| e.apply(rt));
    assert_eq!(notes_of(&e, 9000), Some(4));
}

#[test]
fn the_transport_and_launch_orders_land_in_the_state() {
    let (mut p, mut e) = sides();
    cycle(&mut p, &mut e, create(0));
    let region = LoopRegion::new(Tick(0), Tick(3840));
    for msg in [
        M::Play { from: Tick(10) },
        M::Seek { tick: Tick(20) },
        M::SetLoop { region },
        M::SetRecording { recording: true },
        M::StopTrack { track: T0 },
        M::StopAll,
        M::BackToArrangement,
        M::Stop,
    ] {
        cycle(&mut p, &mut e, msg);
    }
    let s = e.state();
    assert!(!s.playback.playing && s.playback.recording);
    assert_eq!(s.playback.cue, Some(Tick(20)));
    assert_eq!(s.playback.loop_region, region);
    // `BackToArrangement` found the track engaged (its stop was pending) and released it.
    assert!(s.track(T0).unwrap().back);
    assert_eq!(s.track(T0).unwrap().launch, None);
}

#[test]
fn a_full_track_table_is_swapped_for_a_larger_one_without_allocating() {
    let (mut p, mut e) = sides();
    let half = limits::MAX_TRACKS / 2;
    for slot in 0..half {
        let rt = p.prepare(create(slot)).unwrap().unwrap();
        e.apply(rt);
        p.collect();
    }
    assert_eq!(e.state().tracks.capacity(), half as usize);
    assert!(e.state().tracks.is_full());
    assert_eq!(
        e.state().tracks.as_slice().iter().flatten().count(),
        half as usize
    );
    // The next slot: prepare builds the table of MAX_TRACKS slots, apply moves the 2048
    // tracks into it, swaps and puts the new one; no allocation, no free.
    let rt = p.prepare(create(half)).unwrap().unwrap();
    assert!(matches!(
        &rt,
        RtMsg::TrackPut(Put { grown: Some(g), .. }) if g.capacity() == limits::MAX_TRACKS as usize
    ));
    assert_no_alloc(|| e.apply(rt));
    assert_eq!(e.state().tracks.capacity(), limits::MAX_TRACKS as usize);
    assert_eq!(
        e.state().tracks.as_slice().iter().flatten().count(),
        half as usize + 1
    );
    assert_eq!(
        p.collect(),
        1,
        "the old table, emptied, is one garbage item"
    );
    // The last slot of the limit fits without another growth.
    let rt = p.prepare(create(limits::MAX_TRACKS - 1)).unwrap().unwrap();
    assert!(matches!(&rt, RtMsg::TrackPut(Put { grown: None, .. })));
    assert_no_alloc(|| e.apply(rt));
    assert_eq!(e.skipped(), 0);

    // The table mechanism itself beyond the engine's limit: 4096 boxed entries into 8192.
    let mut full: Table<Option<Box<[u8; 64]>>> = Table::try_filled(4096).unwrap();
    for entry in full.as_mut_slice() {
        *entry = Some(Box::new([1; 64]));
    }
    let mut next: Table<Option<Box<[u8; 64]>>> = Table::try_filled(8192).unwrap();
    assert_no_alloc(|| next.take_slots_from(&mut full));
    assert_eq!(next.as_slice().iter().flatten().count(), 4096);
    assert_eq!(full.as_slice().iter().flatten().count(), 0);
}

#[test]
fn the_device_row_the_scene_row_and_the_paged_tables_grow_in_apply() {
    let (mut p, mut e) = sides();
    cycle(&mut p, &mut e, create(0));
    // Device row: 4 → 8 when slot 4 is used, the four devices kept.
    for d in 0..4 {
        cycle(&mut p, &mut e, insert(0, d, GAIN.kind_id));
    }
    cycle(&mut p, &mut e, insert(0, 4, TONE.kind_id));
    let t = e.state().track(T0).unwrap();
    assert_eq!(t.devices.capacity(), 8);
    assert_eq!(t.devices.as_slice().iter().flatten().count(), 5);
    // The last device slot of the limit.
    cycle(
        &mut p,
        &mut e,
        insert(0, (limits::MAX_DEVICES_PER_TRACK - 1) as u16, GAIN.kind_id),
    );
    let t = e.state().track(T0).unwrap();
    assert_eq!(t.devices.capacity(), limits::MAX_DEVICES_PER_TRACK as usize);
    assert_eq!(t.devices.as_slice().iter().flatten().count(), 6);
    // Scene row: 8 → 4096 (the limit) in one growth to the slot.
    cycle(
        &mut p,
        &mut e,
        M::SceneSetSlot {
            track: T0,
            scene: SceneSlot(0),
            clip: Some(ClipSlot(7)),
        },
    );
    cycle(
        &mut p,
        &mut e,
        M::SceneSetSlot {
            track: T0,
            scene: SceneSlot(4095),
            clip: Some(ClipSlot(9)),
        },
    );
    let t = e.state().track(T0).unwrap();
    assert_eq!(t.scenes.capacity(), limits::MAX_SCENES as usize);
    assert_eq!(t.scenes.as_slice()[0], Some(ClipSlot(7)));
    assert_eq!(t.scenes.as_slice()[4095], Some(ClipSlot(9)));
    // Clips: the last slot of the limit grows the directory to MAX_CLIPS / PAGE_SLOTS pages
    // and moves only page pointers.
    cycle(&mut p, &mut e, upload(0, 0, 1, 1));
    cycle(&mut p, &mut e, upload(limits::MAX_CLIPS - 1, 0, 2, 2));
    let s = e.state();
    assert_eq!(
        s.clips.directory_capacity(),
        limits::MAX_CLIPS as usize / PAGE_SLOTS
    );
    assert!(s.clip(ClipSlot(0)).is_some());
    assert!(s.clip(ClipSlot(limits::MAX_CLIPS - 1)).is_some());
    assert_eq!(s.clips.iter().count(), 2);
    // A second clip in an allocated page needs no growth at all.
    let rt = p.prepare(upload(1, 0, 1, 1)).unwrap().unwrap();
    assert!(matches!(&rt, RtMsg::ClipPut(put) if put.growth.is_none()));
    assert_no_alloc(|| e.apply(rt));
    // Lanes alike.
    let lane = |slot: u32| M::AutomationLaneUpload {
        lane: LaneSlot(slot),
        track: T0,
        target: AutomationTarget::MixerGain,
        first: 0,
        total: 0,
        points: vec![],
    };
    cycle(&mut p, &mut e, lane(0));
    cycle(&mut p, &mut e, lane(limits::MAX_LANES - 1));
    let s = e.state();
    assert_eq!(
        s.lanes.directory_capacity(),
        limits::MAX_LANES as usize / PAGE_SLOTS
    );
    assert_eq!(s.lanes.iter().count(), 2);
    // Sources: 16 → 4096.
    cycle(
        &mut p,
        &mut e,
        M::AudioSourceDeclare {
            source: SourceSlot(limits::MAX_AUDIO_SOURCES - 1),
            sample_rate: 48_000,
            channels: 2,
            frames: Frame(1),
        },
    );
    assert_eq!(
        e.state().sources.capacity(),
        limits::MAX_AUDIO_SOURCES as usize
    );
    assert_eq!(e.skipped(), 0);
}

#[test]
fn a_reset_swaps_the_whole_state_in_one_message() {
    let (mut p, mut e) = sides();
    for slot in 0..40 {
        cycle(&mut p, &mut e, create(slot));
    }
    cycle(&mut p, &mut e, insert(3, 0, TONE.kind_id));
    cycle(&mut p, &mut e, M::Reset { epoch: 9 });
    assert_eq!(p.epoch(), 9);
    assert_eq!(e.state().tracks.as_slice().iter().flatten().count(), 0);
    assert_eq!(e.state().tracks.capacity(), engine::engine::initial::TRACKS);
    // The shadow was reset with it: track 3 no longer exists for the prepare side.
    assert_eq!(
        p.prepare(insert(3, 0, TONE.kind_id)).err(),
        Some(MsgError::NoSuchTarget)
    );
    // And a track beyond the initial table grows it again.
    cycle(&mut p, &mut e, create(39));
    assert_eq!(e.skipped(), 0);
}

// ---------------------------------------------------------------- contract violations

#[test]
fn a_slot_beyond_the_table_is_skipped_counted_and_freed_off_the_audio_side() {
    let (mut p, mut e) = sides();
    // A put beyond the capacity without a growth (a sender that bypassed prepare).
    let rt = RtMsg::SourcePut(Put {
        slot: 500,
        value: Box::new(engine::engine::Source {
            sample_rate: 1,
            channels: 1,
            frames: Frame(1),
        }),
        grown: None,
    });
    assert_no_alloc(|| e.apply(rt));
    assert_eq!(e.skipped(), 1);
    assert_eq!(p.collect(), 1, "the refused box went to the garbage");
    // A remove beyond the table, a copy for a missing track: skipped, no garbage.
    assert_no_alloc(|| {
        e.apply(RtMsg::TrackRemove(TrackSlot(9999)));
        e.apply(RtMsg::TrackSetSolo {
            track: TrackSlot(3),
            solo: true,
        });
        e.apply(RtMsg::ClipRemove(ClipSlot(u32::MAX)));
    });
    assert_eq!(e.skipped(), 4);
    assert_eq!(p.collect(), 0);
    // A remove of an empty slot inside the table is not a violation.
    assert_no_alloc(|| e.apply(RtMsg::TrackRemove(TrackSlot(2))));
    assert_eq!(e.skipped(), 4);
}

#[test]
fn a_growth_that_is_not_larger_never_loses_a_live_entry() {
    let (mut p, mut e) = sides();
    for slot in 0..4 {
        cycle(&mut p, &mut e, create(slot));
    }
    // A "grown" table no larger than the live one: apply refuses to move into it.
    let rt = RtMsg::TrackPut(Put {
        slot: 5,
        value: Box::new(engine::engine::Track {
            kind: TrackKind::Audio,
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            armed: false,
            monitor: false,
            devices: Table::try_filled(1).unwrap(),
            scenes: Table::try_filled(1).unwrap(),
            placements: Table::from_vec(Vec::new()),
            launch: None,
            back: false,
            graph: Default::default(),
            player: Default::default(),
        }),
        grown: Some(Table::try_filled(2).unwrap()),
    });
    assert_no_alloc(|| e.apply(rt));
    assert_eq!(e.skipped(), 1);
    assert_eq!(e.state().tracks.capacity(), engine::engine::initial::TRACKS);
    assert_eq!(e.state().tracks.as_slice().iter().flatten().count(), 5);
    assert_eq!(p.collect(), 1);
}

#[test]
fn a_put_on_an_occupied_slot_sends_the_old_value_to_the_garbage() {
    let (mut p, mut e) = sides();
    cycle(&mut p, &mut e, create(0));
    cycle(&mut p, &mut e, insert(0, 0, TONE.kind_id));
    let rt = p.prepare(insert(0, 0, GAIN.kind_id)).unwrap().unwrap();
    assert_no_alloc(|| e.apply(rt));
    assert_eq!(p.collect(), 1);
    assert_eq!(
        e.state()
            .device(T0, DeviceSlot(0))
            .unwrap()
            .device
            .descriptor()
            .kind_id,
        GAIN.kind_id
    );
    // A track created again in its slot replaces it, devices included: the old one fades
    // out in the graph (MO-3), then comes back through the retire ring: one item.
    let rt = p.prepare(create(0)).unwrap().unwrap();
    assert_no_alloc(|| e.apply(rt));
    assert_eq!(p.collect(), 0);
    assert_eq!(e.graph().retiring(), 1);
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    for _ in 0..4 {
        assert_no_alloc(|| e.render(&mut l, &mut r));
    }
    assert_eq!(e.graph().retiring(), 0);
    assert_eq!(p.collect(), 1);
    assert!(e.state().device(T0, DeviceSlot(0)).is_none());
    assert_eq!(e.skipped(), 0);
}

#[test]
fn messages_naming_a_missing_target_are_refused_by_prepare() {
    let (mut p, _e) = sides();
    for msg in [
        M::TrackRemove { track: T0 },
        M::TrackSetSolo {
            track: T0,
            solo: true,
        },
        insert(0, 0, TONE.kind_id),
        M::ParamSet {
            track: T0,
            device: DeviceSlot(0),
            param: ParamId(0),
            value: 1.0,
        },
        M::SelectTrack { track: Some(T0) },
        M::LaunchClip {
            track: T0,
            scene: SceneSlot(0),
        },
    ] {
        assert_eq!(p.prepare(msg).err(), Some(MsgError::NoSuchTarget));
    }
    let (mut p, mut e) = sides();
    cycle(&mut p, &mut e, create(0));
    assert_eq!(
        p.prepare(insert(0, 0, DeviceKind(77))).err(),
        Some(MsgError::UnknownDevice(DeviceKind(77)))
    );
    assert_eq!(
        p.prepare(M::DeviceRemove {
            track: T0,
            device: DeviceSlot(3)
        })
        .err(),
        Some(MsgError::NoSuchTarget)
    );
}

#[test]
fn a_device_is_built_with_its_initial_state() {
    let (mut p, mut e) = sides();
    cycle(&mut p, &mut e, create(0));
    cycle(
        &mut p,
        &mut e,
        M::DeviceInsert {
            track: T0,
            device: DeviceSlot(0),
            kind: GAIN.kind_id,
            state: encode_state(&[(gain::MUTE, 1.0)]),
        },
    );
    // The gain device is muted from its first block: an input goes out silent.
    let entry = e.state_mut().track_mut(T0).unwrap().devices.as_mut_slice()[0]
        .as_mut()
        .unwrap();
    let mut left = [1.0f32; 64];
    let mut right = [1.0f32; 64];
    let mut out = engine::device::EventBuffer::try_with_capacity(1).unwrap();
    let ctx = engine::device::BlockCtx {
        sample_rate: 48_000.0,
        frames: 64,
        transport: Default::default(),
    };
    let mut io = engine::device::AudioIo {
        left: &mut left,
        right: &mut right,
        events_out: &mut out,
    };
    entry.device.process(&ctx, &[], &mut io);
    assert!(left.iter().chain(&right).all(|s| *s == 0.0));
}

// ---------------------------------------------------------------- chunked uploads

fn notes_of(e: &Engine, clip: u32) -> Option<usize> {
    match &e.state().clip(ClipSlot(clip))?.content {
        ClipContent::Notes(n) => Some(n.notes()),
        ClipContent::Audio { .. } => None,
    }
}

#[test]
fn a_chunked_upload_reaches_the_engine_whole_or_not_at_all() {
    let (mut p, mut e) = sides();
    // fresh → partial → partial → complete.
    assert!(p.prepare(upload(1, 0, 6, 2)).unwrap().is_none());
    assert!(p.prepare(upload(1, 2, 6, 2)).unwrap().is_none());
    cycle(&mut p, &mut e, upload(1, 4, 6, 2));
    assert_eq!(notes_of(&e, 1), Some(6));
    assert_eq!(p.rejected(), 0);
    // partial → a first chunk again: restarts, counted.
    assert!(p.prepare(upload(2, 0, 6, 2)).unwrap().is_none());
    assert!(p.prepare(upload(2, 0, 6, 2)).unwrap().is_none());
    assert_eq!(p.rejected(), 1);
    // partial → out of order: dropped, counted; then a chunk with nothing to continue.
    assert!(p.prepare(upload(2, 4, 6, 2)).unwrap().is_none());
    assert_eq!(p.rejected(), 2);
    assert!(p.prepare(upload(2, 2, 6, 2)).unwrap().is_none());
    assert_eq!(p.rejected(), 3);
    // partial → changed total: dropped, counted.
    assert!(p.prepare(upload(3, 0, 6, 2)).unwrap().is_none());
    assert!(p.prepare(upload(3, 2, 7, 2)).unwrap().is_none());
    assert_eq!(p.rejected(), 4);
    // partial → removed: the staging is gone, the next chunk continues nothing.
    assert!(p.prepare(upload(4, 0, 4, 2)).unwrap().is_none());
    // Clip 4 shares the page of clip 1: its remove reaches the engine (an empty slot).
    cycle(&mut p, &mut e, M::ClipRemove { clip: ClipSlot(4) });
    assert!(p.prepare(upload(4, 2, 4, 2)).unwrap().is_none());
    assert_eq!(p.rejected(), 5);
    // partial → Reset: gone too.
    assert!(p.prepare(upload(5, 0, 4, 2)).unwrap().is_none());
    cycle(&mut p, &mut e, M::Reset { epoch: 2 });
    assert!(p.prepare(upload(5, 2, 4, 2)).unwrap().is_none());
    assert_eq!(p.rejected(), 6);
    // Nothing partial ever reached the audio side.
    for clip in 2..=5 {
        assert_eq!(notes_of(&e, clip), None, "clip {clip}");
    }
    // An empty upload is a clip with no notes.
    cycle(&mut p, &mut e, upload(6, 0, 0, 0));
    assert_eq!(notes_of(&e, 6), Some(0));
    // Placements and lanes follow the same rules; a removed track drops its staging.
    cycle(&mut p, &mut e, create(0));
    let placements = |first: u32, total: u32| M::TrackPlacementsUpload {
        track: T0,
        first,
        total,
        placements: vec![placement()],
    };
    assert!(p.prepare(placements(0, 2)).unwrap().is_none());
    cycle(&mut p, &mut e, M::TrackRemove { track: T0 });
    cycle(&mut p, &mut e, create(0));
    assert!(p.prepare(placements(1, 2)).unwrap().is_none());
    assert_eq!(p.rejected(), 7);
    let lane = |first: u32| M::AutomationLaneUpload {
        lane: LaneSlot(1),
        track: T0,
        target: AutomationTarget::MixerGain,
        first,
        total: 2,
        points: vec![AutomationPoint::default()],
    };
    assert!(p.prepare(lane(0)).unwrap().is_none());
    // The lane was never put: the remove only drops its staging.
    assert!(
        p.prepare(M::AutomationLaneRemove { lane: LaneSlot(1) })
            .unwrap()
            .is_none()
    );
    assert!(p.prepare(lane(1)).unwrap().is_none());
    assert_eq!(p.rejected(), 8);
}

// ---------------------------------------------------------------- epochs and the stream

fn reset_stream(epoch: u16) -> Vec<u8> {
    encode_stream(0, 0, &[M::Reset { epoch }]).unwrap()
}

#[test]
fn the_stream_adopts_a_newer_epoch_and_drops_the_rest() {
    let (mut p, mut e) = sides();
    p.submit(&reset_stream(3)).unwrap();
    assert_eq!(p.epoch(), 3);
    // A message of the old epoch is dropped and counted, the stream goes on.
    let mut stream = encode_stream(2, 0, &[create(0)]).unwrap();
    stream.extend(encode_stream(3, 1, &[create(1)]).unwrap());
    p.submit(&stream).unwrap();
    assert_eq!(p.rejected(), 1);
    // A duplicate Reset of the current epoch, a late one of a past epoch: stale.
    p.submit(&reset_stream(3)).unwrap();
    p.submit(&reset_stream(1)).unwrap();
    assert_eq!(p.rejected(), 3);
    assert_eq!(p.epoch(), 3);
    e.begin_block();
    assert!(e.state().track(TrackSlot(1)).is_some());
    assert!(e.state().track(TrackSlot(0)).is_none());
}

#[test]
fn a_bad_message_is_skipped_and_counted_and_the_stream_goes_on() {
    let (mut p, mut e) = sides();
    let mut stream = encode_stream(0, 0, &[create(0)]).unwrap();
    // A known tag with a malformed payload (a bool of 2).
    let mut bad = encode_stream(
        0,
        1,
        &[M::TrackSetSolo {
            track: T0,
            solo: true,
        }],
    )
    .unwrap();
    bad[engine::msg::HEADER_BYTES + 4] = 2;
    stream.extend(&bad);
    // An unknown tag.
    let mut unknown = encode_stream(0, 2, &[M::StopAll]).unwrap();
    unknown[2..4].copy_from_slice(&999u16.to_le_bytes());
    stream.extend(&unknown);
    // A limit exceeded, a missing target, an unknown device.
    stream.extend(encode_stream(0, 3, &[create(limits::MAX_TRACKS)]).unwrap());
    stream.extend(
        encode_stream(
            0,
            4,
            &[M::TrackRemove {
                track: TrackSlot(5),
            }],
        )
        .unwrap(),
    );
    stream.extend(encode_stream(0, 5, &[insert(0, 0, DeviceKind(12345))]).unwrap());
    stream.extend(encode_stream(0, 6, &[create(1)]).unwrap());
    p.submit(&stream).unwrap();
    assert_eq!(p.rejected(), 5);
    e.begin_block();
    assert!(e.state().track(T0).is_some() && e.state().track(TrackSlot(1)).is_some());
    // A corrupt stream stops: a wrong version.
    let mut corrupt = encode_stream(0, 7, &[create(2)]).unwrap();
    corrupt[0] = 9;
    assert_eq!(p.submit(&corrupt), Err(MsgError::UnsupportedVersion(9)));
    // A truncated one too.
    let whole = encode_stream(0, 8, &[create(2)]).unwrap();
    assert!(matches!(
        p.submit(&whole[..whole.len() - 4]),
        Err(MsgError::Truncated { .. })
    ));
}

// ---------------------------------------------------------------- backpressure

#[test]
fn a_full_ring_queues_in_order_and_flushes_in_order() {
    let (mut p, mut e) = small(2);
    for slot in 0..6 {
        p.handle(create(slot)).unwrap();
    }
    // A message for the last track waits behind it.
    p.handle(M::TrackSetSolo {
        track: TrackSlot(5),
        solo: true,
    })
    .unwrap();
    assert_eq!(p.pending(), 5);
    for _ in 0..8 {
        assert_no_alloc(|| e.begin_block());
        p.flush();
    }
    assert_eq!(p.pending(), 0);
    assert_eq!(e.skipped(), 0, "the solo landed after its track");
    assert!(e.state().track(TrackSlot(5)).unwrap().solo);
}

#[test]
fn a_full_queue_is_engine_busy_with_nothing_committed_then_resumes() {
    let (mut p, mut e) = small(1);
    let pending = limits::PENDING_QUEUE_CAPACITY as u32;
    // One in the ring, the queue full.
    for slot in 0..=pending {
        p.handle(create(slot % 64)).unwrap();
    }
    assert_eq!(p.pending(), pending as usize);
    // Busy: neither the track nor its shadow exist; a Reset does not adopt its epoch.
    assert_eq!(p.handle(create(4095)), Err(MsgError::EngineBusy));
    // A stream whose first message is handled and whose second is refused: the offset of
    // the refused one says where to resume.
    // (a live note: it does not wait in the queue).
    let mut stream = encode_stream(
        0,
        0,
        &[M::AllNotesOff {
            track: TrackSlot(0),
        }],
    )
    .unwrap();
    let second = stream.len();
    stream.extend(reset_stream(7));
    assert_eq!(p.submit(&stream), Err(MsgError::EngineBusy));
    assert_eq!(p.stopped_at(), Some(second));
    assert_eq!(p.epoch(), 0, "a refused Reset leaves the epoch");
    assert_eq!(
        p.prepare(M::TrackSetSolo {
            track: TrackSlot(4095),
            solo: true
        })
        .err(),
        Some(MsgError::NoSuchTarget),
        "the refused track was not committed to the shadow"
    );
    // The engine resumes; the host's pump (`flush`) drains the queue without any submit.
    while p.pending() > 0 {
        assert_no_alloc(|| e.begin_block());
        p.flush();
    }
    assert_no_alloc(|| e.begin_block());
    // The project resends from the offset: the same Reset is accepted now.
    p.submit(&stream[second..]).unwrap();
    assert_eq!(p.stopped_at(), None);
    assert_eq!(p.epoch(), 7);
    p.handle(create(4095)).unwrap();
    assert_no_alloc(|| e.begin_block());
    p.flush();
    assert_no_alloc(|| e.begin_block());
    assert!(e.state().track(TrackSlot(4095)).is_some());
    assert_eq!(e.skipped(), 0);
}

#[test]
fn the_garbage_ring_never_fills_under_a_concurrent_audio_thread() {
    // Several rounds: one round caught a removed drain in two runs out of three.
    for _ in 0..5 {
        garbage_round();
    }
}

fn garbage_round() {
    // A small ring, an audio thread applying as fast as it can, a control side flooding
    // messages that each yield garbage, and copies that depend on the message before them:
    // apply must never find the garbage ring full, nor a message overtake another.
    let (mut p, mut e) = small(4);
    let done = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&done);
    let audio = std::thread::spawn(move || {
        // The graph renders too: the replaced tracks fade out and retire meanwhile.
        let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
        while !stop.load(Ordering::Acquire) {
            e.process(&mut l, &mut r);
        }
        e.process(&mut l, &mut r);
        while e.graph().retiring() > 0 {
            e.process(&mut l, &mut r);
        }
        e
    });
    let send = |p: &mut Preparer, msg: M| loop {
        match p.handle(msg.clone()) {
            Ok(()) => break,
            Err(MsgError::EngineBusy) => std::thread::yield_now(),
            Err(other) => panic!("{other:?}"),
        }
    };
    for i in 0..10_000u32 {
        let slot = 1 + i % 64;
        send(&mut p, create(slot));
        send(&mut p, insert(slot, 0, GAIN.kind_id));
        send(
            &mut p,
            M::ParamSet {
                track: TrackSlot(slot),
                device: DeviceSlot(0),
                param: gain::GAIN,
                value: 0.5,
            },
        );
    }
    while p.pending() > 0 {
        p.flush();
        std::thread::yield_now();
    }
    done.store(true, Ordering::Release);
    let e = audio.join().unwrap();
    assert_eq!(e.leaked(), 0, "apply always found a free garbage slot");
    assert_eq!(e.skipped(), 0, "no message overtook the one it depends on");
    assert_eq!(e.applied(), 30_000);
    p.collect();
    // Every create after the first 64 replaced a track: one retired track each, plus the
    // grown tables.
    assert!(p.collected() >= 10_000 - 64, "{}", p.collected());
}

// ---------------------------------------------------------------- live input

#[test]
fn a_full_live_queue_counts_the_overflow_and_the_block_releases_once() {
    let (mut p, mut e) = sides();
    cycle(&mut p, &mut e, create(0));
    for i in 0..limits::LIVE_QUEUE_CAPACITY {
        p.handle(M::NoteOn {
            track: T0,
            pitch: (i % 128) as u8,
            velocity: 1,
        })
        .unwrap();
    }
    // The lost note-off.
    p.handle(M::NoteOff {
        track: T0,
        pitch: 0,
    })
    .unwrap();
    assert_eq!(p.overflow().total(), 1);
    assert_no_alloc(|| e.begin_block());
    // The queued events are all taken, in order, then the release.
    assert_eq!(e.live_events().len(), limits::LIVE_QUEUE_CAPACITY);
    assert_eq!(
        e.live_events()[0],
        LiveEvent::NoteOn {
            track: T0,
            pitch: 0,
            velocity: 1
        }
    );
    assert!(e.release_live());
    assert_no_alloc(|| e.begin_block());
    assert!(!e.release_live(), "released once per overflow");
    assert!(e.live_events().is_empty());
}

#[test]
fn a_refused_allocation_is_a_typed_error() {
    // The table mechanism; every allocation site of `prepare` is driven to a refusal by the
    // failing allocator of `tests/oom.rs`.
    let huge = (isize::MAX as usize) / 2;
    assert!(matches!(
        Table::<Note>::try_with_capacity(huge),
        Err(MsgError::OutOfMemory { .. })
    ));
}
