import { beforeEach, describe, expect, it } from 'vitest'
import { DENSITY_KEY, UiStore } from './ui.svelte'

describe('the density of the rows (#490)', () => {
  beforeEach(() => {
    localStorage.clear()
    delete document.documentElement.dataset.density
  })

  it('is comfortable by default, with no attribute on the document', () => {
    const ui = new UiStore('arrangement', 'devices')
    expect(ui.density).toBe('comfortable')
    expect(document.documentElement.dataset.density).toBeUndefined()
  })

  it('compact sets data-density, is kept in the browser and comes back on the next page', () => {
    const ui = new UiStore('arrangement', 'devices')
    ui.setDensity('compact')
    expect(document.documentElement.dataset.density).toBe('compact')
    expect(localStorage.getItem(DENSITY_KEY)).toBe('compact')

    delete document.documentElement.dataset.density
    const later = new UiStore('arrangement', 'devices')
    expect(later.density).toBe('compact')
    expect(document.documentElement.dataset.density).toBe('compact')

    later.setDensity('comfortable')
    expect(document.documentElement.dataset.density).toBeUndefined()
    expect(localStorage.getItem(DENSITY_KEY)).toBe('comfortable')
  })

  it('reads an unknown stored value as comfortable', () => {
    localStorage.setItem(DENSITY_KEY, 'tiny')
    expect(new UiStore('arrangement', 'devices').density).toBe('comfortable')
  })
})
