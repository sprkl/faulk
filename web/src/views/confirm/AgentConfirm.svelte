<script lang="ts">
  /**
   * The question before an agent's call that cannot be undone runs (AG-4, #100): which agent asks, what it would do,
   * Refuse (focused: the safe answer, also Escape) or Allow, and the seconds left before the page refuses on its own.
   * One question at a time, in the order the calls came (`AgentConfirmations`). A native modal dialog: it opens above
   * any other one (Go to, Save as…), whose page goes inert behind it, and Escape refuses. It waits while the storage
   * asks the user something of their own or offers a link, one question at a time (the deadline still runs).
   */
  import { questionBody, questionTitle } from '../../lib/agents/confirm-text'
  import { useStores } from '../../lib/stores'

  const { ui, storage, dispatcher } = useStores()
  const t = ui.t
  const confirmations = dispatcher.confirmations

  const current = $derived(storage.confirm || storage.linkOffer ? null : confirmations.current)
  const more = $derived(confirmations.pending.length - 1)

  // The seconds left, read again while a question is shown.
  let now = $state(confirmations.clock.now())
  $effect(() => {
    if (!current) return
    now = confirmations.clock.now()
    const tick = setInterval(() => (now = confirmations.clock.now()), 250)
    return () => clearInterval(tick)
  })
  const seconds = $derived(current ? Math.max(0, Math.ceil((current.deadline - now) / 1000)) : 0)

  let dialog = $state<HTMLDialogElement | null>(null)
  let refuse = $state<HTMLButtonElement | null>(null)
  // The dialog follows the question shown; each new question takes the focus on Refuse. Closing gives the focus back
  // where it was (the browser does it for a modal dialog).
  let shown: number | null = null
  $effect(() => {
    if (!dialog) return
    const id = current?.id ?? null
    if (id !== null && !dialog.open) dialog.showModal()
    else if (id === null && dialog.open) dialog.close()
    if (id !== null && id !== shown) refuse?.focus()
    shown = id
  })

  /** Escape: the browser would close the dialog; it refuses instead, and the next question (if any) takes its place. */
  function onCancel(event: Event) {
    event.preventDefault()
    if (current) confirmations.answer(false)
  }
</script>

<dialog
  bind:this={dialog}
  class="dialog"
  role="alertdialog"
  aria-modal={current ? 'true' : undefined}
  aria-labelledby="agent-confirm-title"
  aria-describedby="agent-confirm-body"
  data-testid="agent-confirm"
  oncancel={onCancel}
>
  {#if current}
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
      <button data-testid="agent-confirm-allow" onclick={() => confirmations.answer(true)}>{t('agentConfirm.allow')}</button>
    </div>
  {/if}
</dialog>

<style>
  .dialog {
    width: min(440px, calc(100vw - 32px));
    padding: var(--fk-space-5, 20px);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius);
    background: var(--fk-surface);
    color: var(--fk-text);
  }
  .dialog::backdrop {
    background: rgb(0 0 0 / 0.5);
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
