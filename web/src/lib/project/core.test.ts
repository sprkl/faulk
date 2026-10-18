// The project core (project.wasm, built by `pnpm test`) under Node, driven as the page drives
// it: a project, tracks and notes created by commands (PR-4).
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import type { Command, Note, ProjectChanges, RunReport } from '../generated/protocol'
import { CoreError, coreModule, engineStep, ProjectCore } from './core'

let module: WebAssembly.Module

beforeAll(async () => {
  module = await coreModule(readFileSync(new URL('./wasm/project.wasm', import.meta.url)))
})

const seed = { timeMs: 1_760_000_000_000, random: [1, 2, 3] as [number, number, number] }

function applied(report: RunReport) {
  if (report.status !== 'applied') throw new Error(`not applied: ${JSON.stringify(report)}`)
  return report
}

/** Runs a command that must apply and returns its output. */
function apply(core: ProjectCore, command: Command) {
  return applied(core.run(command).report).output
}

function changes(core: ProjectCore): ProjectChanges {
  const report = core.save()
  if (report.status !== 'changes') throw new Error(report.message)
  return report.changes
}

describe('ProjectCore', () => {
  it('creates a project, tracks and notes by commands', () => {
    const core = ProjectCore.create(module, 'empty', 1, seed)
    const setup = core.setup()
    expect(setup.report).toEqual({ status: 'encoded', epoch: 1 })
    expect(setup.engine.length).toBeGreaterThan(0)

    const created = core.run({ type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth', name: 'Bass' })
    const report = applied(created.report)
    if (report.output.type !== 'track.create') throw new Error('a track.create output')
    const track = report.output.track
    expect(report.events[0]).toEqual({ type: 'track_created', track })
    expect(report.events).toContainEqual({ type: 'history_changed', can_undo: true, can_redo: false })
    expect(report.stream_error ?? null).toBeNull()
    expect(created.engine.length).toBeGreaterThan(0)
    const drums = apply(core, { type: 'track.create', kind: 'midi', name: 'Drums' })
    if (drums.type !== 'track.create') throw new Error('a track.create output')
    expect(drums.track).not.toBe(track)

    const clipOutput = apply(core, { type: 'clip.create', track, at: { at: 'arrangement', start: 0 }, length: 3840 })
    if (clipOutput.type !== 'clip.create') throw new Error('a clip.create output')
    const clip = clipOutput.clip
    const notes: Note[] = [
      { start: 0, pitch: 60, length: 480, velocity: 100 },
      { start: 960, pitch: 64, length: 480, velocity: 90 },
    ]
    const added = core.run({ type: 'clip.notes.add', clip, notes })
    expect(applied(added.report).events.length).toBeGreaterThan(0)
    expect(added.engine.length).toBeGreaterThan(0)

    const view = core.project()
    expect(Object.keys(view.project.tracks)).toHaveLength(2)
    expect(view.project.tracks[track]?.name).toBe('Bass')
    const content = view.project.tracks[track]?.clips[clip]?.content
    expect(content && 'midi' in content ? content.midi.notes : null).toEqual(notes)

    // Undo goes through the same path, with its engine bytes.
    const undo = core.run({ type: 'history.undo' })
    expect(undo.report.status).toBe('applied')
    expect(undo.engine.length).toBeGreaterThan(0)
    const undone = core.project().project.tracks[track]?.clips[clip]?.content
    expect(undone && 'midi' in undone ? undone.midi.notes : null).toEqual([])
    apply(core, { type: 'history.redo' })
    core.free()
  })

  it('reports refused and invalid commands without engine bytes', () => {
    const core = ProjectCore.create(module, 'single_synth', 1, seed)
    core.setup()
    const refused = core.run({ type: 'track.rename', track: '00000000000000000000000009', name: 'x' })
    expect(refused.report).toMatchObject({ status: 'refused', error: { code: 'not_found' } })
    expect(refused.engine.length).toBe(0)
    const invalid = core.run({ type: 'nope' } as unknown as Command)
    expect(invalid.report.status).toBe('invalid')
    expect(invalid.engine.length).toBe(0)
    core.free()
  })

  it('saves its files, loads them into another core, and tracks what is written', () => {
    const source = ProjectCore.create(module, 'empty', 1, seed)
    apply(source, { type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth' })
    const first = changes(source)
    expect(Object.keys(first.write).sort()).toEqual(['.faulk/session.json', 'project.json'])
    expect(first.remove).toEqual([])
    // Not recorded as written: still dirty.
    expect(changes(source)).toEqual(first)
    source.saved(first)
    expect(changes(source)).toEqual({ write: {}, remove: [] })

    const target = ProjectCore.create(module, 'single_synth', 1, { ...seed, random: [4, 5, 6] })
    target.setup()
    const loaded = target.load(first.write)
    expect(loaded.report).toEqual({ status: 'loaded', snapshot: { status: 'encoded', epoch: 2 }, session_reset: false })
    expect(loaded.engine.length).toBeGreaterThan(0)
    expect(target.project()).toEqual(source.project())
    expect(changes(target)).toEqual({ write: {}, remove: [] })

    const refused = target.load({})
    expect(refused.report).toEqual({ status: 'refused', error: { code: 'missing_file', file: 'project.json' } })
    expect(refused.engine.length).toBe(0)
    expect(target.project()).toEqual(source.project())

    expect(() => target.saved([] as unknown as ProjectChanges)).toThrow(CoreError)

    // An edit made between `save` and `saved` stays dirty: the written text is the reference.
    apply(source, { type: 'project.rename', name: 'Song' })
    const pending = changes(source)
    apply(source, { type: 'project.rename', name: 'Song 2' })
    source.saved(pending)
    expect(changes(source).write['project.json']).toContain('Song 2')
    source.saved(changes(source))
    source.free()
    target.free()
    expect(() => target.project()).toThrow(CoreError)
  })

  it('tells what a run means for the engine', () => {
    const core = ProjectCore.create(module, 'single_synth', 1, seed)
    core.setup()
    const note = { type: 'live.note_on', track: '00000000000000000000000001', note: 60, velocity: 100 } as const
    const push = engineStep(() => core.run(note))
    expect('push' in push && push.push.length > 0).toBe(true)
    expect('push' in engineStep(() => core.run({ type: 'midi.list_inputs' }))).toBe(true)
    const refused = engineStep(() => core.run({ type: 'track.rename', track: '00000000000000000000000009', name: 'x' }))
    expect('refused' in refused && refused.refused.status).toBe('refused')
    const applied = { status: 'applied', output: { type: 'history.undo' }, events: [] } as unknown as RunReport
    expect(engineStep(() => ({ report: { ...applied, stream_error: -4 }, engine: new Uint8Array() }))).toEqual({ streamError: -4, events: [] })
    expect(engineStep(() => { throw new CoreError('run', -3) })).toEqual({ streamError: -3, events: [] })
    expect(() => engineStep(() => { throw new TypeError('other') })).toThrow(TypeError)
    core.free()
    // A freed core is a CoreError too: reported, not thrown.
    expect(engineStep(() => core.run(note))).toEqual({ streamError: 0, events: [] })
  })

  it('takes a snapshot in the next epoch', () => {
    const core = ProjectCore.create(module, 'single_synth', 1, seed)
    expect(core.setup().report).toEqual({ status: 'encoded', epoch: 1 })
    expect(core.snapshot().report).toEqual({ status: 'encoded', epoch: 2 })
    core.free()
  })
})
