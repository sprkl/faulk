import type { Command } from '../generated/protocol'

/**
 * The audio context's frame at which a note was played (#499): the frame the worklet was rendering
 * at `timeStamp`, an instant of the page's performance clock (`MIDIMessageEvent.timeStamp`,
 * `KeyboardEvent.timeStamp`). `stamp` is `AudioContext.getOutputTimestamp()`: the context time heard
 * at `performanceTime`. The frame being rendered runs ahead of the one heard by the output latency,
 * so `latencySeconds` is added back: the same estimate `record.start` takes as `latency_ms` and
 * moves every note of the take earlier by, so that the two cancel and the note lands where it was
 * heard, whatever the estimate. `null` when the context gives no usable time (not running yet).
 */
export function contextFrameAt(
  stamp: { contextTime?: number; performanceTime?: number } | null | undefined,
  timeStamp: number | undefined,
  latencySeconds: number,
  sampleRate: number,
): number | null {
  if (timeStamp === undefined || !Number.isFinite(timeStamp)) return null
  const { contextTime, performanceTime } = stamp ?? {}
  // A context that has not output anything yet reports a performance time of 0.
  if (contextTime === undefined || !performanceTime || !Number.isFinite(contextTime) || !Number.isFinite(performanceTime)) return null
  const seconds = contextTime + (timeStamp - performanceTime) / 1000 + (Number.isFinite(latencySeconds) ? latencySeconds : 0)
  const frame = Math.round(seconds * sampleRate)
  return frame >= 0 && Number.isSafeInteger(frame) ? frame : null
}

/** A live input note with the frame it was played at (`frame` of the command); any other command, or no frame, as it is. */
export function withFrame(command: Command, frame: number | null): Command {
  if (frame === null) return command
  if (command.type === 'live.input_note_on' || command.type === 'live.input_note_off') return { ...command, frame }
  return command
}

/**
 * The page clock ↔ audio frame correspondence, smoothed (#725). `getOutputTimestamp()` pairs a context time with a
 * performance time, and the pair is refreshed once per audio callback with the jitter of that callback's own
 * timestamp: converting each note with the pair at hand moved single notes of a batch by up to 3.5 ms (measured in
 * Chromium on Windows). The offset between the two clocks drifts far slower than that, so the correspondence is the
 * median offset of the pairs seen in the last `WINDOW_MS`, which drops those outliers; drift is followed because old
 * pairs leave the window. With a single pair it is that pair.
 */
const WINDOW_MS = 1000
const MAX_SAMPLES = 64
/**
 * An offset further than this from the window's median is a step, not jitter. The spread of single pairs measured in
 * Chromium on Windows is 3.5 ms at most; a suspension or a device switch moves the offset by tens of milliseconds or
 * more. An isolated outlier above it only restarts the window: the conversion is then the pair itself, as before #725.
 */
const STEP_MS = 10

export class OutputClock {
  private readonly samples: { perf: number; offset: number }[] = []

  /** Takes the context's current pair in (a pair already seen, or an unusable one, is ignored) and returns the smoothed one. */
  observe(stamp: { contextTime?: number; performanceTime?: number } | null | undefined): { contextTime: number; performanceTime: number } | null {
    const { contextTime, performanceTime } = stamp ?? {}
    if (contextTime === undefined || !performanceTime || !Number.isFinite(contextTime) || !Number.isFinite(performanceTime)) return null
    const last = this.samples[this.samples.length - 1]
    if (last && performanceTime < last.perf)
      this.samples.length = 0 // The performance clock went back: start again.
    else if (last && performanceTime === last.perf) return this.pair(last.perf)
    const offset = contextTime - performanceTime / 1000
    // A step (a resume after a suspension, a device switch) is no jitter: the window starts again from the new pair.
    if (this.samples.length > 0 && Math.abs(offset - this.median()) * 1000 > STEP_MS) this.samples.length = 0
    this.samples.push({ perf: performanceTime, offset })
    while (this.samples.length > MAX_SAMPLES || (this.samples[0] && performanceTime - this.samples[0].perf > WINDOW_MS)) this.samples.shift()
    return this.pair(performanceTime)
  }

  private median(): number {
    const offsets = this.samples.map((s) => s.offset).sort((a, b) => a - b)
    const mid = offsets.length >> 1
    return offsets.length % 2 ? offsets[mid]! : (offsets[mid - 1]! + offsets[mid]!) / 2
  }

  private pair(performanceTime: number) {
    return { contextTime: performanceTime / 1000 + this.median(), performanceTime }
  }
}
