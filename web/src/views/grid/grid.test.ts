import { render } from 'svelte/server'
import { describe, expect, it } from 'vitest'
import type { FeedbackReport } from '../../lib/engine/feedback'
import type { Clip, ProjectView } from '../../lib/generated/protocol'
import { messages } from '../../lib/i18n'
import { createStores, storesContext } from '../../lib/stores'
import { activeCells, activeSignature, cellClips, clipCounts, clipHomes, sortedScenes, visibleRange } from './cells'
import GridView from './GridView.svelte'

const midi = (name = ''): Clip => ({ name, slot: null, content: { midi: { length: 7680, notes: [] } } }) as unknown as Clip
const inSlot = (scene: string, name = ''): Clip => ({ ...midi(name), slot: scene })
const audio = (scene: string): Clip => ({ name: '', slot: scene, content: { audio: {} } }) as unknown as Clip

const slots = { epoch: 3, tracks: { t1: 0, t2: 1 }, scenes: { s1: 0, s2: 1, s3: 2 } }

function view(): ProjectView {
  return {
    project: {
      meter: [{ tick: 0, signature: { numerator: 4, denominator: 4 } }],
      scenes: { s2: { name: '', order: 'b' }, s1: { name: 'Intro', order: 'a' }, s3: { name: '', order: 'c' } },
      tracks: {
        t1: {
          kind: 'midi',
          order: 'a',
          name: 'Bass',
          color: '#58c27d',
          clips: { c1: inSlot('s1', 'Riff'), c2: inSlot('s2', 'Clip 2'), c0: midi('Arrangement only') },
          placements: {},
          devices: {},
        },
        t2: { kind: 'audio', order: 'b', name: 'Vox', clips: { c3: audio('s1') }, placements: {}, devices: {} },
      },
    },
    session: { selected_track: 't1' },
    slots,
  } as unknown as ProjectView
}

const report = (clips: FeedbackReport['clips'], epoch = 3): Pick<FeedbackReport, 'epoch' | 'clips'> => ({ epoch, clips })

describe('the cells of the grid', () => {
  it('orders the scenes by order key, then by id', () => {
    const scenes = { b: { name: '', order: 'a' }, a: { name: '', order: 'a' }, c: { name: '', order: '0' } } as never
    expect(sortedScenes({ scenes }).map((entry) => [entry.id, entry.index])).toEqual([
      ['c', 0],
      ['a', 1],
      ['b', 2],
    ])
  })

  it('finds the clip of each cell, leaves out the clips that are only in the arrangement, and counts a scene', () => {
    const cells = cellClips(view().project)
    expect([...cells.keys()].sort()).toEqual(['t1|s1', 't1|s2', 't2|s1'])
    expect(cells.get('t1|s1')).toMatchObject({ id: 'c1', launchable: true, editable: true })
    // An audio clip is launched (AU-4), not edited (AU-6).
    expect(cells.get('t2|s1')).toMatchObject({ id: 'c3', launchable: true, editable: false })
    expect(Object.fromEntries(clipCounts(cells))).toEqual({ s1: 2, s2: 1 })
  })

  it('finds where each clip sits', () => {
    const project = view().project
    const homes = clipHomes(cellClips(project))
    expect(homes.get('c2')).toMatchObject({ track: 't1', scene: 's2' })
    expect(homes.get('c3')).toMatchObject({ track: 't2', scene: 's1' })
    expect(homes.has('c0')).toBe(false)
  })

  it('reads the report by slots, back into ids', () => {
    const states = activeCells(
      report([
        { track: 0, scene: 1, state: 'playing', anchor: 0 },
        { track: 1, scene: 0, state: 'queued', anchor: 0 },
      ]),
      slots,
    )
    expect(Object.fromEntries(states)).toEqual({
      't1|s2': { state: 'playing', restarting: false },
      't2|s1': { state: 'queued', restarting: false },
    })
  })

  it('ignores a cell the page has no slot for, and a report of another epoch', () => {
    expect(
      activeCells(
        report([
          { track: 5, scene: 0, state: 'playing', anchor: 0 },
          { track: 0, scene: 9, state: 'playing', anchor: 0 },
        ]),
        slots,
      ).size,
    ).toBe(0)
    expect(activeCells(report([{ track: 0, scene: 0, state: 'playing', anchor: 0 }], 2), slots).size).toBe(0)
    expect(activeCells(null, slots).size).toBe(0)
  })

  it('marks a playing clip that was launched again, in either order, instead of losing one of its two entries', () => {
    for (const clips of [
      [
        { track: 0, scene: 0, state: 'playing', anchor: 0 },
        { track: 0, scene: 0, state: 'queued', anchor: 0 },
      ],
      [
        { track: 0, scene: 0, state: 'queued', anchor: 0 },
        { track: 0, scene: 0, state: 'playing', anchor: 0 },
      ],
      [
        { track: 0, scene: 0, state: 'playing', anchor: 0 },
        { track: 0, scene: 0, state: 'armed', anchor: 0 },
      ],
    ] as FeedbackReport['clips'][]) {
      expect(activeCells(report(clips), slots).get('t1|s1')).toEqual({ state: 'playing', restarting: true })
    }
    // A stopping clip stays a stopping clip.
    expect(activeCells(report([{ track: 0, scene: 0, state: 'stopping', anchor: 0 }]), slots).get('t1|s1')).toEqual({ state: 'stopping', restarting: false })
  })

  it('changes its signature with a state or an epoch, and with nothing else', () => {
    const a = activeSignature({ epoch: 1, clips: [{ track: 0, scene: 0, state: 'queued', anchor: 0 }] })
    expect(activeSignature({ epoch: 1, clips: [{ track: 0, scene: 0, state: 'queued', anchor: 0 }] })).toBe(a)
    expect(activeSignature({ epoch: 1, clips: [{ track: 0, scene: 0, state: 'playing', anchor: 0 }] })).not.toBe(a)
    expect(activeSignature({ epoch: 2, clips: [{ track: 0, scene: 0, state: 'queued', anchor: 0 }] })).not.toBe(a)
    expect(activeSignature(null)).toBe('')
  })
})

