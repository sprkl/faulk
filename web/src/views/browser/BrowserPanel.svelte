<script lang="ts">
  /**
   * The device browser (#190): the registry's instruments, audio effects and MIDI effects, each with its presets
   * (factory, the project's, the user's library), and a live search over their names. An item is dragged onto a track,
   * onto a chain (`DevicesView`) or onto the empty area of the track list (`TrackList`), or inserted from the keyboard
   * (Enter, or a double-click) into the selected track. The search field is a combobox over the list: the arrows move
   * the highlighted row, Enter inserts it, Escape clears the search and then closes the panel. Every outcome is a
   * command (`lib/browser/drop.ts`); nothing here writes the project. A device added to the registry is listed with
   * no change here.
   */
  import { buildCatalog, filterCatalog, rowsOf, type BrowserGroup, type BrowserItem } from '../../lib/browser/catalog'
  import { endDrag, startDrag } from '../../lib/browser/dnd'
  import { putItem } from '../../lib/browser/put'
  import { useStores } from '../../lib/stores'

  let { onclose }: { onclose: () => void } = $props()

  const stores = useStores()
  const { project, ui, dispatcher } = stores
  const t = ui.t

  let query = $state('')
  let highlighted = $state<string | null>(null)
  let search = $state<HTMLInputElement | null>(null)

  // The list comes from the registry and from `preset.list`; it is read again when the presets change (`presetsRev`) and
  // when the project core appears (`project.view`: the presets of the first project come with the core).
  const catalog = $derived.by((): BrowserGroup[] => {
    void project.presetsRev
    void project.view
    return buildCatalog((kind) => dispatcher.listPresetsOfKind(kind)?.presets ?? [])
  })
  const shown = $derived(filterCatalog(catalog, query))
  const rows = $derived(rowsOf(shown))
  const active = $derived(rows.find((row) => row.id === highlighted) ?? rows[0] ?? null)
  const searching = $derived(query.trim() !== '')

  const domId = (id: string) => `fk-browse-${id.replace(/[^A-Za-z0-9_-]/g, '_')}`

  // Asked for (Alt+B, the button): the search field takes the focus, with its text selected to type over.
  $effect(() => {
    if (ui.browserFocus > 0) {
      search?.focus()
      search?.select()
    }
  })

  // The highlighted row stays in view as the arrows walk the list.
  $effect(() => {
    if (active) document.getElementById(domId(active.id))?.scrollIntoView({ block: 'nearest' })
  })

  const said = $derived(ui.browserSaid ? t(ui.browserSaid.key, ui.browserSaid.vars) : '')

  function insert(item: BrowserItem) {
    putItem(stores, item, { type: 'selection' })
  }

  function move(step: 1 | -1) {
    if (rows.length === 0) return
    const at = active ? rows.findIndex((row) => row.id === active.id) : -1
    highlighted = rows[Math.min(rows.length - 1, Math.max(0, at + step))]!.id
  }

  function onSearchKey(event: KeyboardEvent) {
    if (event.ctrlKey || event.metaKey || event.altKey) return
    if (event.key === 'ArrowDown') move(1)
    else if (event.key === 'ArrowUp') move(-1)
    else if (event.key === 'Enter') {
      if (active) insert(active.item)
    } else if (event.key === 'Escape') {
      // The first Escape clears the search, the next one closes the panel.
      if (query !== '') query = ''
      else onclose()
    } else return
    event.preventDefault()
    event.stopPropagation()
  }

  function clear() {
    query = ''
    search?.focus()
  }

  const where = (location: 'factory' | 'project' | 'library') => t(`presets.where.${location}` as const)
</script>

