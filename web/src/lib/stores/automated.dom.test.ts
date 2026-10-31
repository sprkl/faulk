// The controls that follow a lane at the play position (AM-4) and say a lane drives them (#388), fed by the real project core as the dispatcher feeds it.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync } from 'svelte'
import { beforeAll, describe, expect, it } from 'vitest'
import { addPoints, createLane, createMasterLane } from '../automation-commands'
import { insertMasterEffect, setDeviceParam } from '../device-commands'
import type { FeedbackReport } from '../engine/feedback'
import type { Command } from '../generated/protocol'
import { coreModule, ProjectCore } from '../project/core'
import { setMasterVolume, setVolume } from '../track-commands'
import { AutomatedValues } from './automated.svelte'
import { EngineStore } from './engine.svelte'
import { ProjectStore } from './project.svelte'
import { UiStore } from './ui.svelte'

let module: WebAssembly.Module
beforeAll(async () => {
  module = await coreModule(readFileSync(resolve(process.cwd(), 'src/lib/project/wasm/project.wasm')))
})

function session() {
  const core = ProjectCore.create(module, 'single_synth', 1)
  const project = new ProjectStore()
  project.show(core.project())
  const engine = new EngineStore()
  const ui = new UiStore('play', 'devices')
  ui.lang = 'en'
  const automated = new AutomatedValues(project, engine, ui)
  const run = (command: Command) => {
    const { report } = core.run(command)
    if (report.status !== 'applied') throw new Error(JSON.stringify(report))
    project.apply(report.events, () => core.project())
  }
  const track = project.tracks[0]!
  const synth = Object.keys(track.track.devices)[0]!
  const report = (transport: 'playing' | 'stopped', tick: number) => {
    engine.started = true
    engine.feedback = { transport, playheadTick: tick } as unknown as FeedbackReport
    // The effect that ends a hold at a jump runs on the next flush.
    flushSync()
  }
  /** The transport runs at `tick`. */
  const play = (tick: number) => report('playing', tick)
  /** The transport is stopped with the playhead at `tick` (a stop, or a seek while stopped). */
  const stop = (tick: number) => report('stopped', tick)
  const lane = (n = 0) => Object.keys(project.tracks[0]!.track.automation)[n]!
  return { project, engine, automated, ui, run, track: track.id, synth, play, stop, lane }
}

describe('the value of a lane at the play position', () => {
  it('follows the lane while the transport runs, and the playhead while it is stopped', () => {
    const s = session()
    s.run(createLane(s.track, { device: { device: s.synth, param: 'sustain' } }))
    s.run(
      addPoints(s.lane(), [
        { tick: 0, value: 0, shape: 'linear' },
        { tick: 1000, value: 1, shape: 'linear' },
      ]),
    )
    // The engine has not reported: the playhead is at the start.
    expect(s.automated.device(s.synth, 'sustain')).toBe(0)
    s.stop(400)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.4, 6)
    s.play(250)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.25, 6)
    s.play(5000)
    expect(s.automated.device(s.synth, 'sustain')).toBe(1)
    // A seek while stopped: the controls show the lane's value there.
    s.stop(5000)
    s.stop(500)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.5, 6)
    // A parameter with no lane is not driven.
    expect(s.automated.device(s.synth, 'attack')).toBeUndefined()
  })

  it('holds the first point value before the first point', () => {
    const s = session()
    s.run(createLane(s.track, { device: { device: s.synth, param: 'sustain' } }))
    s.run(addPoints(s.lane(), [{ tick: 3840, value: 0.7, shape: 'linear' }]))
    expect(s.automated.device(s.synth, 'sustain')).toBe(0.7)
    s.stop(100)
    expect(s.automated.device(s.synth, 'sustain')).toBe(0.7)
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

  it('holds a change made while stopped until the next seek or play, and after the last point holds until a jump', () => {
    const s = session()
    s.run(createLane(s.track, { device: { device: s.synth, param: 'sustain' } }))
    s.run(
      addPoints(s.lane(), [
        { tick: 0, value: 0.2, shape: 'linear' },
        { tick: 1000, value: 0.8, shape: 'linear' },
      ]),
    )
    s.stop(100)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.26, 6)
    // A change while stopped holds, whatever the next point is (AM-2).
    s.run(setDeviceParam(s.synth, 'sustain', 0.9))
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    // The same report again (the engine reports all the time): still held.
    s.stop(100)
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    // A seek gives the lane the hand.
    s.stop(500)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.5, 6)
    // So does a play, after another change.
    s.run(setDeviceParam(s.synth, 'sustain', 0.4))
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    s.play(500)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.5, 6)
    // Running past the last point: the change holds until a jump.
    s.play(1200)
    s.run(setDeviceParam(s.synth, 'sustain', 0.1))
    s.play(1900)
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    s.play(10)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.206, 6)
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
    // Stop at 700: the engine applies the lane's value there, and Play from there is a jump too.
    s.stop(700)
    expect(s.automated.device(s.synth, 'sustain')).toBeCloseTo(0.7, 6)
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

  it('lets go at a stop or a seek to where the playhead already stands: the engine frees the lanes at every cue (#394)', () => {
    const s = session()
    s.run(createLane(s.track, { device: { device: s.synth, param: 'sustain' } }))
    s.run(addPoints(s.lane(), [{ tick: 0, value: 0.4, shape: 'step' }]))
    s.stop(0)
    s.run(setDeviceParam(s.synth, 'sustain', 0.9))
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    // The report of the stopped transport does not change: the playhead stays at 0, only the cue tells.
    s.stop(0)
    expect(s.automated.device(s.synth, 'sustain')).toBeUndefined()
    s.project.cued?.()
    expect(s.automated.device(s.synth, 'sustain')).toBe(0.4)
  })
})

