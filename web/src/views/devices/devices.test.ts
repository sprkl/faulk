import { render } from 'svelte/server'
import { describe, expect, it } from 'vitest'
import type { DeviceDescriptor } from '../../lib/devices'
import type { Device } from '../../lib/generated/protocol'
import { chainOf } from '../../lib/project/chain'
import { createStores, storesContext } from '../../lib/stores'
import DeviceBrowser from './DeviceBrowser.svelte'
import DeviceCard from './DeviceCard.svelte'

// A device the page has no text for: it still gets a generated panel, whatever its name or parameters.
const param = (key: string, values: string[] = []) => ({ id: 0, key, description: `What ${key} does.`, unit: 'none', min: 0, max: values.length ? values.length - 1 : 1, default: 0, scale: 'linear', step: values.length ? 1 : 0, smoothing_seconds: 0, values })
const fakeEffect = { kind: 'faulk.fake_fx', kind_id: 99, version: 1, name: 'Fake FX', role: 'effect', memory_bytes: 0, midi_output: null, params: [param('depth'), param('shape', ['round', 'sharp'])] } as unknown as DeviceDescriptor
const fakeInstrument = { ...fakeEffect, kind: 'faulk.fake_synth', name: 'Fake Synth', role: 'instrument' } as unknown as DeviceDescriptor

const builtin = (id: string) => ({ builtin: { id, version: 1 } })
const effect: Device = { order: 'b', role: 'effect', enabled: false, params: { depth: 0.5 }, source: builtin('faulk.fake_fx') }
const instrument: Device = { order: 'a', role: 'instrument', enabled: true, params: {}, source: builtin('faulk.fake_synth') }

const stores = () => createStores('play', 'devices')

describe('the device browser', () => {
  it('lists the effects of the registry it is given, and an empty state when there is none', () => {
    const withFx = render(DeviceBrowser, { props: { track: 't', midi: true, chain: [], descriptors: [fakeInstrument, fakeEffect] }, context: storesContext(stores()) }).body
    expect(withFx).toContain('data-testid="browse-effect" data-kind="faulk.fake_fx"')
    expect(withFx).toContain('data-testid="browse-instrument" data-kind="faulk.fake_synth"')
    expect(withFx).not.toContain('browse-no-effects')
    const none = render(DeviceBrowser, { props: { track: 't', midi: true, chain: [], descriptors: [fakeInstrument] }, context: storesContext(stores()) }).body
    expect(none).toContain('data-testid="browse-no-effects"')
    expect(none).not.toContain('browse-effect"')
  })

  it('offers no instrument on an audio track', () => {
    const body = render(DeviceBrowser, { props: { track: 't', midi: false, chain: [], descriptors: [fakeInstrument, fakeEffect] }, context: storesContext(stores()) }).body
    expect(body).toContain('data-testid="browse-audio"')
    expect(body).not.toContain('browse-instrument')
  })
})

describe('a device card', () => {
  const track = { devices: { i: instrument, e: effect } } as never
  const chain = chainOf(track, [fakeInstrument, fakeEffect])

  it('generates a control per parameter of the descriptor, with the key as label when the catalogue has no text', () => {
    const body = render(DeviceCard, { props: { entry: chain[1], chain }, context: storesContext(stores()) }).body
    expect(body).toContain('Fake FX')
    expect(body).toContain('data-testid="param-depth"')
    expect(body).toContain('data-testid="param-shape"')
    expect(body).toContain('>depth<')
    expect(body).toContain('title="What depth does."')
    expect(body).toContain('data-bypassed="true"')
  })

  it('gives an effect its move and remove buttons, and the instrument bypass only', () => {
    const fx = render(DeviceCard, { props: { entry: chain[1], chain }, context: storesContext(stores()) }).body
    for (const id of ['device-bypass', 'device-left', 'device-right', 'device-remove']) expect(fx, id).toContain(`data-testid="${id}"`)
    // The only effect: it cannot move either way.
    expect(fx).toMatch(/data-testid="device-left" disabled/)
    expect(fx).toMatch(/data-testid="device-right" disabled/)
    const inst = render(DeviceCard, { props: { entry: chain[0], chain }, context: storesContext(stores()) }).body
    expect(inst).toContain('data-testid="device-bypass"')
    for (const id of ['device-left', 'device-right', 'device-remove']) expect(inst, id).not.toContain(`data-testid="${id}"`)
  })

  it('still shows a device it has no descriptor for, by its kind', () => {
    const unknown = chainOf({ devices: { x: { ...effect, source: builtin('faulk.mystery') } } } as never, [])
    const body = render(DeviceCard, { props: { entry: unknown[0], chain: unknown }, context: storesContext(stores()) }).body
    expect(body).toContain('faulk.mystery')
    expect(body).toContain('data-testid="device-remove"')
  })
})
