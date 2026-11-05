//! The offline render's own mechanics (`engine::offline`, #297), without a project: a setup
//! longer than the engine's ring and queue goes in by pieces and in order, the length and the
//! formats are checked at the door, a message the engine skips fails the render, and the WAV
//! header is the one every reader expects. The renders of whole songs are in
//! `crates/test-support/tests/it/export.rs`.

use commands::engine_msgs::snapshot;
use engine::OSC_SYNTH;
use engine::msg::{ControlMsg as M, DeviceSlot, TrackSlot, encode_state, encode_stream};
use engine::offline::{
    MAX_BODY_SECS, MAX_STEP_FRAMES, Offline, OfflineError, Phase, Plan, QUANTUM, SampleFormat,
    wav_header,
};
use engine::time::Tick;
use test_support::export::demo_song;

const RATE: u32 = 48_000;
const EPOCH: u16 = 1;

fn plan(frames: u64) -> Plan {
    Plan {
        body_frames: frames,
        format: SampleFormat::Float32,
    }
}

fn master(gain: f32, mute: bool) -> M {
    M::SetMaster {
        gain,
        pan: 0.0,
        mute,
    }
}

/// The setup of a two-bar song (a bass and a lead, a note every beat) with `masters` master
/// settings between its snapshot and its `Play`, the last one set to `last`.
fn stream(masters: usize, last: M) -> Vec<u8> {
    let state = demo_song(2, None);
    let (snapshot, _) = snapshot(&state.project, &state.session);
    let mut msgs = vec![M::Reset { epoch: EPOCH }];
    msgs.extend(snapshot);
    msgs.extend((0..masters).map(|i| master(0.5 + (i % 2) as f32 * 0.25, false)));
    msgs.push(last);
    msgs.push(M::Play { from: Tick(0) });
    encode_stream(EPOCH, 0, &msgs).unwrap()
}

/// Renders `frames` frames of a stream and returns the offline render and the peak it wrote.
fn run(stream: &[u8], frames: u64) -> (Offline, f32) {
    let mut offline = Offline::new(RATE, plan(frames)).unwrap();
    offline.submit(stream).unwrap();
    while offline.step(MAX_STEP_FRAMES).unwrap() != Phase::Done {}
    let peak = offline.peak();
    (offline, peak)
}

#[test]
fn a_setup_longer_than_the_ring_goes_in_by_pieces_and_in_order() {
    // 30 000 messages: the ring holds 4 096 and the queue 4 096 more.
    let (offline, loud) = run(&stream(30_000, master(0.8, false)), 4_800);
    assert!(
        offline.busy() > 0,
        "the engine was never full: make the setup longer"
    );
    assert!(loud > 0.05, "the note is silent: {loud}");
    // The last of the messages is the one that counts: a mute at its end silences the note.
    let (muted, quiet) = run(&stream(30_000, master(0.8, true)), 4_800);
    assert!(muted.busy() > 0);
    // The mute is a short fade, so not exactly nothing: but a tenth of the note's level.
    assert!(
        quiet < loud / 10.0,
        "a message was applied out of order: {quiet} against {loud}"
    );
}

