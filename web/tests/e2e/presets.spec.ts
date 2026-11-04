import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #191: the presets of a device in a real browser. The header of the Oscillator's card steps through
// its presets (one undo takes a load back), marks a turned knob as modified, saves into the
// project and into the library; the library is in the browser's storage and comes back after a reload.

const TEXTS = {
  en: {
    none: 'No preset',
    saveProject: 'Save as a preset of this project…',
    rename: 'Rename this preset…',
    remove: 'Delete this preset',
    renameAction: 'Rename',
    next: 'Next preset of Oscillator',
    prev: 'Previous preset of Oscillator',
    menu: 'Presets of Oscillator',
    saveLibrary: 'Save in my library…',
    name: 'Name of the preset',
    save: 'Save',
  },
  fr: {
    none: 'Aucun preset',
    saveProject: 'Enregistrer comme preset de ce projet…',
    rename: 'Renommer ce preset…',
    remove: 'Supprimer ce preset',
    renameAction: 'Renommer',
    next: 'Preset suivant de Oscillator',
    prev: 'Preset précédent de Oscillator',
    menu: 'Presets de Oscillator',
    saveLibrary: 'Enregistrer dans ma bibliothèque…',
    name: 'Nom du preset',
    save: 'Enregistrer',
  },
} as const

async function start(page: Page, lang: 'en' | 'fr') {
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

for (const lang of ['en', 'fr'] as const) {
  test(`presets, ${lang}: step through, modified, undo, save to the library and find it after a reload`, async ({ page }) => {
    const x = TEXTS[lang]
    const { errors } = watch(page)
    await start(page, lang)
    const bar = page.getByTestId('preset-bar').first()
    await expect(bar.getByTestId('preset-name')).toHaveText(x.none)

    // Next: the first factory preset of the synth (by name: Glass Pluck), its values in the card.
    await bar.getByRole('button', { name: x.next }).click()
    await expect(bar.getByTestId('preset-name')).toHaveText('Glass Pluck')
    await expect(bar.getByTestId('preset-name')).toHaveAttribute('data-location', 'factory')
    await expect(bar.getByTestId('preset-modified')).toHaveCount(0)
    await bar.getByRole('button', { name: x.next }).click()
    await expect(bar.getByTestId('preset-name')).toHaveText('Round Bass')
    // One undo takes the whole load back.
    await page.keyboard.press('Control+z')
    await expect(bar.getByTestId('preset-name')).toHaveText('Glass Pluck')

    // A knob turned: modified.
    const card = page.getByTestId('device-card').first()
    await card.getByTestId('param-sustain').getByRole('slider').focus()
    await page.keyboard.press('ArrowUp')
    await expect(bar.getByTestId('preset-modified')).toBeVisible()

    // Save in the library under a name of its own.
    await bar.getByRole('button', { name: x.menu }).click()
    await page.getByRole('menuitem', { name: x.saveLibrary }).click()
    const field = page.getByRole('textbox', { name: x.name })
    // Typed key by key: a text field keeps the keys, no note is played.
    await field.fill('')
    await field.pressSequentially('My Pluck')
    await expect(field).toHaveValue('My Pluck')
    await page.getByTestId('preset-form').getByRole('button', { name: x.save, exact: true }).click()
    await expect(bar.getByTestId('preset-name')).toHaveText('My Pluck')
    await expect(bar.getByTestId('preset-name')).toHaveAttribute('data-location', 'library')
    await expect(bar.getByTestId('preset-modified')).toHaveCount(0)

    // The same settings saved in the project, renamed there, then deleted: the device keeps the name it knew.
    await bar.getByRole('button', { name: x.menu }).click()
    await page.getByRole('menuitem', { name: x.saveProject }).click()
    await field.fill('In Song')
    await page.getByTestId('preset-form').getByRole('button', { name: x.save, exact: true }).click()
    await expect(bar.getByTestId('preset-name')).toHaveAttribute('data-location', 'project')
    await bar.getByRole('button', { name: x.menu }).click()
    await page.getByRole('menuitem', { name: x.rename }).click()
    await field.fill('In Song 2')
    await page.getByTestId('preset-form').getByRole('button', { name: x.renameAction, exact: true }).click()
    await expect(bar.getByTestId('preset-name')).toHaveText('In Song 2')
    await bar.getByRole('button', { name: x.menu }).click()
    await page.getByRole('menuitem', { name: x.remove }).click()
    // The device keeps the name it knew, and the preset is gone: nothing to rename or delete any more.
    await expect(bar.getByTestId('preset-name')).toHaveText('In Song')
    await bar.getByRole('button', { name: x.menu }).click()
    await expect(page.getByRole('menuitem', { name: x.rename })).toBeDisabled()
    await expect(page.getByRole('menuitem', { name: x.remove })).toBeDisabled()
    await page.keyboard.press('Escape')

    // The library file is in the browser's storage, in the preset format.
    await expect
      .poll(() =>
        page.evaluate(async () => {
          let dir = await navigator.storage.getDirectory()
          for (const part of ['faulk', 'presets']) dir = await dir.getDirectoryHandle(part)
          const texts: string[] = []
          for await (const entry of (dir as unknown as { values(): AsyncIterable<FileSystemFileHandle> }).values())
            texts.push(await (await entry.getFile()).text())
          return texts.find((t) => t.includes('"My Pluck"')) ?? null
        }),
      )
      .toContain('"format": 1')

    // After a reload (a new project in the page), the library still has it.
    await page.reload()
    await page.getByTestId('engine-state').getByRole('button').click()
    await engineUp(page)
    // Through the panel alone (no page hook: the same in the production build): stepping through the
    // synth's presets meets it, in the library.
    const after = page.getByTestId('preset-bar').first()
    const seen: string[] = []
    for (let i = 0; i < 12 && !seen.includes('My Pluck'); i++) {
      await after.getByRole('button', { name: x.next }).click()
      seen.push((await after.getByTestId('preset-name').textContent()) ?? '')
    }
    expect(seen).toContain('My Pluck')
    await expect(after.getByTestId('preset-name')).toHaveAttribute('data-location', 'library')
    expect(errors).toEqual([])
  })
}
