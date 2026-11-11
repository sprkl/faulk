/**
 * Repeat a selection of notes end to end (#710): the page's copy of the rules of `clip.notes.repeat`, so that it can select
 * the copies the command writes and say what it did. The two are held together by one table of cases
 * (`crates/commands/tests/it/notes_repeat_cases.json`), run against the command and against these functions.
 */

import type { Note } from '../../lib/generated/protocol'
import { barTicks, beatTicks, type Signature } from '../transport/time'
import { noteKey } from './roll'

/** Why a repeat cannot be done: a grid step below one tick, `until` not after the first copy, or no room in the clip. */
export type RepeatRefusal = 'grid' | 'until' | 'room'

export type RepeatResult =
  { ok: true; copies: Note[]; atClipEnd: boolean; firstCopy: number; period: number } | { ok: false; refusal: RepeatRefusal; firstCopy: number }

/** The first start and the last end (a loop: a selection of many thousand notes would overflow a spread). */
function span(notes: readonly Note[]): { first: number; last: number } {
  let first = Infinity
  let last = -Infinity
  for (const n of notes) {
    first = Math.min(first, n.start)
    last = Math.max(last, n.start + n.length)
  }
  return { first, last }
}

/**
 * The period of the repetition (#770): the extent of the notes, from the `grid` line at or before the first start to the
 * last end, rounded up to whole grid steps for a figure of one beat or less, to whole beats up to half a bar, to whole bars
 * beyond (never a unit finer than the grid); beat and bar are the signature's. Null grid: the exact span, first start to
 * last end.
 */
export function repeatPeriod(selected: readonly Note[], grid: number | null, sig: Signature): number {
  if (selected.length === 0) return 0
  const { first, last } = span(selected)
  if (grid === null) return Math.max(1, last - first)
  const step = Math.max(1, grid)
  const extent = Math.max(1, last - Math.floor(first / step) * step)
  const beat = beatTicks(sig)
  const bar = barTicks(sig)
  const unit = extent <= beat ? step : extent * 2 <= bar ? Math.max(beat, step) : Math.max(bar, step)
  return Math.max(1, Math.ceil(extent / unit) * unit)
}

/**
 * The copies `clip.notes.repeat` writes: every note `k` periods later for each copy of the figure that starts before
 * `until`, a note left out from `until` (or the clip's end, whichever comes first) and shortened to end there. Every copy,
 * new or already in the clip, without duplicates, in time order. Refused as the command refuses.
 */
export function repeatCopies(selected: readonly Note[], grid: number | null, until: number, clipLength: number, sig: Signature): RepeatResult {
  if (grid !== null && !(grid >= 1)) return { ok: false, refusal: 'grid', firstCopy: 0 }
  if (selected.length === 0) return { ok: true, copies: [], atClipEnd: false, firstCopy: 0, period: 0 }
  const period = repeatPeriod(selected, grid, sig)
  const { first } = span(selected)
  const firstCopy = first + period
  if (!(until > firstCopy)) return { ok: false, refusal: 'until', firstCopy }
  if (firstCopy >= clipLength) return { ok: false, refusal: 'room', firstCopy }
  const end = Math.min(until, clipLength)
  const rounds = Math.max(1, Math.ceil((end - first) / period) - 1)
  const copies = new Map<string, Note>()
  for (let k = 1; k <= rounds; k++) {
    for (const n of selected) {
      const start = n.start + k * period
      if (start >= end) continue
      const copy = { ...n, start, length: Math.min(n.length, end - start) }
      copies.set(noteKey(copy), copy)
    }
  }
  const ordered = [...copies.values()].sort((a, b) => a.start - b.start || a.pitch - b.pitch || a.length - b.length || a.velocity - b.velocity)
  // The clip's end, not `until`, stopped a copy: one is shortened there, or the next copy of a note starts there or later
  // but before `until`.
  const atClipEnd =
    until > clipLength &&
    selected.some((n) => {
      const k = Math.max(1, Math.ceil((clipLength - n.start) / period))
      const next = n.start + k * period
      const before = next - period
      return (before > n.start && before + n.length > clipLength) || (next >= clipLength && next < until)
    })
  return { ok: true, copies: ordered, atClipEnd, firstCopy, period }
}

/**
 * Where Repeat stops by default (#710): the end of the bar where the copies begin, that is of the bar that holds the start of
 * the first copy. A figure of a beat at the start of a bar fills that bar; a figure that ends on a bar line (a whole bar,
 * or the last beat of a bar) fills the next bar, so the default always has room for a copy.
 */
export function defaultRepeatUntil(selected: readonly Note[], grid: number | null, sig: Signature): number {
  const bar = barTicks(sig)
  if (selected.length === 0) return bar
  const firstCopy = span(selected).first + repeatPeriod(selected, grid, sig)
  return (Math.floor(firstCopy / bar) + 1) * bar
}

/**
 * What the "Repeat until" field reads in the piano roll: a bar `N` (up to the end of bar N, as in the arrangement), or
 * `N.B` (up to the end of beat B of bar N, B from 1 to the beats of the bar; a comma or a colon too, as "Go to" reads).
 * Null for anything else.
 */
export function parseRepeatUntil(text: string, sig: Signature): number | null {
  const match = /^(\d{1,6})(?:[.,:](\d{1,2}))?$/.exec(text.trim())
  if (!match) return null
  const barNumber = Number(match[1])
  if (barNumber < 1) return null
  if (match[2] === undefined) return barNumber * barTicks(sig)
  const beat = Number(match[2])
  if (beat < 1 || beat > sig.numerator) return null
  return (barNumber - 1) * barTicks(sig) + beat * beatTicks(sig)
}
