<script lang="ts">
  /**
   * The project in the top bar (SV-2): its name, the menu (new, open, save, save as, recent projects)
   * and, always in sight, whether it is saved: "Saved at 14:32", "Saving…", "Unsaved changes", "Not saved"
   * (no folder yet). A musician reads it to know whether the tab can be closed.
   */
  import { exportPack, importPack, save, saveAs } from '../../lib/project-commands'
  import { useStores } from '../../lib/stores'

  const { ui, storage, project, dispatcher } = useStores()
  const t = ui.t

  let open = $state(false)
  let root = $state<HTMLElement | null>(null)

  const ready = $derived(project.view !== null)
  const name = $derived(project.view?.project.name ?? t('project.untitled'))
  const time = $derived(
    storage.savedAt === null ? null : new Intl.DateTimeFormat(ui.lang, { timeStyle: 'short' }).format(storage.savedAt),
  )
  const LABELS = {
    saving: 'save.saving',
    pending: 'save.pending',
    paused: 'save.paused',
    failed: 'save.failed',
    conflict: 'save.conflict',
    unsaved: 'save.unsaved',
  } as const
  // A project kept in the browser's storage is a draft where a folder can be chosen, and the project itself where not.
  const savedKey = $derived(storage.place === 'browser' ? (storage.supported ? 'save.savedDraft' : 'save.savedBrowser') : 'save.saved')
  const status = $derived(
    storage.status === 'saved' ? (time ? t(savedKey, { time }) : t('save.savedPlain')) : t(LABELS[storage.status]),
  )
  const where = $derived(
    storage.folder
      ? t('save.folder', { name: storage.folder.name })
      : storage.place === 'browser'
        ? t(storage.supported ? 'save.draft' : 'save.browser')
        : t('save.noFolder'),
  )

  function choose(action: () => unknown) {
    open = false
    void action()
  }

  function toggle() {
    open = !open
    if (open) void storage.refreshBrowserProjects()
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.key === 'Escape' && open) {
      open = false
      root?.querySelector<HTMLElement>('.trigger')?.focus()
    }
  }

  function onPointerDown(event: PointerEvent) {
    if (open && root && !root.contains(event.target as Node)) open = false
  }
</script>

<svelte:window onpointerdown={onPointerDown} />

