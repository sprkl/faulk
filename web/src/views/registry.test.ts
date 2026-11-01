import { afterEach, describe, expect, it, vi } from 'vitest'
import { UiStore } from '../lib/stores/ui.svelte'
import { DEFAULT_PANEL, DEFAULT_VIEW, startView, viewById, views } from './registry'

describe('view registry', () => {
  it('has unique ids, sorted', () => {
    const ids: string[] = views.map((view) => view.id)
    expect(new Set(ids).size).toBe(ids.length)
    expect(ids).toEqual([...ids].sort())
  })

  it('resolves the default view, and nothing for an unknown id', () => {
    expect(viewById(DEFAULT_VIEW)?.place).toBe('main')
    expect(viewById(DEFAULT_PANEL)?.place).toBe('panel')
    expect(viewById('nope')).toBeUndefined()
  })

  it('gives every view a component', () => {
    for (const view of views) expect(view.component, view.id).toBeTypeOf('function')
  })
})

describe('view labels', () => {
  it('names every view in both languages', async () => {
    const { messages } = await import('../lib/i18n/index')
    for (const view of views) {
      expect(messages.en[view.label], view.id).toBeTruthy()
      expect(messages.fr[view.label], view.id).toBeTruthy()
    }
  })
})

describe('the view the app opens on', () => {
  const keep = (value: string | null) => vi.stubGlobal('localStorage', { getItem: () => value, setItem: () => {} })
  afterEach(() => vi.unstubAllGlobals())

  it('is the arrangement, not the instrument', () => {
    expect(DEFAULT_VIEW).toBe('arrangement')
    keep(null)
    expect(startView()).toBe('arrangement')
  })

  it('is the view used last, when it is still a main view', () => {
    keep('mixer')
    expect(startView()).toBe('mixer')
    keep('devices')
    expect(startView()).toBe('arrangement')
    keep('gone')
    expect(startView()).toBe('arrangement')
  })

  it('is kept when a view is shown', () => {
    const saved = new Map<string, string>()
    vi.stubGlobal('localStorage', { getItem: (k: string) => saved.get(k) ?? null, setItem: (k: string, v: string) => saved.set(k, v) })
    new UiStore('arrangement', 'devices').show('grid')
    expect(startView()).toBe('grid')
  })

  it('hands each view its zoom and scroll back', () => {
    const ui = new UiStore('arrangement', 'devices')
    expect(ui.recall('arrangement')).toBeUndefined()
    ui.remember('arrangement', { scrollY: 120 })
    expect(ui.recall('arrangement')).toEqual({ scrollY: 120 })
    expect(ui.recall('grid')).toBeUndefined()
  })
})
