<script lang="ts">
  /**
   * What the project's storage needs the user to see or decide (SV-2): the project of last time waiting
   * for a click, the browser's permission gone, a write that failed, a folder edited from outside while
   * changes are unsaved, a notice (a folder refused), and the question before New or Open discards changes.
   * A project opened by its link (#497): the question before a link of the address is downloaded, the
   * download under way, and a download that failed, with the way out (import a file instead).
   */
  import {
    allowAccess,
    exportPack,
    importPack,
    keepMine,
    loadFolderVersion,
    openBrowserProject,
    openFolder,
    openUrl,
    reopenLast,
    save,
  } from '../../lib/project-commands'
  import { useStores } from '../../lib/stores'

  const { ui, storage, dispatcher } = useStores()
  const t = ui.t
  const name = $derived(storage.folder?.name ?? '')
  // What each question says (the storage asks whoever started the action: a click or an agent's command).
  const TITLES = {
    new: 'confirm.new.title',
    open: 'confirm.open.title',
    delete: 'confirm.delete.title',
    keepMine: 'confirm.keepMine.title',
    loadFolder: 'confirm.loadFolder.title',
  } as const
  const CONFIRMS = {
    new: 'confirm.discard',
    open: 'confirm.discard',
    delete: 'confirm.delete.confirm',
    keepMine: 'confirm.keepMine.confirm',
    loadFolder: 'confirm.loadFolder.confirm',
  } as const

  let cancel = $state<HTMLButtonElement | null>(null)
  $effect(() => {
    if (storage.confirm) cancel?.focus()
  })
  let linkCancel = $state<HTMLButtonElement | null>(null)
  $effect(() => {
    if (storage.linkOffer && !storage.confirm) linkCancel?.focus()
  })

  function answerLink(open: boolean) {
    const url = storage.answerLink(open)
    if (url) dispatcher.run(openUrl(url))
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.key !== 'Escape') return
    if (storage.confirm) storage.respond(false)
    else if (storage.linkOffer) answerLink(false)
  }
</script>

<svelte:window onkeydown={onKeyDown} />