describe('the mark of a control that a lane drives (#388)', () => {
  it('names the lane in the language of the page, for a device parameter and for the mixer, and only for the automated ones', () => {
    const s = session()
    s.run(createLane(s.track, { device: { device: s.synth, param: 'sustain' } }))
    s.run(createLane(s.track, { mixer: { param: 'gain' } }))
    s.run(addPoints(s.lane(0), [{ tick: 0, value: 0.2, shape: 'linear' }]))
    s.run(addPoints(s.lane(1), [{ tick: 0, value: -12, shape: 'linear' }]))
    const volume = s.automated.mixerMark(s.track, 'gain')
    expect(volume).toContain('Mixer › Volume')
    expect(s.automated.deviceMark(s.synth, 'sustain')).toContain('Sustain')
    expect(s.automated.deviceMark(s.synth, 'attack')).toBeUndefined()
    expect(s.automated.mixerMark(s.track, 'pan')).toBeUndefined()
    expect(s.automated.mixerMark('someone-else', 'gain')).toBeUndefined()
    s.ui.lang = 'fr'
    expect(s.automated.mixerMark(s.track, 'gain')).toContain('Mixeur › Volume')
    expect(s.automated.mixerMark(s.track, 'gain')).not.toBe(volume)
  })

  it('marks the pan when a pan lane drives it, apart from the volume mark', () => {
    const s = session()
    s.run(createLane(s.track, { mixer: { param: 'pan' } }))
    s.run(addPoints(s.lane(), [{ tick: 0, value: -0.5, shape: 'step' }]))
    expect(s.automated.mixerMark(s.track, 'pan')).toContain('Pan')
    expect(s.automated.mixerMark(s.track, 'gain')).toBeUndefined()
    expect(s.automated.mixer(s.track, 'pan')).toBe(-0.5)
  })

  it('does not mark a parameter whose lane has no point yet: such a lane does nothing', () => {
    const s = session()
    s.run(createLane(s.track, { mixer: { param: 'gain' } }))
    expect(s.automated.mixerMark(s.track, 'gain')).toBeUndefined()
    s.run(addPoints(s.lane(), [{ tick: 0, value: -12, shape: 'linear' }]))
    expect(s.automated.mixerMark(s.track, 'gain')).toBeDefined()
  })

  it('keeps the mark while a change holds the lane, and shows the lane value on the track list and the mixer, silence included', () => {
    const s = session()
    s.run(createLane(s.track, { mixer: { param: 'gain' } }))
    s.run(
      addPoints(s.lane(), [
        { tick: 0, value: -96, shape: 'linear' },
        { tick: 1000, value: 0, shape: 'linear' },
      ]),
    )
    expect(s.automated.shownGain(s.track, -3)).toBeNull()
    s.stop(1000)
    expect(s.automated.shownGain(s.track, -3)).toBe(0)
    s.run(setVolume(s.track, -10))
    expect(s.automated.shownGain(s.track, -10)).toBe(-10)
    expect(s.automated.mixerMark(s.track, 'gain')).toBeDefined()
    expect(s.automated.shownPan(s.track, 0.3)).toBe(0.3)
  })
})

describe('the master (#378)', () => {
  it('follows a volume lane of the master and marks the fader, apart from the tracks', () => {
    const s = session()
    s.run(createMasterLane({ mixer: { param: 'gain' } }))
    const lane = Object.keys(s.project.view!.project.master_automation!)[0]!
    expect(s.automated.mixerMark('master', 'gain')).toBeUndefined()
    s.run(
      addPoints(lane, [
        { tick: 0, value: -96, shape: 'linear' },
        { tick: 1000, value: 0, shape: 'linear' },
      ]),
    )
    s.play(500)
    expect(s.automated.mixer('master', 'gain')).toBeCloseTo(-21, 6)
    expect(s.automated.shownGain('master', 0)).toBeCloseTo(-21, 6)
    expect(s.automated.mixerMark('master', 'gain')).toContain('Mixer')
    // The track's own volume is not driven by the master's lane.
    expect(s.automated.mixer(s.track, 'gain')).toBeUndefined()
    // A gesture on the master's fader holds the lane until its next point, as for a track.
    s.run(setMasterVolume(-3))
    expect(s.automated.mixer('master', 'gain')).toBeUndefined()
    s.play(700)
    expect(s.automated.mixer('master', 'gain')).toBeUndefined()
    // A seek lets go of the hold.
    s.stop(200)
    expect(s.automated.mixer('master', 'gain')).toBeDefined()
  })

  it('follows a lane on a parameter of a master effect', () => {
    const s = session()
    s.run(insertMasterEffect('faulk.compressor', 0))
    const effect = Object.keys(s.project.view!.project.master_devices!)[0]!
    s.run(createMasterLane({ device: { device: effect, param: 'threshold' } }))
    const lane = Object.keys(s.project.view!.project.master_automation!)[0]!
    s.run(
      addPoints(lane, [
        { tick: 0, value: -40, shape: 'linear' },
        { tick: 1000, value: 0, shape: 'linear' },
      ]),
    )
    s.play(500)
    expect(s.automated.device(effect, 'threshold')).toBeCloseTo(-20, 6)
    expect(s.automated.deviceMark(effect, 'threshold')).toBeDefined()
  })
})
