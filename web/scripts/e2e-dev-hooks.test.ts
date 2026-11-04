import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { checkSpec, describeFinding, devHookNames } from './e2e-dev-hooks'

const root = join(__dirname, '..')
const hooks = devHookNames(readFileSync(join(root, 'src/lib/stores/index.ts'), 'utf8'))
const check = (src: string) => checkSpec('x.spec.ts', src, hooks)

describe('the dev hooks of the page (#626)', () => {
  it('are read from the dev block of the stores', () => {
    expect(hooks).toEqual(expect.arrayContaining(['__faulkRun', '__faulkView', '__faulkHistory', '__faulkOpen', '__faulkWrite']))
    // The flags a spec sets for the page to read are not hooks.
    expect(hooks).not.toContain('__faulkNoBrowserStorage')
    expect(hooks).not.toContain('__faulkStoreInPage')
  })
})

describe('the guard on the specs (#626)', () => {
  const run = `await page.evaluate(() => (window as unknown as { __faulkRun: (c: unknown) => void }).__faulkRun({}))`

  it('names the file and line of a test that calls a hook without a skip', () => {
    const found = check(`test('adds a track', async ({ page }) => {\n  await engineUp(page)\n  ${run}\n})\n`)
    expect(found).toHaveLength(1)
    expect(found[0]).toMatchObject({ file: 'x.spec.ts', line: 3, hook: '__faulkRun', test: 'adds a track' })
    expect(describeFinding(found[0])).toContain('x.spec.ts:3')
  })

  it('accepts the shared helper, a hook probe or a project test, before the first use', () => {
    expect(check(`test('a', async ({ page }) => {\n  await skipWithoutDevHooks(page)\n  ${run}\n})\n`)).toEqual([])
    expect(
      check(
        `test('a', async ({ page }) => {\n  test.skip((await page.evaluate(() => typeof (window as unknown as { __faulkView?: unknown }).__faulkView)) !== 'function', 'dev only')\n  ${run}\n})\n`,
      ),
    ).toEqual([])
    expect(check(`test('a', async ({ page }, info) => {\n  test.skip(info.project.name !== 'dev', 'dev only')\n  ${run}\n})\n`)).toEqual([])
    expect(check(`test('a', async ({ page }) => {\n  test.skip(test.info().project.name !== 'dev', 'dev only')\n  ${run}\n})\n`)).toEqual([])
  })

  it('refuses a skip that comes after the first use', () => {
    const found = check(`test('a', async ({ page }) => {\n  ${run}\n  await skipWithoutDevHooks(page)\n})\n`)
    expect(found).toHaveLength(1)
    expect(found[0].why).toContain('before the skip at line 3')
  })

  it('refuses a skip that tests neither the hooks nor the project', () => {
    expect(check(`test('a', async ({ page }) => {\n  test.skip(SOAK_MIN <= 0, 'off')\n  ${run}\n})\n`)).toHaveLength(1)
  })

  it('does not take a presence probe for a use', () => {
    const probe = `const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)\ntest('a', async ({ page }) => {\n  await page.goto('/')\n})\n`
    expect(check(probe)).toEqual([])
  })

  it('ignores a hook named in a comment', () => {
    expect(check(`test('a', async ({ page }) => {\n  // __faulkRun would add a track here\n  await page.goto('/')\n})\n`)).toEqual([])
  })

  it('counts a top-level helper that calls a hook as a use by the tests that name it', () => {
    const spec = `async function addTrack(page: Page) {\n  ${run}\n}\ntest('a', async ({ page }) => {\n  await addTrack(page)\n})\ntest('b', async ({ page }) => {\n  await page.goto('/')\n})\n`
    expect(check(spec).map((f) => f.test)).toEqual(['a'])
    expect(check(spec.replace('await addTrack(page)', 'await skipWithoutDevHooks(page)\n  await addTrack(page)'))).toEqual([])
  })

  it('takes a guard outside the test bodies for a guard of the whole file', () => {
    expect(
      check(
        `test.skip(({ browserName }) => Boolean(browserName) && test.info().project.name !== 'dev', 'dev only')\ntest('a', async ({ page }) => {\n  ${run}\n})\n`,
      ),
    ).toEqual([])
  })

  it('catches a hook used in a beforeEach that no skip precedes', () => {
    expect(check(`test.beforeEach(async ({ page }) => {\n  ${run}\n})\ntest('a', async ({ page }) => {\n  await page.goto('/')\n})\n`)).toHaveLength(1)
  })

  it('catches a real spec once its skips are taken out', () => {
    const file = 'tests/e2e/markers.spec.ts'
    const source = readFileSync(join(root, file), 'utf8')
    expect(checkSpec(file, source, hooks)).toEqual([])
    const stripped = source.replace(/^ *test\.skip\(\(await hooks\(page\)\) !== 'function'.*$/gm, '')
    expect(stripped).not.toBe(source)
    const found = checkSpec(file, stripped, hooks)
    expect(found.length).toBeGreaterThan(0)
    expect(found.every((f) => f.file === file && f.line > 0)).toBe(true)
  })

  it('holds for every spec of tests/e2e', () => {
    const dir = join(root, 'tests/e2e')
    const specs = readdirSync(dir).filter((f) => f.endsWith('.spec.ts'))
    expect(specs.length).toBeGreaterThan(50)
    const found = specs.flatMap((f) => checkSpec(`tests/e2e/${f}`, readFileSync(join(dir, f), 'utf8'), hooks))
    // Fix: call `await skipWithoutDevHooks(page)` (tests/e2e/support.ts) before the first use of a hook.
    expect(found.map(describeFinding)).toEqual([])
  })
})