{#if storage.banner === 'reopen' && storage.reopen}
  <p class="banner" role="status" data-testid="reopen-banner">
    <span>{t('save.banner.reopen', { name: storage.reopen.name })}</span>
    <button class="primary" data-testid="reopen" onclick={() => dispatcher.run(reopenLast())}
      >{t('save.banner.reopenButton', { name: storage.reopen.name })}</button
    >
    <button onclick={() => storage.dismissReopen()}>{t('save.banner.dismiss')}</button>
  </p>
{:else if storage.banner === 'warning'}
  <p class="banner" role="status" data-testid="browser-warning">
    <span>{t(storage.browserWarning === 'volatile' ? 'save.banner.browserVolatile' : 'save.banner.browserOnly')}</span>
    <button data-testid="warning-export" onclick={() => dispatcher.run(exportPack())}>{t('project.menu.exportPack')}</button>
    <button onclick={() => storage.dismissBrowserWarning()}>{t('save.banner.dismiss')}</button>
  </p>
{:else if storage.banner === 'permission'}
  <p class="banner" role="alert" data-testid="permission-banner">
    <span>{t('save.banner.permission', { name })}</span>
    <button class="primary" data-testid="allow" onclick={() => dispatcher.run(allowAccess())}>{t('save.banner.allow')}</button>
  </p>
{:else if storage.banner === 'full'}
  <p class="banner" role="alert" data-testid="full-banner" title={storage.errorDetail ?? ''}>
    <span>{t('save.banner.full')}</span>
    <button class="primary" data-testid="full-export" onclick={() => dispatcher.run(exportPack())}>{t('project.menu.exportPack')}</button>
    <button onclick={() => dispatcher.run(save())}>{t('save.banner.retry')}</button>
  </p>
{:else if storage.banner === 'error'}
  <p class="banner" role="alert" data-testid="error-banner" title={storage.errorDetail ?? ''}>
    <span>{storage.folder ? t('save.banner.error', { name }) : t('save.banner.errorBrowser')}</span>
    <button class="primary" onclick={() => dispatcher.run(save())}>{t('save.banner.retry')}</button>
  </p>
{:else if storage.banner === 'external'}
  <p class="banner" role="alert" data-testid="external-banner">
    <span>{t('save.banner.external', { name })}</span>
    <button class="primary" data-testid="keep-mine" onclick={() => dispatcher.run(keepMine())}>{t('save.banner.keepMine')}</button>
    <button data-testid="load-disk" onclick={() => dispatcher.run(loadFolderVersion())}>{t('save.banner.loadDisk')}</button>
  </p>
{:else if storage.banner === 'downloading'}
  <p class="banner" role="status" data-testid="download-banner">
    <span>{t('openUrl.downloading', { host: storage.downloading ?? '' })}</span>
  </p>
{:else if storage.banner === 'downloadFailed'}
  <p class="banner" role="alert" data-testid="download-failed" title={storage.errorDetail ?? ''}>
    <span>{t('openUrl.failed', { host: storage.downloadFailed ?? '' })}</span>
    <button class="primary" data-testid="download-import" onclick={() => (storage.dismissDownloadFailed(), dispatcher.run(importPack()))}
      >{t('openUrl.importFile')}</button
    >
    <button onclick={() => storage.dismissDownloadFailed()}>{t('save.banner.dismiss')}</button>
  </p>
{:else if storage.banner === 'notice' && storage.notice}
  <p class="banner" role="status" data-testid="storage-notice">
    <span>{t(storage.notice, storage.noticeVars ?? undefined)}</span>
    {#if storage.noticeOpen && storage.noticeVars}
      {@const dir = storage.noticeOpen}
      <button class="primary" data-testid="notice-open" onclick={() => dispatcher.run(openBrowserProject(dir))}>{t('pack.openIt', storage.noticeVars)}</button>
    {/if}
    {#if storage.failedReopen}
      <!-- The project of last time would not open (#504): try it again, or go to another through the same commands as the menu. -->
      <button class="primary" data-testid="notice-retry" onclick={() => dispatcher.run(reopenLast())}>{t('save.banner.reopenRetry')}</button>
      {#if storage.failedReopen.kind === 'folder'}
        <button data-testid="notice-open-other" onclick={() => dispatcher.run(openFolder())}>{t('save.banner.openOther')}</button>
      {:else}
        <button data-testid="notice-import" onclick={() => dispatcher.run(importPack())}>{t('openUrl.importFile')}</button>
      {/if}
    {/if}
    <button onclick={() => storage.dismissNotice()}>{t('save.banner.dismiss')}</button>
  </p>
{/if}

{#if storage.linkOffer && !storage.confirm}
  <div class="overlay">
    <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="link-title" aria-describedby="link-body" data-testid="open-link">
      <h2 id="link-title">{t('openUrl.confirm.title', { host: storage.linkOffer.host })}</h2>
      <p id="link-body">{t('openUrl.confirm.body')}</p>
      <p class="link" data-testid="open-link-url">{storage.linkOffer.url}</p>
      <div class="actions">
        <button bind:this={linkCancel} data-testid="open-link-cancel" onclick={() => answerLink(false)}>{t('confirm.cancel')}</button>
        <button class="primary" data-testid="open-link-open" onclick={() => answerLink(true)}>{t('openUrl.confirm.open')}</button>
      </div>
    </div>
  </div>
{/if}

{#if storage.confirm}
  <div class="overlay">
    <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" aria-describedby="confirm-body" data-testid="confirm">
      <h2 id="confirm-title">{t(TITLES[storage.confirm.action])}</h2>
      <p id="confirm-body">
        {#if storage.confirm.action === 'delete'}
          {t('confirm.delete.body', { name: storage.confirm.name ?? '' })}
        {:else if storage.confirm.action === 'keepMine'}
          {t('confirm.keepMine.body', { name: storage.confirm.name ?? '' })}
        {:else if storage.confirm.action === 'loadFolder'}
          {t('confirm.loadFolder.body', { name: storage.confirm.name ?? '' })}
        {:else}
          {storage.confirm.name ? t('confirm.bodyFolder', { name: storage.confirm.name }) : t('confirm.body')}
        {/if}
      </p>
      <div class="actions">
        <button bind:this={cancel} class="primary" data-testid="confirm-cancel" onclick={() => storage.respond(false)}>{t('confirm.cancel')}</button>
        <button data-testid="confirm-discard" onclick={() => storage.respond(true)}>{t(CONFIRMS[storage.confirm.action])}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .banner {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--fk-space-3);
    margin: 0;
    padding: var(--fk-space-2) var(--fk-space-4);
    border-bottom: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  .banner span {
    flex: 1 1 24rem;
  }
  button {
    height: 28px;
    padding: 0 var(--fk-space-3);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text);
    font-size: 12px;
    cursor: pointer;
  }
  button:hover {
    background: var(--fk-surface-2);
  }
  button.primary {
    border-color: var(--fk-accent);
    background: var(--fk-accent);
    color: #fff;
    font-weight: 600;
  }
  button.primary:hover {
    background: var(--fk-accent-strong);
  }
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.5);
  }
  .dialog {
    width: min(420px, calc(100vw - 32px));
    padding: var(--fk-space-5, 20px);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius);
    background: var(--fk-surface);
  }
  h2 {
    margin: 0 0 var(--fk-space-2);
    font-size: 16px;
  }
  .dialog p {
    margin: 0 0 var(--fk-space-4);
  }
  /* The whole link, so the user sees where the file comes from; a long one wraps instead of widening the dialog. */
  .dialog p.link {
    padding: var(--fk-space-2) var(--fk-space-3);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius);
    background: var(--fk-surface-2);
    color: var(--fk-text-muted, var(--fk-text));
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--fk-space-2);
  }
</style>
