// A drop with several steps against the real project core, while a history group is already open (#643): the fake host
// of `drop.test.ts` knows neither groups nor authors.
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import type { Command } from '../generated/protocol'
import { coreModule, ProjectCore } from '../project/core'
import type { BrowserItem } from './catalog'
import { applyItem, type DropHost } from './drop'

let module: WebAssembly.Module

beforeAll(async () => {
  module = await coreModule(readFileSync(new URL('../project/wasm/project.wasm', import.meta.url)))
})

const seed = { timeMs: 1_760_000_000_000, random: [1, 2, 3] as [number, number, number] }
const reverb: BrowserItem = { kind: 'faulk.reverb', name: 'reverb', category: 'effect', preset: null }

function setup() {
  const core = ProjectCore.create(module, 'empty', 1, seed)
  core.setup()
  const host: DropHost = {
    run: (command: Command) => core.run(command, 'user').report.status === 'applied',
    tracks: () => Object.entries(core.project().project.tracks).flatMap(([id, track]) => (track ? [{ id, track }] : [])),
    masterDevices: () => core.project().project.master_devices,
    selectedTrack: () => null,
    masterSelected: () => false,
  }
  const as = (author: string, command: Command) => core.run(command, author).report.status
  return { core, host, as }
}

describe('a drop while a history group is open', () => {
  it('opened by an agent: no error, the drop applies in separate entries, and the agent still ends its group', () => {
    const { core, host, as } = setup()
    expect(as('agent:claude', { type: 'history.begin_group' })).toBe('applied')
    const result = applyItem(host, reverb, { type: 'new-track' })
    expect(result.ok).toBe(true)
    expect(host.tracks()).toHaveLength(1)
    expect(core.history().undo).toHaveLength(2)
    expect(as('agent:claude', { type: 'history.end_group' })).toBe('applied')
    // The drop never closed the agent's group: undo takes the last step back, then the track.
    expect(as('user', { type: 'history.undo' })).toBe('applied')
    expect(as('user', { type: 'history.undo' })).toBe('applied')
    expect(host.tracks()).toHaveLength(0)
  })

  it('opened by the page itself: the drop joins it, one undo entry, and the page closes it', () => {
    const { core, host, as } = setup()
    expect(as('user', { type: 'history.begin_group' })).toBe('applied')
    const result = applyItem(host, reverb, { type: 'new-track' })
    expect(result.ok).toBe(true)
    expect(as('user', { type: 'history.end_group' })).toBe('applied')
    expect(core.history().undo).toHaveLength(1)
    expect(as('user', { type: 'history.undo' })).toBe('applied')
    expect(host.tracks()).toHaveLength(0)
  })

  it('no group open: the drop opens and closes its own, one undo entry', () => {
    const { core, host, as } = setup()
    expect(applyItem(host, reverb, { type: 'new-track' }).ok).toBe(true)
    expect(core.history().undo).toHaveLength(1)
    expect(as('user', { type: 'history.end_group' })).toBe('refused')
  })
})
