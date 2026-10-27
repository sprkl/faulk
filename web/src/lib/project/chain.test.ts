import { describe, expect, it } from 'vitest'
import { insertEffect, insertMasterEffect, moveDevice, removeDevice, setBypass, setInstrument } from '../device-commands'
import type { DeviceDescriptor } from '../devices'
import type { Device, Track } from '../generated/protocol'
import { addEffectCommand, addMasterEffectCommand, cardActions, chainOf, effectCount, masterChainOf, moveTarget } from './chain'

const builtin = (id: string) => ({ builtin: { id, version: 1 } })
const device = (kind: string, role: 'instrument' | 'effect', order: string, enabled = true): Device => ({ order, role, enabled, params: {}, source: builtin(kind) })
const track = (devices: Record<string, Device>): Track => ({ devices }) as unknown as Track

const fake = (kind: string, role: string): DeviceDescriptor => ({ kind, name: kind, role, params: [] }) as unknown as DeviceDescriptor
const descriptors = [fake('faulk.osc_synth', 'instrument'), fake('faulk.test_gain', 'effect')]

// The instrument, then two effects, stored out of order on purpose.
const t = track({
  b: device('faulk.test_gain', 'effect', 'n'),
  a: device('faulk.osc_synth', 'instrument', 'a'),
  c: device('faulk.test_gain', 'effect', 'z', false),
})

describe('the chain of a track', () => {
  it('lists the instrument first, then the effects in order, with their place among the effects', () => {
    const chain = chainOf(t, descriptors)
    expect(chain.map((e) => [e.id, e.effectIndex])).toEqual([
      ['a', null],
      ['b', 0],
      ['c', 1],
    ])
    expect(effectCount(chain)).toBe(2)
    expect(chain[0].descriptor?.kind).toBe('faulk.osc_synth')
  })

  it('leaves out the clip player of an audio track, and keeps a device it has no descriptor for', () => {
    const audio = track({ p: device('faulk.clip_player', 'instrument', 'a'), x: device('faulk.unknown', 'effect', 'b') })
    const chain = chainOf(audio, descriptors)
    expect(chain.map((e) => e.id)).toEqual(['x'])
    expect(chain[0].descriptor).toBeNull()
    expect(chain[0].effectIndex).toBe(0)
  })

  it('moves an effect by the index among the effects, never past either end, never the instrument', () => {
    const three = track({
      i: device('faulk.osc_synth', 'instrument', 'a'),
      e0: device('faulk.test_gain', 'effect', 'b'),
      e1: device('faulk.test_gain', 'effect', 'c'),
      e2: device('faulk.test_gain', 'effect', 'd'),
    })
    const chain = chainOf(three, descriptors)
    expect(moveTarget(chain, 'i', 'left')).toBeNull()
    expect(moveTarget(chain, 'i', 'right')).toBeNull()
    expect(moveTarget(chain, 'e0', 'left')).toBeNull()
    expect(moveTarget(chain, 'e0', 'right')).toBe(1)
    expect(moveTarget(chain, 'e1', 'left')).toBe(0)
    expect(moveTarget(chain, 'e1', 'right')).toBe(2)
    expect(moveTarget(chain, 'e2', 'left')).toBe(1)
    expect(moveTarget(chain, 'e2', 'right')).toBeNull()
    expect(moveTarget(chain, 'nope', 'right')).toBeNull()
  })
})

describe('the commands of the device panel', () => {
  it('are the DV-8 commands, with their payloads', () => {
    expect(insertEffect('t', 'faulk.test_gain', 2)).toEqual({ type: 'device.insert', track: 't', kind: 'faulk.test_gain', index: 2 })
    expect(removeDevice('d')).toEqual({ type: 'device.remove', device: 'd' })
    expect(moveDevice('d', 1)).toEqual({ type: 'device.move', device: 'd', index: 1 })
    expect(setBypass('d', true)).toEqual({ type: 'device.set_bypass', device: 'd', bypass: true })
    expect(setInstrument('t', 'faulk.drums')).toEqual({ type: 'track.set_instrument', track: 't', instrument: 'faulk.drums' })
  })
})

describe('the buttons of a card', () => {
  const three = track({
    i: device('faulk.osc_synth', 'instrument', 'a'),
    e0: device('faulk.test_gain', 'effect', 'b'),
    e1: device('faulk.test_gain', 'effect', 'c', false),
    e2: device('faulk.test_gain', 'effect', 'd'),
  })
  const chain = chainOf(three, descriptors)
  const [instrument, e0, e1, e2] = chain

  it('send the move to the index among the effects, never past either end', () => {
    expect(cardActions(chain, e1)).toEqual({
      bypass: { type: 'device.set_bypass', device: 'e1', bypass: false },
      moveLeft: { type: 'device.move', device: 'e1', index: 0 },
      moveRight: { type: 'device.move', device: 'e1', index: 2 },
      remove: { type: 'device.remove', device: 'e1' },
    })
    expect(cardActions(chain, e0).moveLeft).toBeNull()
    expect(cardActions(chain, e0).moveRight).toEqual({ type: 'device.move', device: 'e0', index: 1 })
    expect(cardActions(chain, e2).moveLeft).toEqual({ type: 'device.move', device: 'e2', index: 1 })
    expect(cardActions(chain, e2).moveRight).toBeNull()
  })

  it('toggle the bypass from the device state, the instrument included', () => {
    expect(cardActions(chain, e0).bypass).toEqual({ type: 'device.set_bypass', device: 'e0', bypass: true })
    expect(cardActions(chain, instrument).bypass).toEqual({ type: 'device.set_bypass', device: 'i', bypass: true })
  })

  it('add an effect after the last one', () => {
    expect(addEffectCommand('t', 'faulk.test_gain', chain)).toEqual({ type: 'device.insert', track: 't', kind: 'faulk.test_gain', index: 3 })
    expect(addEffectCommand('t', 'faulk.test_gain', [instrument])).toEqual({ type: 'device.insert', track: 't', kind: 'faulk.test_gain', index: 0 })
  })
})

describe('the chain of the master', () => {
  const master = { y: device('faulk.test_gain', 'effect', 'z'), x: device('faulk.test_gain', 'effect', 'a', false) }

  it('lists the effects in order, none being the instrument, and is empty when the project has none', () => {
    const chain = masterChainOf(master, descriptors)
    expect(chain.map((e) => [e.id, e.effectIndex])).toEqual([
      ['x', 0],
      ['y', 1],
    ])
    expect(masterChainOf(undefined, descriptors)).toEqual([])
  })

  it('adds an effect at the end with master: true, and the buttons of a card send the same commands as a track', () => {
    const chain = masterChainOf(master, descriptors)
    expect(addMasterEffectCommand('faulk.test_gain', chain)).toEqual({ type: 'device.insert', master: true, kind: 'faulk.test_gain', index: 2 })
    expect(insertMasterEffect('k', 0)).toEqual({ type: 'device.insert', master: true, kind: 'k', index: 0 })
    const actions = cardActions(chain, chain[0])
    expect(actions.moveLeft).toBeNull()
    expect(actions.moveRight).toEqual(moveDevice('x', 1))
    expect(actions.remove).toEqual(removeDevice('x'))
    expect(actions.bypass).toEqual(setBypass('x', false))
  })
})
