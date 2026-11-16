<script lang="ts">
  /**
   * The command palette (#108), opened by Ctrl or Cmd + K in every view: it lists every command of the catalogue (`edit`,
   * `live`, `setting`), finds them by what is typed and runs the one chosen through the dispatcher, like any other action.
   * A command that takes input asks for it in a form (the fields of its schema; the selected track fills a track field).
   * A native modal dialog like the shortcut list: the page behind is inert, the focus stays inside, Escape closes it and the
   * focus goes back to what opened it.
   */
  import type { CatalogEntry, CommandField } from '../../lib/generated/protocol'
  import { buildCommand, initialText, MAX_RESULTS, paletteEntries, searchCommands, type FieldProblem, type PaletteEntry } from '../../lib/command-palette'
  import { commandShortcutText } from '../../lib/shortcuts'
  import { useStores } from '../../lib/stores'

  const { engine, project, dispatcher, ui } = useStores()
  const t = ui.t

  let dialog = $state<HTMLDialogElement | null>(null)
  let search = $state<HTMLInputElement | null>(null)
  let list = $state<HTMLElement | null>(null)
  let query = $state('')
  let active = $state(0)
  /** The command being filled in, or `null` while the list is shown. */
  let chosen = $state<PaletteEntry | null>(null)
  let texts = $state<Record<string, string>>({})
  let problems = $state<Record<string, FieldProblem>>({})
  let refused = $state(false)

  /** The keys that run a command, as the platform and the keyboard layout write them (empty when none does). */
  const keysOf = (name: string) => commandShortcutText(name, undefined, ui.keyLayout)

  const entries = $derived(paletteEntries(t))
  const found = $derived(searchCommands(entries, query))
  const shown = $derived(found.slice(0, MAX_RESULTS))
  const selectedTrack = $derived(project.selected?.id ?? null)
  /** What to say when a command did not run: the audio is not started (the project loads with it), or the project refused. */
  const refusedText = $derived(engine.started && project.view !== null ? t('palette.refused') : t('palette.notReady'))

  // The store says whether it is open; the element follows (and tells the store when the browser closed it: Escape).
  $effect(() => {
    if (!dialog) return
    if (ui.paletteOpen && !dialog.open) {
      query = ''
      active = 0
      chosen = null
      refused = false
      dialog.showModal()
      search?.focus()
    } else if (!ui.paletteOpen && dialog.open) dialog.close()
  })

  // A new query starts at its best result.
  $effect(() => {
    void query
    active = 0
    refused = false
  })

  // The active result stays in view as the arrows move through a long list.
  $effect(() => {
    if (chosen) return
    list?.querySelector<HTMLElement>(`#palette-option-${active}`)?.scrollIntoView({ block: 'nearest' })
  })

  function choose(item: PaletteEntry) {
    if (item.entry.fields.length === 0) {
      // Nothing to ask: it runs now. When it did not run, the palette stays open and says why.
      run(item.entry, {})
      return
    }
    texts = Object.fromEntries(item.entry.fields.map((f) => [f.name, initialText(f, { selectedTrack })]))
    problems = {}
    refused = false
    chosen = item
    // The first field takes the focus once the form is drawn.
    queueMicrotask(() => dialog?.querySelector<HTMLElement>('form [data-field]')?.focus())
  }

  function run(entry: CatalogEntry, given: Record<string, string>): boolean {
    const built = buildCommand(entry, given)
    if (!built.ok) {
      problems = built.problems
      return false
    }
    // The one path of every action: the dispatcher. A refusal of the project is told by its own toast, as for any caller;
    // a command that cannot run yet (no audio, so no project) gives no toast, so the palette says it.
    if (dispatcher.run(built.command)) {
      ui.paletteOpen = false
      return true
    }
    refused = true
    return false
  }

  function submit(event: SubmitEvent) {
    event.preventDefault()
    if (chosen) run(chosen.entry, texts)
  }

  function back() {
    chosen = null
    queueMicrotask(() => search?.focus())
  }

  function onSearchKey(event: KeyboardEvent) {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      active = Math.min(active + 1, shown.length - 1)
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      active = Math.max(active - 1, 0)
    } else if (event.key === 'Enter') {
      event.preventDefault()
      const item = shown[active]
      if (item) choose(item)
    }
  }

  /** A click on the backdrop (the dialog element itself, outside its box) closes it. */
  function onBackdrop(event: MouseEvent) {
    if (event.target === dialog) ui.paletteOpen = false
  }

  function kindWords(field: CommandField): string {
    return t(`palette.kind.${field.kind}` as const)
  }
