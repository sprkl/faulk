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
