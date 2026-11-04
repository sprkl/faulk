import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #190: the device browser in a real browser. Alt+B opens it with the focus in its search field; the list holds the
// registry's devices with their presets; the search finds by device and preset name (case, accents, nothing, cleared);
// Enter and a drag put a device or a preset on a track, at a place of its chain, or on the empty area of the track list
// (a new MIDI track, one undo); audio files there start an audio track and are imported into it (AU-4); no console error.

const TEXTS = {
  en: {
    toggle: 'Browser',
    search: 'Search devices and presets',
    clear: 'Clear the search',
    instruments: 'Instruments',
    effects: 'Audio effects',
    none: 'No device or preset matches "zzzz".',
    drop: 'Drop an instrument or a preset here to start a MIDI track, or audio files to start an audio track.',
    audio: 'A new audio track takes the files: its Clips panel shows the import.',
    inserted: 'Reverb inserted in',
    undo: 'Undo',
  },
  fr: {
    toggle: 'Navigateur',
    search: 'Chercher un device ou un preset',
    clear: 'Effacer la recherche',
    instruments: 'Instruments',
    effects: 'Effets audio',
    none: 'Aucun device ni preset ne correspond à « zzzz ».',
    drop: 'Dépose un instrument ou un preset ici pour créer une piste MIDI, ou des fichiers audio pour une piste audio.',
    audio: 'Une nouvelle piste audio reçoit les fichiers : son panneau Clips montre l’import.',
    inserted: 'Reverb inséré dans',
    undo: 'Annuler',
  },
} as const

