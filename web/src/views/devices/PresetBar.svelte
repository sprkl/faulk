<script lang="ts">
  /**
   * The presets of one device (#191), in its card's header: the preset it shows (or "No preset"), the
   * previous and next presets of its kind, a mark when its settings no longer match the preset, and a
   * menu to save, copy, rename and delete. Every action is a command (`preset-commands.ts`); the list
   * and the "modified" answer come from `preset.list` (`dispatcher.listPresets`).
   */
  import { tick } from 'svelte'
  import { copyPreset, deletePreset, loadPreset, renamePreset, savePreset } from '../../lib/preset-commands'
  import type { MenuItem } from '../../lib/context-menu'
  import { useStores } from '../../lib/stores'
  import { neighbour, presetPanel } from '../../lib/presets'

  let { device, name }: { device: string; name: string } = $props()

  const { ui, dispatcher, project } = useStores()
  const t = ui.t

  // Read again when the project changes (a load, a knob turned, a preset saved) or the library does.
  const panel = $derived.by(() => {
    void project.view
    void project.presetsRev
    return presetPanel(dispatcher.listPresets(device))
  })

  /** The name field: `save` to project or library, or `rename` the preset shown; `null` when closed. */
  let editing = $state<'project' | 'library' | 'rename' | null>(null)
  /** The text of the name field (apart from `editing`, so that closing the field never reads a value that is gone). */
  let draft = $state('')
  let field = $state<HTMLInputElement | null>(null)

  function step(direction: -1 | 1) {
    const next = neighbour(panel, direction)
    if (next) dispatcher.run(loadPreset(device, next.preset))
  }

  async function edit(action: 'project' | 'library' | 'rename') {
    draft = action === 'rename' ? (panel.current?.name ?? '') : (panel.current?.name ?? name)
    editing = action
    await tick()
    field?.select()
  }

  function submit(event: SubmitEvent) {
    event.preventDefault()
    if (!editing) return
    const value = draft.trim()
    if (!value) return
    const ok = editing === 'rename' ? panel.current && dispatcher.run(renamePreset(panel.current, value)) : dispatcher.run(savePreset(device, value, editing))
    if (ok) editing = null
  }

  function openMenu(event: MouseEvent) {
    const shown = panel.current
    const writable = shown !== null && shown.location !== 'factory' && panel.reachable
    const items: MenuItem[] = [
      { id: 'preset-save-project', label: t('presets.saveProject'), run: () => void edit('project') },
      { id: 'preset-save-library', label: t('presets.saveLibrary'), run: () => void edit('library') },
      {
        id: 'preset-copy-library',
        label: t('presets.copyLibrary'),
        disabled: !shown || shown.location === 'library' || !panel.reachable,
        separator: true,
        run: () => shown && dispatcher.run(copyPreset(shown, 'library')),
      },
      { id: 'preset-rename', label: t('presets.rename'), disabled: !writable, run: () => void edit('rename') },
      { id: 'preset-delete', label: t('presets.delete'), disabled: !writable, run: () => shown && dispatcher.run(deletePreset(shown)) },
    ]
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect()
    ui.openMenuAt(box.left, box.bottom, items, t('presets.menu', { name }))
  }
</script>

{#if editing}
  <form class="bar" onsubmit={submit} data-testid="preset-form">
    <input
      bind:this={field}
      bind:value={draft}
      aria-label={t('presets.name')}
      maxlength="200"
      data-testid="preset-name-input"
      onkeydown={(e) => e.key === 'Escape' && (editing = null)}
    />
    <button type="submit" data-testid="preset-form-ok">{editing === 'rename' ? t('presets.renameAction') : t('presets.save')}</button>
    <button type="button" onclick={() => (editing = null)}>{t('presets.cancel')}</button>
  </form>
{:else}
  <div class="bar" data-testid="preset-bar" data-modified={panel.modified === true}>
    <button
      aria-label={t('presets.previous', { name })}
      title={t('presets.previous', { name })}
      disabled={panel.entries.length === 0}
      data-testid="preset-previous"
      onclick={() => step(-1)}>‹</button
    >
    <span
      class="name"
      title={panel.current ? t(`presets.where.${panel.current.location}`) : undefined}
      data-testid="preset-name"
      data-location={panel.current?.location ?? ''}>{panel.current?.name ?? t('presets.none')}</span
    >
    {#if panel.modified === true}
      <span class="modified" title={t('presets.modified')} aria-label={t('presets.modified')} data-testid="preset-modified">•</span>
    {/if}
    <button
      aria-label={t('presets.next', { name })}
      title={t('presets.next', { name })}
      disabled={panel.entries.length === 0}
      data-testid="preset-next"
      onclick={() => step(1)}>›</button
    >
    <button
      aria-label={t('presets.menu', { name })}
      title={t('presets.menu', { name })}
      aria-haspopup="menu"
      class="menu"
      data-testid="preset-menu"
      onclick={openMenu}>⋯</button
    >
  </div>
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    font-size: 12px;
  }
  .bar button {
    min-width: 22px;
    height: 22px;
    padding: 0 var(--fk-space-1);
    border: 0;
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text-muted);
    cursor: pointer;
  }
  .bar button:hover:not(:disabled) {
    background: var(--fk-surface-2);
    color: var(--fk-text);
  }
  .bar button:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .name {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fk-text-muted);
  }
  /* Previous, the name and next read as one control; the menu stands apart at the end. */
  .bar .menu {
    margin-left: auto;
  }
  .modified {
    color: var(--fk-accent, var(--fk-text));
    font-weight: 700;
  }
  input {
    flex: 1;
    min-width: 0;
    height: 22px;
    padding: 0 var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
    background: var(--fk-bg);
    color: var(--fk-text);
    font: inherit;
  }
</style>
