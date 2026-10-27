import { insertEffect, insertMasterEffect, moveDevice, removeDevice, setBypass } from '../device-commands'
import { DEVICES, type Command, type Device, type Track } from '../generated/protocol'
import type { DeviceDescriptor } from '../devices'

/** One device of a track's chain as the panel shows it. */
export interface ChainEntry {
  id: string
  device: Device
  /** The built-in kind, or null for a device the page cannot name (a module). */
  kind: string | null
  /** Its descriptor in `DEVICES`, or null when the page has none for it. */
  descriptor: DeviceDescriptor | null
  /** Position among the effects (0 right after the instrument); null for the instrument. */
  effectIndex: number | null
}

/** The clip player's id (`project::CLIP_PLAYER`): an audio track's own player, not a device of the user's chain. */
const CLIP_PLAYER = 'faulk.clip_player'

/**
 * The chain of a track in order, the instrument first: what `device.insert` and `device.move`
 * index against (their `index` counts the effects only). The audio track's clip player is left out.
 */
export function chainOf(track: { devices: Track['devices'] }, descriptors: readonly DeviceDescriptor[] = DEVICES): ChainEntry[] {
  const ids = Object.keys(track.devices).filter((id) => track.devices[id])
  const key = (id: string) => track.devices[id]!.order
  ids.sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : a < b ? -1 : a > b ? 1 : 0))
  let effects = 0
  const entries: ChainEntry[] = []
  for (const id of ids) {
    const device = track.devices[id]!
    const kind = 'builtin' in device.source ? device.source.builtin.id : null
    if (kind === CLIP_PLAYER) continue
    const effectIndex = device.role === 'instrument' ? null : effects++
    entries.push({ id, device, kind, descriptor: descriptors.find((d) => d.kind === kind) ?? null, effectIndex })
  }
  return entries
}

/** The master's chain (effects only) in order: what `device.insert` with `master: true` indexes against. */
export const masterChainOf = (devices: Track['devices'] | undefined, descriptors: readonly DeviceDescriptor[] = DEVICES): ChainEntry[] =>
  chainOf({ devices: devices ?? {} }, descriptors)

/** How many effects the chain holds. */
export const effectCount = (chain: readonly ChainEntry[]): number => chain.filter((e) => e.effectIndex !== null).length

/**
 * The index `device.move` takes to move an effect one place left or right, or null when it
 * cannot (the instrument, the first effect going left, the last going right).
 */
export function moveTarget(chain: readonly ChainEntry[], id: string, direction: 'left' | 'right'): number | null {
  const entry = chain.find((e) => e.id === id)
  if (!entry || entry.effectIndex === null) return null
  const target = entry.effectIndex + (direction === 'left' ? -1 : 1)
  return target < 0 || target >= effectCount(chain) ? null : target
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

/** The command the browser sends to add an effect: at the end of the chain. */
export const addEffectCommand = (track: string, kind: string, chain: readonly ChainEntry[]): Command => insertEffect(track, kind, effectCount(chain))
