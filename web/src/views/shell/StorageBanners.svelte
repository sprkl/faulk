<script lang="ts">
  /**
   * What the project's storage needs the user to see or decide (SV-2): the project of last time waiting
   * for a click, the browser's permission gone, a write that failed, a folder edited from outside while
   * changes are unsaved, a notice (a folder refused), and the question before New or Open discards changes.
   */
  import { useStores } from '../../lib/stores'

  const { ui, storage } = useStores()
  const t = ui.t
  const name = $derived(storage.folder?.name ?? '')

  let cancel = $state<HTMLButtonElement | null>(null)
  $effect(() => {
    if (storage.confirm) cancel?.focus()
  })

  function onKeyDown(event: KeyboardEvent) {
    if (event.key === 'Escape' && storage.confirm) storage.respond(false)
  }
</script>

<svelte:window onkeydown={onKeyDown} />

{#if storage.reopen}
  <p class="banner" role="status" data-testid="reopen-banner">
    <span>{t('save.banner.reopen', { name: storage.reopen.name })}</span>
    <button class="primary" data-testid="reopen" onclick={() => storage.reopenLast()}>{t('save.banner.reopenButton')}</button>
    <button onclick={() => storage.dismissReopen()}>{t('save.banner.dismiss')}</button>
  </p>
{/if}

{#if storage.block === 'permission'}
  <p class="banner" role="alert" data-testid="permission-banner">
    <span>{t('save.banner.permission', { name })}</span>
    <button class="primary" data-testid="allow" onclick={() => storage.resume()}>{t('save.banner.allow')}</button>
  </p>
{:else if storage.block === 'error'}
  <p class="banner" role="alert" data-testid="error-banner" title={storage.errorDetail ?? ''}>
    <span>{t('save.banner.error', { name })}</span>
    <button class="primary" onclick={() => storage.save()}>{t('save.banner.retry')}</button>
  </p>
{:else if storage.block === 'external'}
  <p class="banner" role="alert" data-testid="external-banner">
    <span>{t('save.banner.external', { name })}</span>
    <button class="primary" data-testid="keep-mine" onclick={() => storage.keepMine()}>{t('save.banner.keepMine')}</button>
    <button data-testid="load-disk" onclick={() => storage.loadDisk()}>{t('save.banner.loadDisk')}</button>
  </p>
{/if}

{#if storage.notice}
  <p class="banner" role="status" data-testid="storage-notice">
    <span>{t(storage.notice)}</span>
    <button onclick={() => storage.dismissNotice()}>{t('save.banner.dismiss')}</button>
  </p>
{/if}

{#if storage.confirm}
  <div class="overlay">
    <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" aria-describedby="confirm-body" data-testid="confirm">
      <h2 id="confirm-title">{t(storage.confirm.action === 'new' ? 'confirm.new.title' : 'confirm.open.title')}</h2>
      <p id="confirm-body">
        {storage.confirm.name ? t('confirm.bodyFolder', { name: storage.confirm.name }) : t('confirm.body')}
      </p>
      <div class="actions">
        <button bind:this={cancel} class="primary" data-testid="confirm-cancel" onclick={() => storage.respond(false)}>{t('confirm.cancel')}</button>
        <button data-testid="confirm-discard" onclick={() => storage.respond(true)}>{t('confirm.discard')}</button>
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
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--fk-space-2);
  }
</style>
