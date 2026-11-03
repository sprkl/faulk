<script lang="ts">
  /**
   * "Repeat until" (#555): the field that asks for the bar a clip is repeated up to. A native modal dialog like the "Go to"
   * field: the page behind is inert, the focus stays inside, Escape closes it and the focus goes back to what opened it. It
   * only reads a whole bar; the arrangement turns it into `arrangement.repeat`.
   */
  import { useStores } from '../../lib/stores'
  import { MAX_GOTO_BAR } from '../transport/time'

  /** `min` is the first bar that is after the clip; `onsubmit` gets the bar typed. */
  let { open = false, min, onsubmit, onclose }: { open: boolean; min: number; onsubmit: (bar: number) => void; onclose: () => void } = $props()

  const { ui } = useStores()
  const t = ui.t

  let dialog = $state<HTMLDialogElement | null>(null)
  let field = $state<HTMLInputElement | null>(null)
  let text = $state('')
  let invalid = $state(false)

  $effect(() => {
    if (!dialog) return
    if (open && !dialog.open) {
      text = String(min)
      invalid = false
      dialog.showModal()
      field?.focus()
      field?.select()
    } else if (!open && dialog.open) dialog.close()
  })

  function submit(event: SubmitEvent) {
    event.preventDefault()
    const bar = /^\d{1,6}$/.test(text.trim()) ? Number(text.trim()) : NaN
    if (!(bar >= min && bar <= MAX_GOTO_BAR)) {
      invalid = true
      field?.focus()
      return
    }
    onsubmit(bar)
  }

  /** A click on the backdrop (the dialog element itself, outside its box) closes it. */
  function onBackdrop(event: MouseEvent) {
    if (event.target === dialog) onclose()
  }
</script>

<!-- svelte-ignore a11y_no_redundant_roles -->
<dialog
  bind:this={dialog}
  class="repeat-dialog"
  role="dialog"
  aria-modal={open ? 'true' : undefined}
  aria-labelledby="repeat-title"
  data-testid="repeat-dialog"
  {onclose}
  onclick={onBackdrop}
>
  <form onsubmit={submit}>
    <h2 id="repeat-title">{t('arrangement.repeatTitle')}</h2>
    <label for="repeat-field">{t('arrangement.repeatLabel', { min })}</label>
    <input
      id="repeat-field"
      bind:this={field}
      bind:value={text}
      type="text"
      inputmode="numeric"
      autocomplete="off"
      aria-invalid={invalid ? 'true' : undefined}
      aria-describedby={invalid ? 'repeat-error' : undefined}
      data-testid="repeat-field"
      oninput={() => (invalid = false)}
    />
    {#if invalid}<p id="repeat-error" class="error" role="alert" data-testid="repeat-error">
        {t('arrangement.repeatInvalid', { min, max: MAX_GOTO_BAR })}
      </p>{/if}
    <div class="actions">
      <button type="button" class="ghost" onclick={onclose}>{t('arrangement.repeatCancel')}</button>
      <button type="submit" data-testid="repeat-submit">{t('arrangement.repeatGo')}</button>
    </div>
  </form>
</dialog>

<style>
  .repeat-dialog {
    width: min(360px, calc(100vw - 2 * var(--fk-space-4)));
    margin: auto;
    padding: var(--fk-space-4) var(--fk-space-5);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    color: var(--fk-text);
    box-shadow: var(--fk-shadow-float);
  }
  .repeat-dialog::backdrop {
    background: rgb(0 0 0 / 0.5);
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
    color: var(--fk-danger, var(--fk-text));
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--fk-space-2);
    margin-top: var(--fk-space-2);
  }
</style>
