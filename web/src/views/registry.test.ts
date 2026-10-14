import { describe, expect, it } from 'vitest'
import { DEFAULT_VIEW, viewById, views } from './registry'

describe('view registry', () => {
  it('has unique ids, sorted', () => {
    const ids: string[] = views.map((view) => view.id)
    expect(new Set(ids).size).toBe(ids.length)
    expect(ids).toEqual([...ids].sort())
  })

  it('resolves the default view, and nothing for an unknown id', () => {
    expect(viewById(DEFAULT_VIEW)?.id).toBe(DEFAULT_VIEW)
    expect(viewById('nope')).toBeUndefined()
  })

  it('gives every view a component', () => {
    for (const view of views) expect(view.component, view.id).toBeTypeOf('function')
  })
})