function html(options: { project?: ProjectView | null; clips?: FeedbackReport['clips']; started?: boolean; lang?: 'en' | 'fr' } = {}) {
  const stores = createStores('grid', 'piano-roll')
  stores.ui.lang = options.lang ?? 'en'
  const project = options.project === undefined ? view() : options.project
  if (project) stores.project.show(project)
  stores.engine.started = options.started ?? true
  stores.engine.feedback = { epoch: 3, clips: options.clips ?? [] } as FeedbackReport
  return render(GridView, { context: storesContext(stores) }).body
}

/** The `data-state` of each cell button, by track and scene. */
function states(body: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const m of body.matchAll(/<button[^>]*?data-state="(\w+)"[^>]*?>/g)) {
    const tag = m[0]
    out[`${/data-track="(\w+)"/.exec(tag)![1]}|${/data-scene="(\w+)"/.exec(tag)![1]}`] = m[1]!
  }
  return out
}

describe('what the grid shows', () => {
  it('says to start audio before there is a project, and to add a track when there is none', () => {
    expect(html({ project: null })).toContain('Start audio to load your project.')
    const empty = view()
    empty.project.tracks = {}
    expect(html({ project: empty })).toContain('data-testid="grid-no-tracks"')
  })

  it('shows tracks as columns and scenes as rows, in order, with the default names', () => {
    const body = html()
    expect(body.indexOf('Intro')).toBeLessThan(body.indexOf('Scene 2'))
    expect(body.indexOf('Bass')).toBeLessThan(body.indexOf('Vox'))
    expect(body.match(/data-testid="scene-row"/g)).toHaveLength(3)
    expect(body).toContain('aria-colcount="3"')
    expect(body).toContain('aria-rowcount="4"')
  })

  it('tells an empty project from one with no scene', () => {
    const none = view()
    none.project.scenes = {}
    for (const track of Object.values(none.project.tracks)) for (const clip of Object.values(track!.clips)) (clip as { slot: string | null }).slot = null
    expect(html({ project: none })).toContain('data-testid="grid-no-scenes"')
  })

  it('shows idle clips, empty cells and an audio clip with its state', () => {
    const found = states(html())
    expect(found).toEqual({ 't1|s1': 'idle', 't1|s2': 'idle', 't2|s1': 'idle', 't2|s2': 'empty', 't1|s3': 'empty', 't2|s3': 'empty' })
    const body = html()
    expect(body).toContain('Vox, Intro: Clip, audio clip, stopped')
    expect(body).toContain('Vox, Scene 2: empty')
    expect(body).toContain('Bass, Intro: Riff, stopped')
    expect(body).toContain('Bass, Scene 2: Clip 2, stopped')
    expect(html({ lang: 'fr' })).toContain('Bass, Scène 2 : Clip 2, à l’arrêt')
  })

  it('gives every state a glyph of its own, besides its colour and its words', () => {
    const shapes = new Map<string, string>()
    for (const [clips, state, words] of [
      [[], 'idle', 'stopped'],
      [[{ track: 0, scene: 0, state: 'queued', anchor: 0 }], 'queued', 'queued'],
      [[{ track: 0, scene: 0, state: 'playing', anchor: 0 }], 'playing', 'playing'],
      [[{ track: 0, scene: 0, state: 'stopping', anchor: 0 }], 'stopping', 'stopping'],
      [[{ track: 0, scene: 0, state: 'armed', anchor: 0 }], 'armed', 'ready, starts with the transport'],
      [
        [
          { track: 0, scene: 0, state: 'playing', anchor: 0 },
          { track: 0, scene: 0, state: 'queued', anchor: 0 },
        ],
        'restarting',
        'playing, restarts at the next boundary',
      ],
    ] as [FeedbackReport['clips'], string, string][]) {
      const body = html({ clips })
      expect(states(body)['t1|s1']).toBe(state)
      expect(body).toContain(`Bass, Intro: Riff, ${words}`)
      const cell = /<button[^>]*data-clip="c1"[\s\S]*?<\/button>/.exec(body)![0]
      expect(cell).toContain(`data-glyph="${state}"`)
      shapes.set(state, /<svg[\s\S]*?<\/svg>/.exec(cell)![0].replace(/data-glyph="\w+"/, ''))
    }
    // Six states, six different drawings.
    expect(new Set(shapes.values()).size).toBe(6)
  })

  it('shows no state while the engine is not started, whatever its last report says', () => {
    expect(states(html({ started: false, clips: [{ track: 0, scene: 0, state: 'playing', anchor: 0 }] }))['t1|s1']).toBe('idle')
  })

  it('reads the engine report by the ids of the slots', () => {
    const found = states(html({ clips: [{ track: 0, scene: 1, state: 'playing', anchor: 0 }] }))
    expect(found['t1|s2']).toBe('playing')
    expect(found['t1|s1']).toBe('idle')
  })

  it('offers an empty cell of a MIDI track a clip, an audio track none', () => {
    const body = html()
    expect(body).toContain('Double-click or press Enter to create a clip.')
    const cells = body.match(/<button[^>]*data-state="empty"[^>]*>/g)!
    expect(cells).toHaveLength(3)
    expect(cells.filter((cell) => cell.includes('data-midi="true"'))).toHaveLength(1)
    expect(body).toContain('Vox, Scene 2: empty"')
  })

  it('has one cell in the tab order', () => {
    const body = html()
    expect(body.match(/<button[^>]*tabindex="0"[^>]*data-cell/g)).toHaveLength(1)
  })

  it('is in French too', () => {
    const body = html({ lang: 'fr' })
    expect(body).toContain('Tout arrêter')
    expect(body).toContain('Scène 2')
    expect(body).toContain('Bass, Intro : Riff, à l’arrêt')
  })
})

