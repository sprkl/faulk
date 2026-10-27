// The live input in the page's own core (Web mode runs the same command layer as the helper): MIDI bytes
// decoded to commands go to the selected and the armed tracks, and a note off to where its note on went.
import { readFileSync } from 'node:fs'
import { beforeAll, describe, expect, it } from 'vitest'
import type { Command, CommandOutput } from '../generated/protocol'
import { coreModule, ProjectCore } from '../project/core'
import { sortedTracks } from '../project/tracks'
import * as commands from '../track-commands'
import { decodeMidi } from './decode'

let module: WebAssembly.Module
beforeAll(async () => {
  module = await coreModule(readFileSync(new URL('../project/wasm/project.wasm', import.meta.url)))
})

function session() {
  const core = ProjectCore.create(module, 'empty', 1)
  const events: string[] = []
  const run = (command: Command): CommandOutput => {
    const { report } = core.run(command)
    if (report.status !== 'applied') throw new Error(JSON.stringify(report))
    for (const event of report.events) events.push(event.type)
    return report.output
  }
  const midi = (...bytes: number[]) => {
    const command = decodeMidi(bytes)
    if (!command) throw new Error(`not decoded: ${bytes.join(' ')}`)
    return run(command)
  }
  run(commands.createTrack('midi', 'faulk.osc_synth', '#58c27d'))
  run(commands.createTrack('midi', 'faulk.drums', '#5b8def'))
  run(commands.createTrack('audio', null, '#e86fb0'))
  const [a, b, audio] = sortedTracks(core.project().project).map((e) => e.id)
  return { core, run, midi, events, a, b, audio }
}

const tracksOf = (output: CommandOutput): string[] => {
  if (!('tracks' in output)) throw new Error(`no tracks in ${JSON.stringify(output)}`)
  return [...output.tracks].sort()
}

describe('the live input from MIDI bytes', () => {
  it('plays nowhere until a MIDI track is selected or armed', () => {
    const { midi, run, a, audio } = session()
    expect(tracksOf(midi(0x90, 60, 100))).toEqual([])
    run(commands.selectTrack(audio))
    expect(tracksOf(midi(0x90, 61, 100))).toEqual([])
    run(commands.selectTrack(a))
    expect(tracksOf(midi(0x90, 62, 100))).toEqual([a])
  })

  it('goes to the selected track and the armed MIDI tracks', () => {
    const { midi, run, a, b, audio } = session()
    run(commands.selectTrack(a))
    run(commands.armTrack(b, true))
    run(commands.armTrack(audio, true))
    expect(tracksOf(midi(0x90, 64, 100))).toEqual([a, b].sort())
  })

  it('releases a held note on the tracks it started on, whatever changed meanwhile', () => {
    const { midi, run, a, b } = session()
    run(commands.selectTrack(a))
    midi(0x90, 60, 100)
    run(commands.selectTrack(b))
    run(commands.armTrack(b, true))
    expect(tracksOf(midi(0x80, 60, 0))).toEqual([a])
    // Note on with velocity 0 is a note off as well.
    midi(0x90, 65, 100)
    run(commands.selectTrack(a))
    expect(tracksOf(midi(0x90, 65, 0))).toEqual([b])
  })

  it('reports control changes for the automation without touching the project', () => {
    const { core, midi, events } = session()
    const before = JSON.stringify(core.project())
    midi(0xb0, 1, 99)
    expect(events.at(-1)).toBe('control_change')
    expect(JSON.stringify(core.project())).toBe(before)
  })
})
