// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { UiStore } from '../../lib/stores/ui.svelte'
import { clampHeight, COLLAPSED_KEY, HEIGHTS_KEY, MAIN_MIN, PANEL_MIN, panelMax, panelMin, parseHeights, ROLL_TOOLBAR_ROOM } from './panel-size'

// One octave of 16 px rows, under the ruler (28), the toolbar (36), the tabs (41) and above the velocity lane (56).
const OCTAVE_ROOM = 12 * 16 + 28 + 36 + 41 + 56

describe('panel size', () => {
  it('raises the roll minimum by the toolbar height beyond its two rows (#887), and no other view', () => {
    expect(panelMin('piano-roll', 0)).toBe(PANEL_MIN['piano-roll'])
    expect(panelMin('piano-roll', 18.2)).toBe(PANEL_MIN['piano-roll'] + 19)
    expect(panelMin('piano-roll', -5)).toBe(PANEL_MIN['piano-roll'])
    expect(panelMin('devices', 40)).toBe(PANEL_MIN.devices)
    expect(clampHeight('piano-roll', 0, 2000, 32)).toBe(PANEL_MIN['piano-roll'] + 32)
    expect(panelMax('piano-roll', 100, 32)).toBe(PANEL_MIN['piano-roll'] + 32)
    expect(ROLL_TOOLBAR_ROOM).toBeLessThan(PANEL_MIN['piano-roll'])
  })

  it('keeps one readable octave of the piano roll at its minimum', () => {
    expect(PANEL_MIN['piano-roll']).toBeGreaterThanOrEqual(OCTAVE_ROOM)
  })

  it('clamps between the minimum of the view and the room the main view leaves', () => {
    expect(clampHeight('piano-roll', 10, 800)).toBe(PANEL_MIN['piano-roll'])
    expect(clampHeight('devices', 10, 800)).toBe(PANEL_MIN.devices)
    expect(clampHeight('devices', 5000, 800)).toBe(800 - MAIN_MIN)
    expect(clampHeight('devices', 200.4, 800)).toBe(200)
    expect(clampHeight('devices', Number.NaN, 800)).toBe(PANEL_MIN.devices)
  })

  it('never goes under the minimum, even in a body too short for it', () => {
    expect(panelMax('piano-roll', 300)).toBe(PANEL_MIN['piano-roll'])
    expect(clampHeight('piano-roll', 500, 300)).toBe(PANEL_MIN['piano-roll'])
  })

  it('reads stored heights, dropping unknown views and bad values', () => {
    expect(parseHeights('{"devices":200,"piano-roll":410.6}')).toEqual({ devices: 200, 'piano-roll': 411 })
    expect(parseHeights('{"devices":-3,"piano-roll":"x","other":5}')).toEqual({})
    expect(parseHeights('not json')).toEqual({})
    expect(parseHeights('null')).toEqual({})
    expect(parseHeights('[1]')).toEqual({})
    expect(parseHeights(null)).toEqual({})
  })
})

describe('the panel preferences of the page', () => {
  afterEach(() => {
    localStorage.clear()
    vi.restoreAllMocks()
  })

  it('keeps the height per view and the fold, and reads them back', () => {
    const ui = new UiStore('arrangement', 'devices')
    ui.setPanelHeight('devices', 200)
    ui.setPanelHeight('piano-roll', 420)
    ui.setPanelCollapsed(true)
    const again = new UiStore('arrangement', 'devices')
    expect(again.panelHeights).toEqual({ devices: 200, 'piano-roll': 420 })
    expect(again.panelCollapsed).toBe(true)
    again.setPanelCollapsed(false)
    expect(new UiStore('arrangement', 'devices').panelCollapsed).toBe(false)
  })

  it('starts from the defaults when the stored value is corrupt', () => {
    localStorage.setItem(HEIGHTS_KEY, '{oops')
    localStorage.setItem(COLLAPSED_KEY, 'maybe')
    const ui = new UiStore('arrangement', 'devices')
    expect(ui.panelHeights).toEqual({})
    expect(ui.panelCollapsed).toBe(false)
  })

  it('works, without keeping anything, when the browser refuses its storage', () => {
    const getItem = Storage.prototype.getItem
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(function (this: Storage, key: string) {
      if (key.startsWith('faulk.panel')) throw new Error('blocked')
      return getItem.call(this, key)
    })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('full')
    })
    const ui = new UiStore('arrangement', 'devices')
    expect(() => ui.setPanelHeight('devices', 220)).not.toThrow()
    expect(() => ui.setPanelCollapsed(true)).not.toThrow()
    expect(ui.panelHeights.devices).toBe(220)
    expect(ui.panelCollapsed).toBe(true)
  })
})
