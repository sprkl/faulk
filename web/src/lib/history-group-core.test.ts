// The shared history-group helper against the real project core, while another author's group is open (#653).
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { createGesture } from './gesture'
import type { Command } from './generated/protocol'
import { inGroup, runAll, type GroupRun } from './history-group'
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

describe('runAll: all or nothing (#867)', () => {
  it('a refused command in the middle takes back the first ones: project unchanged, no entry, redo kept', () => {
    const { core, as, run, toasts, track } = setup()
    // Something to redo, which the group must not cost.
    as('user', setVolume(track, -12))
    as('user', { type: 'history.undo' })
    const snapshot = JSON.stringify(core.project().project)
    const history = core.history()
    const result = runAll(run, [setVolume(track, -3), setVolume('no-such-track', -3), setVolume(track, -9)])
    expect(result).toEqual({ ok: false, refused: 1, rolledBack: true })
    expect(JSON.stringify(core.project().project)).toBe(snapshot)
    expect(core.history().undo).toEqual(history.undo)
    expect(core.history().redo).toEqual(history.redo)
    // One refusal message, the group commands that never ran were not sent, and the history is not stuck in a group.
    expect(toasts).toEqual(['mixer.set_volume'])
    expect(as('user', { type: 'history.redo' })).toBe('applied')
  })

  it('every command applied: one entry, ok', () => {
    const { core, run, track } = setup()
    const before = core.history().undo.length
    expect(runAll(run, [setVolume(track, -3), setVolume(track, -9)])).toEqual({ ok: true })
    expect(core.history().undo).toHaveLength(before + 1)
  })

  it('the first command refused: nothing applied, the group closed', () => {
    const { core, as, run, track } = setup()
    const before = core.history().undo.length
    expect(runAll(run, [setVolume('no-such-track', 0), setVolume(track, -9)])).toEqual({ ok: false, refused: 0, rolledBack: true })
    expect(core.history().undo).toHaveLength(before)
    expect(as('user', { type: 'history.undo' })).toBe('applied')
  })

  it('an abort the core refuses: the page closes its group, says it could not take back, and the history is not stuck', () => {
    const { core, as, track } = setup()
    let sent = 0
    // Another author's edit lands inside the group after the first command: the core refuses to abort.
    const run: GroupRun = (command) => {
      const applied = as('user', command) === 'applied'
      if (applied && command.type === 'mixer.set_volume' && ++sent === 1) as('agent:claude', setVolume(track, -1))
      return applied
    }
    const result = runAll(run, [setVolume(track, -3), setVolume('no-such-track', 0)])
    expect(result).toEqual({ ok: false, refused: 1, rolledBack: false })
    // Not stuck in an open group: undo applies.
    expect(as('user', { type: 'history.undo' })).toBe('applied')
    expect(core.history().undo.length).toBeGreaterThan(0)
  })

  it("another author's group open: the page cannot take back, says so, and sends nothing after the refusal", () => {
    const { core, as, run, track } = setup()
    as('agent:claude', { type: 'history.begin_group' })
    const result = runAll(run, [setVolume(track, -3), setVolume('no-such-track', 0), setVolume(track, -9)])
    expect(result).toEqual({ ok: false, refused: 1, rolledBack: false })
    expect(as('agent:claude', { type: 'history.end_group' })).toBe('applied')
    expect(core.history().undo.length).toBeGreaterThan(0)
  })
})
