import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #604: the top bar is one row that never overlaps. It was reported twice (the engine's load, then the tap button): a
// local fix each time. The bar now folds its least-read parts one step at a time (`fit`, TopBar.svelte) so that no two
// controls share pixels, whatever the language, the width and the state: a track armed (the take's target shows), a
// long project name, and the widest engine pill.

interface Box {
  id: string
  x: number
  y: number
  r: number
  b: number
}

const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun(c), command)
/** The dev server's hooks (`__faulkRun`) exist in the `dev` project only: the `preview` project serves the production build. */

/** Every control and readout of the bar that paints something, as a box (the screen-reader-only ones are not seen). */
const LEAVES = [
  'a',
  'button',
  'input',
  'select',
  '[data-testid="save-state"]',
  '[data-testid="top-meter"]',
  '[data-testid="transport-record-target"]',
  '[data-testid="transport-position"]',
  '[data-testid="transport-time"]',
  '[data-testid="keys-octave-bar"]',
  '[data-bar-item]',
].join(',')

async function layout(page: Page) {
  return page.evaluate((selector) => {
    const bar = document.querySelector('header.top')!
    const box = (el: Element, id: string): Box => {
      const r = el.getBoundingClientRect()
      return { id, x: r.left, y: r.top, r: r.right, b: r.bottom }
    }
    const leaves = [...bar.querySelectorAll(selector)].filter((el) => {
      const r = el.getBoundingClientRect()
      return r.width > 0 && r.height > 0 && !el.closest('.sr-only')
    })
    const name = document.querySelector('[data-testid=project-name]')!
    return {
      bar: box(bar, 'bar'),
      overflow: bar.scrollWidth - bar.clientWidth,
      fit: Number((bar as HTMLElement).dataset.fit),
      boxes: leaves.map((el, i) => box(el, el.getAttribute('data-testid') ?? `${el.tagName.toLowerCase()}-${i}`)),
      nameWidth: name.clientWidth,
    }
  }, LEAVES)
}

/** The pairs of boxes that share pixels (half a pixel of rounding is allowed). */
function overlaps(boxes: Box[]): string[] {
  const found: string[] = []
  for (let i = 0; i < boxes.length; i++)
    for (let j = i + 1; j < boxes.length; j++) {
      const a = boxes[i]!
      const c = boxes[j]!
      if (a.x < c.r - 0.5 && c.x < a.r - 0.5 && a.y < c.b - 0.5 && c.y < a.b - 0.5) found.push(`${a.id} / ${c.id}`)
    }
  return found
}

/**
 * Adds what the bar can carry beyond the real controls, as fixed-width stand-ins: the wider text of Boost (16 px) and, when a
 * test needs the quantization folded whatever the language, `extra` more pixels in the engine pill. The tap button (#603)
 * and the CPU figure (#594) are real now; their stand-ins (34 and 38 px) stay only for a branch that has not got them yet.
 */
async function widest(page: Page, extra = 0) {
  await page.evaluate((more) => {
    const standIn = (parent: Element, before: Element | null, width: number, own = true) => {
      const el = document.createElement('span')
      if (own) el.setAttribute('data-bar-item', '')
      el.style.cssText = `display:inline-block;flex:none;height:28px;width:${width}px`
      parent.insertBefore(el, before)
    }
    const transport = document.querySelector('[data-testid=transport]')!
    if (!document.querySelector('[data-testid=transport-tap]'))
      standIn(transport, document.querySelector('[data-testid=transport-numerator]')!.parentElement, 34)
    const pill = document.querySelector('[data-testid=engine-mode]')!
    if (!document.querySelector('[data-testid=engine-load]')) standIn(pill, null, 38, false)
    standIn(pill, null, 16 + more, false)
  }, extra)
}