async function start(page: Page, lang: 'en' | 'fr') {
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

const kindsOfChain = (page: Page) => page.getByTestId('device-card').evaluateAll((cards) => cards.map((c) => c.getAttribute('data-kind')))

for (const lang of ['en', 'fr'] as const) {
  test(`device browser, ${lang}: open by key, list, search, insert by key and by drag, new track, audio file`, async ({ page }) => {
    const x = TEXTS[lang]
    const { errors } = watch(page)
    await start(page, lang)
    const panel = page.getByTestId('browser-panel')
    const search = page.getByTestId('browser-search')

    // Closed at first; Alt+B opens it with the focus in the search field, and closes it from there.
    await expect(panel).toHaveCount(0)
    await page.keyboard.press('Alt+KeyB')
    await expect(panel).toBeVisible()
    await expect(search).toBeFocused()
    await expect(search).toHaveAccessibleName(x.search)
    await page.keyboard.press('Alt+KeyB')
    await expect(panel).toHaveCount(0)
    await expect(page.getByTestId('browser-toggle')).toBeFocused()
    await page.getByRole('button', { name: x.toggle }).click()
    await expect(search).toBeFocused()

    // The list is the registry's: every device, grouped, with its presets under it.
    const groups = panel.getByTestId('browser-group')
    await expect(groups.first()).toContainText(x.instruments)
    await expect(groups.nth(1)).toContainText(x.effects)
    const kinds = await panel.getByTestId('browse-device').evaluateAll((rows) => rows.map((r) => r.getAttribute('data-kind')))
    expect(kinds).toEqual(['faulk.osc_synth', 'faulk.drums', 'faulk.echo', 'faulk.reverb', 'faulk.compressor', 'faulk.filter'])
    await expect(panel.locator('[data-testid="browse-preset"][data-kind="faulk.osc_synth"]').first()).toBeVisible()
    await expect(panel.getByRole('option', { name: /Round Bass/ })).toHaveCount(1)

    // Search: case, accents, a preset name, nothing, cleared.
    await search.fill('REV')
    await expect(panel.getByTestId('browse-device')).toHaveText(['Reverb'])
    await search.fill('Écho')
    await expect(panel.getByTestId('browse-device')).toHaveText(['Echo'])
    await search.fill('large hall')
    await expect(panel.getByTestId('browse-device')).toHaveText(['Reverb'])
    await expect(panel.getByTestId('browse-preset')).toHaveCount(1)
    await search.fill('zzzz')
    await expect(panel.getByTestId('browse-device')).toHaveCount(0)
    await expect(panel.getByTestId('browser-results')).toHaveText(x.none)
    await page.getByRole('button', { name: x.clear }).click()
    await expect(search).toHaveValue('')
    await expect(panel.getByTestId('browse-device')).toHaveCount(6)
    // Escape clears the search, the next one closes the panel.
    await search.fill('echo')
    await page.keyboard.press('Escape')
    await expect(search).toHaveValue('')
    await expect(panel).toBeVisible()

    // Enter inserts the highlighted device into the selected track: an effect at the end of its chain.
    await expect(page.getByTestId('device-card')).toHaveCount(1)
    await search.fill('reverb')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('device-card')).toHaveCount(2)
    expect(await kindsOfChain(page)).toEqual(['faulk.osc_synth', 'faulk.reverb'])
    await expect(panel.getByTestId('browser-said')).toContainText(x.inserted)
    // The arrows move the highlighted row; a preset of an effect loads on the device it just made.
    await search.fill('reverb')
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('device-card')).toHaveCount(3)
    await expect(page.getByTestId('preset-name').nth(2)).toHaveAttribute('data-location', 'factory')
    // One undo takes the insertion and its preset back together.
    await page.getByTestId('track-list').getByRole('button', { name: x.undo }).click()
    await expect(page.getByTestId('device-card')).toHaveCount(2)
    await search.fill('')

    // A drag onto the chain puts the effect at the dropped place: dropped on the first half of the Reverb's card, before it.
    const reverbCard = page.locator('[data-testid="device-card"][data-kind="faulk.reverb"]')
    await panel.locator('[data-testid="browse-device"][data-kind="faulk.echo"]').dragTo(reverbCard, { targetPosition: { x: 5, y: 20 } })
    await expect(page.getByTestId('device-card')).toHaveCount(3)
    expect(await kindsOfChain(page)).toEqual(['faulk.osc_synth', 'faulk.echo', 'faulk.reverb'])
    // Dropped on the second half of the last card: after it, at the end.
    await reverbCard.scrollIntoViewIfNeeded()
    const box = (await reverbCard.boundingBox())!
    await panel.locator('[data-testid="browse-device"][data-kind="faulk.filter"]').dragTo(reverbCard, { targetPosition: { x: box.width - 5, y: 20 } })
    await expect(page.getByTestId('device-card')).toHaveCount(4)
    expect(await kindsOfChain(page)).toEqual(['faulk.osc_synth', 'faulk.echo', 'faulk.reverb', 'faulk.filter'])

    // A preset of an instrument on the empty area starts a MIDI track with that instrument and preset: one undo.
    const list = page.getByTestId('track-list')
    await expect(list).toHaveAttribute('data-track-count', '1')
    await expect(list.getByTestId('track-drop')).toContainText(x.drop)
    await panel.locator('[data-testid="browse-preset"][data-kind="faulk.drums"]').first().dragTo(list.getByTestId('track-drop'))
    await expect(list).toHaveAttribute('data-track-count', '2')
    const selectedChain = page.getByTestId('device-card')
    await expect(selectedChain).toHaveCount(1)
    await expect(selectedChain.first()).toHaveAttribute('data-kind', 'faulk.drums')
    await expect(page.getByTestId('preset-name').first()).toHaveAttribute('data-location', 'factory')
    await expect(page.getByTestId('preset-name').first()).not.toHaveText(/^(No preset|Aucun preset)$/)
    await list.getByRole('button', { name: x.undo }).click()
    await expect(list).toHaveAttribute('data-track-count', '1')

    // An effect is not taken by the empty area: nothing changes.
    await panel.locator('[data-testid="browse-device"][data-kind="faulk.compressor"]').dragTo(list.getByTestId('track-drop'), { force: true })
    await expect(list).toHaveAttribute('data-track-count', '1')

    // An audio file there starts an audio track and goes to its import (AU-4); this one is no real audio, so the import
    // refuses it in the Clips panel and the track stays empty. Undo takes the track away.
    await list.getByTestId('track-drop').evaluate((node) => {
      const data = new DataTransfer()
      data.items.add(new File(['x'], 'a.wav', { type: 'audio/wav' }))
      for (const type of ['dragenter', 'dragover', 'drop']) node.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: data }))
    })
    await expect(list.getByTestId('track-drop-note')).toHaveText(x.audio)
    await expect(list).toHaveAttribute('data-track-count', '2')
    await expect(page.getByTestId('audio-import-failure')).toBeVisible()
    await list.getByRole('button', { name: x.undo }).click()
    await expect(list).toHaveAttribute('data-track-count', '1')
    // The drop showed the Clips panel: back to the devices.
    await page.getByTestId('panel-devices').click()

    // A drag onto a track's row in the arrangement (Alt+1; the page opens on the launch grid) puts the device in that track's chain.
    await page.getByTestId('browser-toggle').focus()
    await page.keyboard.press('Alt+Digit1')
    await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'arrangement')
    await panel.locator('[data-testid="browse-device"][data-kind="faulk.compressor"]').dragTo(page.getByTestId('track-header').first())
    await expect(page.getByTestId('device-card')).toHaveCount(5)
    expect(await kindsOfChain(page)).toContain('faulk.compressor')

    expect(errors).toEqual([])
  })
}
