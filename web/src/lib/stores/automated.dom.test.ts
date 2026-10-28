// The controls that follow a lane while the transport runs (AM-4), fed by the real project core as the dispatcher feeds it.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync } from 'svelte'
import { beforeAll, describe, expect, it } from 'vitest'
import { addPoints, createLane } from '../automation-commands'
import { setDeviceParam } from '../device-commands'
import type { FeedbackReport } from '../engine/feedback'
import type { Command } from '../generated/protocol'
import { coreModule, ProjectCore } from '../project/core'
import { setVolume } from '../track-commands'
import { AutomatedValues } from './automated.svelte'
import { EngineStore } from './engine.svelte'
import { ProjectStore } from './project.svelte'

let module: WebAssembly.Module
beforeAll(async () => {
  module = await coreModule(readFileSync(resolve(process.cwd(), 'src/lib/project/wasm/project.wasm')))
})

function session() {
  const core = ProjectCore.create(module, 'single_synth', 1)
  const project = new ProjectStore()
  project.show(core.project())
  const engine = new EngineStore()
  const automated = new AutomatedValues(project, engine)
  const run = (command: Command) => {
    const { report } = core.run(command)
    if (report.status !== 'applied') throw new Error(JSON.stringify(report))
    project.apply(report.events, () => core.project())
  }
  const track = project.tracks[0]!
  const synth = Object.keys(track.track.devices)[0]!
  const play = (tick: number | null) => {
    engine.started = true
    engine.feedback = { transport: tick === null ? 'stopped' : 'playing', playheadTick: tick ?? 0 } as unknown as FeedbackReport
    // The effect that ends a hold at a jump runs on the next flush.
    flushSync()
  }
  const lane = (n = 0) => Object.keys(project.tracks[0]!.track.automation)[n]!
  return { project, engine, automated, run, track: track.id, synth, play, lane }
}

describe('the value of a lane at the play position', () => {
  it('is none while the transport is stopped, and follows the lane while it runs', () => {
    const s = session()
    s.run(createLane(s.track, { device: { device: s.synth, param: 'sustain' } }))
    s.run(
      addPoints(s.lane(), [
        { tick: 0, value: 0, shape: 'linear' },
        { tick: 1000, value: 1, shape: 'linear' },
      ]),
    )
    s.play(null)
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    s.play(250)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.25, 6)
    s.play(5000)
    expect(s.automated.device(s.synth, 'sustain')).toBe(1)
    // A parameter with no lane is not driven.
    expect(s.automated.device(s.synth, 'attack')).toBeUndefined()
  })

  it('reads a volume lane along the fader travel and gives it by track', () => {
    const s = session()
    s.run(createLane(s.track, { mixer: { param: 'gain' } }))
    s.run(
      addPoints(s.lane(), [
        { tick: 0, value: -96, shape: 'linear' },
        { tick: 1000, value: 0, shape: 'linear' },
      ]),
    )
    s.play(500)
    expect(s.automated.mixer(s.track, 'gain')).toBeCloseTo(-21, 6)
    expect(s.automated.mixer(s.track, 'pan')).toBeUndefined()
    expect(s.automated.mixer('someone-else', 'gain')).toBeUndefined()
  })
})

describe('a change of the parameter while the lane plays', () => {
  it('holds the lane until the play position reaches its next point, as the engine does (AM-2), whoever changed it', () => {
    const s = session()
    s.run(createLane(s.track, { device: { device: s.synth, param: 'sustain' } }))
    s.run(
      addPoints(s.lane(), [
        { tick: 0, value: 0, shape: 'linear' },
        { tick: 1000, value: 1, shape: 'linear' },
        { tick: 2000, value: 0.5, shape: 'linear' },
      ]),
    )
    s.play(250)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.25, 6)
    // A knob, an undo or an agent: the event is the same.
    s.run(setDeviceParam(s.synth, 'sustain', 0.9))
    s.play(500)
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    s.play(999)
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    // The next point takes the lane back at its tick.
    s.play(1000)
    expect(s.automated.device(s.synth, 'sustain')).toBe(1)
    s.play(1500)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.75, 6)
  })

  it('ends the hold on a jump back, and holds a volume change too', () => {
    const s = session()
    s.run(createLane(s.track, { mixer: { param: 'gain' } }))
    s.run(
      addPoints(s.lane(), [
        { tick: 0, value: -12, shape: 'step' },
        { tick: 1000, value: 0, shape: 'step' },
      ]),
    )
    s.play(600)
    s.run(setVolume(s.track, -3))
    s.play(700)
    expect(s.automated.mixer(s.track, 'gain')).toBeUndefined()
    // The play position goes back before where the change was made: the lane has the hand again.
    s.play(100)
    expect(s.automated.mixer(s.track, 'gain')).toBe(-12)
    s.play(1500)
    expect(s.automated.mixer(s.track, 'gain')).toBe(0)
  })

  it('holds nothing when the transport is stopped, and after the last point holds until a jump', () => {
    const s = session()
    s.run(createLane(s.track, { device: { device: s.synth, param: 'sustain' } }))
    s.run(addPoints(s.lane(), [{ tick: 0, value: 0.2, shape: 'linear' }]))
    s.play(null)
    s.run(setDeviceParam(s.synth, 'sustain', 0.9))
    s.play(500)
    expect(s.automated.device(s.synth, 'sustain')).toBe(0.2)
    s.run(setDeviceParam(s.synth, 'sustain', 0.4))
    s.play(900)
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    s.play(10)
    expect(s.automated.device(s.synth, 'sustain')).toBe(0.2)
  })

  it('lets go when the transport stops and plays again, or leaps ahead: the engine frees every lane at a jump', () => {
    const s = session()
    s.run(createLane(s.track, { device: { device: s.synth, param: 'sustain' } }))
    s.run(
      addPoints(s.lane(), [
        { tick: 0, value: 0, shape: 'linear' },
        { tick: 1000, value: 1, shape: 'linear' },
        { tick: 4000, value: 0, shape: 'linear' },
      ]),
    )
    s.play(600)
    s.run(setDeviceParam(s.synth, 'sustain', 0.9))
    s.play(700)
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    // Stop at 700 and Play: the position is inside the span the change held, and the lane has the hand.
    s.play(null)
    s.play(700)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.7, 6)
    // A change at 1200 holds until 4000; a leap to 3500 (more than half a second of music) frees the lane.
    s.play(1200)
    s.run(setDeviceParam(s.synth, 'sustain', 0.2))
    s.play(1300)
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    s.play(3500)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(1 - 2500 / 3000, 6)
  })
})
