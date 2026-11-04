<script lang="ts">
  /**
   * The question before an agent's call that cannot be undone runs (AG-4, #100): which agent asks, what it would do,
   * Refuse (focused: the safe answer, also Escape) or Allow, and the seconds left before the page refuses on its own.
   * One question at a time, in the order the calls came (`AgentConfirmations`); hidden while the storage asks the
   * user something of their own or offers a link, one modal at a time (the deadline still runs).
   */
  import { questionBody, questionTitle } from '../../lib/agents/confirm-text'
  import { useStores } from '../../lib/stores'

  const { ui, storage, dispatcher } = useStores()
  const t = ui.t
  const confirmations = dispatcher.confirmations

  const current = $derived(storage.confirm || storage.linkOffer ? null : confirmations.current)
  const more = $derived(confirmations.pending.length - 1)

  // The seconds left, read again every second while a question is shown.
  let now = $state(confirmations.clock.now())
  $effect(() => {
    if (!current) return
    now = confirmations.clock.now()
    const tick = setInterval(() => (now = confirmations.clock.now()), 250)
    return () => clearInterval(tick)
  })
  const seconds = $derived(current ? Math.max(0, Math.ceil((current.deadline - now) / 1000)) : 0)

  let refuse = $state<HTMLButtonElement | null>(null)
  let allow = $state<HTMLButtonElement | null>(null)
  // Each new question takes the focus on Refuse; the focus goes back where it was once none is left.
  let returnTo: HTMLElement | null = null
  let shown: number | null = null
  $effect(() => {
    const id = current?.id ?? null
    if (id === shown) return
    if (shown === null && id !== null && document.activeElement instanceof HTMLElement) returnTo = document.activeElement
    shown = id
    if (id !== null) refuse?.focus()
    else {
      returnTo?.focus()
      returnTo = null
    }
  })

  function onKeyDown(event: KeyboardEvent) {
    if (!current) return
    if (event.key === 'Escape') {
      event.preventDefault()
      confirmations.answer(false)
    } else if (event.key === 'Tab') {
      // Two buttons: the focus stays in the dialog.
      event.preventDefault()
      ;(document.activeElement === refuse ? allow : refuse)?.focus()
    }
  }
</script>

<svelte:window onkeydown={onKeyDown} />

{#if current}
  <div class="overlay">
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="agent-confirm-title"
      aria-describedby="agent-confirm-body"
      data-testid="agent-confirm"
    >
      <p class="eyebrow">{t('agentConfirm.eyebrow')}</p>
      <h2 id="agent-confirm-title">{questionTitle(current, t)}</h2>
      <p id="agent-confirm-body">{questionBody(current, t)}</p>
      <p class="meta" data-testid="agent-confirm-countdown">
        {t('agentConfirm.countdown', { seconds })}
        {#if more === 1}
          {t('agentConfirm.moreOne')}
        {:else if more > 1}
          {t('agentConfirm.moreMany', { count: more })}
        {/if}
      </p>
      <div class="actions">
        <button bind:this={refuse} class="primary" data-testid="agent-confirm-refuse" onclick={() => confirmations.answer(false)}
          >{t('agentConfirm.refuse')}</button
        >
        <button bind:this={allow} data-testid="agent-confirm-allow" onclick={() => confirmations.answer(true)}>{t('agentConfirm.allow')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.5);
  }
  .dialog {
    width: min(440px, calc(100vw - 32px));
    padding: var(--fk-space-5, 20px);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius);
    background: var(--fk-surface);
  }
  .eyebrow {
    margin: 0 0 var(--fk-space-1, 4px);
    color: var(--fk-text-muted, var(--fk-text));
    font-size: 11px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  h2 {
    margin: 0 0 var(--fk-space-2);
    font-size: 16px;
    overflow-wrap: anywhere;
  }
  .dialog p {
    margin: 0 0 var(--fk-space-4);
  }
  .dialog p.eyebrow {
    margin-bottom: var(--fk-space-1, 4px);
  }
  .meta {
    color: var(--fk-text-muted, var(--fk-text));
    font-size: 12px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--fk-space-2);
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
</style>
