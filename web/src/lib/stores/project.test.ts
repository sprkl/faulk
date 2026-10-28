// The project store's view of the tracks, fed as the dispatcher feeds it, from the real core.
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import { createLane, deleteLane } from '../automation-commands'
import { setInstrument } from '../device-commands'
import type { Command } from '../generated/protocol'
import { coreModule, ProjectCore } from '../project/core'
import * as commands from '../track-commands'
import { ProjectStore } from './project.svelte'

let module: WebAssembly.Module
beforeAll(async () => {
  module = await coreModule(readFileSync(new URL('../project/wasm/project.wasm', import.meta.url)))
})

/** A core and a store that follows it event by event, as the dispatcher does. */
function session(kind: 'empty' | 'single_synth' = 'empty') {
  const core = ProjectCore.create(module, kind, 1)
  const store = new ProjectStore()
  store.show(core.project())
  const run = (command: Command) => {
    const { report } = core.run(command)
    if (report.status !== 'applied') throw new Error(JSON.stringify(report))
    store.apply(report.events, () => core.project())
  }
  return { core, store, run }
}

describe('the tracks in the project store', () => {
  it('lists the tracks in order, with the selection and the solo of the session', () => {
    const { store, run } = session()
    expect(store.tracks).toEqual([])
    expect(store.selected).toBeNull()
    run(commands.createTrack('midi', 'faulk.osc_synth', '#58c27d'))
    run(commands.createTrack('midi', 'faulk.drums', '#5b8def'))
    const [a, b] = store.tracks
    expect(store.tracks.map((e) => e.index)).toEqual([0, 1])
    run(commands.selectTrack(b.id))
    expect(store.selected?.id).toBe(b.id)
    run(commands.setSolo(a.id, true))
    expect(store.soloed).toEqual([a.id])
    run(commands.setMute(a.id, true))
    expect(store.tracks[0].track.mixer.mute).toBe(true)
    run(commands.armTrack(a.id, true))
    run(commands.setMonitor(b.id, true))
    expect([store.armed, store.monitored]).toEqual([[a.id], [b.id]])
    run(commands.armTrack(a.id, false))
    expect(store.armed).toEqual([])
    run(commands.armTrack(a.id, true))
    run(commands.deleteTrack(b.id))
    expect(store.tracks.map((e) => e.id)).toEqual([a.id])
    expect(store.selected).toBeNull()
    expect(store.monitored).toEqual([])
    expect(store.armed).toEqual([a.id])
  })

  it('follows a rename, a colour, a volume and a pan', () => {
    const { store, run } = session()
    run(commands.createTrack('midi', null, '#58c27d'))
    const id = store.tracks[0].id
    run(commands.renameTrack(id, 'Lead'))
    run(commands.setTrackColor(id, '#e86fb0'))
    run(commands.setVolume(id, -6))
    run(commands.setPan(id, 0.25))
    const { track } = store.tracks[0]
    expect([track.name, track.color, track.mixer.gain_db, track.mixer.pan]).toEqual(['Lead', '#e86fb0', -6, 0.25])
  })

  it('has no tracks before a project is shown', () => {
    const store = new ProjectStore()
    expect([store.tracks, store.selected, store.soloed, store.armed, store.monitored]).toEqual([[], null, [], [], []])
  })
})

describe('the lanes an instrument swap takes (AM-4)', () => {
  it('are noticed whoever swapped, not when a lane goes alone, and forgotten when undo brings them back', () => {
    const { store, run } = session('single_synth')
    const { id, track } = store.tracks[0]
    const synth = Object.keys(track.devices)[0]!
    run(createLane(id, { device: { device: synth, param: 'gain' } }))
    run(createLane(id, { mixer: { param: 'gain' } }))
    const lanes = () => Object.keys(store.tracks[0].track.automation)
    expect(lanes()).toHaveLength(2)
    // A lane deleted by itself, then given back: no swap.
    run(deleteLane(lanes()[0]!))
    expect(store.swapLanes).toBeNull()
    run(commands.undo())
    expect(lanes()).toHaveLength(2)
    expect(store.swapLanes).toBeNull()
    // The swap takes the instrument's lane (one of the two) and the store says so.
    run(setInstrument(id, 'faulk.drums'))
    expect(lanes()).toHaveLength(1)
    expect(store.swapLanes).toEqual({ track: id, n: 1 })
    // Undo gives it back and the note goes; redo takes it again and the note is back.
    run(commands.undo())
    expect(lanes()).toHaveLength(2)
    expect(store.swapLanes).toBeNull()
    run(commands.redo())
    expect(store.swapLanes).toEqual({ track: id, n: 1 })
  })
})
