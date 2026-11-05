/**
 * Audio frames on the timeline (AU-6): an audio placement stores frames of its file, and its ticks are computed at the
 * project's tempo, the rule of `crates/commands` (`clip::placement_ticks`, `Tempo::frames_to_ticks`): the frames times
 * `bpm × 960 / (60 × rate)`, rounded to the nearest tick, at least one. The same floating-point steps as the Rust side,
 * so the page and the project agree to the tick.
 */

const PPQ = 960

/** The ticks `frames` frames at `rate` Hz take at `bpm`, at least one (a placement covers a tick at least). */
export function framesToTicks(frames: number, rate: number, bpm: number): number {
  if (!(frames > 0) || !(rate > 0) || !(bpm > 0)) return 1
  return Math.max(1, Math.round(frames * ((bpm * PPQ) / (60 * rate))))
}

/** How many frames at `rate` Hz one tick holds at `bpm` (a fraction). */
export const framesPerTick = (rate: number, bpm: number): number => (60 * rate) / (bpm * PPQ)
