<script lang="ts">
  /**
   * The field that asks for a whole number of bars: "Repeat until" (#555, the bar a clip is repeated up to) and "Insert
   * bars" (#575, how many). A native modal dialog like the "Go to" field: the page behind is inert, the focus stays inside,
   * Escape closes it and the focus goes back to what opened it. It only reads a number from `min` to `max`; the
   * arrangement turns it into a command. `name` prefixes the test ids and the element ids (`<name>-dialog`, `-field`,
   * `-error`, `-submit`).
   */
  /** `initial` fills the field; `invalid` is the sentence shown when the text is not a whole number in range; `onsubmit` gets the number. */
  let {
    open = false,
    name,
    title,
    label,
    invalid: invalidText,
    go,
    cancel,
    initial,
    min,
    max,
    onsubmit,
    onclose,
  }: {
    open: boolean
    name: string
    title: string
    label: string
    invalid: string
    go: string
    cancel: string
    initial: number
    min: number
    max: number
    onsubmit: (value: number) => void
    onclose: () => void
  } = $props()

  let dialog = $state<HTMLDialogElement | null>(null)
  let field = $state<HTMLInputElement | null>(null)
  let text = $state('')
  let invalid = $state(false)

  $effect(() => {
    if (!dialog) return
    if (open && !dialog.open) {
      text = String(initial)
      invalid = false
      dialog.showModal()
      field?.focus()
      field?.select()
    } else if (!open && dialog.open) dialog.close()
  })

  function submit(event: SubmitEvent) {
    event.preventDefault()
    const value = /^\d{1,6}$/.test(text.trim()) ? Number(text.trim()) : NaN
    if (!(value >= min && value <= max)) {
      invalid = true
      field?.focus()
      return
    }
    onsubmit(value)
  }

  /** A click on the backdrop (the dialog element itself, outside its box) closes it. */
  function onBackdrop(event: MouseEvent) {
    if (event.target === dialog) onclose()
  }
</script>

<!-- svelte-ignore a11y_no_redundant_roles -->
<dialog
  bind:this={dialog}
  class="bar-dialog"
  role="dialog"
  aria-modal={open ? 'true' : undefined}
  aria-labelledby="{name}-title"
  data-testid="{name}-dialog"
  {onclose}
  onclick={onBackdrop}
>
  <form onsubmit={submit}>
    <h2 id="{name}-title">{title}</h2>
    <label for="{name}-field">{label}</label>
    <input
      id="{name}-field"
      bind:this={field}
      bind:value={text}
      type="text"
      inputmode="numeric"
      autocomplete="off"
      aria-invalid={invalid ? 'true' : undefined}
      aria-describedby={invalid ? `${name}-error` : undefined}
      data-testid="{name}-field"
      oninput={() => (invalid = false)}
    />
    {#if invalid}<p id="{name}-error" class="error" role="alert" data-testid="{name}-error">{invalidText}</p>{/if}
    <div class="actions">
      <button type="button" class="ghost" onclick={onclose}>{cancel}</button>
      <button type="submit" data-testid="{name}-submit">{go}</button>
    </div>
  </form>
</dialog>

<style>
  .bar-dialog {
    width: min(360px, calc(100vw - 2 * var(--fk-space-4)));
    margin: auto;
    padding: var(--fk-space-4) var(--fk-space-5);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    color: var(--fk-text);
    box-shadow: var(--fk-shadow-float);
  }
  .bar-dialog::backdrop {
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
