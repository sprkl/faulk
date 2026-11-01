// Decodes the engine's report (the `Feedback` layout of crates/engine/src/msg.rs, version 4). The
// same bytes reach the page two ways: the Web engine's render instance writes them under a
// seqlock into the shared memory (`readFeedback`), the helper sends them as binary WebSocket
// frames (`decodeFeedback` on the frame). One decoder, so that both modes show the same thing.

/** Size of the fixed part of a report. */
export const FEEDBACK_HEADER_BYTES = 56
/** Size of one meter. */
export const TRACK_METER_BYTES = 20
/** Size of one active clip (track u32, scene u16, state u8, one padding byte, anchor i64). */
export const ACTIVE_CLIP_BYTES = 16
/** The layout version this page reads. */
export const FEEDBACK_VERSION = 4

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

export interface FeedbackReport {
  seq: number
  epoch: number
  transport: TransportState
  /** Playhead, in ticks (negative during a count-in). */
  playheadTick: number
  /** Playhead, in frames since the clock started. */
  playheadFrame: number
  /** Render time over block time, 0 to 1; `null` where the host cannot measure it. */
  cpuLoad: number | null
  liveOverflows: number
  rejected: number
  diskUnderruns: number
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
  const meters: Meter[] = []
  for (let i = 0; i < tracks; i++) meters.push(meterAt(view, FEEDBACK_HEADER_BYTES + i * TRACK_METER_BYTES))
  const load = view.getFloat32(24, true)
  return {
    seq: view.getUint32(4, true),
    epoch: view.getUint16(2, true),
    transport: TRANSPORT[view.getUint8(1)] ?? 'stopped',
    playheadTick: Number(view.getBigInt64(8, true)),
    playheadFrame: Number(view.getBigUint64(16, true)),
    cpuLoad: load >= 0 ? Math.min(load, 1) : null,
    liveOverflows: view.getUint32(28, true),
    rejected: view.getUint32(32, true),
    diskUnderruns: view.getUint32(36, true),
    voices: view.getUint32(52, true),
    tracks: meters,
    master: meterAt(view, FEEDBACK_HEADER_BYTES + tracks * TRACK_METER_BYTES),
    clips,
    held,
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
  const used = Math.min(len, FEEDBACK_HEADER_BYTES + (tracks + 1) * TRACK_METER_BYTES + clips * ACTIVE_CLIP_BYTES)
  const copy = new Uint8Array(used)
  copy.set(new Uint8Array(buffer, at, used))
  const report = decodeFeedback(copy.buffer)
  return report && Atomics.load(words, 1) >>> 0 === seq ? report : null
}
