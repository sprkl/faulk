// The project core (project.wasm, built by `pnpm test`) under Node, driven as the page drives
// it: a project, tracks and notes created by commands (PR-4).
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import type { Command, Note, RunReport } from '../generated/protocol'
import {
  CoreError,
  coreModule,
  engineStep,
  folderFrame,
  MAX_FOLDER_FRAME,
  needsBytes,
  parseSaveFrame,
  ProjectCore,
  type FolderEntry,
  type SaveMode,
} from './core'
import { readFolder, SONG_FOLDER } from './reference-folder'

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

  it('saves the folder format, only what changed, and the folder opens as the project', () => {
    const source = ProjectCore.create(module, 'empty', 1, seed)
    apply(source, { type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth' })
    expect(source.dirty()).toBe(true)
    const folder = new Map<string, Uint8Array>()
    const write = (mode: SaveMode) => {
      const frame = source.save(mode)
      if (frame.status !== 'ok') throw new Error(frame.message)
      for (const entry of frame.entries) {
        if (entry.type === 'write') folder.set(entry.path, entry.bytes)
        else folder.delete(entry.path)
      }
      return frame.entries.map((entry) => entry.path)
    }

    // The whole project into an empty folder; `project.json` is not the first file.
    const first = write('everything')
    expect(first).toContain('project.json')
    expect(first).toContain('.faulk/session.json')
    expect(first.indexOf('project.json')).toBeGreaterThan(0)
    // Not recorded as written until the page says so: still dirty, and the same changes come again.
    expect(source.dirty()).toBe(true)
    source.saved(false)
    expect(write('changed')).toEqual(first)
    source.saved(true)
    expect(source.dirty()).toBe(false)
    expect(write('changed')).toEqual([])
    source.saved(true)

    // An edit rewrites the one file it changes; one made between `save` and `saved` stays dirty.
    apply(source, { type: 'project.rename', name: 'Song' })
    expect(write('changed')).toEqual(['project.json'])
    apply(source, { type: 'project.rename', name: 'Song 2' })
    source.saved(true)
    expect(source.dirty()).toBe(true)
    expect(write('changed')).toEqual(['project.json'])
    source.saved(true)
    expect(source.dirty()).toBe(false)

    // A solo is session state, saved in the session file alone.
    const track = Object.keys(source.project().project.tracks)[0]!
    apply(source, { type: 'mixer.set_solo', track, solo: true })
    expect(source.dirty()).toBe(true)
    expect(write('changed')).toEqual(['.faulk/session.json'])
    source.saved(true)

    // The folder opens as the project, session included.
    const entries: FolderEntry[] = [...folder].map(([path, bytes]) => ({ path, bytes }))
    const copy = ProjectCore.create(module, 'empty', 2, seed)
    copy.stage('saved', entries)
    expect(applied(copy.open('saved').report).output).toMatchObject({ type: 'project.open', name: 'Song 2' })
    expect(copy.project()).toEqual({ ...source.project(), slots: copy.project().slots })
    expect(copy.dirty()).toBe(false)
    copy.free()

    source.free()
    expect(() => source.project()).toThrow(CoreError)
  })

  it('tells a change to the session (a solo) from a change to the project', () => {
    const core = ProjectCore.create(module, 'single_synth', 1, seed)
    expect([core.dirty(), core.projectDirty()]).toEqual([false, false])
    const track = Object.keys(core.project().project.tracks)[0]!
    apply(core, { type: 'mixer.set_solo', track, solo: true })
    expect([core.dirty(), core.projectDirty()]).toEqual([true, false])
    apply(core, { type: 'project.rename', name: 'Mine' })
    expect([core.dirty(), core.projectDirty()]).toEqual([true, true])
    core.save('everything')
    core.saved(true)
    expect([core.dirty(), core.projectDirty()]).toEqual([false, false])
    core.free()
    expect(() => core.projectDirty()).toThrow(CoreError)
  })

  describe('.faulkpack', () => {
    /** A project with a track and a note, as the files of its folder. */
    function folderOf(name: string) {
      const source = ProjectCore.create(module, 'empty', 1, seed)
      const track = apply(source, { type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth' }) as { track: string }
      const clip = apply(source, { type: 'clip.create', track: track.track, at: { at: 'arrangement', start: 0 }, length: 3840 }) as { clip: string }
      apply(source, { type: 'clip.notes.add', clip: clip.clip, notes: [{ start: 0, pitch: 60, length: 480, velocity: 100 }] })
      apply(source, { type: 'project.rename', name })
      const frame = source.save('everything')
      if (frame.status !== 'ok') throw new Error(frame.message)
      const files = frame.entries.flatMap((e) => (e.type === 'write' ? [{ path: e.path, bytes: e.bytes }] : []))
      return { source, files }
    }

    it('packs a folder and unpacks the same files, minus the session; the pack is the same bytes each time', () => {
      const { source, files } = folderOf('Song')
      const packed = source.packExport(files)
      if (packed.status !== 'ok') throw new Error(packed.message)
      expect(packed.missingBlobs).toBe(0)
      expect(packed.files).toBe(files.filter((f) => !f.path.startsWith('.faulk/')).length)
      const again = source.packExport(files)
      expect(again.status === 'ok' && Array.from(again.zip)).toEqual(Array.from(packed.zip))

      const unpacked = source.packImport(packed.zip)
      if (unpacked.status !== 'ok') throw new Error(unpacked.message)
      const wanted = files.filter((f) => !f.path.startsWith('.faulk/')).map((f) => f.path)
      expect(unpacked.files.map((f) => f.path).sort()).toEqual(wanted.sort())
      for (const file of unpacked.files) {
        expect(Array.from(file.bytes)).toEqual(Array.from(files.find((f) => f.path === file.path)!.bytes))
      }

      // And it opens as the project it was made from.
      const copy = ProjectCore.create(module, 'empty', 2, seed)
      copy.stage('pack', unpacked.files)
      expect(applied(copy.open('pack').report).output).toMatchObject({ type: 'project.open', name: 'Song' })
      expect(copy.project().project).toEqual(source.project().project)
      copy.free()
      source.free()
    })

    it('says why a pack or a folder is refused, and is still able to answer after', () => {
      const { source, files } = folderOf('Song')
      const packed = source.packExport(files)
      if (packed.status !== 'ok') throw new Error(packed.message)
      expect(source.packImport(new TextEncoder().encode('not a zip'))).toMatchObject({ status: 'failed', message: expect.stringContaining('faulkpack') })
      expect(source.packImport(packed.zip.subarray(0, 40))).toMatchObject({ status: 'failed' })
      expect(source.packExport([{ path: 'README.md', bytes: new TextEncoder().encode('hi') }])).toMatchObject({ status: 'failed' })
      // A path outside the folder is a malformed frame, not a trap.
      expect(source.packExport([{ path: '../escape', bytes: new Uint8Array(1) }])).toMatchObject({ status: 'failed' })
      expect(source.packImport(packed.zip).status).toBe('ok')
      source.free()
    })
  })

  it('writes a copy under a new identity, and a new project under its own', () => {
    const core = ProjectCore.create(module, 'single_synth', 1, seed)
    core.setup()
    const idOf = () => core.project().project.id
    const first = idOf()
    expect(core.save('everything').status).toBe('ok')
    core.saved(true)
    expect(core.save('copy').status).toBe('ok')
    expect(idOf()).not.toBe(first)
    core.saved(true)
    const copy = idOf()
    applied(core.run({ type: 'project.new' }).report)
    expect(idOf()).not.toBe(copy)
    expect(core.dirty()).toBe(false)
    core.free()
  })

  it('parses a save frame: writes with their bytes, removals, and a failure message', () => {
    const encoder = new TextEncoder()
    const path = encoder.encode('a/b.json')
    const gone = encoder.encode('c.json')
    const body = new Uint8Array([9, 8, 7])
    const frame = new Uint8Array(1 + 4 + 4 + (1 + 4 + path.length + 8 + body.length) + (1 + 4 + gone.length))
    const view = new DataView(frame.buffer)
    let at = 1
    view.setUint32(at, 5, true)
    view.setUint32((at += 4), 2, true)
    at += 4
    frame[at++] = 0
    view.setUint32(at, path.length, true)
    frame.set(path, (at += 4))
    view.setBigUint64((at += path.length), BigInt(body.length), true)
    frame.set(body, (at += 8))
    frame[(at += body.length)] = 1
    view.setUint32(++at, gone.length, true)
    frame.set(gone, at + 4)
    expect(parseSaveFrame(frame)).toEqual({
      status: 'ok',
      entries: [
        { type: 'write', path: 'a/b.json', bytes: body },
        { type: 'remove', path: 'c.json' },
      ],
    })
    expect(parseSaveFrame(new Uint8Array([1, ...encoder.encode('boom')]))).toEqual({ status: 'failed', message: 'boom' })
  })

  describe('project.open', () => {
    const open = (core: ProjectCore, folder: string) => core.open(folder)

    it('replaces the project with a folder, starts the engine over, and refuses what it cannot read', () => {
      const entries = readFolder(SONG_FOLDER)
      expect(entries.some((e) => 'size' in e)).toBe(false) // the reference blob is a lane's: its bytes are needed
      const core = ProjectCore.create(module, 'single_synth', 1, seed)
      core.setup()
      apply(core, { type: 'project.rename', name: 'Before' })
      const before = core.project()

      // Nothing staged under that name: a typed refusal, the project stays.
      const refused = open(core, 'song')
      expect(refused.report).toMatchObject({ status: 'refused', error: { code: 'folder', error: { reason: 'not_staged', folder: 'song' } } })
      expect(refused.engine.length).toBe(0)
      expect(core.project()).toEqual(before)

      core.stage('song', entries)
      const opened = open(core, 'song')
      const report = applied(opened.report)
      expect(report.output).toMatchObject({ type: 'project.open', name: 'Song "one"', migrated_from: null, session_reset: false })
      expect(report.events).toContainEqual({ type: 'project_opened', name: 'Song "one"' })
      expect(report.events).toContainEqual({ type: 'history_changed', can_undo: false, can_redo: false })
      expect(report.stream_error ?? null).toBeNull()
      expect(opened.engine.length).toBeGreaterThan(0)
      const view = core.project()
      expect(view.project.name).toBe('Song "one"')
      expect(Object.keys(view.project.tracks)).toHaveLength(2)
      expect(view.slots.epoch).toBe(2) // the engine starts over in the next epoch
      expect(Object.keys(view.slots.tracks)).toHaveLength(2)
      expect(applied(core.run({ type: 'history.undo' }).report).events).toEqual([])

      // The folder served once.
      expect(open(core, 'song').report.status).toBe('refused')

      // A folder that is not a project, or is a newer one: refused, with the reason.
      core.stage('empty', [{ path: 'readme.txt', bytes: new TextEncoder().encode('hi') }])
      expect(open(core, 'empty').report).toMatchObject({ status: 'refused', error: { error: { reason: 'not_a_project' } } })
      const newer = entries.map((e) =>
        e.path === 'project.json' && 'bytes' in e
          ? { path: e.path, bytes: new TextEncoder().encode(new TextDecoder().decode(e.bytes).replace('"format": 2', '"format": 99')) }
          : e,
      )
      core.stage('newer', newer)
      expect(open(core, 'newer').report).toMatchObject({ status: 'refused', error: { error: { reason: 'newer_format', found: 99 } } })
      expect(core.project()).toEqual(view)
      core.free()
    })

    it('lists the blobs it does not read by size, and refuses a lane blob left out', () => {
      const entries = readFolder(SONG_FOLDER)
      const core = ProjectCore.create(module, 'single_synth', 1, seed)
      core.setup()
      const asSizes = entries.map((e) => (e.path.startsWith('blobs/') && 'bytes' in e ? { path: e.path, size: e.bytes.length } : e))
      core.stage('song', asSizes)
      expect(open(core, 'song').report).toMatchObject({ status: 'refused', error: { error: { reason: 'missing' } } })
      core.free()
    })

    it('decides which files to carry, and refuses a frame that is not a folder', () => {
      const magic = new TextEncoder().encode('FKAUTO01')
      expect(needsBytes('project.json', new Uint8Array())).toBe(true)
      expect(needsBytes('blobs/ab', magic)).toBe(true)
      expect(needsBytes('blobs/ab', new TextEncoder().encode('RIFF....'))).toBe(false)
      expect(needsBytes('blobs/ab', new Uint8Array())).toBe(false)
      const core = ProjectCore.create(module, 'empty', 1, seed)
      for (const path of ['', '/a', 'a//b', '../a', 'a/./b', 'a\\b']) {
        expect(() => core.stage('x', [{ path, bytes: new Uint8Array() }])).toThrow(CoreError)
      }
      expect(() => core.stage('x', [{ path: 'a', size: 3 }])).toThrow(CoreError) // only blobs are listed
      expect(() =>
        core.stage('x', [
          { path: 'a', bytes: new Uint8Array() },
          { path: 'a', bytes: new Uint8Array() },
        ]),
      ).toThrow(CoreError)
      expect(folderFrame('x', []).length).toBe(5)
      expect(MAX_FOLDER_FRAME).toBe(512 * 1024 * 1024)
      core.free()
    })
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
    expect(
      engineStep(() => {
        throw new CoreError('run', -3)
      }),
    ).toEqual({ streamError: -3, events: [] })
    expect(() =>
      engineStep(() => {
        throw new TypeError('other')
      }),
    ).toThrow(TypeError)
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

describe('history', () => {
  it('reads a 10 000-entry history well inside a frame (#432: 3.5 ms median measured; the bound leaves room for a slow machine)', () => {
    const core = ProjectCore.create(module, 'empty', 1, seed)
    core.setup()
    for (let i = 0; i < 10_000; i++) core.run({ type: 'project.rename', name: `p${i}` })
    const times: number[] = []
    for (let i = 0; i < 15; i++) {
      const start = performance.now()
      expect(core.history().undo).toHaveLength(10_000)
      times.push(performance.now() - start)
    }
    times.sort((a, b) => a - b)
    expect(times[7]!).toBeLessThan(50)
  })

  it('lists the entries with their author and the clock the page gave, and the author is put back after a run', () => {
    const core = ProjectCore.create(module, 'empty', 1, seed)
    core.setup()
    apply(core, { type: 'project.rename', name: 'One' })
    expect(core.run({ type: 'project.rename', name: 'Two' }, 'agent:claude', 5000).report.status).toBe('applied')
    core.run({ type: 'project.rename', name: 'Three' }, 'user', 6000)
    const view = core.history()
    expect(view.undo.map((e) => e.author)).toEqual(['user', 'agent:claude', 'user'])
    expect(view.undo.map((e) => e.at).slice(1)).toEqual([5000, 6000])
    core.run({ type: 'history.undo' })
    const after = core.history()
    expect(after.redo.map((e) => e.id)).toEqual([view.undo[2]!.id])
    expect(after.undo).toHaveLength(2)
  })

  it('puts the author back to user when a run throws, and refuses an author it cannot read', () => {
    const core = ProjectCore.create(module, 'empty', 1, seed)
    core.setup()
    expect(() => core.run({ type: 'project.rename', name: 'x' }, 'robot')).toThrow(CoreError)
    apply(core, { type: 'project.rename', name: 'Mine' })
    expect(core.history().undo.at(-1)!.author).toBe('user')
    const circular = {} as Record<string, unknown>
    circular.self = circular
    expect(() => core.run(circular as unknown as Command, 'agent:claude')).toThrow()
    apply(core, { type: 'project.rename', name: 'Mine again' })
    expect(core.history().undo.at(-1)!.author).toBe('user')
  })
})