async function open(page: Page, lang: 'en' | 'fr', width: number) {
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.setViewportSize({ width, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  // The long name goes in through the command in dev; the preview build keeps the default name and checks the rest of the bar.
  const named = test.info().project.name === 'dev'
  if (named) await run(page, { type: 'project.rename', name: 'A very long project name that goes on and on' })
  await page.getByTestId('nav-arrangement').click()
  const arm = page.getByTestId('arr-arm').first()
  await arm.click()
  await expect(arm).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByTestId('transport-record-target')).toBeAttached()
}

/** The checks of every width: one row, nothing outside the bar, no two controls over each other. */
async function assertFits(page: Page, width: number) {
  // The bar re-measures itself on the next frame after a change (a ResizeObserver): the checks settle for a moment, never past it.
  let out!: Awaited<ReturnType<typeof layout>>
  await expect(async () => {
    out = await layout(page)
    checkFits(out, width)
  }).toPass({ timeout: 3000 })
  return out
}

function checkFits(out: Awaited<ReturnType<typeof layout>>, width: number) {
  expect(out.overflow, `the bar's content is wider than the bar by ${out.overflow}px at fit ${out.fit}`).toBeLessThanOrEqual(0)
  expect(overlaps(out.boxes), `overlapping boxes at ${width} px, fit ${out.fit}`).toEqual([])
  for (const b of out.boxes) {
    expect(b.x, `${b.id} left of the bar`).toBeGreaterThanOrEqual(out.bar.x - 0.5)
    expect(b.r, `${b.id} right of the bar`).toBeLessThanOrEqual(out.bar.r + 0.5)
    expect(b.y, `${b.id} above the bar`).toBeGreaterThanOrEqual(out.bar.y - 0.5)
    expect(b.b, `${b.id} below the bar`).toBeLessThanOrEqual(out.bar.b + 0.5)
  }
}

for (const [lang, width] of [
  ['en', 1280],
  ['fr', 1280],
  ['en', 1440],
  ['fr', 1440],
  ['en', 1920],
  ['fr', 1920],
] as const)
  test(`the top bar does not overlap at ${width} px in ${lang === 'en' ? 'English' : 'French'}, in its busiest state (#604)`, async ({ page }) => {
    const { errors } = watch(page)
    await open(page, lang, width)

    // The real bar first (armed, long name), then the widest one.
    await assertFits(page, width)
    // 24 px more in the pill: the CPU figure at 100 %, or a longer latency, is wider than the 0 % the stand-in engine reports.
    await widest(page, 24)
    const out = await assertFits(page, width)

    // The project name stays readable (about ten characters), its full text a click or a hover away.
    expect(out.nameWidth, 'the project name keeps room to read').toBeGreaterThanOrEqual(64)
    if (test.info().project.name === 'dev')
      await expect(page.getByTestId('project-menu')).toHaveAttribute('title', /A very long project name that goes on and on/)
    // Never folded: the views, the transport's main controls, the engine pill, the help.
    for (const id of [
      'nav-arrangement',
      'nav-grid',
      'nav-mixer',
      'transport-play',
      'transport-record',
      'transport-tempo',
      'engine-mode',
      'shortcuts-open',
      'project-menu',
    ]) {
      await expect(page.getByTestId(id)).toBeVisible()
    }
    // Nothing needs folding at 1920 px; something does at 1280 px.
    if (width >= 1920) expect(out.fit, 'nothing folded when there is room').toBe(0)
    else if (width === 1280 && test.info().project.name === 'dev') expect(out.fit, 'the bar folds something at 1280 px in the busiest state').toBeGreaterThan(0)

    // Recording: the count-in and the take's target are the live text of the bar, they stay in it and nothing overlaps.
    const record = page.getByTestId('transport-record')
    await record.click()
    await expect(record).toHaveAttribute('aria-pressed', 'true')
    await expect(page.getByTestId('transport-record-target')).toBeVisible()
    await assertFits(page, width)
    await record.click()
    await expect(record).toHaveAttribute('aria-pressed', 'false')

    expect(errors).toEqual([])
  })

test('the focus of a control the bar folds goes to the more button (#604)', async ({ page }) => {
  const { errors } = watch(page)
  await open(page, 'en', 1920)
  await widest(page, 150)
  const select = page.getByTestId('transport-quantize')
  await select.focus()
  await expect(select).toBeFocused()
  await page.setViewportSize({ width: 1280, height: 900 })
  await expect(select).toBeHidden()
  await expect(page.getByTestId('bar-more')).toBeFocused()
  expect(errors).toEqual([])
})

test('a control that stays keeps the focus while the bar unfolds when the room comes back (#604)', async ({ page }) => {
  const { errors } = watch(page)
  await open(page, 'en', 1280)
  await widest(page, 150)
  await expect(page.getByTestId('transport-quantize')).toBeHidden()
  await page.getByTestId('transport-rewind').focus()
  await page.setViewportSize({ width: 1920, height: 900 })
  await expect(page.getByTestId('transport-quantize')).toBeVisible()
  await expect(page.getByTestId('bar-more')).toBeHidden()
  await expect(page.getByTestId('transport-rewind')).toBeFocused()
  expect(errors).toEqual([])
})

test('what the bar folds is in its more menu, and the menu runs the same commands (#604)', async ({ page }) => {
  const { errors } = watch(page)
  await open(page, 'en', 1280)
  await widest(page, 150)
  const more = page.getByTestId('bar-more')
  await expect(more).toBeVisible()
  await expect(page.getByTestId('transport-quantize')).toBeHidden()

  await more.focus()
  await page.keyboard.press('Enter')
  const menu = page.getByTestId('context-menu')
  await expect(menu).toBeVisible()
  await expect(more).toHaveAttribute('aria-expanded', 'true')
  // The octave readout, folded from the bar, has its line here.
  await expect(page.getByTestId('keys-octave-bar')).toBeHidden()
  await expect(page.getByTestId('ctx-more-octave')).toContainText('Oct')
  // 1 bar is the default; picking another one is the command `transport.set_launch_quantize`.
  await expect(page.getByTestId('ctx-more-quantize-bar')).toContainText('✓')
  await page.getByTestId('ctx-more-quantize-bars_4').click()
  await expect(menu).toHaveCount(0)
  await more.focus()
  await page.keyboard.press('Enter')
  await expect(page.getByTestId('ctx-more-quantize-bars_4')).toContainText('✓')
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)
  await expect(more).toBeFocused()

  // Where there is room the select is back and the menu is gone.
  await more.blur()
  await page.setViewportSize({ width: 1920, height: 900 })
  await expect(page.getByTestId('bar-more')).toBeHidden()
  await expect(page.getByTestId('transport-quantize')).toHaveValue('bars_4')

  expect(errors).toEqual([])
})

test('the key and scale button folds before the launch quantization, and the more menu opens its panel (#695)', async ({ page }) => {
  const { errors } = watch(page)
  await open(page, 'en', 1280)
  await widest(page, 150)
  await expect(page.getByTestId('transport-quantize')).toBeHidden()
  await expect(page.getByTestId('transport-scale')).toBeHidden()

  const more = page.getByTestId('bar-more')
  await more.click()
  await expect(page.getByTestId('ctx-more-scale')).toContainText('Key and scale: No scale')
  await page.getByTestId('ctx-more-scale').click()
  await expect(page.getByTestId('scale-panel')).toBeVisible()
  await expect(page.getByTestId('scale-root')).toBeFocused()
  await page.getByTestId('scale-root').selectOption('7')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('scale-panel')).toHaveCount(0)
  await expect(more).toBeFocused()
  await more.click()
  await expect(page.getByTestId('ctx-more-scale')).toContainText('Key and scale: G major')
  await page.keyboard.press('Escape')

  // Where there is room the button is back, with the scale.
  await more.blur()
  await page.setViewportSize({ width: 1920, height: 900 })
  await expect(page.getByTestId('transport-scale')).toHaveText('G')
  expect(errors).toEqual([])
})
