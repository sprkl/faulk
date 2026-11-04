/**
 * What a device or a preset of the browser does when it is put somewhere (#190): dropped on a track's chain or on the
 * empty area of the track list, or entered on the keyboard. Each outcome is a few commands of the one command set
 * (`track.create`, `track.set_instrument`, `device.insert`, `preset.load`), never a private path; when there are
 * several, they are one history group, so one undo takes the whole drop back.
 */

import type { Command, Track } from '../generated/protocol'
import { inGroup } from '../history-group'
import { createTrack } from '../track-commands'
import { insertEffect, insertMasterEffect, setInstrument } from '../device-commands'
import { instrumentDescriptors } from '../devices'
import { loadPreset } from '../preset-commands'
import { chainOf, effectCount, masterChainOf, type ChainEntry } from '../project/chain'
import { isMidi } from '../project/tracks'
import type { BrowserItem } from './catalog'

/** Where an item goes. `selection` is the keyboard's: the selected track, else the master when it is selected, else a new track. */
export type DropTarget =
  | { type: 'new-track' }
  | { type: 'track'; track: string; /** Among the effects, 0 right after the instrument; the end of the chain when absent. */ index?: number }
  | { type: 'master'; index?: number }
  | { type: 'selection' }

/** What the page offers the drop to read and run: the dispatcher and the project store, narrowed (and faked in the tests). */
export interface DropHost {
  run(command: Command, options?: { quiet?: boolean }): boolean
  tracks(): readonly { id: string; track: Track }[]
  masterDevices(): Track['devices'] | undefined
  selectedTrack(): string | null
  masterSelected(): boolean
}

/** What to tell the person, as a message key of `browser.said.*` and its values. */
export interface DropSaid {
  key:
    | 'browser.said.created'
    | 'browser.said.instrument'
    | 'browser.said.inserted'
    | 'browser.said.insertedMaster'
    | 'browser.said.presetLoaded'
    | 'browser.said.noInstrumentOnMaster'
    | 'browser.said.refused'
  /** The device, with its preset when one was loaded ("Reverb, Large Hall"). */
  device: string
  /** The track it went to, by id (the page names it). */
  track?: string
  /** The device's place in the chain, 1 for the instrument. */
  position?: number
}

export interface DropResult {
  ok: boolean
  said: DropSaid
  /** The track the item went to or created, so the page selects it. */
  track: string | null
}

const label = (item: BrowserItem): string => (item.preset ? `${item.name}, ${item.preset.name}` : item.name)

/** What an item may be dropped on the empty area of the track list: an instrument, or a preset of one. */
export const createsTrack = (item: Pick<BrowserItem, 'category'>): boolean => item.category === 'instrument'

const idsOf = (chain: readonly ChainEntry[]): Set<string> => new Set(chain.map((entry) => entry.id))

/**
 * Runs `steps` as one history group when there are several; a refused step ends the sequence, the group is closed either
 * way. A refused `history.begin_group` (a group is already open, e.g. an agent's turn) is not an error: `end_group` is
 * sent only when this drop opened the group (#643), the group of someone else is never closed here. The steps then run
 * anyway; the history joins them to the open group only when the page (the same author) opened it, otherwise each step
 * is its own undo entry, and undo stays refused until that group ends.
 */
function grouped(host: DropHost, steps: ((host: DropHost) => boolean)[]): boolean {
  if (steps.length === 1) return steps[0]!(host)
  return inGroup(
    (command, options) => host.run(command, options),
    () => {
      for (const step of steps) if (!step(host)) return false
      return true
    },
  )
}

/** Puts `item` on `target`. Never throws on a state it did not expect: it says it refused. */
export function applyItem(host: DropHost, item: BrowserItem, target: DropTarget): DropResult {
  const refused = (key: DropSaid['key'] = 'browser.said.refused'): DropResult => ({ ok: false, said: { key, device: label(item) }, track: null })
  let at: DropTarget = target
  if (at.type === 'selection') {
    const selected = host.selectedTrack()
    // The master shows its own chain: an effect goes there; an instrument has no track in view to replace, so it starts one.
    if (host.masterSelected()) at = item.category === 'instrument' ? { type: 'new-track' } : { type: 'master' }
    else at = selected ? { type: 'track', track: selected } : { type: 'new-track' }
  }
  if (at.type === 'master' && item.category === 'instrument') return refused('browser.said.noInstrumentOnMaster')
  if (at.type === 'track') {
    const entry = host.tracks().find((e) => e.id === (at as { track: string }).track)
    if (!entry) return refused()
    // An audio track keeps its clip player: an instrument starts a MIDI track instead.
    if (item.category === 'instrument' && !isMidi(entry.track)) at = { type: 'new-track' }
  }

  if (at.type === 'new-track') return onNewTrack(host, item)
  if (at.type === 'master') return onMaster(host, item, at.index)
  return onTrack(host, item, at.track, at.index)
}

