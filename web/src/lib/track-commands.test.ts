// Every action of a track header is a command the project core takes: each builder is run against
// the real core (project.wasm, built by `pnpm test`), and the ones that edit undo to the same project.
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import type { Command, ProjectView, RunReport } from './generated/protocol'
import { coreModule, ProjectCore } from './project/core'
import { sortedTracks, TRACK_COLORS } from './project/tracks'
import * as commands from './track-commands'
import { CLIP_ROW_MAX, CLIP_ROW_MIN } from '../views/arrangement/automation/layout'

let module: WebAssembly.Module
beforeAll(async () => {
  module = await coreModule(readFileSync(new URL('./project/wasm/project.wasm', import.meta.url)))
})

function run(core: ProjectCore, command: Command): RunReport & { status: 'applied' } {
  const { report } = core.run(command)
  if (report.status !== 'applied') throw new Error(`${command.type} not applied: ${JSON.stringify(report)}`)
  return report
}

const project = (core: ProjectCore): ProjectView['project'] => core.project().project

describe('the height of a track (#490)', () => {
  it('is kept with the project, goes back to the default with null, and is refused outside the range the page offers', () => {
    const core = ProjectCore.create(module, 'empty', 1)
    run(core, commands.createTrack('midi', 'faulk.osc_synth', '#58c27d'))
    const id = sortedTracks(project(core))[0].id
    run(core, commands.setTrackHeight(id, 120))
    expect(project(core).tracks[id]!.height).toBe(120)
    run(core, commands.setTrackHeight(id, null))
    expect(project(core).tracks[id]!.height ?? null).toBeNull()
    // The page's clamp is the command's range: the ends are accepted, one step past them is refused.
    for (const edge of [CLIP_ROW_MIN, CLIP_ROW_MAX]) run(core, commands.setTrackHeight(id, edge))
    for (const bad of [CLIP_ROW_MIN - 1, CLIP_ROW_MAX + 1]) expect(core.run(commands.setTrackHeight(id, bad)).report.status).not.toBe('applied')
    expect(project(core).tracks[id]!.height).toBe(CLIP_ROW_MAX)
  })
})

