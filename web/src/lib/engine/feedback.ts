// Decodes the engine's report (the `Feedback` layout of crates/engine/src/msg.rs, version 10). The
// same bytes reach the page two ways: the Web engine's render instance writes them under a
// seqlock into the shared memory (`readFeedback`), the helper sends them as binary WebSocket
// frames (`decodeFeedback` on the frame). One decoder, so that both modes show the same thing.

/** Size of the fixed part of a report. */
export const FEEDBACK_HEADER_BYTES = 108
/** Where the number of device readouts sits (`FEEDBACK_READOUTS_OFFSET`, #686). */
export const READOUTS_OFFSET = 100
/** Where the device latency sits (`FEEDBACK_LATENCY_OFFSET`, #749): one place, so the field moves in one place. */
export const LATENCY_OFFSET = 104
/** Size of one meter. */
export const TRACK_METER_BYTES = 20
/** Size of one active clip (track u32, scene u16, state u8, one padding byte, anchor i64). */
export const ACTIVE_CLIP_BYTES = 16
/** Size of one recorded note (track u32, pad u32, start i64, length i64, pitch u8, velocity u8, pad). */
export const RECORDED_NOTE_BYTES = 32
/** Most recorded notes one report carries (`MAX_RECORDED_PER_REPORT`). */
export const MAX_RECORDED_PER_REPORT = 256
/** Size of one device readout (track u32, device slot u16, parameter id u16, value f32). */
export const DEVICE_READOUT_BYTES = 12
/** Most device readouts one report carries (`MAX_READOUTS_PER_REPORT`). */
export const MAX_READOUTS_PER_REPORT = 128
/** The track of a readout of the master's chain (`TrackSlot::MASTER`). */
export const MASTER_TRACK_SLOT = 0xffffffff
/** The layout version this page reads. */
export const FEEDBACK_VERSION = 10

/** Peak and RMS of both channels, linear amplitude, and the gain reduction of the strip's devices. */
export interface Meter {
  peakL: number
  peakR: number
  rmsL: number
  rmsR: number
  /** The largest gain reduction, in dB (>= 0), a device of the strip took during the period (a Compressor); 0 when none works. */
  gainReductionDb: number
}

export type TransportState = 'stopped' | 'playing' | 'recording'

/**
 * What a cell of the launch grid is doing (GR-1); idle cells are not reported. `queued`: a launch
 * waits for its boundary; `playing`; `stopping`: the clip plays and a stop (or a launch that leaves
 * the track silent) waits for its boundary; `armed`: a launch waits for the transport to run.
 */
export type ClipState = 'queued' | 'playing' | 'stopping' | 'armed'

/** A grid cell that is not idle: at most two per track (the clip that plays, the one that waits). */
export interface ActiveClip {
  /** Track slot. */
  track: number
  /** Scene slot of the cell. */
  scene: number
  state: ClipState
  /**
   * The transport tick of the clip's position 0 when it plays or stops (the tick of its launch
   * boundary); for a waiting launch, its boundary when the engine knows it, else 0.
   */
  anchor: number
}

/** A note the engine recorded (RE-2): its track slot, its start in transport ticks, how long it was held. */
export interface RecordedNote {
  /** Its number in the take, from 0. */
  index: number
  track: number
  start: number
  length: number
  pitch: number
  velocity: number
}

/**
 * The live value of a parameter a device moves by itself, after the last block of the period (#686: the Filter's
 * cutoff under its LFO and envelope), by the device's engine slots (`ViewSlots.devices`).
 */
export interface DeviceReadout {
  /** Track slot; `null` for the master's chain. */
  track: number | null
  /** Device slot in the chain. */
  device: number
  /** The parameter's id in the device's descriptor. */
  param: number
  /** In the parameter's unit. */
  value: number
}

