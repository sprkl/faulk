// The project store's view of the tracks, fed as the dispatcher feeds it, from the real core.
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
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

  it('exposes the oscillator of the selected track only, with its settings over the defaults', () => {
    const { store, run } = session()
    run(commands.createTrack('midi', 'faulk.osc_synth', '#58c27d'))
    run(commands.createTrack('midi', 'faulk.drums', '#5b8def'))
    run(commands.createTrack('audio', null, '#e86fb0'))
    const [osc, drums, audio] = store.tracks
    expect(store.synth).toBeNull()
    run(commands.selectTrack(drums.id))
    expect(store.synth).toBeNull()
    run(commands.selectTrack(audio.id))
    expect(store.synth).toBeNull()
    run(commands.selectTrack(osc.id))
    expect(store.synth?.track).toBe(osc.id)
    expect(store.synth?.params.gain).toBeTypeOf('number')
    const device = store.synth!.device
    run({ type: 'device.set_param', device, param: 'gain', value: 0.25 })
    expect(store.synth?.params.gain).toBe(0.25)
  })

  it('has no tracks before a project is shown', () => {
    const store = new ProjectStore()
    expect([store.tracks, store.selected, store.synth, store.soloed, store.armed, store.monitored]).toEqual([[], null, null, [], [], []])
  })
})