describe('the commands of the track headers', () => {
  it('creates a MIDI track for each instrument and an audio track, unnamed, with a colour', () => {
    const core = ProjectCore.create(module, 'empty', 1)
    for (const [kind, instrument] of [
      ['midi', 'faulk.osc_synth'],
      ['midi', 'faulk.drums'],
      ['audio', null],
    ] as const) {
      run(core, commands.createTrack(kind, instrument, '#58c27d'))
    }
    const tracks = sortedTracks(project(core))
    expect(tracks.map((e) => e.track.kind)).toEqual(['midi', 'midi', 'audio'])
    expect(tracks.map((e) => e.track.name)).toEqual(['', '', ''])
    expect(tracks.every((e) => e.track.color === '#58c27d')).toBe(true)
    const sources = tracks.map((e) => JSON.stringify(Object.values(e.track.devices).map((d) => d?.source)))
    expect(sources[0]).toContain('faulk.osc_synth')
    expect(sources[1]).toContain('faulk.drums')
  })

  it('stores a colour at creation, so reordering and deleting tracks changes no other colour', () => {
    const core = ProjectCore.create(module, 'empty', 1)
    for (let i = 0; i < 4; i++) run(core, commands.createTrack('midi', null))
    const first = sortedTracks(project(core))
    const colors = new Map(first.map((e) => [e.id, e.track.color]))
    expect(first.map((e) => e.track.color)).toEqual(TRACK_COLORS.slice(0, 4))
    run(core, commands.moveTrack(first[3].id, 0))
    run(core, commands.deleteTrack(first[1].id))
    for (const e of sortedTracks(project(core))) expect(e.track.color).toBe(colors.get(e.id))
  })

  it('runs every other action, and each edit undoes to the project it started from', () => {
    const core = ProjectCore.create(module, 'empty', 1)
    run(core, commands.createTrack('midi', 'faulk.osc_synth', '#58c27d'))
    const id = sortedTracks(project(core))[0].id
    const actions: Command[] = [
      commands.renameTrack(id, 'Bass'),
      commands.setTrackColor(id, '#5b8def'),
      commands.setTrackHeight(id, 96),
      commands.setMute(id, true),
      commands.setVolume(id, -12),
      commands.setVolume(id, null),
      commands.setPan(id, -0.5),
    ]
    // A second track, so that a move has somewhere to go.
    run(core, commands.createTrack('midi', 'faulk.drums', '#58c27d'))
    actions.push(commands.moveTrack(id, 1), commands.duplicateTrack(id, 'Bass 2'))
    for (const action of actions) {
      const before = JSON.stringify(project(core))
      run(core, action)
      expect(JSON.stringify(project(core)), action.type).not.toBe(before)
      run(core, commands.undo())
      expect(JSON.stringify(project(core)), `undo of ${action.type}`).toBe(before)
    }
  })

  it('selects and solos in the session, not in the project', () => {
    const core = ProjectCore.create(module, 'empty', 1)
    run(core, commands.createTrack('midi', null, '#58c27d'))
    run(core, commands.createTrack('midi', null, '#5b8def'))
    const [a, b] = sortedTracks(project(core)).map((e) => e.id)
    const before = JSON.stringify(project(core))
    run(core, commands.selectTrack(b))
    run(core, commands.setSolo(a, true))
    run(core, commands.setSolo(b, true))
    const { session } = core.project()
    expect(session.selected_track).toBe(b)
    expect([...session.solo].sort()).toEqual([a, b].sort())
    expect(JSON.stringify(project(core))).toBe(before)
    run(core, commands.setSolo(a, false))
    expect(core.project().session.solo).toEqual([b])
  })

  it('arms and monitors in the session, never in the project nor the history', () => {
    const core = ProjectCore.create(module, 'empty', 1)
    run(core, commands.createTrack('midi', null, '#58c27d'))
    run(core, commands.createTrack('audio', null, '#5b8def'))
    const [a, b] = sortedTracks(project(core)).map((e) => e.id)
    const before = JSON.stringify(project(core))
    const reports = [commands.armTrack(a, true), commands.armTrack(b, true), commands.setMonitor(a, true)].map((c) => run(core, c))
    expect(reports.flatMap((r) => r.events.map((e) => e.type))).toEqual(['arm_changed', 'arm_changed', 'monitor_changed'])
    let { session } = core.project()
    expect([[...(session.armed ?? [])].sort(), session.monitor]).toEqual([[a, b].sort(), [a]])
    expect(JSON.stringify(project(core))).toBe(before)
    run(core, commands.armTrack(b, false))
    run(core, commands.deleteTrack(a))
    ;({ session } = core.project())
    expect([session.armed, session.monitor]).toEqual([[], []])
  })

  it('deletes a track and brings it back whole with an undo; a gesture is one undo', () => {
    const core = ProjectCore.create(module, 'empty', 1)
    run(core, commands.createTrack('midi', 'faulk.drums', '#58c27d'))
    const id = sortedTracks(project(core))[0].id
    run(core, commands.renameTrack(id, 'Kit'))
    run(core, commands.setMute(id, true))
    const before = JSON.stringify(project(core))
    run(core, commands.deleteTrack(id))
    expect(sortedTracks(project(core))).toEqual([])
    run(core, commands.undo())
    expect(JSON.stringify(project(core))).toBe(before)
    expect(run(core, commands.redo()).events.length).toBeGreaterThan(0)
    run(core, commands.undo())

    run(core, commands.beginGroup())
    for (const db of [-3, -6, -9, -12]) run(core, commands.setVolume(id, db))
    run(core, commands.endGroup())
    expect(project(core).tracks[id]!.mixer.gain_db).toBe(-12)
    run(core, commands.undo())
    expect(project(core).tracks[id]!.mixer.gain_db).toBe(0)
  })

  it('refuses a second group while one is open: the page ends its gesture, it never nests', () => {
    const core = ProjectCore.create(module, 'empty', 1)
    run(core, commands.beginGroup())
    expect(core.run(commands.beginGroup()).report.status).not.toBe('applied')
    run(core, commands.endGroup())
  })
})