</script>

<!-- svelte-ignore a11y_no_redundant_roles -->
<dialog
  bind:this={dialog}
  class="palette-dialog"
  role="dialog"
  aria-modal={ui.paletteOpen ? 'true' : undefined}
  aria-labelledby="palette-title"
  data-testid="palette-dialog"
  onclose={() => (ui.paletteOpen = false)}
  onclick={onBackdrop}
>
  <h2 id="palette-title" class="sr-only">{t('palette.title')}</h2>
  {#if chosen === null}
    <label for="palette-search" class="sr-only">{t('palette.label')}</label>
    <input
      id="palette-search"
      bind:this={search}
      bind:value={query}
      type="text"
      role="combobox"
      aria-expanded="true"
      aria-controls="palette-list"
      aria-autocomplete="list"
      aria-activedescendant={shown.length > 0 ? `palette-option-${active}` : undefined}
      autocomplete="off"
      spellcheck="false"
      placeholder={t('palette.placeholder')}
      data-testid="palette-search"
      onkeydown={onSearchKey}
    />
    <ul id="palette-list" bind:this={list} role="listbox" aria-label={t('palette.results')} data-testid="palette-list">
      {#each shown as item, i (item.entry.name)}
        <!-- Keyboard: the search field holds the focus and drives the list (combobox pattern). -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          id={`palette-option-${i}`}
          role="option"
          aria-selected={i === active}
          class:active={i === active}
          data-command={item.entry.name}
          onclick={() => choose(item)}
          onpointermove={() => (active = i)}
        >
          <span class="label">{item.label}</span>
          <span class="side">
            {#if keysOf(item.entry.name) !== ''}<kbd data-testid="palette-keys">{keysOf(item.entry.name)}</kbd>{/if}
            <code class="name">{item.entry.name}</code>
          </span>
          <span class="summary">{item.entry.summary}</span>
        </li>
      {/each}
    </ul>
    {#if refused}<p class="error status" role="alert" data-testid="palette-refused">{refusedText}</p>{/if}
    <p class="status" aria-live="polite" data-testid="palette-status">
      {#if shown.length === 0}{t('palette.none')}{:else}{t('palette.count', { shown: shown.length, total: found.length })}{/if}
    </p>
    <p class="hint">{t('palette.hint')}</p>
  {:else}
    <form onsubmit={submit} data-testid="palette-form">
      <h3>{chosen.label}</h3>
      <code class="name">{chosen.entry.name}</code>
      <p class="summary">{chosen.entry.summary}</p>
      {#if chosen.entry.destructive}<p class="note warning" data-testid="palette-destructive">{t('palette.destructive')}</p>{/if}
      {#each chosen.entry.fields as field (field.name)}
        {@const id = `palette-field-${field.name}`}
        <div class="field">
          <label for={id}>
            <code>{field.name}</code>
            {#if !field.required}<span class="muted">{t('palette.optional')}</span>{/if}
          </label>
          {#if field.kind === 'enum'}
            <select {id} data-field aria-invalid={problems[field.name] ? 'true' : undefined} bind:value={texts[field.name]}>
              {#if !field.required}<option value="">{t('palette.default')}</option>{:else if (texts[field.name] ?? '') === ''}<option value=""></option>{/if}
              {#each field.options ?? [] as option (option)}<option value={option}>{option}</option>{/each}
            </select>
          {:else if field.kind === 'boolean'}
            <select {id} data-field aria-invalid={problems[field.name] ? 'true' : undefined} bind:value={texts[field.name]}>
              {#if !field.required}<option value="">{t('palette.default')}</option>{/if}
              <option value="true">{t('palette.yes')}</option>
              <option value="false">{t('palette.no')}</option>
            </select>
          {:else}
            <input
              {id}
              data-field
              type="text"
              autocomplete="off"
              spellcheck="false"
              inputmode={field.kind === 'integer' || field.kind === 'number' ? 'decimal' : undefined}
              placeholder={field.kind === 'json' ? t('palette.json') : undefined}
              aria-invalid={problems[field.name] ? 'true' : undefined}
              aria-describedby={problems[field.name] ? `${id}-error` : undefined}
              bind:value={texts[field.name]}
              oninput={() => {
                delete problems[field.name]
                refused = false
              }}
            />
          {/if}
          {#if field.ref === 'TrackId' && selectedTrack && texts[field.name] === selectedTrack}
            <p class="note">{t('palette.selectedTrack')}</p>
          {/if}
          {#if field.nullable}<p class="note">{t('palette.nullable')}</p>{/if}
          {#if problems[field.name]}
            <p id={`${id}-error`} class="error" role="alert">
              {problems[field.name] === 'required' ? t('palette.required') : t('palette.invalid', { kind: kindWords(field) })}
            </p>
          {/if}
        </div>
      {/each}
      {#if refused}<p class="error" role="alert" data-testid="palette-refused">{refusedText}</p>{/if}
      <div class="actions">
        <button type="button" class="ghost" onclick={back} data-testid="palette-back">{t('palette.back')}</button>
        <button type="submit" data-testid="palette-run">{t('palette.run')}</button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  .palette-dialog {
    width: min(640px, calc(100vw - 2 * var(--fk-space-4)));
    max-height: min(560px, calc(100dvh - 2 * var(--fk-space-6)));
    margin: 12vh auto auto;
    padding: var(--fk-space-3);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    color: var(--fk-text);
    box-shadow: var(--fk-shadow-float);
    overflow: auto;
  }
  .palette-dialog::backdrop {
    background: var(--fk-scrim);
  }
  #palette-search {
    width: 100%;
    box-sizing: border-box;
  }
  ul {
    list-style: none;
    margin: var(--fk-space-2) 0 0;
    padding: 0;
    max-height: 340px;
    overflow: auto;
  }
  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    column-gap: var(--fk-space-3);
    padding: var(--fk-space-1) var(--fk-space-2);
    border-radius: var(--fk-radius-panel);
    cursor: pointer;
  }
  li.active {
    background: var(--fk-surface-2);
  }
  .label {
    font-weight: 600;
  }
  .side {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
  }
  kbd {
    font-family: var(--fk-font-mono);
    font-size: 11px;
    padding: 0 var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
  }
  .name {
    font-family: var(--fk-font-mono);
    font-size: 11px;
    color: var(--fk-text-muted);
    align-self: center;
  }
  .summary {
    grid-column: 1 / -1;
    color: var(--fk-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status,
  .hint {
    margin: var(--fk-space-2) var(--fk-space-1) 0;
    color: var(--fk-text-muted);
    font-size: 12px;
  }
  .hint {
    margin-top: 0;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-2);
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  form .summary {
    white-space: normal;
    margin: 0;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-1);
  }
  .field code,
  input,
  select {
    font-family: var(--fk-font-mono);
  }
  .muted,
  .note {
    margin: 0;
    color: var(--fk-text-muted);
    font-size: 12px;
  }
  .error {
    margin: 0;
    color: var(--fk-danger);
  }
  .warning {
    color: var(--fk-danger);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--fk-space-2);
    margin-top: var(--fk-space-2);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