<aside class="browser" aria-labelledby="fk-browser-title" data-testid="browser-panel">
  <div class="head">
    <h2 id="fk-browser-title" class="label">{t('browser.title')}</h2>
    <button type="button" class="ghost" data-testid="browser-close" onclick={onclose}>{t('browser.close')}</button>
  </div>
  <div class="field">
    <input
      type="search"
      bind:this={search}
      bind:value={query}
      role="combobox"
      aria-expanded="true"
      aria-controls="fk-browser-list"
      aria-autocomplete="list"
      aria-activedescendant={active ? domId(active.id) : undefined}
      aria-label={t('browser.search')}
      placeholder={t('browser.placeholder')}
      autocomplete="off"
      spellcheck="false"
      data-browser-search
      data-testid="browser-search"
      oninput={() => (highlighted = null)}
      onkeydown={onSearchKey}
    />
    {#if query !== ''}
      <button type="button" class="clear" aria-label={t('browser.clear')} title={t('browser.clear')} data-testid="browser-clear" onclick={clear}>×</button>
    {/if}
  </div>
  <p class="status" role="status" data-testid="browser-results" data-n={rows.length}>
    {#if !searching}
      {t('browser.hint')}
    {:else if rows.length === 0}
      {t('browser.noResult', { q: query.trim() })}
    {:else}
      {t('browser.results', { n: rows.length })}
    {/if}
  </p>
  <div id="fk-browser-list" class="list" role="listbox" aria-label={t('browser.list')} data-testid="browser-list">
    {#each shown as group (group.category)}
      <div role="group" aria-labelledby="fk-browse-group-{group.category}" data-testid="browser-group" data-category={group.category}>
        <p id="fk-browse-group-{group.category}" class="group label" role="presentation">{t(`browser.group.${group.category}` as const)}</p>
        {#each group.devices as device (device.kind)}
          {@const deviceRow = rows.find((row) => row.id === device.kind)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            id={domId(device.kind)}
            class="row device"
            role="option"
            tabindex="-1"
            aria-selected={active?.id === device.kind}
            draggable="true"
            data-testid="browse-device"
            data-kind={device.kind}
            ondragstart={(event) => deviceRow && startDrag(ui, event, deviceRow.item)}
            ondragend={() => endDrag(ui)}
            onclick={() => (highlighted = device.kind)}
            ondblclick={() => deviceRow && insert(deviceRow.item)}
          >
            {device.name}
          </div>
          {#each device.presets as preset (`${preset.ref.location}|${preset.ref.id}`)}
            {@const id = `${device.kind}|${preset.ref.location}|${preset.ref.id}`}
            {@const presetRow = rows.find((row) => row.id === id)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              id={domId(id)}
              class="row preset"
              role="option"
              tabindex="-1"
              aria-selected={active?.id === id}
              aria-label="{t('browser.presetOption', { preset: preset.name, name: device.name })}, {where(preset.ref.location)}"
              draggable="true"
              data-testid="browse-preset"
              data-kind={device.kind}
              data-location={preset.ref.location}
              ondragstart={(event) => presetRow && startDrag(ui, event, presetRow.item)}
              ondragend={() => endDrag(ui)}
              onclick={() => (highlighted = id)}
              ondblclick={() => presetRow && insert(presetRow.item)}
            >
              <span class="name">{preset.name}</span>
              {#if preset.ref.location !== 'factory'}<span class="where" aria-hidden="true">{where(preset.ref.location)}</span>{/if}
            </div>
          {/each}
        {/each}
      </div>
    {/each}
  </div>
  <p class="said" role="status" data-testid="browser-said">{said}</p>
</aside>

<style>
  .browser {
    grid-area: browser;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-2);
    padding: var(--fk-space-3);
    border-right: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
  }
  .head h2 {
    flex: 1;
    margin: 0;
  }
  button {
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text);
    cursor: pointer;
  }
  button:hover {
    background: var(--fk-surface-2);
  }
  .ghost {
    min-height: var(--fk-row);
    padding: 0 var(--fk-space-3);
    font-size: 12px;
  }
  .field {
    position: relative;
  }
  input {
    width: 100%;
    box-sizing: border-box;
    min-height: var(--fk-control);
    padding: 0 var(--fk-space-6) 0 var(--fk-space-3);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
    background: var(--fk-bg);
    color: var(--fk-text);
  }
  input::-webkit-search-cancel-button {
    display: none;
  }
  .clear {
    position: absolute;
    top: 50%;
    right: var(--fk-space-1);
    transform: translateY(-50%);
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    line-height: 1;
  }
  .status,
  .said {
    margin: 0;
    color: var(--fk-text-muted);
    font-size: 12px;
  }
  .said {
    min-height: 1.4em;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-2);
  }
  .group {
    margin: var(--fk-space-2) 0 var(--fk-space-1);
    padding: 0 var(--fk-space-3);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    min-height: var(--fk-row);
    padding: 0 var(--fk-space-3);
    border-radius: var(--fk-radius-pill);
    cursor: grab;
    user-select: none;
  }
  .row:hover {
    background: var(--fk-surface-2);
  }
  .row[aria-selected='true'] {
    background: var(--fk-accent-tint);
    color: var(--fk-accent-fg);
  }
  .device {
    font-weight: 600;
  }
  .preset {
    padding-left: var(--fk-space-6);
    font-weight: 400;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .where {
    flex: none;
    font-size: 11px;
    color: var(--fk-text-muted);
  }
  @media (max-width: 760px) {
    .browser {
      border-right: 0;
      border-top: 1px solid var(--fk-line);
    }
  }
</style>
