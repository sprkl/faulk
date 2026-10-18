import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// UI-2 (#106): the track list and its headers in a real browser. Real worklet, engine and project
// core: every action below goes through the dispatcher as a command, and the headers show what the
// project store mirrors back. Screenshots go to the folder named by FAULK_E2E_SHOTS, when set.

const TEXTS = {
  en: {
    track: (n: number) => `Track ${n}`,
    mute: (name: string) => `Mute ${name}`,
    solo: (name: string) => `Solo ${name}`,
    volume: (name: string) => `Volume of ${name}`,
    pan: (name: string) => `Pan of ${name}`,
    remove: (name: string) => `Delete ${name}`,
    undo: 'Undo',
    rename: 'Track name',
    midi: 'MIDI',
    audio: 'Audio',
  },
  fr: {
    track: (n: number) => `Piste ${n}`,
    mute: (name: string) => `Couper ${name}`,
    solo: (name: string) => `Solo ${name}`,
    volume: (name: string) => `Volume de ${name}`,
    pan: (name: string) => `Panoramique de ${name}`,
    remove: (name: string) => `Supprimer ${name}`,
    undo: 'Annuler',
    rename: 'Nom de la piste',
    midi: 'MIDI',
    audio: 'Audio',
  },
} as const

const header = (page: Page, name: string) => page.getByTestId('track-header').filter({ has: page.getByRole('button', { name, exact: true }) })
const names = (page: Page) => page.getByTestId('track-name').allInnerTexts()

/** Nothing of the shell or of the track list sticks out sideways, whatever the number of tracks. */
async function horizontalOverflow(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const bad: string[] = []
    for (const selector of ['header.top', '[data-testid=track-list]', '[data-testid=view]', '[data-testid=bottom-panel]', '[data-testid=track-header]']) {
      for (const el of document.querySelectorAll(selector)) {
        if (el.scrollWidth > el.clientWidth + 1) bad.push(`${selector}: scrollWidth ${el.scrollWidth} > ${el.clientWidth}`)
      }
    }
    if (document.documentElement.scrollWidth > innerWidth) bad.push('page: scrolls sideways')
    return bad
  })
}

async function addTrack(page: Page, item: string) {
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}

