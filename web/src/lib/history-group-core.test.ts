// The shared history-group helper against the real project core, while another author's group is open (#653).
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { createGesture } from './gesture'
import type { Command } from './generated/protocol'
import { inGroup, type GroupRun } from './history-group'
import { coreModule, ProjectCore } from './project/core'
import { createTrack, setVolume } from './track-commands'

let module: WebAssembly.Module

beforeAll(async () => {
  module = await coreModule(readFileSync(new URL('./project/wasm/project.wasm', import.meta.url)))
})

const seed = { timeMs: 1_760_000_000_000, random: [1, 2, 3] as [number, number, number] }

function setup() {
  const core = ProjectCore.create(module, 'empty', 1, seed)
  core.setup()
  const as = (author: string, command: Command) => core.run(command, author).report.status
  // The page's `run`, as the dispatcher would have it: it counts what a non-quiet refusal would show as a toast.
  const toasts: string[] = []
  const run: GroupRun = vi.fn((command: Command, options?: { quiet?: boolean }) => {
    const status = as('user', command)
    if (status !== 'applied' && !options?.quiet) toasts.push(command.type)
    return status === 'applied'
  })
  as('user', createTrack('midi', 'faulk.osc_synth'))
  const track = Object.keys(core.project().project.tracks)[0]!
  return { core, as, run, toasts, track }
}

describe('a gesture while an agent group is open', () => {
  it('shows no refusal, applies its steps, and never closes the agent group', () => {
    const { core, as, run, toasts, track } = setup()
    const before = core.history().undo.length
    expect(as('agent:claude', { type: 'history.begin_group' })).toBe('applied')
    const gesture = createGesture(run)
    gesture.begin()
    expect(run(setVolume(track, -6))).toBe(true)
    gesture.end()
    expect(toasts).toEqual([])
    // The agent still owns its group: its own end_group applies, and undo works again afterwards.
    expect(as('agent:claude', { type: 'history.end_group' })).toBe('applied')
    expect(core.history().undo.length).toBeGreaterThan(before)
    expect(as('user', { type: 'history.undo' })).toBe('applied')
    expect(as('user', { type: 'history.undo' })).toBe('applied')
    // The first undo took the volume change back, the second the track creation: the history is empty again.
    expect(core.history().undo).toHaveLength(before - 1)
  })

  it('inGroup: runs the body and leaves the agent group open, quietly', () => {
    const { as, run, toasts, track } = setup()
    as('agent:claude', { type: 'history.begin_group' })
    inGroup(run, () => {
      run(setVolume(track, -3))
      run(setVolume(track, -9))
    })
    expect(toasts).toEqual([])
    expect(as('user', { type: 'history.undo' })).toBe('refused')
    expect(as('agent:claude', { type: 'history.end_group' })).toBe('applied')
  })

  it('no group open: one group of its own, one undo entry', () => {
    const { core, run, track } = setup()
    const before = core.history().undo.length
    inGroup(run, () => {
      run(setVolume(track, -3))
      run(setVolume(track, -9))
    })
    expect(core.history().undo).toHaveLength(before + 1)
  })
})