export interface FeedbackReport {
  seq: number
  epoch: number
  transport: TransportState
  /** Playhead, in ticks (negative during a count-in). */
  playheadTick: number
  /** Playhead, in frames since the clock started. */
  playheadFrame: number
  /**
   * The host's frame at the playhead, at offset 92 (version 9, #733): with the Web engine the audio
   * context's frame (the worklet's `currentFrame`, the clock the page stamps MIDI notes in) the playhead sits at, with the
   * helper its own sample counter. `host frame - playheadFrame` is the host frame of the transport's position 0 only while it runs at one tempo, with no loop wrap and no sample-rate change since the Play (the host frame goes on while stopped, the playhead frame does not).
   */
  hostFrame: number
  /** Render time over the duration of the blocks timed in the period, 0 to 1; `null` until the host timed a block (#486). */
  cpuLoad: number | null
  /** The largest load of one peak window of the period (one block with the helper's clock, about 100 ms with the worklet's), 0 to 1; `null` like `cpuLoad`. */
  cpuPeak: number | null
  liveOverflows: number
  rejected: number
  /** Blocks in which a voice had fewer frames from disk than it needed (cumulative, AU-3). */
  diskUnderruns: number
  /** Declared audio sources the engine cannot read: blob missing, PCM cache not ready, no disk (AU-3). */
  offlineSources: number
  /** Something did not fit the budget of PCM in memory (AU-3). */
  pcmOverBudget: boolean
  /** PCM in memory (rings and resident segments), in KiB (AU-3). */
  pcmKib: number
  /**
   * Stereo frames the output protection turned to silence because a sample was non-finite (NaN, infinity), cumulative
   * since the engine was built (#667). The meters measure before the protection and skip such samples: this is the signal.
   */
  silencedFrames: number
  /**
   * The delay the devices add to what is heard, in frames at the engine's rate (#749): the longest chain latency of the
   * tracks plus the master's. The engine delays the other tracks and the metronome to match.
   */
  latencyFrames: number
  /** A track needs more than the engine compensates and is no longer aligned (bit 31 of the latency word, #749). */
  latencyCapped: boolean
  /** Most voices sounding in the tracks' devices during the period. */
  voices: number
  /** Per track slot, after the track's fader (all zeros for an empty slot). */
  tracks: Meter[]
  /** The output, after the master and the metronome. */
  master: Meter
  /** The launch grid's cells that are not idle. */
  clips: ActiveClip[]
  /**
   * The track slots the launch grid holds without a clip playing (after a stop, or when the clip
   * ended): their arrangement is silent until `launch.back_to_arrangement`. A track that plays a
   * clip is held too, and shows it in `clips`.
   */
  held: number[]
  /** The engine's number of the current or last take of recording, 0 before the first. */
  recordTake: number
  /** Notes finished in that take. */
  recordTotal: number
  /** Notes of that take the engine could not record (too many keys held at once). */
  recordDropped: number
  /** The engine captures: the take may still grow. Once false for `recordTake`, all its notes are in. */
  recordCapturing: boolean
  /** The last notes of the take (at most 256), oldest first, numbered `recordTotal - recorded.length` on. */
  recorded: RecordedNote[]
  /** The devices' readouts: enabled devices only, by track slot then device slot, the master's last. */
  readouts: DeviceReadout[]
}

const TRANSPORT: TransportState[] = ['stopped', 'playing', 'recording']
/** Indexed by the wire value; 0 and values above 4 are unknown. */
const CLIP_STATES: (ClipState | undefined)[] = [undefined, 'queued', 'playing', 'stopping', 'armed']
/** The wire value of a track the grid holds with no clip to show: it is in `held`, in no cell. */
const HELD_STATE = 5

function meterAt(view: DataView, at: number): Meter {
  return {
    peakL: view.getFloat32(at, true),
    peakR: view.getFloat32(at + 4, true),
    rmsL: view.getFloat32(at + 8, true),
    rmsR: view.getFloat32(at + 12, true),
    // A reduction is a positive number of dB: anything else (a corrupt float) shows as none.
    gainReductionDb: Math.max(0, view.getFloat32(at + 16, true)) || 0,
  }
}

/** The loudest peak of a meter, both channels. */
export function peakOf(meter: Meter): number {
  return Math.max(meter.peakL, meter.peakR)
}

/**
 * Decodes a report from its bytes: `null` for a version this page does not read, or bytes
 * shorter than the report they announce. The seqlock is the caller's (`readFeedback`).
 */