/** A MIDI track with the instrument (an effect from the keyboard gets the first instrument of the registry), then the effect and the preset. */
function onNewTrack(host: DropHost, item: BrowserItem): DropResult {
  const instrument = item.category === 'instrument' ? item.kind : instrumentDescriptors()[0]?.kind
  if (!instrument) return { ok: false, said: { key: 'browser.said.refused', device: label(item) }, track: null }
  const before = new Set(host.tracks().map((e) => e.id))
  let created: string | null = null
  const find = (): ChainEntry[] => {
    const entry = host.tracks().find((e) => e.id === created)
    return entry ? chainOf(entry.track) : []
  }
  const steps: ((host: DropHost) => boolean)[] = [
    (h) => {
      if (!h.run(createTrack('midi', instrument))) return false
      created = h.tracks().find((e) => !before.has(e.id))?.id ?? null
      return created !== null
    },
  ]
  if (item.category === 'instrument') {
    if (item.preset) {
      const preset = item.preset
      steps.push((h) => {
        const device = find().find((e) => e.effectIndex === null)
        return device ? h.run(loadPreset(device.id, preset.ref)) : false
      })
    }
  } else {
    const preset = item.preset
    steps.push((h) => {
      if (!h.run(insertEffect(created!, item.kind, 0))) return false
      const added = find().find((e) => e.effectIndex === 0)
      return !preset || !added ? true : h.run(loadPreset(added.id, preset.ref))
    })
  }
  const ok = grouped(host, steps)
  if (created === null) return { ok: false, said: { key: 'browser.said.refused', device: label(item) }, track: null }
  return { ok, said: { key: 'browser.said.created', device: label(item), track: created }, track: created }
}

function onTrack(host: DropHost, item: BrowserItem, track: string, index: number | undefined): DropResult {
  const entry = () => host.tracks().find((e) => e.id === track)
  const chain = () => (entry() ? chainOf(entry()!.track) : [])
  const refused = (): DropResult => ({ ok: false, said: { key: 'browser.said.refused', device: label(item), track }, track })

  if (item.category === 'instrument') {
    const current = chain().find((e) => e.effectIndex === null)
    const steps: ((host: DropHost) => boolean)[] = []
    // The track already has this instrument: a preset of it loads there, the device itself changes nothing.
    const same = current?.kind === item.kind
    if (!same) steps.push((h) => h.run(setInstrument(track, item.kind)))
    if (item.preset) {
      const preset = item.preset
      steps.push((h) => {
        const device = chain().find((e) => e.effectIndex === null)
        return device ? h.run(loadPreset(device.id, preset.ref)) : false
      })
    }
    if (steps.length === 0) return { ok: true, said: { key: 'browser.said.instrument', device: label(item), track, position: 1 }, track }
    const ok = grouped(host, steps)
    const key = same ? 'browser.said.presetLoaded' : 'browser.said.instrument'
    return { ok, said: { key, device: label(item), track, position: 1 }, track }
  }

  const before = idsOf(chain())
  const place = Math.max(0, Math.min(index ?? Infinity, effectCount(chain())))
  const preset = item.preset
  const steps: ((host: DropHost) => boolean)[] = [(h) => h.run(insertEffect(track, item.kind, place))]
  if (preset) {
    steps.push((h) => {
      const added = chain().find((e) => !before.has(e.id))
      return added ? h.run(loadPreset(added.id, preset.ref)) : false
    })
  }
  if (!entry()) return refused()
  const ok = grouped(host, steps)
  return { ok, said: { key: 'browser.said.inserted', device: label(item), track, position: place + 2 }, track }
}

function onMaster(host: DropHost, item: BrowserItem, index: number | undefined): DropResult {
  const chain = () => masterChainOf(host.masterDevices())
  const before = idsOf(chain())
  const place = Math.max(0, Math.min(index ?? Infinity, effectCount(chain())))
  const preset = item.preset
  const steps: ((host: DropHost) => boolean)[] = [(h) => h.run(insertMasterEffect(item.kind, place))]
  if (preset) {
    steps.push((h) => {
      const added = chain().find((e) => !before.has(e.id))
      return added ? h.run(loadPreset(added.id, preset.ref)) : false
    })
  }
  const ok = grouped(host, steps)
  return { ok, said: { key: 'browser.said.insertedMaster', device: label(item), position: place + 1 }, track: null }
}