#[test]
fn a_long_setup_is_applied_whole_before_the_first_frame() {
    // The engine applies 128 messages per block: if the render did not wait for it, a setup
    // of thousands would start playing blocks late, and the file would begin with silence.
    // The song's first note is at the very start.
    let mut offline = Offline::new(RATE, plan(9_600)).unwrap();
    offline.submit(&stream(30_000, master(0.8, false))).unwrap();
    offline.step(MAX_STEP_FRAMES).unwrap();
    let first_10_ms = offline
        .pcm()
        .as_chunks::<4>()
        .0
        .iter()
        .take(2 * 480)
        .map(|b| f32::from_le_bytes(*b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        first_10_ms > 0.01,
        "the file starts with silence: {first_10_ms}"
    );
}

#[test]
fn a_short_setup_is_never_held_up() {
    let (offline, loud) = run(&stream(10, master(0.8, false)), 4_800);
    assert_eq!(offline.busy(), 0);
    assert!(loud > 0.05);
}

#[test]
fn the_content_is_written_whole_and_quiet_frames_after_it_are_not_kept() {
    // A muted master: silent from the start. The file is the content, no more, though the
    // render listened for a second of silence before it stopped.
    let (silent, peak) = run(&stream(0, master(0.8, true)), 4_800);
    assert!(peak < 0.05, "{peak}");
    assert!(!silent.tail_cut());
    // The fade of the mute (a few milliseconds) is the only sound: it is in the content.
    assert_eq!(silent.written(), 4_800);
    let listened = silent.rendered() - 4_800;
    assert!(
        (u64::from(RATE)..u64::from(RATE) + QUANTUM as u64).contains(&listened),
        "{listened}"
    );
    // Sounding: the content, then what rings after it, up to the last audible frame.
    let (sounding, peak) = run(&stream(0, master(0.8, false)), 4_800);
    assert!(peak > 0.05);
    assert!(sounding.written() >= 4_800);
}

#[test]
fn a_step_renders_whole_blocks_and_reports_its_phase() {
    let mut offline = Offline::new(RATE, plan(1_000)).unwrap();
    offline.submit(&stream(0, master(0.8, true))).unwrap();
    assert_eq!(offline.phase(), Phase::Body);
    // A step smaller than a block renders one block.
    assert_eq!(offline.step(1).unwrap(), Phase::Body);
    assert_eq!(offline.rendered(), QUANTUM as u64);
    assert_eq!(
        offline.pcm().len(),
        QUANTUM * SampleFormat::Float32.frame_bytes()
    );
    while offline.step(MAX_STEP_FRAMES).unwrap() == Phase::Body {}
    assert_eq!(offline.phase(), Phase::Tail);
    while offline.step(MAX_STEP_FRAMES).unwrap() != Phase::Done {}
    // Done stays done, and writes nothing more.
    assert_eq!(offline.step(MAX_STEP_FRAMES).unwrap(), Phase::Done);
    assert!(offline.pcm().is_empty());
}

#[test]
fn the_length_is_checked_at_the_door() {
    assert_eq!(
        Offline::new(RATE, plan(0)).unwrap_err(),
        OfflineError::Length
    );
    let longest = (MAX_BODY_SECS * f64::from(RATE)) as u64;
    assert!(Offline::new(RATE, plan(longest)).is_ok());
    assert_eq!(
        Offline::new(RATE, plan(longest + 1)).unwrap_err(),
        OfflineError::Length
    );
}

#[test]
fn a_message_the_engine_rejects_fails_the_render() {
    // A note on a track nobody created is rejected by the prepare side: the render would not
    // be the project.
    let bad = encode_stream(
        EPOCH,
        0,
        &[
            M::Reset { epoch: EPOCH },
            M::DeviceInsert {
                track: TrackSlot(7),
                device: DeviceSlot(0),
                kind: OSC_SYNTH.kind_id,
                state: encode_state(&[]),
            },
        ],
    )
    .unwrap();
    let mut offline = Offline::new(RATE, plan(1_000)).unwrap();
    offline.submit(&bad).unwrap();
    assert!(matches!(
        offline.step(MAX_STEP_FRAMES),
        Err(OfflineError::Messages { rejected: 1, .. })
    ));
    // A corrupt stream stops at the door.
    let mut offline = Offline::new(RATE, plan(1_000)).unwrap();
    assert!(matches!(
        offline.submit(&[1, 2, 3]),
        Err(OfflineError::Stream(_))
    ));
}

#[test]
fn a_stop_message_stops_the_transport_after_the_content() {
    // Content of one block, then a stop (sent by the offline render): the note that was held
    // by the transport (a clip would be) ends. Here the stop is a no-op for a live note, so
    // only the order is checked: the stop is submitted once, after the content.
    let mut offline = Offline::new(RATE, plan(QUANTUM as u64)).unwrap();
    offline.submit(&stream(0, master(0.8, true))).unwrap();
    offline.set_stop(&encode_stream(EPOCH, 10_000, &[M::Stop]).unwrap());
    while offline.step(MAX_STEP_FRAMES).unwrap() != Phase::Done {}
    assert!(offline.written() >= QUANTUM as u64);
    assert_eq!(offline.phase(), Phase::Done);
}

#[test]
fn the_header_is_what_a_reader_expects() {
    for (format, bytes, tag, bits) in [
        (SampleFormat::Pcm16, 44, 1u16, 16u16),
        (SampleFormat::Pcm24, 44, 1, 24),
        (SampleFormat::Float32, 58, 3, 32),
    ] {
        let header = wav_header(44_100, format, 1_000);
        assert_eq!(header.len(), bytes);
        assert_eq!(header.len(), format.header_bytes());
        let u16_at = |at: usize| u16::from_le_bytes([header[at], header[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
        let frame = u32::from(bits) / 8 * 2;
        assert_eq!(&header[..4], b"RIFF");
        assert_eq!(u32_at(4), bytes as u32 - 8 + 1_000 * frame);
        assert_eq!(&header[8..12], b"WAVE");
        assert_eq!(&header[12..16], b"fmt ");
        assert_eq!(u16_at(20), tag);
        assert_eq!(u16_at(22), 2);
        assert_eq!(u32_at(24), 44_100);
        assert_eq!(u32_at(28), 44_100 * frame);
        assert_eq!(u16_at(32), frame as u16);
        assert_eq!(u16_at(34), bits);
        assert_eq!(&header[bytes - 8..bytes - 4], b"data");
        assert_eq!(u32_at(bytes - 4), 1_000 * frame);
    }
    // A file past what a WAV header holds saturates instead of wrapping.
    let huge = wav_header(48_000, SampleFormat::Pcm24, u64::MAX);
    assert_eq!(
        u32::from_le_bytes(huge[40..44].try_into().unwrap()),
        u32::MAX - 64
    );
    // Formats by their bits.
    assert_eq!(SampleFormat::from_bits(24), Some(SampleFormat::Pcm24));
    assert_eq!(SampleFormat::from_bits(8), None);
}

#[test]
fn the_content_is_measured_per_track_and_the_tail_is_not() {
    let frames = u64::from(RATE) * 2;
    let mut offline = Offline::new(RATE, plan(frames)).unwrap();
    offline.submit(&stream(0, master(1.0, false))).unwrap();
    assert!(
        offline.measure().is_none(),
        "nothing is measured before the render"
    );
    let mut frozen = None;
    while offline.step(QUANTUM).unwrap() != Phase::Done {
        if frozen.is_none() && offline.phase() != Phase::Body {
            frozen = offline.measure().cloned();
        }
    }
    let frozen = frozen.expect("the measure exists as soon as the tail starts");
    assert_eq!(
        offline.measure(),
        Some(&frozen),
        "the tail changes nothing of the measure"
    );
    assert_eq!(frozen.frames, frames, "the content, not the tail");
    assert_eq!(frozen.tracks.len(), 2, "the bass and the lead");
    assert_eq!(frozen.silenced, 0);
    for level in frozen.tracks.iter().chain([&frozen.master]) {
        for (peak, rms) in [(level.peak_l, level.rms_l), (level.peak_r, level.rms_r)] {
            assert!(peak.is_finite() && rms.is_finite());
            assert!(
                (0.0..=2.0).contains(&peak) && rms <= peak + 1e-6,
                "peak {peak}, rms {rms}"
            );
        }
    }
    assert!(frozen.master.peak_l > 0.01, "the song makes sound");
    assert!(frozen.master.rms_l > 0.0);
    assert!(
        frozen.tracks.iter().any(|t| t.peak_l > 0.01),
        "a track sounds"
    );
}

#[test]
fn integer_formats_round_and_the_output_protection_keeps_the_ceiling() {
    // The synth at full gain into a master far above 0 dB: the output protection (#402)
    // holds the file under its ceiling, so nothing reaches full scale and nothing clips.
    let loud = stream(0, master(4.0, false));
    for (format, scale) in [
        (SampleFormat::Pcm16, 32_767.0f32),
        (SampleFormat::Pcm24, 8_388_607.0),
    ] {
        let mut offline = Offline::new(
            RATE,
            Plan {
                body_frames: 4_800,
                format,
            },
        )
        .unwrap();
        offline.submit(&loud).unwrap();
        let mut file = Vec::new();
        while offline.step(MAX_STEP_FRAMES).unwrap() != Phase::Done {
            file.extend_from_slice(offline.pcm());
        }
        file.extend_from_slice(offline.pcm());
        assert!(
            offline.peak() > 0.9 && offline.peak() <= engine::limiter::CEILING,
            "{format:?}: the overload is limited at the ceiling ({})",
            offline.peak()
        );
        assert_eq!(offline.clipped(), 0, "{format:?}");
        // No sample is past the ceiling, and the limited ones sit on it.
        let bytes = format.bits() as usize / 8;
        let max = file
            .chunks_exact(bytes)
            .map(|b| {
                let v = if bytes == 2 {
                    i32::from(i16::from_le_bytes([b[0], b[1]]))
                } else {
                    i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8
                };
                v.unsigned_abs()
            })
            .max()
            .unwrap();
        assert_eq!(
            max as f32,
            (engine::limiter::CEILING * scale).round(),
            "{format:?}"
        );
    }
}

// ---------------------------------------------------------------- audio clips (#647)

const TONE: [u8; 32] = [0x64; 32];
/// Frames of the tone: longer than a short source, so its clip streams through a ring.
const TONE_FRAMES: u64 = 40 * RATE as u64;

/// A mono 16-bit WAV of a 440 Hz sine at half scale.
fn tone_wav() -> Vec<u8> {
    use engine::audio_source::scenario::wav16;
    wav16(RATE, 1, TONE_FRAMES, |f, _| {
        ((f as f64 * 440.0 * core::f64::consts::TAU / f64::from(RATE)).sin() * 16_384.0) as i16
    })
}

/// The setup of an audio track whose grid clip is the whole tone, and the start that plays it
/// (`Play` and the launch).
fn tone_streams() -> (Vec<u8>, Vec<u8>) {
    use engine::msg::{ClipSlot, SceneSlot, SourceSlot, TrackKind};
    use engine::time::{Frame, Quantize, Tempo, TimeSignature};
    let setup = [
        M::Reset { epoch: EPOCH },
        M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::default(),
            launch_quantize: Quantize::None,
        },
        M::TrackCreate {
            track: TrackSlot(0),
            kind: TrackKind::Audio,
        },
        M::AudioSourceDeclare {
            source: SourceSlot(0),
            sample_rate: RATE,
            channels: 1,
            frames: Frame(TONE_FRAMES),
            media: TONE,
        },
        M::ClipAudioSet {
            clip: ClipSlot(0),
            source: SourceSlot(0),
            start: Frame(0),
            length: Frame(TONE_FRAMES),
            gain: 1.0,
            looping: None,
        },
        M::SceneSetSlot {
            track: TrackSlot(0),
            scene: SceneSlot(0),
            clip: Some(ClipSlot(0)),
        },
    ];
    let start = [
        M::Play { from: Tick(0) },
        M::LaunchClip {
            track: TrackSlot(0),
            scene: SceneSlot(0),
        },
    ];
    (
        encode_stream(EPOCH, 0, &setup).unwrap(),
        encode_stream(EPOCH, 1, &start).unwrap(),
    )
}

/// Answers the render's reads from `wav` (the tone's blob), or as not found without it.
fn serve(offline: &mut Offline, wav: Option<&[u8]>) {
    use engine::audio_source::{Failure, FileKind};
    for job in offline.take_disk_jobs() {
        match wav {
            Some(file) if job.file == FileKind::Blob && job.media == TONE => {
                let from = (job.offset as usize).min(file.len());
                let to = from.saturating_add(job.len as usize).min(file.len());
                offline.disk_done(job.id, &file[from..to], file.len() as u64);
            }
            _ => offline.disk_failed(job.id, Failure::NotFound),
        }
    }
}

fn floats(pcm: &[u8]) -> impl Iterator<Item = f32> + '_ {
    pcm.as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
}

#[test]
fn an_audio_clip_is_rendered_from_its_file_and_the_audio_path_never_allocates() {
    let wav = tone_wav();
    let (setup, start) = tone_streams();
    let body = 10 * u64::from(RATE);
    let mut offline = Offline::new(RATE, plan(body)).unwrap();
    // Every block's render runs under the detector; the planner around it may allocate.
    offline.set_render_guard(|render| assert_no_alloc::assert_no_alloc(render));
    offline.set_start(&start);
    offline.submit(&setup).unwrap();
    let mut left = Vec::new();
    loop {
        let phase = offline.step(MAX_STEP_FRAMES).unwrap();
        // Interleaved stereo floats: the left channel.
        left.extend(floats(offline.pcm()).step_by(2));
        if phase == Phase::Done {
            break;
        }
        serve(&mut offline, Some(&wav));
    }
    assert!(
        offline.disk_waits() > 0,
        "the render never waited for the disk"
    );
    assert_eq!(offline.underruns(), 0);
    assert!(offline.missing().is_empty());
    assert_eq!(offline.silenced(), 0);
    // The tone from its first block to the end of the content, past the handover from its
    // preloaded start (1.5 s) to the ring and through its refills: every 100 ms of the file
    // holds the tone's peak.
    let window = RATE as usize / 10;
    for (i, chunk) in left[..body as usize].chunks(window).enumerate().skip(1) {
        let peak = chunk.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.01, "at {} ms: {peak}", i * 100);
    }
}

#[test]
fn a_render_waits_for_its_reads_and_a_failed_one_does_not_stop_it() {
    let (setup, start) = tone_streams();
    let mut offline = Offline::new(RATE, plan(u64::from(RATE))).unwrap();
    offline.set_start(&start);
    offline.submit(&setup).unwrap();
    // The source's probe is under way: nothing is rendered while it is not answered.
    for _ in 0..3 {
        assert_eq!(offline.step(MAX_STEP_FRAMES).unwrap(), Phase::Body);
        assert!(offline.waiting());
        assert_eq!(offline.rendered(), 0);
        assert!(offline.pcm().is_empty());
    }
    let jobs = offline.take_disk_jobs();
    assert!(!jobs.is_empty());
    // Taken but not answered: still waiting.
    assert!(offline.waiting());
    offline.step(MAX_STEP_FRAMES).unwrap();
    assert_eq!(offline.rendered(), 0);
    for job in jobs {
        offline.disk_failed(job.id, engine::audio_source::Failure::NotFound);
    }
    // The file is missing: the render goes on, silent, every probe again answered at once,
    // and reports the media once the clip asked for it.
    let mut peak = 0.0f32;
    loop {
        let phase = offline.step(MAX_STEP_FRAMES).unwrap();
        peak = floats(offline.pcm()).fold(peak, |m, v| m.max(v.abs()));
        if phase == Phase::Done {
            break;
        }
        serve(&mut offline, None);
    }
    assert_eq!(offline.written(), u64::from(RATE));
    assert_eq!(peak, 0.0);
    assert_eq!(offline.missing(), &[TONE]);
}

#[test]
fn a_render_without_audio_never_waits() {
    let (offline, loud) = run(&stream(10, master(0.8, false)), 4_800);
    assert_eq!(offline.disk_waits(), 0);
    assert!(!offline.waiting());
    assert!(loud > 0.05);
}

// #755: the device latency the engine compensates is dropped from the start of the file.

const IMPULSES: [u8; 32] = [0x75; 32];
/// The impulses of the source, `(frame, level)`, at the sample rate: on the clip's second frame
/// (its first is the start of the clip player's fade in, at a gain of 0) and on its last (in its
/// fade out, near 0), the loudest on frame 120 and another 120 frames before the end (past the
/// 2 ms fades at 44.1 and 48 kHz, within the longest latency of the tests from the content's
/// edges), and one inside, each of its own level so that a level measured over the wrong
/// frames shows.
fn impulses(rate: u32) -> [(u64, i16); 5] {
    let frames = impulse_frames(rate);
    [
        (1, 8_192),
        (120, 12_000),
        (frames / 3, 4_096),
        (frames - 120, 6_144),
        (frames - 1, 8_192),
    ]
}

/// The source's length: half a second, a short source kept whole in memory.
fn impulse_frames(rate: u32) -> u64 {
    u64::from(rate) / 2
}

/// A mono 16-bit WAV of silence with [`impulses`].
fn impulse_wav(rate: u32) -> Vec<u8> {
    use engine::audio_source::scenario::wav16;
    let at = impulses(rate);
    wav16(rate, 1, impulse_frames(rate), |f, _| {
        at.iter().find(|(i, _)| *i == f).map_or(0, |(_, v)| *v)
    })
}

/// A Ceiling below its ceiling (0 dB) at `device` of `track` with `lookahead` (seconds; `None`:
/// the default), with or without the true-peak detection: its output is its input delayed by
/// its latency, bit for bit.
fn ceiling(track: TrackSlot, device: u16, true_peak: bool, lookahead: Option<f32>) -> M {
    use engine::devices::ceiling::{CEILING, DESCRIPTOR, LOOKAHEAD, TRUE_PEAK};
    use engine::msg::ParamId;
    let mut state = vec![
        (ParamId(CEILING), 0.0),
        (ParamId(TRUE_PEAK), if true_peak { 1.0 } else { 0.0 }),
    ];
    if let Some(seconds) = lookahead {
        state.push((ParamId(LOOKAHEAD), seconds));
    }
    M::DeviceInsert {
        track,
        device: DeviceSlot(device),
        kind: DESCRIPTOR.kind_id,
        state: encode_state(&state),
    }
}

/// The latency of a Ceiling at its default look-ahead at `rate`.
fn ceiling_latency(rate: u32, true_peak: bool) -> u64 {
    use engine::devices::ceiling::{
        DESCRIPTOR, LOOKAHEAD, TP_DELAY, TP_MIN_LOOKAHEAD, lookahead_frames,
    };
    let default = DESCRIPTOR
        .params
        .iter()
        .find(|p| p.id.0 == LOOKAHEAD)
        .unwrap()
        .default;
    let frames = lookahead_frames(default as f32, rate as f32);
    u64::from(if true_peak {
        frames.max(TP_MIN_LOOKAHEAD) + TP_DELAY
    } else {
        frames
    })
}

/// The setup of two audio tracks playing the impulses from bar 1, with `devices` inserted after
/// them, and the start that plays them (`Play` and the launches).
fn impulse_streams(rate: u32, devices: &[M]) -> (Vec<u8>, Vec<u8>) {
    use engine::msg::{ClipSlot, SceneSlot, SourceSlot, TrackKind};
    use engine::time::{Frame, Quantize, Tempo, TimeSignature};
    let frames = impulse_frames(rate);
    let mut setup = vec![
        M::Reset { epoch: EPOCH },
        M::SetTransport {
            tempo: Tempo::new(120.0).unwrap(),
            signature: TimeSignature::default(),
            launch_quantize: Quantize::None,
        },
        M::AudioSourceDeclare {
            source: SourceSlot(0),
            sample_rate: rate,
            channels: 1,
            frames: Frame(frames),
            media: IMPULSES,
        },
    ];
    let mut start = vec![M::Play { from: Tick(0) }];
    for slot in 0..2 {
        setup.extend([
            M::TrackCreate {
                track: TrackSlot(slot),
                kind: TrackKind::Audio,
            },
            M::ClipAudioSet {
                clip: ClipSlot(slot),
                source: SourceSlot(0),
                start: Frame(0),
                length: Frame(frames),
                gain: 1.0,
                looping: None,
            },
            M::SceneSetSlot {
                track: TrackSlot(slot),
                scene: SceneSlot(0),
                clip: Some(ClipSlot(slot)),
            },
        ]);
        start.push(M::LaunchClip {
            track: TrackSlot(slot),
            scene: SceneSlot(0),
        });
    }
    setup.extend_from_slice(devices);
    (
        encode_stream(EPOCH, 0, &setup).unwrap(),
        encode_stream(EPOCH, 1, &start).unwrap(),
    )
}

/// A finished export: the render and the left and right channels of its file.
type Exported = (Offline, Vec<f32>, Vec<f32>);

/// An export of the impulses' song at `rate` (its body: the clip), every block's render under
/// the allocation detector.
fn export_impulses(rate: u32, devices: &[M]) -> Exported {
    use engine::audio_source::{Failure, FileKind};
    let wav = impulse_wav(rate);
    let (setup, start) = impulse_streams(rate, devices);
    let mut offline = Offline::new(rate, plan(impulse_frames(rate))).unwrap();
    offline.set_render_guard(|render| assert_no_alloc::assert_no_alloc(render));
    offline.set_start(&start);
    offline.submit(&setup).unwrap();
    let mut samples = Vec::new();
    loop {
        let phase = offline.step(MAX_STEP_FRAMES).unwrap();
        samples.extend(floats(offline.pcm()));
        if phase == Phase::Done {
            break;
        }
        for job in offline.take_disk_jobs() {
            if job.file == FileKind::Blob && job.media == IMPULSES {
                let from = (job.offset as usize).min(wav.len());
                let to = from.saturating_add(job.len as usize).min(wav.len());
                offline.disk_done(job.id, &wav[from..to], wav.len() as u64);
            } else {
                offline.disk_failed(job.id, Failure::NotFound);
            }
        }
    }
    assert_eq!(offline.silenced(), 0);
    assert_eq!(offline.underruns(), 0);
    assert!(samples.iter().all(|s| s.is_finite()));
    let left = samples.iter().step_by(2).copied().collect();
    let right = samples.iter().skip(1).step_by(2).copied().collect();
    (offline, left, right)
}

/// The frames of `channel` that are not silent.
fn sounding(channel: &[f32]) -> Vec<usize> {
    (0..channel.len()).filter(|&i| channel[i] != 0.0).collect()
}

/// What two exports wrote: the same file to the sample, and the same levels of the content.
fn assert_same_export(what: &str, a: &Exported, b: &Exported) {
    let ((x, xl, xr), (y, yl, yr)) = (a, b);
    assert_eq!(xl.len(), yl.len(), "{what}: the length");
    let differs = (0..xl.len()).find(|&i| xl[i] != yl[i] || xr[i] != yr[i]);
    assert_eq!(differs, None, "{what}: the first frame that differs");
    // Every strip's peak and RMS over the same frames of the song, to the bit.
    assert_eq!(x.measure(), y.measure(), "{what}: the levels");
}

/// A case of the latency test: its name, where each Ceiling goes and whether it detects true
/// peaks, and the latency `T + M` it gives.
type Case<'a> = (&'a str, &'a [(TrackSlot, bool)], u64);

#[test]
fn an_export_starts_on_the_songs_first_frame_whatever_the_device_latency() {
    let (track, master) = (TrackSlot(1), TrackSlot::MASTER);
    for rate in [44_100, 48_000, 96_000] {
        let body = impulse_frames(rate);
        let plain = export_impulses(rate, &[]);
        assert_eq!(plain.0.lead(), 0);
        // The impulses on their frames of the file and nothing else, from the song's second
        // frame (its first is the clip's fade in at 0) to its last.
        let at: Vec<usize> = impulses(rate).iter().map(|(f, _)| *f as usize).collect();
        assert_eq!(sounding(&plain.1), at, "{rate} Hz: without a device");
        assert_eq!(
            plain.0.written(),
            body,
            "the body; a silent tail is not kept"
        );
        assert_eq!(plain.0.measure().unwrap().frames, body);

        let (l, tp) = (ceiling_latency(rate, false), ceiling_latency(rate, true));
        // Where each Ceiling goes, whether it detects true peaks, and the latency `T + M`.
        let cases: [Case; 5] = [
            ("a Ceiling on a track", &[(track, false)], l),
            ("a Ceiling on a track, true peak", &[(track, true)], tp),
            ("a Ceiling on the master", &[(master, false)], l),
            ("a Ceiling on the master, true peak", &[(master, true)], tp),
            (
                "a Ceiling on a track and one on the master, true peak",
                &[(track, false), (master, true)],
                l + tp,
            ),
        ];
        for (name, at, lead) in cases {
            let what = format!("{rate} Hz, {name}");
            let latent: Vec<M> = at.iter().map(|&(t, tp)| ceiling(t, 0, tp, None)).collect();
            // The same devices without a look-ahead nor the true-peak detection: no latency.
            let flat: Vec<M> = at
                .iter()
                .map(|&(t, _)| ceiling(t, 0, false, Some(0.0)))
                .collect();
            let latent = export_impulses(rate, &latent);
            let flat = export_impulses(rate, &flat);
            println!(
                "{what}: lead {} frames, {} frames written, the first impulse at frame {:?}",
                latent.0.lead(),
                latent.0.written(),
                sounding(&latent.1).first()
            );
            assert_eq!(latent.0.lead(), lead, "{what}: the engine's T + M");
            assert_eq!(flat.0.lead(), 0, "{what}");
            // The song's start on the file's first frame, its last frame in the body, the same
            // tail; the levels of the same stretch of the song, each strip's after its own
            // latency (track 0 has none, track 1 may, the output `T + M`).
            assert_same_export(&what, &latent, &flat);
            assert_eq!(sounding(&latent.1), sounding(&plain.1), "{what}");
            if at.iter().all(|&(t, _)| t == master) {
                // A Ceiling on the master leaves the file as the project without it.
                assert_same_export(&format!("{what}, against no device"), &latent, &plain);
            }
        }
    }
}

#[test]
fn under_a_capped_compensation_the_file_starts_where_the_first_track_comes_out() {
    use engine::msg::limits::MAX_COMPENSATION_FRAMES;
    let rate = 48_000;
    // 66 Ceilings at their longest look-ahead with the true-peak detection on track 1: more
    // latency than the compensation delays track 0 by, so track 0 comes out early.
    let devices: Vec<M> = (0..66)
        .map(|d| ceiling(TrackSlot(1), d, true, Some(0.005)))
        .collect();
    let (offline, left, _) = export_impulses(rate, &devices);
    let track1 = 66 * u64::from(ceiling_latency_at(rate, 0.005));
    assert!(track1 > u64::from(MAX_COMPENSATION_FRAMES));
    println!(
        "track 1: {track1} frames, lead {} frames, first sound at {:?}",
        offline.lead(),
        sounding(&left).first()
    );
    // Track 0, delayed by the limit, comes out first: the file starts on its first frame.
    assert_eq!(offline.lead(), u64::from(MAX_COMPENSATION_FRAMES));
    assert_eq!(sounding(&left).first(), Some(&1));
    // Track 1 follows by what the compensation could not give.
    let late = (track1 - u64::from(MAX_COMPENSATION_FRAMES)) as usize;
    assert!(
        left[1 + late] != 0.0,
        "track 1's first impulse at {}",
        1 + late
    );
}

/// The latency of a Ceiling at `lookahead` seconds with the true-peak detection at `rate`.
fn ceiling_latency_at(rate: u32, lookahead: f32) -> u32 {
    use engine::devices::ceiling::{TP_DELAY, TP_MIN_LOOKAHEAD, lookahead_frames};
    lookahead_frames(lookahead, rate as f32).max(TP_MIN_LOOKAHEAD) + TP_DELAY
}