describe('the texts of the grid', () => {
  it('has each state in both languages', () => {
    for (const lang of ['en', 'fr'] as const) {
      for (const state of ['idle', 'queued', 'playing', 'restarting', 'stopping', 'armed']) {
        expect(messages[lang][`grid.state.${state}` as keyof typeof messages.en]).toBeTruthy()
      }
    }
  })
})

describe('the window of drawn cells', () => {
  it('shows the items that meet the viewport, with an overscan, inside the list', () => {
    // 40-px rows under a 40-px header, a 400-px view: rows 0 to 8 show.
    expect(visibleRange(0, 400, 40, 64, 0, 40)).toEqual({ first: 0, last: 9 })
    expect(visibleRange(0, 400, 40, 64, 3, 40)).toEqual({ first: 0, last: 12 })
    // Scrolled by 1 000 px: the first row seen is the one under the header.
    expect(visibleRange(1000, 400, 40, 64, 3, 40)).toEqual({ first: 21, last: 37 })
    // At the end, and with fewer items than the view holds.
    expect(visibleRange(99999, 400, 40, 64, 3, 40)).toEqual({ first: 64, last: 64 })
    expect(visibleRange(0, 400, 40, 3, 3, 40)).toEqual({ first: 0, last: 3 })
    expect(visibleRange(0, 400, 40, 0, 3, 40)).toEqual({ first: 0, last: 0 })
  })

  it('draws a window of a 32 by 64 grid, not its 2 048 cells, and keeps the room of the rest', () => {
    const stores = createStores('grid', 'piano-roll')
    const big = view()
    big.project.scenes = Object.fromEntries(
      Array.from({ length: 64 }, (_, i) => [`s${i}`, { name: '', order: String.fromCharCode(48 + (i >> 3)) + String(i & 7) }]),
    ) as never
    big.project.tracks = Object.fromEntries(
      Array.from({ length: 32 }, (_, i) => [`t${i}`, { kind: 'midi', order: String(i).padStart(2, '0'), name: '', clips: {}, placements: {}, devices: {} }]),
    ) as never
    stores.project.show(big)
    stores.engine.started = true
    const body = render(GridView, { context: storesContext(stores) }).body
    const cells = body.match(/data-testid="cell"/g)!.length
    // 1 280 x 720 view, 3 and 2 items of overscan: well under the 2 048 cells of the project.
    expect(cells).toBeGreaterThan(0)
    expect(cells).toBeLessThan(400)
    expect(body).toContain('aria-rowcount="65"')
    expect(body).toContain('aria-colcount="33"')
    // The rows below the window keep their room.
    expect(body).toMatch(/class="skip[^"]*" style="height: \d+px;"/)
  })
})
