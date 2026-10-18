// Decodes the engine's report (the `Feedback` layout of crates/engine/src/msg.rs, version 2). The
// same bytes reach the page two ways: the Web engine's render instance writes them under a
// seqlock into the shared memory (`readFeedback`), the helper sends them as binary WebSocket
// frames (`decodeFeedback` on the frame). One decoder, so that both modes show the same thing.

/** Size of the fixed part of a report. */
export const FEEDBACK_HEADER_BYTES = 56
/** Size of one meter. */
export const TRACK_METER_BYTES = 16
/** The layout version this page reads. */
export const FEEDBACK_VERSION = 2

/** Peak and RMS of both channels, linear amplitude. */
export interface Meter {
  peakL: number
  peakR: number
  rmsL: number
  rmsR: number
}

export type TransportState = 'stopped' | 'playing' | 'recording'

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
}

const TRANSPORT: TransportState[] = ['stopped', 'playing', 'recording']

function meterAt(view: DataView, at: number): Meter {
  return {
    peakL: view.getFloat32(at, true),
    peakR: view.getFloat32(at + 4, true),
    rmsL: view.getFloat32(at + 8, true),
    rmsR: view.getFloat32(at + 12, true),
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
  const tracks = new DataView(buffer, at, len).getUint32(40, true)
  const used = Math.min(len, FEEDBACK_HEADER_BYTES + (tracks + 1) * TRACK_METER_BYTES)
  const copy = new Uint8Array(used)
  copy.set(new Uint8Array(buffer, at, used))
  const report = decodeFeedback(copy.buffer)
  return report && Atomics.load(words, 1) >>> 0 === seq ? report : null
}
