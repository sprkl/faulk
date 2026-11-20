/**
 * A device's detection source (#952, a sidechain): the track a Compressor listens to instead of its own, chosen in its
 * panel by name. The rule (another track, no loop, the safety limit) is the core's (`device.set_sidechain`); the page
 * only lists the tracks, names them, and words the core's answers.
 */
import type { Command, CommandError, Project, ProjectEvent } from './generated/protocol'
import type { MessageKey } from './i18n'
import { sortedTracks, trackLabel } from './project/tracks'
import type { Refusal } from './stores/refusal'

type DefaultName = (key: 'tracks.defaultName', vars: { n: number }) => string

/** The command that sets the track `device` listens to; `null`: its own track. */
export const setSidechain = (device: string, source: string | null): Command => ({ type: 'device.set_sidechain', device, source })

/** A track the select offers: its id and the name shown, with its position when another track is shown alike. */
export interface SourceOption {
  id: string
  label: string
}

/** The tracks a device of track `own` may listen to, in the list's order, each by the name the rest of the page gives it. */
export function sourceOptions(project: Pick<Project, 'tracks'>, own: string, t: DefaultName): SourceOption[] {
  const entries = sortedTracks(project)
  const labels = entries.map((e) => trackLabel(e, t))
  const count = new Map<string, number>()
  for (const l of labels) count.set(l, (count.get(l) ?? 0) + 1)
  return entries
    .map((e, i) => ({ id: e.id, label: (count.get(labels[i]!) ?? 0) > 1 ? `${labels[i]} (${e.index + 1})` : labels[i]! }))
    .filter((o) => o.id !== own)
}

/** A track named for a message the page words later: by its name, or by its position when it has none. */
export type TrackRef = { name: string } | { n: number }

export function trackRef(project: Pick<Project, 'tracks'> | undefined, id: string): TrackRef | null {
  if (!project) return null
  const entry = sortedTracks(project).find((e) => e.id === id)
  if (!entry) return null
  const name = entry.track.name.trim()
  return name ? { name } : { n: entry.index + 1 }
}

/** A message key and its variables, the unnamed variant (`…Unnamed`, `{n}`) when the track has no name. */
function named(key: 'refusal.sidechainCycle' | 'devices.sidechainLost', ref: TrackRef, more: Record<string, number> = {}) {
  return 'name' in ref ? { key: key as MessageKey, vars: { track: ref.name, ...more } } : { key: `${key}Unnamed` as MessageKey, vars: { n: ref.n, ...more } }
}

/**
 * The words of a refused `device.set_sidechain` that would loop (`invalid` on `source`: the source listens to this
 * track already), naming the chosen track; `null` for any other refusal, which keeps its generic text.
 */
export function sidechainRefusal(command: Command, error: CommandError, project: Pick<Project, 'tracks'> | undefined): Omit<Refusal, 'count'> | null {
  if (command.type !== 'device.set_sidechain' || error.code !== 'invalid' || error.field !== 'source' || !command.source) return null
  // Its own track is refused on `source` too, for another reason: the generic text then.
  const device = command.device
  const own = Object.entries(project?.tracks ?? {}).find(([, track]) => track?.devices[device])?.[0]
  if (own === command.source) return null
  const ref = trackRef(project, command.source)
  return ref ? named('refusal.sidechainCycle', ref) : null
}

/**
 * What the page says when a step deleted a track that devices of other tracks listened to (they listen to their own track
 * again: the core sets them back in the same step), whatever the command (`track.delete`, a redo of it, an agent's).
 * `before` is the project as it was before the step, where the track and its listeners still are. `null` when no
 * deleted track had a listener.
 */
export function lostNotice(events: readonly ProjectEvent[], before: Pick<Project, 'tracks'> | undefined): Omit<Refusal, 'count'> | null {
  if (!before) return null
  for (const event of events) {
    if (event.type !== 'track_deleted') continue
    const listeners = Object.entries(before.tracks)
      .filter(([id]) => id !== event.track)
      .flatMap(([, track]) => Object.values(track?.devices ?? {}))
      .filter((device) => device?.sidechain === event.track).length
    const ref = trackRef(before, event.track)
    if (listeners > 0 && ref) return named('devices.sidechainLost', ref, { count: listeners })
  }
  return null
}

/** Whether the source is not heard (muted, or another track soloed): it still triggers the detection, which the panel says. */
export function sourceUnheard(project: Pick<Project, 'tracks'>, solo: readonly string[], source: string): boolean {
  const track = project.tracks[source]
  if (!track) return false
  return track.mixer.mute || (solo.length > 0 && !solo.includes(source))
}
