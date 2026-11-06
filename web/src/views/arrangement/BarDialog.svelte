<script lang="ts">
  /**
   * The field that asks for a whole number of bars: "Repeat until" (#555, the bar a clip is repeated up to) and "Insert
   * bars" (#575, how many); the piano roll's "Repeat until" (#710) reads a bar or a bar and a beat through `parse`. A
   * native modal dialog like the "Go to" field: the page behind is inert, the focus stays inside, Escape closes it and the
   * focus goes back to what opened it. It only reads a number from `min` to `max`; the view that opens it turns it into a
   * command. `name` prefixes the test ids and the element ids (`<name>-dialog`, `-field`,
   * `-error`, `-submit`).
   */
  /**
   * `initial` fills the field (`initialText` instead, when given); `invalid` is the sentence shown when the text is not a
   * value in range; `onsubmit` gets the value. `parse` reads the text (a whole number of up to six digits by default) and
   * returns null for one it refuses; `min` and `max` bound what it returns.
   */
  let {
    open = false,
    name,
    title,
    label,
    invalid: invalidText,
    go,
    cancel,
    initial,
    initialText,
    min,
    max,
    parse = (text: string) => (/^\d{1,6}$/.test(text.trim()) ? Number(text.trim()) : null),
    inputmode = 'numeric',
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
    initialText?: string
    min: number
    max: number
    parse?: (text: string) => number | null
    inputmode?: 'numeric' | 'decimal'
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
      text = initialText ?? String(initial)
      invalid = false
      dialog.showModal()
      field?.focus()
      field?.select()
    } else if (!open && dialog.open) dialog.close()
  })

  function submit(event: SubmitEvent) {
    event.preventDefault()
    const value = parse(text)
    if (value === null || !(value >= min && value <= max)) {
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
      {inputmode}
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
