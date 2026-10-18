import { DEVICES, type Project, type Track, type TrackKind } from '../generated/protocol'

/** One track as the track list shows it: its id, and its place in the order. */
export interface TrackEntry {
  id: string
  track: Track
  /** Position in the list, 0 first. */
  index: number
}

/** The tracks in the order the user sees: by order key, then by id (two tracks may share a key after a merge). */
export function sortedTracks(project: Pick<Project, 'tracks'>): TrackEntry[] {
  const ids = Object.keys(project.tracks).filter((id) => project.tracks[id])
  const key = (id: string) => project.tracks[id]!.order
  ids.sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : a < b ? -1 : a > b ? 1 : 0))
  return ids.map((id, index) => ({ id, track: project.tracks[id]!, index }))
}

/** The kind of the built-in device a track's instrument is, or null (an audio track's clip player is no instrument). */
export function instrumentKind(track: Track): string | null {
  for (const device of Object.values(track.devices)) {
    if (device?.role === 'instrument' && 'builtin' in device.source) return device.source.builtin.id
  }
  return null
}

/** The instrument's name: its descriptor's one English name, null when the track has none the page knows. */
export function instrumentName(track: Track): string | null {
  const kind = instrumentKind(track)
  return DEVICES.find((d) => d.kind === kind)?.name ?? null
}

/** The first device of a kind in a track's chain, with its id. */
export function deviceOfKind(track: Track, kind: string): { id: string; params: Record<string, number | undefined> } | null {
  for (const [id, device] of Object.entries(track.devices)) {
    if (device && 'builtin' in device.source && device.source.builtin.id === kind) return { id, params: device.params }
  }
  return null
}

/** The instruments a MIDI track can be created with, from the device descriptors. */
export const INSTRUMENTS = DEVICES.filter((d) => d.role === 'instrument').map((d) => ({ kind: d.kind, name: d.name }))

/** The kind of a track, as the page tells MIDI from audio. */
export const isMidi = (track: Track): boolean => track.kind === ('midi' satisfies TrackKind)

/** Colours the new tracks take in turn, readable on the light and the dark theme. */
export const TRACK_COLORS = ['#ff6b5b', '#f2a33a', '#e6c84a', '#58c27d', '#3fb8c9', '#5b8def', '#9b72f2', '#e86fb0'] as const

/** The colour of the next track created, given how many tracks exist. */
export const nextColor = (count: number): string => TRACK_COLORS[count % TRACK_COLORS.length]

/** The colour a track without one shows. */
export const NO_COLOR = '#8f8a85'

/** The lowest finite gain a command accepts, in dB; the fader's bottom stands for silence (`null`). */
export const MIN_GAIN_DB = -96
export const MAX_GAIN_DB = 6
/** The step of the fader, in dB. */
export const GAIN_STEP_DB = 0.5

/** The fader's value for a gain: silence is the floor. */
export const faderValue = (gainDb: number | null | undefined): number =>
  gainDb == null ? MIN_GAIN_DB : Math.min(MAX_GAIN_DB, Math.max(MIN_GAIN_DB, gainDb))

/** The gain a fader value means: the floor is silence (`null`), as the project writes it. */
export const gainOf = (value: number): number | null => (value <= MIN_GAIN_DB ? null : Math.min(MAX_GAIN_DB, value))

/** The gain as text: "−∞" for silence, else dB with one decimal and a true minus sign. */
export function formatGain(gainDb: number | null | undefined): string {
  if (gainDb == null) return '−∞'
  const text = Math.abs(gainDb).toFixed(1)
  return gainDb < 0 && Number(text) !== 0 ? `−${text}` : text
}

/** The pan as text: "C" at the centre, else "L" or "R" and the percentage. */
export function formatPan(pan: number): string {
  const percent = Math.round(Math.abs(pan) * 100)
  return percent === 0 ? 'C' : `${pan < 0 ? 'L' : 'R'}${percent}`
}

/** What a track is called on screen: its name, or the default name of its position when it has none. */
export function trackLabel(entry: Pick<TrackEntry, 'track' | 'index'>, t: (key: 'tracks.defaultName', vars: { n: number }) => string): string {
  return entry.track.name.trim() || t('tracks.defaultName', { n: entry.index + 1 })
}
