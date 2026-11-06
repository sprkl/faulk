import { insertEffect, insertMasterEffect, moveDevice, removeDevice, setBypass } from '../device-commands'
import { DEVICES, type Command, type Device, type Track } from '../generated/protocol'
import type { DeviceDescriptor } from '../devices'

/**
 * Where a device sits in a track's chain (#219): the MIDI effects first (notes in, notes out), then the instrument, then the
 * audio effects. The master's chain holds audio effects only.
 */
export type ChainSection = 'midi' | 'instrument' | 'effect'

/** One device of a track's chain as the panel shows it. */
export interface ChainEntry {
  id: string
  device: Device
  /** The built-in kind, or null for a device the page cannot name (a module). */
  kind: string | null
  /** Its descriptor in `DEVICES`, or null when the page has none for it. */
  descriptor: DeviceDescriptor | null
  /** Its section of the chain. */
  section: ChainSection
  /**
   * Position among the devices of its section, what `device.insert` and `device.move` take: among the MIDI effects (0 at
   * the start of the chain) or among the audio effects (0 right after the instrument); null for the instrument.
   */
  index: number | null
}

/** The clip player's id (`project::CLIP_PLAYER`): an audio track's own player, not a device of the user's chain. */
const CLIP_PLAYER = 'faulk.clip_player'

const sectionOf = (role: string): ChainSection => (role === 'instrument' ? 'instrument' : role === 'midi_effect' ? 'midi' : 'effect')

/**
 * The chain of a track in order, the MIDI effects first, then the instrument, then the audio effects: what `device.insert`
 * and `device.move` index against (their `index` counts the devices of the same section). The audio track's clip player is
 * left out.
 */
export function chainOf(track: { devices: Track['devices'] }, descriptors: readonly DeviceDescriptor[] = DEVICES): ChainEntry[] {
  const ids = Object.keys(track.devices).filter((id) => track.devices[id])
  const key = (id: string) => track.devices[id]!.order
  ids.sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : a < b ? -1 : a > b ? 1 : 0))
  const counts: Record<ChainSection, number> = { midi: 0, instrument: 0, effect: 0 }
  const entries: ChainEntry[] = []
  for (const id of ids) {
    const device = track.devices[id]!
    const kind = 'builtin' in device.source ? device.source.builtin.id : null
    if (kind === CLIP_PLAYER) continue
    const section = sectionOf(device.role)
    const index = section === 'instrument' ? null : counts[section]++
    entries.push({ id, device, kind, descriptor: descriptors.find((d) => d.kind === kind) ?? null, section, index })
  }
  return entries
}

/** The master's chain (effects only) in order: what `device.insert` with `master: true` indexes against. */
export const masterChainOf = (devices: Track['devices'] | undefined, descriptors: readonly DeviceDescriptor[] = DEVICES): ChainEntry[] =>
  chainOf({ devices: devices ?? {} }, descriptors)

/** How many devices the chain holds in `section`. */
export const sectionCount = (chain: readonly ChainEntry[], section: ChainSection): number => chain.filter((e) => e.section === section).length

/** How many audio effects the chain holds. */
export const effectCount = (chain: readonly ChainEntry[]): number => sectionCount(chain, 'effect')

/** How many MIDI effects the chain holds. */
export const midiEffectCount = (chain: readonly ChainEntry[]): number => sectionCount(chain, 'midi')

/** The chain's instrument, if it has one. */
export const instrumentOf = (chain: readonly ChainEntry[]): ChainEntry | undefined => chain.find((e) => e.section === 'instrument')

/**
 * The index `device.move` takes to move an effect (audio or MIDI) one place left or right among the effects of its section,
 * or null when it cannot (the instrument, the first one going left, the last going right: an effect never crosses the
 * instrument).
 */
export function moveTarget(chain: readonly ChainEntry[], id: string, direction: 'left' | 'right'): number | null {
  const entry = chain.find((e) => e.id === id)
  const index = entry?.index ?? null
  if (!entry || index === null) return null
  const target = index + (direction === 'left' ? -1 : 1)
  return target < 0 || target >= sectionCount(chain, entry.section) ? null : target
}

/** The command each button of a device card sends; `null` for a move that goes nowhere. */
export interface CardActions {
  /** The bypass button toggles: it bypasses a device that runs, and puts a bypassed one back. */
  bypass: Command
  moveLeft: Command | null
  moveRight: Command | null
  remove: Command
}

/** What the card of `entry` sends: the one place where a button is tied to its command, id and index. */
export function cardActions(chain: readonly ChainEntry[], entry: ChainEntry): CardActions {
  const move = (direction: 'left' | 'right') => {
    const index = moveTarget(chain, entry.id, direction)
    return index === null ? null : moveDevice(entry.id, index)
  }
  return { bypass: setBypass(entry.id, entry.device.enabled), moveLeft: move('left'), moveRight: move('right'), remove: removeDevice(entry.id) }
}

/** The command the browser sends to add an effect to the master's chain: at its end. */
export const addMasterEffectCommand = (kind: string, chain: readonly ChainEntry[]): Command => insertMasterEffect(kind, effectCount(chain))

/** The command the browser sends to add an audio effect: at the end of the chain. */
export const addEffectCommand = (track: string, kind: string, chain: readonly ChainEntry[]): Command => insertEffect(track, kind, effectCount(chain))

/** The command the browser sends to add a MIDI effect: after the track's other MIDI effects, right before the instrument. */
export const addMidiEffectCommand = (track: string, kind: string, chain: readonly ChainEntry[]): Command => insertEffect(track, kind, midiEffectCount(chain))
