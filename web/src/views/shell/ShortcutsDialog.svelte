<script lang="ts">
  /**
   * The shortcut list (#474), opened by the "?" button of the top bar or the `?` key. It is built from
   * `lib/shortcuts.ts`, which a test holds against `keyboard.ts`, so it lists every key. A native modal
   * dialog like the engine's: the page behind is inert, the focus stays inside, Escape closes it and the
   * focus goes back to what opened it. It writes nothing: it only reads.
   */
  import { chordText, isMac, keyLabel, SHORTCUT_GROUPS } from '../../lib/shortcuts'
  import { useStores } from '../../lib/stores'

  const { ui } = useStores()
  const t = ui.t
  const mac = isMac()

  let dialog = $state<HTMLDialogElement | null>(null)

  // The store says whether it is open; the element follows (and tells the store when the browser closed it: Escape).
  $effect(() => {
    if (!dialog) return
    if (ui.helpOpen && !dialog.open) dialog.showModal()
    else if (!ui.helpOpen && dialog.open) dialog.close()
  })

  /** A click on the backdrop (the dialog element itself, outside its box) closes it. */
  function onBackdrop(event: MouseEvent) {
    if (event.target === dialog) ui.helpOpen = false
  }
</script>

<!-- svelte-ignore a11y_no_redundant_roles -->
<dialog
  bind:this={dialog}
  class="shortcuts-dialog"
  role="dialog"
  aria-modal={ui.helpOpen ? 'true' : undefined}
  aria-labelledby="shortcuts-title"
  data-testid="shortcuts-dialog"
  onclose={() => (ui.helpOpen = false)}
  onclick={onBackdrop}
>
  <header>
    <h2 id="shortcuts-title">{t('shortcut.title')}</h2>
    <button
      class="close"
      type="button"
      aria-label={t('shortcut.close')}
      title={t('shortcut.close')}
      data-testid="shortcuts-close"
      onclick={() => (ui.helpOpen = false)}
    >
      <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"
        ><path d="M3 3 L 11 11 M11 3 L 3 11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg
      >
    </button>
  </header>
  <p class="intro">{t('shortcut.intro')}</p>
  <div class="groups">
    {#each SHORTCUT_GROUPS as group (group.id)}
      <section aria-labelledby={`shortcut-group-${group.id}`}>
        <h3 id={`shortcut-group-${group.id}`}>{t(group.title)}</h3>
        <ul>
          {#each group.shortcuts as shortcut (shortcut.label)}
            <li>
              <span class="what">{t(shortcut.label)}</span>
              {#if shortcut.chords.length > 0}
                <span class="keys">
                  {#each shortcut.chords as chord, i (i)}
                    {#if i > 0}<span class="or" aria-hidden="true">/</span>{/if}
                    <span class="sr-only">{i > 0 ? t('shortcut.or') : ''} {chordText(chord, mac)}</span>
                    <span class="chord" aria-hidden="true">
                      {#each chord as token, k (k)}<kbd>{keyLabel(token, mac)}</kbd>{/each}
                    </span>
                  {/each}
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/each}
    <section aria-labelledby="shortcut-group-mouse">
      <h3 id="shortcut-group-mouse">{t('shortcut.group.mouse')}</h3>
      <p class="mouse">{t('arrangement.help')}</p>
    </section>
  </div>
</dialog>

<style>
  .shortcuts-dialog {
    width: min(920px, calc(100vw - 2 * var(--fk-space-4)));
    max-height: calc(100dvh - 2 * var(--fk-space-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    color: var(--fk-text);
    box-shadow: var(--fk-shadow-float);
    overflow: auto;
  }
  .shortcuts-dialog::backdrop {
    background: rgb(0 0 0 / 0.5);
  }
  header {
    position: sticky;
    top: 0;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--fk-space-4) var(--fk-space-5) var(--fk-space-2);
    background: var(--fk-surface);
  }
  h2 {
    margin: 0;
    font-size: 15px;
  }
  .intro {
    margin: 0;
    padding: 0 var(--fk-space-5) var(--fk-space-3);
    color: var(--fk-text-muted);
    max-width: 70ch;
  }
  .groups {
    columns: 2 380px;
    column-gap: var(--fk-space-8);
    padding: 0 var(--fk-space-5) var(--fk-space-5);
  }
  section {
    break-inside: avoid;
    margin-bottom: var(--fk-space-5);
  }
  h3 {
    margin: 0 0 var(--fk-space-2);
    font-size: 12px;
    font-weight: 600;
    color: var(--fk-text-muted);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: var(--fk-space-4);
    padding: var(--fk-space-1) 0;
    border-top: 1px solid var(--fk-line);
  }
  li:first-child {
    border-top: 0;
  }
  .what {
    flex: 1 1 auto;
    min-width: 0;
  }
  .keys {
    flex: none;
    display: inline-flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    align-items: baseline;
    gap: var(--fk-space-1);
    max-width: 55%;
  }
  .chord {
    display: inline-flex;
    gap: 2px;
  }
  .or {
    color: var(--fk-text-muted);
  }
  kbd {
    min-width: 22px;
    padding: 1px 6px;
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    font-family: var(--fk-font-mono);
    font-size: 11px;
    line-height: 18px;
    text-align: center;
  }
  .mouse {
    margin: 0;
    color: var(--fk-text-muted);
    max-width: 62ch;
  }
  .close {
    display: grid;
    place-items: center;
    width: var(--fk-control);
    min-height: var(--fk-control);
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text-muted);
    cursor: pointer;
  }
  .close:hover {
    background: var(--fk-surface-2);
    color: var(--fk-text);
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