export function decodeFeedback(bytes: ArrayBufferLike, at = 0, len = bytes.byteLength - at): FeedbackReport | null {
  if (len < FEEDBACK_HEADER_BYTES + TRACK_METER_BYTES) return null
  const view = new DataView(bytes, at, len)
  if (view.getUint8(0) !== FEEDBACK_VERSION) return null
  const tracks = view.getUint32(40, true)
  // `tracks` track meters, then the output's: all must be inside the bytes.
  if (tracks > Math.floor((len - FEEDBACK_HEADER_BYTES) / TRACK_METER_BYTES) - 1) return null
  // Then the active clips, at most two per track, all inside the bytes.
  const count = view.getUint32(44, true)
  const clipsAt = FEEDBACK_HEADER_BYTES + (tracks + 1) * TRACK_METER_BYTES
  if (count > 2 * tracks || clipsAt + count * ACTIVE_CLIP_BYTES > len) return null
  const clips: ActiveClip[] = []
  const held: number[] = []
  for (let i = 0; i < count; i++) {
    const at = clipsAt + i * ACTIVE_CLIP_BYTES
    if (view.getUint8(at + 6) === HELD_STATE) {
      held.push(view.getUint32(at, true))
      continue
    }
    const state = CLIP_STATES[view.getUint8(at + 6)]
    if (state === undefined) return null
    clips.push({ track: view.getUint32(at, true), scene: view.getUint16(at + 4, true), state, anchor: Number(view.getBigInt64(at + 8, true)) })
  }
  // Then the take's last notes, all inside the bytes.
  const recordedCount = view.getUint32(48, true)
  const notesAt = clipsAt + count * ACTIVE_CLIP_BYTES
  if (recordedCount > MAX_RECORDED_PER_REPORT || notesAt + recordedCount * RECORDED_NOTE_BYTES > len) return null
  const recordTotal = view.getUint32(60, true)
  const recorded: RecordedNote[] = []
  for (let i = 0; i < recordedCount; i++) {
    const at = notesAt + i * RECORDED_NOTE_BYTES
    recorded.push({
      index: recordTotal - recordedCount + i,
      track: view.getUint32(at, true),
      start: Number(view.getBigInt64(at + 8, true)),
      length: Number(view.getBigInt64(at + 16, true)),
      pitch: view.getUint8(at + 24),
      velocity: view.getUint8(at + 25),
    })
  }
  // Then the devices' readouts, all inside the bytes.
  const readoutCount = view.getUint32(READOUTS_OFFSET, true)
  const readoutsAt = notesAt + recordedCount * RECORDED_NOTE_BYTES
  if (readoutCount > MAX_READOUTS_PER_REPORT || readoutsAt + readoutCount * DEVICE_READOUT_BYTES > len) return null
  const readouts: DeviceReadout[] = []
  for (let i = 0; i < readoutCount; i++) {
    const at = readoutsAt + i * DEVICE_READOUT_BYTES
    const value = view.getFloat32(at + 8, true)
    // A corrupt float is left out: a readout is display only.
    if (!Number.isFinite(value)) continue
    const track = view.getUint32(at, true)
    readouts.push({ track: track === MASTER_TRACK_SLOT ? null : track, device: view.getUint16(at + 4, true), param: view.getUint16(at + 6, true), value })
  }
  const meters: Meter[] = []
  for (let i = 0; i < tracks; i++) meters.push(meterAt(view, FEEDBACK_HEADER_BYTES + i * TRACK_METER_BYTES))
  const load = view.getFloat32(24, true)
  const peak = view.getFloat32(72, true)
  return {
    seq: view.getUint32(4, true),
    epoch: view.getUint16(2, true),
    transport: TRANSPORT[view.getUint8(1)] ?? 'stopped',
    playheadTick: Number(view.getBigInt64(8, true)),
    playheadFrame: Number(view.getBigUint64(16, true)),
    hostFrame: Number(view.getBigUint64(92, true)),
    cpuLoad: load >= 0 ? Math.min(load, 1) : null,
    cpuPeak: peak >= 0 ? Math.min(peak, 1) : null,
    liveOverflows: view.getUint32(28, true),
    rejected: view.getUint32(32, true),
    diskUnderruns: view.getUint32(36, true),
    offlineSources: view.getUint32(76, true),
    pcmOverBudget: (view.getUint32(80, true) & 1) === 1,
    pcmKib: view.getUint32(84, true),
    silencedFrames: view.getUint32(88, true),
    latencyFrames: view.getUint32(LATENCY_OFFSET, true) & 0x7fff_ffff,
    latencyCapped: view.getUint32(LATENCY_OFFSET, true) >>> 31 === 1,
    voices: view.getUint32(52, true),
    tracks: meters,
    master: meterAt(view, FEEDBACK_HEADER_BYTES + tracks * TRACK_METER_BYTES),
    clips,
    held,
    recordTake: view.getUint32(56, true),
    recordTotal,
    recordDropped: view.getUint32(64, true),
    recordCapturing: (view.getUint32(68, true) & 1) === 1,
    recorded,
    readouts,
  }
}

/**
 * One consistent report from the shared memory, or null while the engine writes it (odd `seq`,
 * or `seq` changed during the read) or before its first report: the caller keeps its last value.
 */
export function readFeedback(buffer: ArrayBufferLike, at: number, len: number): FeedbackReport | null {
  // `seq` through Atomics: ordered with the engine's writes, so that an unchanged even `seq`
  // after the body proves the body whole (the region is 4-byte aligned, little-endian wasm).
  const words = new Int32Array(buffer, at, 2)
  const seq = Atomics.load(words, 1) >>> 0
  if (seq === 0 || seq % 2 === 1) return null
  // Only the used part is copied out of the shared memory, then decoded from the copy.
  const header = new DataView(buffer, at, len)
  const tracks = header.getUint32(40, true)
  // The meters, then the active clips (at most two per track: a larger count is refused by the decoder).
  const clips = Math.min(header.getUint32(44, true), 2 * tracks)
  // Then the take's last notes and the devices' readouts (a larger count is refused by the decoder).
  const notes = Math.min(header.getUint32(48, true), MAX_RECORDED_PER_REPORT)
  const readouts = Math.min(header.getUint32(READOUTS_OFFSET, true), MAX_READOUTS_PER_REPORT)
  const used = Math.min(
    len,
    FEEDBACK_HEADER_BYTES + (tracks + 1) * TRACK_METER_BYTES + clips * ACTIVE_CLIP_BYTES + notes * RECORDED_NOTE_BYTES + readouts * DEVICE_READOUT_BYTES,
  )
  const copy = new Uint8Array(used)
  copy.set(new Uint8Array(buffer, at, used))
  const report = decodeFeedback(copy.buffer)
  return report && Atomics.load(words, 1) >>> 0 === seq ? report : null
}
