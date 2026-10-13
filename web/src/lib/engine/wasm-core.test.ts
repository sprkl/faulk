// Runs the real engine.wasm (built by `pnpm test`) through the same code as the worklet.
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import type { Command, SynthParam } from '../generated/protocol'
import { WasmCore, type EngineExports } from './wasm-core'
import { commandToOp } from './wasm-ops'

const bytes = readFileSync(new URL('./wasm/engine.wasm', import.meta.url))
const module = new WebAssembly.Module(bytes)

function core() {
  const instance = new WebAssembly.Instance(module, {})
  return new WasmCore(instance.exports as unknown as EngineExports, 48_000)
}

function run(c: WasmCore, command: Command) {
  const op = commandToOp(command)
  if (op) c.apply(op)
}

function renderSeconds(c: WasmCore, seconds: number): Float32Array {
  const out = new Float32Array(Math.round(48_000 * seconds))
  const left = new Float32Array(128)
  const right = new Float32Array(128)
  for (let at = 0; at + 128 <= out.length; at += 128) {
    c.render(left, right)
    out.set(left, at)
  }
  return out
}

const peak = (s: Float32Array) => s.reduce((m, x) => Math.max(m, Math.abs(x)), 0)

describe('wasm engine through commands', () => {
  it('is silent without notes and plays a note', () => {
    const c = core()
    expect(peak(renderSeconds(c, 0.1))).toBe(0)
    run(c, { type: 'note.on', note: 60, velocity: 100 })
    expect(peak(renderSeconds(c, 0.2))).toBeGreaterThan(0.05)
    expect(c.activeVoices()).toBe(1)
    expect(c.takePeak()).toBeGreaterThan(0.05)
    expect(c.takePeak()).toBe(0)
  })

  it('releases with note.off and note.all_off', () => {
    const c = core()
    run(c, { type: 'note.on', note: 60, velocity: 100 })
    run(c, { type: 'note.on', note: 64, velocity: 100 })
    renderSeconds(c, 0.1)
    run(c, { type: 'note.off', note: 60 })
    renderSeconds(c, 1)
    expect(c.activeVoices()).toBe(1)
    run(c, { type: 'note.all_off' })
    renderSeconds(c, 1)
    expect(c.activeVoices()).toBe(0)
  })

  it('maps every synth parameter to the right engine parameter', () => {
    // Each parameter, set through a command, changes the signal in its own way.
    const setAndRender = (param: SynthParam, value: number) => {
      const c = core()
      run(c, { type: 'synth.set_param', param, value })
      run(c, { type: 'note.on', note: 57, velocity: 127 })
      const held = renderSeconds(c, 0.5)
      run(c, { type: 'note.off', note: 57 })
      const tail = renderSeconds(c, 0.5)
      return { held, tail }
    }
    const base = setAndRender('gain', 0.8)
    // Gain is smoothed (10 ms time constant): after 0.2 s it is effectively silent.
    expect(peak(setAndRender('gain', 0).held.subarray(9_600))).toBeLessThan(1e-3)
    // Attack of 0.4 s: the first 10 ms are much quieter than with the default 5 ms.
    expect(peak(setAndRender('attack', 0.4).held.subarray(0, 480))).toBeLessThan(peak(base.held.subarray(0, 480)) / 4)
    // Release of 2 s keeps sounding after 0.4 s; the default 0.3 s does not.
    expect(peak(setAndRender('release', 2).tail.subarray(19_200))).toBeGreaterThan(0.01)
    expect(peak(base.tail.subarray(19_200))).toBe(0)
    // Sustain 0 with a short decay: the held note decays to silence.
    const decayed = (() => {
      const c = core()
      run(c, { type: 'synth.set_param', param: 'sustain', value: 0 })
      run(c, { type: 'synth.set_param', param: 'decay', value: 0.05 })
      run(c, { type: 'note.on', note: 57, velocity: 127 })
      return renderSeconds(c, 0.5)
    })()
    expect(peak(decayed.subarray(12_000))).toBe(0)
    // Waveform: a sine and a square at the same pitch differ.
    const sine = setAndRender('waveform', 0).held
    const square = setAndRender('waveform', 2).held
    expect(sine.some((s, i) => Math.abs(s - square[i]) > 0.05)).toBe(true)
  })

  it('ignores commands for the page', () => {
    expect(commandToOp({ type: 'midi.list_inputs' })).toBeNull()
    expect(commandToOp({ type: 'midi.select_input', id: null })).toBeNull()
  })
})
