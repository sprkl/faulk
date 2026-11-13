<script lang="ts">
  /**
   * "Go to" (#554), opened by Ctrl+G in every view: a bar ("12") or a bar and a beat ("12.3") puts the playhead there with
   * `transport.seek`, like a click on the ruler. A native modal dialog like the shortcut list: the page behind is inert, the
   * focus stays inside, Escape closes it and the focus goes back to what opened it. A position that means nothing is said, not
   * guessed.
   */
  import { seek } from '../../lib/transport-commands'
  import { useStores } from '../../lib/stores'
  import { parseBarBeat } from '../transport/time'

  const { project, dispatcher, ui } = useStores()
  const t = ui.t

  let dialog = $state<HTMLDialogElement | null>(null)
  let field = $state<HTMLInputElement | null>(null)
  let text = $state('')
  let invalid = $state(false)

  const signature = $derived(project.view?.project.meter[0]?.signature ?? { numerator: 4, denominator: 4 })

  // The store says whether it is open; the element follows (and tells the store when the browser closed it: Escape).
  $effect(() => {
    if (!dialog) return
    if (ui.gotoOpen && !dialog.open) {
      text = ''
      invalid = false
      dialog.showModal()
      field?.focus()
    } else if (!ui.gotoOpen && dialog.open) dialog.close()
  })

  function submit(event: SubmitEvent) {
    event.preventDefault()
    const tick = parseBarBeat(text, signature)
    if (tick === null) {
      invalid = true
      field?.focus()
      return
    }
    if (dispatcher.run(seek(tick))) ui.gotoTarget = { tick, n: (ui.gotoTarget?.n ?? 0) + 1 }
    ui.gotoOpen = false
  }

  /** A click on the backdrop (the dialog element itself, outside its box) closes it. */
  function onBackdrop(event: MouseEvent) {
    if (event.target === dialog) ui.gotoOpen = false
  }
</script>

<!-- svelte-ignore a11y_no_redundant_roles -->
<dialog
  bind:this={dialog}
  class="goto-dialog"
  role="dialog"
  aria-modal={ui.gotoOpen ? 'true' : undefined}
  aria-labelledby="goto-title"
  data-testid="goto-dialog"
  onclose={() => (ui.gotoOpen = false)}
  onclick={onBackdrop}
>
  <form onsubmit={submit}>
    <h2 id="goto-title">{t('goto.title')}</h2>
    <label for="goto-field">{t('goto.label', { beats: signature.numerator })}</label>
    <input
      id="goto-field"
      bind:this={field}
      bind:value={text}
      type="text"
      inputmode="decimal"
      autocomplete="off"
      placeholder="12.3"
      aria-invalid={invalid ? 'true' : undefined}
      aria-describedby={invalid ? 'goto-error' : undefined}
      data-testid="goto-field"
      oninput={() => (invalid = false)}
    />
    {#if invalid}<p id="goto-error" class="error" role="alert" data-testid="goto-error">{t('goto.invalid', { beats: signature.numerator })}</p>{/if}
    <div class="actions">
      <button type="button" class="ghost" onclick={() => (ui.gotoOpen = false)}>{t('goto.cancel')}</button>
      <button type="submit" data-testid="goto-submit">{t('goto.go')}</button>
    </div>
  </form>
</dialog>

<style>
  .goto-dialog {
    width: min(360px, calc(100vw - 2 * var(--fk-space-4)));
    margin: auto;
    padding: var(--fk-space-4) var(--fk-space-5);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    color: var(--fk-text);
    box-shadow: var(--fk-shadow-float);
  }
  .goto-dialog::backdrop {
    background: var(--fk-scrim);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-2);
  }
  h2 {
    margin: 0;
    font-size: 15px;
  }
  label {
    color: var(--fk-text-muted);
  }
  input {
    font-family: var(--fk-font-mono);
  }
  .error {
    margin: 0;
    color: var(--fk-danger);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--fk-space-2);
    margin-top: var(--fk-space-2);
  }
</style>
