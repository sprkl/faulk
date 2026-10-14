// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { assertLinked, createStage, type Renderer } from './stages'

/** A renderer that builds nothing real: `quad` either fails or returns a stand-in mesh. */
function fakeRenderer(quad: () => unknown): Renderer {
  return { noFlow: {}, float: false, quad } as unknown as Renderer
}

describe('stage lifecycle', () => {
  let el: HTMLElement
  beforeEach(() => {
    // jsdom has no 2D canvas; the stage only needs a context object to keep.
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({} as CanvasRenderingContext2D)
    el = document.createElement('div')
    el.innerHTML = '<div class="object"></div>'
    document.body.append(el)
  })
  afterEach(() => {
    el.remove()
    vi.restoreAllMocks()
  })

  it('leaves the CSS object in place when a stage fails while being built', () => {
    const r = fakeRenderer(() => {
      throw new Error('shader program did not link')
    })
    expect(() => createStage(r, el, 'kick', 0)).toThrow('did not link')
    expect(el.classList.contains('is-live')).toBe(false)
    expect(el.querySelector('canvas')).toBeNull()
  })

  it('takes over the element once built, and gives it back on destroy', () => {
    const stage = createStage(fakeRenderer(() => ({})), el, 'kick', 0)
    expect(el.classList.contains('is-live')).toBe(true)
    expect(el.firstElementChild?.tagName).toBe('CANVAS')
    expect(el.firstElementChild?.getAttribute('aria-hidden')).toBe('true')
    stage.destroy()
    expect(el.classList.contains('is-live')).toBe(false)
    expect(el.querySelector('canvas')).toBeNull()
  })
})

describe('assertLinked', () => {
  const gl = (linked: boolean) => ({ LINK_STATUS: 0x8b82 as const, getProgramParameter: () => linked })
  const program = { program: {} as WebGLProgram }

  it('throws on a program that did not link (OGL only warns)', () => {
    expect(() => assertLinked(gl(false), program)).toThrow('did not link')
  })

  it('accepts a linked program', () => {
    expect(() => assertLinked(gl(true), program)).not.toThrow()
  })
})
