/**
 * Which clip the piano roll edits. The launch grid (GR-3) points it at a clip (`UiStore.editedClip`); the timeline (AR-4) opens it the same way; with neither, the
 * piano roll finds the clip itself: the MIDI clips of the selected track, the earliest placed one
 * first (then the others by identifier), the first being the one it opens on. The user's choice
 * among them is view state; the roll never keeps an identifier the project no longer has.
 */

import type { Clip, Note, Placement, Tick, TickLoop, Track } from '../../lib/generated/protocol'

/** A MIDI clip of a track, with what the roll needs of its content. */
export interface MidiClip {
  id: string
  clip: Clip
  length: Tick
  looping: TickLoop | null
  /** The notes as the project holds them, sorted by start. */
  notes: readonly Note[]
  /** The scene of the launch-grid cell the clip sits in, null when it is not in the grid. */
  scene: string | null
  /** The tick of its earliest placement in the arrangement, null when it is not placed. */
  firstStart: Tick | null
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
    clips.push({ id, clip, length, looping: looping ?? null, notes, scene: clip.slot ?? null, firstStart: firstStart.get(id) ?? null })
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

/** A note of another track's clip, in the time of the edited clip (#470): drawn hollow and neutral, never edited. */
export type GhostNote = Pick<Note, 'start' | 'length' | 'pitch'>

const ticksOf = (placement: Placement): { length: number; offset: number } | null => ('ticks' in placement.span ? placement.span.ticks : null)

/**
 * The notes of the other MIDI tracks' clips at the same time range as the edited clip.
 * A clip of the arrangement is compared with the placements of the other tracks that overlap its first placement;
 * a clip that is only in the launch grid, with the clips of the other tracks in the same scene. Looping is not unrolled.
 */
export function ghostNotes(tracks: readonly { id: string; track: Track }[], trackId: string, edited: MidiClip | null, editedTrack: Track | null): GhostNote[] {
  if (!edited || !editedTrack) return []
  const out: GhostNote[] = []
  let anchor: { start: number; offset: number } | null = null
  for (const placement of Object.values(editedTrack.placements)) {
    const span = placement && placement.clip === edited.id ? ticksOf(placement) : null
    if (placement && span && (anchor === null || placement.start < anchor.start)) anchor = { start: placement.start, offset: span.offset }
  }
  for (const other of tracks) {
    if (other.id === trackId || other.track.kind !== 'midi') continue
    if (anchor) {
      // Clip time `t` of the edited clip is at timeline tick `anchor.start + t - anchor.offset`.
      const shift = anchor.start - anchor.offset
      for (const placement of Object.values(other.track.placements)) {
        const clip = placement && other.track.clips[placement.clip]
        const span = placement && ticksOf(placement)
        if (!placement || !clip || !span || !('midi' in clip.content)) continue
        for (const n of clip.content.midi.notes) {
          if (n.start < span.offset || n.start >= span.offset + span.length) continue
          const start = placement.start + n.start - span.offset - shift
          if (start >= 0 && start < edited.length) out.push({ start, length: n.length, pitch: n.pitch })
        }
      }
    } else if (edited.scene !== null) {
      for (const clip of Object.values(other.track.clips)) {
        if (clip && clip.slot === edited.scene && 'midi' in clip.content) {
          for (const n of clip.content.midi.notes) if (n.start < edited.length) out.push({ start: n.start, length: n.length, pitch: n.pitch })
        }
      }
    }
  }
  return out.sort((a, b) => a.start - b.start)
}

/** The name the roll shows for a clip: the one stored in the project, the same in every view. */
export function clipLabel(entry: MidiClip, fallback: string): string {
  return entry.clip.name.trim() || fallback
}

/** What says where a clip comes from: a grid cell (by scene id), an arrangement placement (by its start tick). */
export interface OriginLabels {
  grid: (scene: string) => string
  arrangement: (start: Tick) => string
}

/** Where a clip comes from: its grid cell, the arrangement, or both. Empty for a clip that is nowhere. */
export function clipOrigin(entry: MidiClip, labels: OriginLabels): string {
  const where: string[] = []
  if (entry.scene !== null) where.push(labels.grid(entry.scene))
  if (entry.firstStart !== null) where.push(labels.arrangement(entry.firstStart))
  return where.join(', ')
}
