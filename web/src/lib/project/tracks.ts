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

/** What a drag of a header's grip tells the track list (the list owns the drag state, the header draws it). */
export interface DragHooks {
  start: (id: string) => void
  over: (id: string) => void
  drop: (id: string) => void
  end: () => void
}

/** The kind of the built-in device a track's instrument is, or null (an audio track's clip player is no instrument). */
export function instrumentKind(track: Track): string | null {
  for (const device of Object.values(track.devices)) {
    if (device?.role === 'instrument' && 'builtin' in device.source) return device.source.builtin.id
  }
  return null
}

/** The notes the track's instrument names (a drum kit's sounds), sorted by key; empty for a pitched instrument. */
export function instrumentNamedKeys(track: Track): readonly { key: number; name: string }[] {
  const kind = instrumentKind(track)
  return DEVICES.find((d) => d.kind === kind)?.named_keys ?? []
}

/** The instrument's name: its descriptor's one English name, null when the track has none the page knows. */
export function instrumentName(track: Track): string | null {
  const kind = instrumentKind(track)
  return DEVICES.find((d) => d.kind === kind)?.name ?? null
}

/** The instruments a MIDI track can be created with, from the device descriptors. */
export const INSTRUMENTS = DEVICES.filter((d) => d.role === 'instrument').map((d) => ({ kind: d.kind, name: d.name }))

/** The kind of a track, as the page tells MIDI from audio. */
export const isMidi = (track: Track): boolean => track.kind === ('midi' satisfies TrackKind)

/**
 * The sixteen track hues of the design system (DS-0.1: `track-01` to `track-16` of DESIGN.md), equally
 * bright, readable on the light and the dark theme. New tracks take them in turn and the colour picker offers them.
 */
export const TRACK_COLORS = [
  '#f08098',
  '#f18a76',
  '#ea9356',
  '#d99e3f',
  '#c1ab3c',
  '#a0b751',
  '#79bf72',
  '#4ac495',
  '#00c5b7',
  '#00c1d5',
  '#3bb9ed',
  '#6bafff',
  '#92a4fc',
  '#b299f3',
  '#cd90e0',
  '#e189c5',
] as const

/**
 * The colour a track shows: the one stored (a track gets one when it is created, and a project opened
 * without one gets its position's hue stored), else the hue its position in the list would have got.
 */
export const trackColor = (stored: string | null | undefined, index: number): string => stored ?? TRACK_COLORS[Math.abs(index) % TRACK_COLORS.length]

/** The colour of the master's swatch and lanes (the master is no track and has no hue). */
export const MASTER_COLOR = '#8f8a85'

/** The message keys naming the palette colours, in the order of `TRACK_COLORS`. */
export const HUE_KEYS = [
  'tracks.hue.1',
  'tracks.hue.2',
  'tracks.hue.3',
  'tracks.hue.4',
  'tracks.hue.5',
  'tracks.hue.6',
  'tracks.hue.7',
  'tracks.hue.8',
  'tracks.hue.9',
  'tracks.hue.10',
  'tracks.hue.11',
  'tracks.hue.12',
  'tracks.hue.13',
  'tracks.hue.14',
  'tracks.hue.15',
  'tracks.hue.16',
] as const

/** The message key naming a palette colour, or null for a colour outside the palette. */
export const hueKey = (color: string): (typeof HUE_KEYS)[number] | null => HUE_KEYS[TRACK_COLORS.findIndex((c) => c === color.toLowerCase())] ?? null

/** The lowest finite gain a command accepts, in dB; the fader's bottom stands for silence (`null`). */
export const MIN_GAIN_DB = -96
export const MAX_GAIN_DB = 6
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

/**
 * The name of a duplicate: when the name ends with a space and a number of one or two digits, the first
 * free number after it on the same base ("Synth 2" gives "Synth 3"); otherwise " 2", then the first free
 * number ("Synth" gives "Synth 2", "TR 808" gives "TR 808 2": a longer number is part of the name).
 * `taken` holds the names on screen, so a copy never shares one.
 */
export function copyName(label: string, taken: readonly string[]): string {
  const counted = /^(.*\S)\s+(\d{1,2})$/.exec(label)
  const base = counted ? counted[1] : label
  const used = new Set(taken)
  for (let n = counted ? Number(counted[2]) + 1 : 2; ; n++) {
    // A name has at most 200 characters (the command refuses more): shorten the base, never the number.
    const whole = `${base} ${n}`
    const candidate = [...whole].length <= 200 ? whole : `${[...base].slice(0, 190).join('')} ${n}`
    if (!used.has(candidate)) return candidate
  }
}

type DefaultName = (key: 'tracks.defaultName', vars: { n: number }) => string

/** What a track is called on screen, from its stored name and its position: the name, or the default name of its position when it has none. */
export function trackName(name: string, index: number, t: DefaultName): string {
  return name.trim() || t('tracks.defaultName', { n: index + 1 })
}

/** What a track is called on screen: its name, or the default name of its position when it has none. The one place a message names a track (#781). */
export function trackLabel(entry: Pick<TrackEntry, 'track' | 'index'>, t: DefaultName): string {
  return trackName(entry.track.name, entry.index, t)
}
