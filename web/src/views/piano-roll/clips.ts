/**
 * Which clip the piano roll edits. The grid (GR-3) and the timeline (AR-4) are not built, so the
 * piano roll finds the clip itself: the MIDI clips of the selected track, the earliest placed one
 * first (then the others by identifier), the first being the one it opens on. The user's choice
 * among them is view state; the roll never keeps an identifier the project no longer has.
 */

import type { Clip, Note, Tick, TickLoop, Track } from '../../lib/generated/protocol'

/** A MIDI clip of a track, with what the roll needs of its content. */
export interface MidiClip {
  id: string
  clip: Clip
  length: Tick
  looping: TickLoop | null
  /** The notes as the project holds them, sorted by start. */
  notes: readonly Note[]
}

/** The MIDI clips of a track in the order the roll offers them. */
export function midiClips(track: Track): MidiClip[] {
  const firstStart = new Map<string, number>()
  for (const placement of Object.values(track.placements)) {
    if (!placement) continue
    const seen = firstStart.get(placement.clip)
    if (seen === undefined || placement.start < seen) firstStart.set(placement.clip, placement.start)
  }
  const clips: MidiClip[] = []
  for (const [id, clip] of Object.entries(track.clips)) {
    if (!clip || !('midi' in clip.content)) continue
    const { length, looping, notes } = clip.content.midi
    clips.push({ id, clip, length, looping: looping ?? null, notes })
  }
  return clips.sort((a, b) => {
    const sa = firstStart.get(a.id) ?? Infinity
    const sb = firstStart.get(b.id) ?? Infinity
    return sa !== sb ? sa - sb : a.id < b.id ? -1 : a.id > b.id ? 1 : 0
  })
}

/** The clip to edit: the user's choice when it still exists, else the first one. */
export function editedClip(clips: readonly MidiClip[], chosen: string | null): MidiClip | null {
  return clips.find((c) => c.id === chosen) ?? clips[0] ?? null
}

/** The name the roll shows for a clip: its own, or "Clip n" by its place in the list. */
export function clipLabel(entry: MidiClip, index: number, fallback: (n: number) => string): string {
  return entry.clip.name.trim() || fallback(index + 1)
}
