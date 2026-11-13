/**
 * The player's pure rules: time text, the seek keys, the slider's spoken value. Kept apart from the
 * DOM so every rule is tested (`live.test.ts`).
 */

/** `m:ss` from seconds; 0:00 for anything that is not a finite positive number. */
export function clock(seconds: number): string {
  const s = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

/** Replaces `{name}` placeholders; unknown names stay as they are. */
export function fill(template: string, vars: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (all, name: string) => vars[name] ?? all)
}

/** Seconds a seek key moves: arrows 5 s, Page keys 15 s. */
export const SEEK_STEP = 5
export const SEEK_PAGE = 15

/**
 * Where a key moves the playhead, or `undefined` for a key the slider leaves alone. The result is
 * clamped to the track; Home and End go to its ends.
 */
export function seekByKey(key: string, current: number, duration: number): number | undefined {
  const end = Number.isFinite(duration) && duration > 0 ? duration : 0
  const now = Number.isFinite(current) ? current : 0
  const to = (t: number) => Math.min(end, Math.max(0, t))
  switch (key) {
    case 'ArrowRight':
    case 'ArrowUp':
      return to(now + SEEK_STEP)
    case 'ArrowLeft':
    case 'ArrowDown':
      return to(now - SEEK_STEP)
    case 'PageUp':
      return to(now + SEEK_PAGE)
    case 'PageDown':
      return to(now - SEEK_PAGE)
    case 'Home':
      return 0
    case 'End':
      return end
    default:
      return undefined
  }
}

/** The track's length: the media's once it is known, else the one the entry states. */
export function lengthOf(mediaDuration: number, stated: number): number {
  return Number.isFinite(mediaDuration) && mediaDuration > 0 ? mediaDuration : stated
}

/** Seconds in a section of `bars` bars of four beats at `bpm`. */
export function sectionSeconds(bars: number, bpm: number): number {
  return (bars * 4 * 60) / bpm
}

/**
 * The form section playing at `time` and how far through it the playhead is (0 to 1). The sections
 * follow each other from 0 s, their lengths cumulated from `bars × 4 × 60 / bpm`. Before 0 s it is
 * the first section; past the last one's end (the audio's tail) it stays on the last, full. No
 * answer for a time that is not a number, or a form that has no length.
 */
export function sectionAt(time: number, bpm: number, bars: readonly number[]): { index: number; progress: number } | undefined {
  if (!Number.isFinite(time) || !(bpm > 0) || bars.length === 0) return undefined
  let start = 0
  for (let i = 0; i < bars.length; i++) {
    const length = sectionSeconds(bars[i], bpm)
    const end = start + length
    if (time < end || i === bars.length - 1) {
      const progress = length > 0 ? (time - start) / length : 1
      return { index: i, progress: Math.min(1, Math.max(0, progress)) }
    }
    start = end
  }
  return undefined
}