for (const lang of ['en', 'fr'] as const) {
  for (const width of [1280, 1920]) {
    test(`tracks, ${lang}, ${width} px: create, rename, mute, solo, mix, select, delete and undo`, async ({ page }) => {
      const x = TEXTS[lang]
      const { errors } = watch(page)
      await page.setViewportSize({ width, height: 900 })
      await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
      await page.goto('/app')
      await expect(page.locator('html')).toHaveAttribute('lang', lang)

      // Before the audio starts there is no project to list.
      await expect(page.getByTestId('tracks-waiting')).toBeVisible()
      await expect(page.getByTestId('track-header')).toHaveCount(0)
      await page.getByTestId('engine-state').getByRole('button').click()
      await expect(page.getByTestId('engine-mode')).toBeVisible()

      // The project starts with one oscillator track, named and selected; tracks created without a name show a translated default.
      await expect(page.getByTestId('track-header')).toHaveCount(1)
      await expect(header(page, 'Synth')).toContainText(`${x.midi} · Oscillator`)
      await expect(header(page, 'Synth')).toHaveAttribute('aria-current', 'true')

      // Each instrument, and an audio track; the new track is selected.
      await addTrack(page, 'Drums')
      await expect(header(page, x.track(2))).toContainText(`${x.midi} · Drums`)
      await expect(header(page, x.track(2))).toHaveAttribute('aria-current', 'true')
      await addTrack(page, 'Oscillator')
      await expect(header(page, x.track(3))).toContainText(`${x.midi} · Oscillator`)
      await addTrack(page, lang === 'en' ? 'Audio track' : 'Piste audio')
      await expect(header(page, x.track(4))).toContainText(x.audio)
      await expect(page.getByTestId('track-header')).toHaveCount(4)
      await expect(page.getByTestId('stream-error')).toHaveCount(0)
      await expect(page.locator('[data-testid=track-header][aria-current=true]')).toHaveCount(1)

      // Rename from the keyboard: F2 on the name, type, Enter; the keyboard stays on the name.
      await header(page, x.track(2)).getByTestId('track-name').focus()
      await page.keyboard.press('F2')
      await expect(page.getByRole('textbox', { name: x.rename, exact: true })).toBeFocused()
      await page.keyboard.type('Kit')
      await page.keyboard.press('Enter')
      expect(await names(page)).toEqual(['Synth', 'Kit', x.track(3), x.track(4)])
      await expect(header(page, 'Kit').getByTestId('track-name')).toBeFocused()
      // Escape gives the edit up.
      await page.keyboard.press('F2')
      await page.keyboard.type(' and more')
      await page.keyboard.press('Escape')
      expect(await names(page)).toEqual(['Synth', 'Kit', x.track(3), x.track(4)])

      // Mute is in the project, solo in the session; solo adds up.
      const kit = header(page, 'Kit')
      await kit.getByRole('button', { name: x.mute('Kit'), exact: true }).click()
      await expect(kit.getByRole('button', { name: x.mute('Kit'), exact: true })).toHaveAttribute('aria-pressed', 'true')
      await kit.getByRole('button', { name: x.solo('Kit'), exact: true }).click()
      await header(page, x.track(3)).getByRole('button', { name: x.solo(x.track(3)), exact: true }).click()
      await expect(page.locator('.solo[aria-pressed=true]')).toHaveCount(2)

      // Volume and pan: a value, and a drag that one undo takes back whole.
      const volume = kit.getByRole('slider', { name: x.volume('Kit'), exact: true })
      await volume.fill('-12')
      await expect(volume).toHaveAttribute('aria-valuetext', '−12.0 dB')
      await volume.fill('-96')
      await expect(volume).toHaveAttribute('aria-valuetext', '−∞ dB')
      const pan = kit.getByRole('slider', { name: x.pan('Kit'), exact: true })
      await pan.fill('0.5')
      await expect(pan).toHaveAttribute('aria-valuetext', 'R50')
      await volume.dblclick()
      await expect(volume).toHaveAttribute('aria-valuetext', '0.0 dB')
      const box = (await volume.boundingBox())!
      await page.mouse.move(box.x + box.width * 0.9, box.y + box.height / 2)
      await page.mouse.down()
      await page.mouse.move(box.x + box.width * 0.5, box.y + box.height / 2, { steps: 6 })
      await page.mouse.move(box.x + box.width * 0.2, box.y + box.height / 2, { steps: 6 })
      await page.mouse.up()
      await expect(volume).not.toHaveAttribute('aria-valuetext', '0.0 dB')
      await page.keyboard.press('Control+z')
      await expect(volume).toHaveAttribute('aria-valuetext', '0.0 dB')

      // Select, and the computer keyboard plays the selected track.
      await header(page, 'Synth').getByTestId('track-name').click()
      await expect(header(page, 'Synth')).toHaveAttribute('aria-current', 'true')
      await expect(page.getByTestId('keys-target')).toContainText('Synth')
      await page.keyboard.down('KeyA')
      await expect(page.getByTestId('voices')).toContainText(/\b1\b/)
      await page.keyboard.up('KeyA')
      await expect(page.getByTestId('voices')).toContainText(/\b0\b/)
      // A created track sounds too: the oscillator, then the drums.
      await header(page, x.track(3)).getByTestId('track-name').click()
      await page.keyboard.down('KeyA')
      await expect(page.getByTestId('voices')).toContainText(/\b1\b/)
      await page.keyboard.up('KeyA')
      await expect(page.getByTestId('voices')).toContainText(/\b0\b/)
      // The drums answer on C2 (the kick): two octaves down with Z.
      await header(page, 'Kit').getByTestId('track-name').click()
      await page.keyboard.press('KeyZ')
      await page.keyboard.press('KeyZ')
      await page.keyboard.down('KeyA')
      await expect(page.getByTestId('voices')).toContainText(/\b[1-9]/)
      await page.keyboard.up('KeyA')
      await page.keyboard.press('KeyX')
      await page.keyboard.press('KeyX')
      // A held note survives a change elsewhere in the project, and a click on a control of another
      // track neither selects it nor moves the keyboard.
      await header(page, x.track(3)).getByTestId('track-name').click()
      await page.keyboard.down('KeyA')
      await expect(page.getByTestId('voices')).toContainText(/\b1\b/)
      await header(page, 'Kit').getByRole('slider', { name: x.pan('Kit'), exact: true }).fill('0.2')
      await header(page, 'Kit').getByRole('button', { name: x.solo('Kit'), exact: true }).click()
      await header(page, 'Kit').getByRole('button', { name: x.solo('Kit'), exact: true }).click()
      await expect(header(page, x.track(3))).toHaveAttribute('aria-current', 'true')
      await expect(header(page, 'Kit')).not.toHaveAttribute('aria-current', 'true')
      await expect(page.getByTestId('voices')).toContainText(/\b1\b/)
      await page.keyboard.up('KeyA')
      // An audio track is not playable yet: its keys play nothing.
      await header(page, x.track(4)).getByTestId('track-name').click()
      await page.keyboard.down('KeyA')
      await page.waitForTimeout(300)
      await expect(page.getByTestId('voices')).toContainText(/\b0\b/)
      await page.keyboard.up('KeyA')

      expect(await horizontalOverflow(page)).toEqual([])
      const shots = process.env.FAULK_E2E_SHOTS
      if (shots) {
        mkdirSync(shots, { recursive: true })
        await page.screenshot({ path: `${shots}/ui-2-${lang}-${width}.png` })
      }

      // Delete, then undo: the track comes back with its name and mute, in place.
      await kit.getByRole('button', { name: x.remove('Kit'), exact: true }).click()
      expect(await names(page)).toEqual(['Synth', x.track(2), x.track(3)])
      await expect(page.locator('.solo[aria-pressed=true]')).toHaveCount(1)
      await page.getByRole('button', { name: x.undo, exact: true }).click()
      expect(await names(page)).toEqual(['Synth', 'Kit', x.track(3), x.track(4)])
      await expect(header(page, 'Kit').getByRole('button', { name: x.mute('Kit'), exact: true })).toHaveAttribute('aria-pressed', 'true')
      // Deleting every track leaves an empty list that says so; undo brings the last one back.
      for (let n = 4; n > 0; n--) await page.getByRole('button', { name: /^(Delete|Supprimer) /, exact: false }).first().click()
      await expect(page.getByTestId('tracks-empty')).toBeVisible()
      await expect(page.getByTestId('keys-target')).toBeVisible()
      await page.keyboard.press('Control+z')
      await expect(page.getByTestId('track-header')).toHaveCount(1)

      expect(await horizontalOverflow(page)).toEqual([])
      expect(errors).toEqual([])
    })
  }
}
