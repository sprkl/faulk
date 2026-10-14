import { describe, expect, it } from 'vitest'
import { DEFAULT_PANEL, DEFAULT_VIEW, viewById, views } from './registry'

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
