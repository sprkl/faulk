<script lang="ts">
  /**
   * "Save as" (#627), first step: the project's name, prefilled with the current one. The folder dialog follows from the click
   * that confirms (the browser's folder chooser takes no suggested name), and the project is made in a folder of that name
   * inside the folder chosen. A native modal dialog like "Go to": the page behind is inert, Escape or Cancel changes nothing.
   */
  import { useStores } from '../../lib/stores'

  const { ui } = useStores()
  const t = ui.t

  let dialog = $state<HTMLDialogElement | null>(null)
  let field = $state<HTMLInputElement | null>(null)
  let text = $state('')
  let invalid = $state(false)

  // The store says whether it is asking; the element follows (and tells the store when the browser closed it: Escape).
  $effect(() => {
    if (!dialog) return
    const prompt = ui.saveAsPrompt
    if (prompt && !dialog.open) {
      text = prompt.initial
      invalid = false
      dialog.showModal()
      field?.focus()
      field?.select()
    } else if (!prompt && dialog.open) dialog.close()
  })

  function submit(event: SubmitEvent) {
    event.preventDefault()
    const name = text.trim()
    if (!name) {
      invalid = true
      field?.focus()
      return
    }
    ui.saveAsPrompt?.settle(name)
  }

  function cancel() {
    ui.saveAsPrompt?.settle(null)
  }

  /** A click on the backdrop (the dialog element itself, outside its box) cancels. */
  function onBackdrop(event: MouseEvent) {
    if (event.target === dialog) cancel()
  }
</script>

<!-- svelte-ignore a11y_no_redundant_roles -->
<dialog
  bind:this={dialog}
  class="save-as-dialog"
  role="dialog"
  aria-modal={ui.saveAsPrompt ? 'true' : undefined}
  aria-labelledby="save-as-title"
  data-testid="save-as-dialog"
  onclose={cancel}
  onclick={onBackdrop}
>
  <form onsubmit={submit}>
    <h2 id="save-as-title">{t('saveAs.title')}</h2>
    <label for="save-as-field">{t('saveAs.label')}</label>
    <input
      id="save-as-field"
      bind:this={field}
      bind:value={text}
      type="text"
      autocomplete="off"
      maxlength="100"
      aria-invalid={invalid ? 'true' : undefined}
      aria-describedby={invalid ? 'save-as-error' : 'save-as-hint'}
      data-testid="save-as-field"
      oninput={() => (invalid = false)}
    />
    {#if invalid}
      <p id="save-as-error" class="error" role="alert" data-testid="save-as-error">{t('saveAs.empty')}</p>
    {:else}
      <p id="save-as-hint" class="hint">{t('saveAs.hint')}</p>
    {/if}
    <div class="actions">
      <button type="button" class="ghost" data-testid="save-as-cancel" onclick={cancel}>{t('saveAs.cancel')}</button>
      <button type="submit" data-testid="save-as-submit">{t('saveAs.choose')}</button>
    </div>
  </form>
</dialog>

<style>
  .save-as-dialog {
    width: min(400px, calc(100vw - 2 * var(--fk-space-4)));
    margin: auto;
    padding: var(--fk-space-4) var(--fk-space-5);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    color: var(--fk-text);
    box-shadow: var(--fk-shadow-float);
  }
  .save-as-dialog::backdrop {
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
  .hint,
  .error {
    margin: 0;
  }
  .hint {
    color: var(--fk-text-muted);
  }
  .error {
    color: var(--fk-danger);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--fk-space-2);
    margin-top: var(--fk-space-2);
  }
</style>
