// Reads the engine's report (the `Feedback` layout of crates/engine/src/msg.rs, written by the
// render instance under a seqlock) from the page's main thread, through the shared memory.

/** Size of the fixed part of a report. */
export const FEEDBACK_HEADER_BYTES = 56
/** Size of one track meter. */
export const TRACK_METER_BYTES = 16
/** The layout version this page reads. */
export const FEEDBACK_VERSION = 1

export interface FeedbackReport {
  seq: number
  epoch: number
  liveOverflows: number
  rejected: number
  /** Most voices sounding in the synth during the period (interim word, MO-2). */
  voices: number
  /** Per track slot: peak of the left and right channels, linear. */
  peaks: number[]
}

/**
 * One consistent report, or null while the engine writes it (odd `seq`, or `seq` changed
 * during the read) or before its first report: the caller keeps its last value.
 */
export function readFeedback(buffer: ArrayBufferLike, at: number, len: number): FeedbackReport | null {
  const view = new DataView(buffer, at, len)
  const seq = view.getUint32(4, true)
  if (seq === 0 || seq % 2 === 1) return null
  if (view.getUint8(0) !== FEEDBACK_VERSION) return null
  const tracks = Math.min(view.getUint32(40, true), Math.floor((len - FEEDBACK_HEADER_BYTES) / TRACK_METER_BYTES))
  const peaks: number[] = []
  for (let i = 0; i < tracks; i++) {
    const m = FEEDBACK_HEADER_BYTES + i * TRACK_METER_BYTES
    peaks.push(Math.max(view.getFloat32(m, true), view.getFloat32(m + 4, true)))
  }
  const report: FeedbackReport = {
    seq,
    epoch: view.getUint16(2, true),
    liveOverflows: view.getUint32(28, true),
    rejected: view.getUint32(32, true),
    voices: view.getUint32(52, true),
    peaks,
  }
  return view.getUint32(4, true) === seq ? report : null
}