<div class="project" bind:this={root} onkeydown={onKeyDown} role="presentation">
  <button
    class="trigger"
    aria-haspopup="menu"
    aria-expanded={open}
    aria-label={t('project.menu')}
    title={ready ? `${name}: ${status}. ${where}` : name}
    data-testid="project-menu"
    onclick={toggle}
  >
    <svg class="doc" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.3"><path d="M2.5 1.5 H7 L9.5 4 V10.5 H2.5 Z M7 1.5 V4 H9.5" /></svg>
    <span class="name" data-testid="project-name">{name}</span>
    <svg class="chevron" width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.4"><path d="M2 3.5 L5 6.5 L8 3.5" /></svg>
    {#if ready}<span class="dot" data-state={storage.status} aria-hidden="true"></span>{/if}
  </button>
  {#if ready}
    <span class="state" data-state={storage.status} data-testid="save-state" role="status" title={`${status}. ${where}`}>{status}</span>
  {/if}

  {#if open}
    <div class="menu" role="menu" aria-label={t('project.menu')}>
      <p class="summary" data-testid="menu-summary">{name}{ready ? ` · ${status}` : ''}</p>
      <button role="menuitem" disabled={!ready} data-testid="menu-new" onclick={() => choose(() => storage.newProject())}>{t('project.menu.new')}</button>
      <button role="menuitem" disabled={!storage.supported} data-testid="menu-open" onclick={() => choose(() => storage.openFolder())}>{t('project.menu.open')}</button>
      <hr />
      <button role="menuitem" disabled={!ready} data-testid="menu-save" onclick={() => choose(() => dispatcher.run(save()))}>
        {t('project.menu.save')}<kbd>Ctrl+S</kbd>
      </button>
      <button role="menuitem" disabled={!ready || !storage.supported} data-testid="menu-save-as" onclick={() => choose(() => dispatcher.run(saveAs()))}>
        {t('project.menu.saveAs')}<kbd>Ctrl+Shift+S</kbd>
      </button>
      <button role="menuitem" disabled={!ready} data-testid="menu-export-pack" onclick={() => choose(() => dispatcher.run(exportPack()))}>{t('project.menu.exportPack')}</button>
      <button role="menuitem" disabled={!ready} data-testid="menu-import-pack" onclick={() => choose(() => dispatcher.run(importPack()))}>{t('project.menu.importPack')}</button>
      {#if storage.supported}
        <hr />
        <p class="heading">{t('project.menu.recent')}</p>
        {#each storage.recent as entry (entry.id)}
          <button role="menuitem" data-testid="menu-recent" onclick={() => choose(() => storage.openRecent(entry.id))}>{entry.name}</button>
        {:else}
          <p class="none">{t('project.menu.noRecent')}</p>
        {/each}
      {:else}
        <hr />
        <p class="none" data-testid="menu-unsupported">{t(storage.browserAvailable ? 'project.menu.browserOnly' : 'project.menu.unsupported')}</p>
      {/if}
      {#if storage.browserProjects.length > 0}
        <hr />
        <p class="heading">{t(storage.supported ? 'project.menu.drafts' : 'project.menu.browserProjects')}</p>
        {#each storage.browserProjects as entry (entry.dir)}
          <div class="entry">
            <button role="menuitem" data-testid="menu-browser-project" onclick={() => choose(() => storage.openBrowserProject(entry.dir))}>{entry.name}</button>
            <button
              role="menuitem"
              class="delete"
              data-testid="menu-delete-browser-project"
              aria-label={t('project.menu.deleteProject', { name: entry.name })}
              title={t('project.menu.deleteProject', { name: entry.name })}
              onclick={() => choose(() => storage.deleteBrowserProject(entry.dir))}>×</button
            >
          </div>
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .project {
    position: relative;
    display: inline-flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    min-width: 0;
  }
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--fk-space-2);
    height: 22px;
    max-width: 130px;
    padding: 0 var(--fk-space-3);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text);
    font-weight: 600;
    font-size: 12px;
    cursor: pointer;
  }
  .trigger:hover {
    background: var(--fk-surface-2);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .state {
    max-width: 10rem;
    overflow: hidden;
    text-overflow: ellipsis;
    padding-left: var(--fk-space-3);
    font-size: 11px;
    line-height: 1;
    color: var(--fk-text-muted);
    white-space: nowrap;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fk-text-muted);
  }
  .dot[data-state='pending'],
  .dot[data-state='saving'] {
    background: var(--fk-text);
  }
  .dot[data-state='unsaved'] {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px var(--fk-accent);
  }
  .dot[data-state='paused'],
  .dot[data-state='failed'],
  .dot[data-state='conflict'] {
    background: var(--fk-accent);
  }
  .doc {
    display: none;
  }
  /* The top bar is full at this width: the project shows as an icon with its state's dot, and its name
     and state (Saved at 14:32, Unsaved changes...) are read from the tooltip, by screen readers, and from
     the menu's heading. */
  @media (max-width: 1399px) {
    .doc {
      display: block;
    }
    .name,
    .state,
    .chevron {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip: rect(0 0 0 0);
      white-space: nowrap;
    }
    .trigger {
      padding: 0 var(--fk-space-2);
    }
  }
  .state[data-state='pending'],
  .state[data-state='saving'] {
    color: var(--fk-text);
  }
  .state[data-state='paused'],
  .state[data-state='failed'],
  .state[data-state='conflict'] {
    color: var(--fk-accent);
    font-weight: 600;
  }
  .menu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    min-width: 240px;
    max-width: 320px;
    padding: var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius);
    background: var(--fk-surface);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.25);
  }
  .menu button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--fk-space-3);
    padding: var(--fk-space-2) var(--fk-space-3);
    border: 0;
    border-radius: var(--fk-radius-sm, 4px);
    background: transparent;
    color: var(--fk-text);
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .menu button:hover:not(:disabled) {
    background: var(--fk-surface-2);
  }
  .menu button:disabled {
    color: var(--fk-text-muted);
    cursor: default;
  }
  kbd {
    color: var(--fk-text-muted);
    font: inherit;
    font-size: 11px;
  }
  .entry {
    display: flex;
  }
  .entry > :first-child {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu .entry .delete {
    flex: none;
    justify-content: center;
    width: 28px;
    padding: 0;
    color: var(--fk-text-muted);
  }
  hr {
    width: 100%;
    margin: var(--fk-space-1) 0;
    border: 0;
    border-top: 1px solid var(--fk-line);
  }
  .summary {
    margin: 0;
    padding: var(--fk-space-1) var(--fk-space-3) var(--fk-space-2);
    border-bottom: 1px solid var(--fk-line);
    font-size: 12px;
    font-weight: 600;
  }
  .heading,
  .none {
    margin: 0;
    padding: var(--fk-space-1) var(--fk-space-3);
    font-size: 12px;
    color: var(--fk-text-muted);
  }
</style>
