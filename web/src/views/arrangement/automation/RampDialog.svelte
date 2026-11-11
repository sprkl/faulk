<script lang="ts">
  /**
   * "Ramp to…" on an automation lane (#769): a position (a bar, or a bar and a beat, as "Go to" reads it) and a value in the
   * parameter's unit (the grammar of the point field). The view turns them into one `automation.points.restore`: a straight
   * ramp from the lane's value at the playhead. A native modal dialog like "Go to": the page behind is inert, the focus stays
   * inside, Escape closes it and the view gives the focus back. A text that means nothing is said, not guessed.
   */
  import { tick } from 'svelte'
  import { parseBarBeat, type Signature } from '../../transport/time'
  import type { Axis, AxisEntry } from './model'
  import { useStores } from '../../../lib/stores'

  let {
    open = false,
    title,
    from,
    axis,
    signature,
    headTick,
    initialPosition,
    initialValue,
    onsubmit,
    onclose,
  }: {
    open: boolean
    /** The lane's title ("Filter › Cutoff"). */
    title: string
    /** What the ramp starts from, said above the fields: the value at the playhead and where. */
    from: string
    axis: Axis | null
    signature: Signature
    /** The playhead's tick: a ramp ends elsewhere. */
    headTick: number
    initialPosition: string
    initialValue: string
    onsubmit: (tick: number, entry: AxisEntry) => void
    onclose: () => void
  } = $props()

  const { ui } = useStores()
  const t = ui.t

  let dialog = $state<HTMLDialogElement | null>(null)
  let positionField = $state<HTMLInputElement | null>(null)
  let valueField = $state<HTMLInputElement | null>(null)
  let position = $state('')
  let value = $state('')
  let positionError = $state<string | null>(null)
  let valueError = $state<string | null>(null)

  $effect(() => {
    if (!dialog) return
    if (open && !dialog.open) {
      position = initialPosition
      value = initialValue
      positionError = null
      valueError = null
      dialog.showModal()
      // Once the fields hold their texts (the DOM is written after this effect), the position is focused and selected.
      void tick().then(() => {
        positionField?.focus()
        positionField?.select()
      })
    } else if (!open && dialog.open) dialog.close()
  })

  function submit(event: SubmitEvent) {
    event.preventDefault()
    if (!axis) return onclose()
    const tick = parseBarBeat(position, signature)
    positionError = tick === null ? t('automation.rampBadPosition', { beats: signature.numerator }) : tick === headTick ? t('automation.rampSame') : null
    const entry = axis.parse(value)
    valueError = entry === null ? t('automation.rampBadValue', { example: axis.format(axis.clamp((axis.min + axis.max) / 2)) }) : null
    if (positionError) return positionField?.focus()
    if (valueError || !entry || tick === null) return valueField?.focus()
    onsubmit(tick, entry)
  }

  /** A click on the backdrop (the dialog element itself, outside its box) closes it. */
  function onBackdrop(event: MouseEvent) {
    if (event.target === dialog) onclose()
  }
</script>

<!-- svelte-ignore a11y_no_redundant_roles -->
<dialog
  bind:this={dialog}
  class="ramp-dialog"
  role="dialog"
  aria-modal={open ? 'true' : undefined}
  aria-labelledby="ramp-title"
  aria-describedby="ramp-from"
  data-testid="ramp-dialog"
  {onclose}
  onclick={onBackdrop}
>
  <form onsubmit={submit}>
    <h2 id="ramp-title">{t('automation.rampTitle', { title })}</h2>
    <p id="ramp-from" class="from">{from}</p>
    <label for="ramp-position">{t('automation.rampPosition', { beats: signature.numerator })}</label>
    <input
      id="ramp-position"
      bind:this={positionField}
      bind:value={position}
      type="text"
      inputmode="decimal"
      autocomplete="off"
      placeholder="12.3"
      aria-invalid={positionError ? 'true' : undefined}
      aria-describedby={positionError ? 'ramp-position-error' : undefined}
      data-testid="ramp-position"
      oninput={() => (positionError = null)}
    />
    {#if positionError}<p id="ramp-position-error" class="error" role="alert" data-testid="ramp-position-error">{positionError}</p>{/if}
    <label for="ramp-value">{t('automation.rampValue')}</label>
    <input
      id="ramp-value"
      bind:this={valueField}
      bind:value
      type="text"
      inputmode="decimal"
      autocomplete="off"
      aria-invalid={valueError ? 'true' : undefined}
      aria-describedby={valueError ? 'ramp-value-error' : undefined}
      data-testid="ramp-value"
      oninput={() => (valueError = null)}
    />
    {#if valueError}<p id="ramp-value-error" class="error" role="alert" data-testid="ramp-value-error">{valueError}</p>{/if}
    <div class="actions">
      <button type="button" class="ghost" onclick={onclose}>{t('automation.rampCancel')}</button>
      <button type="submit" data-testid="ramp-submit">{t('automation.rampGo')}</button>
    </div>
  </form>
</dialog>

<style>
  .ramp-dialog {
    width: min(380px, calc(100vw - 2 * var(--fk-space-4)));
    margin: auto;
    padding: var(--fk-space-4) var(--fk-space-5);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-card);
    background: var(--fk-surface);
    color: var(--fk-text);
    box-shadow: var(--fk-shadow-float);
  }
  .ramp-dialog::backdrop {
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
  .from {
    margin: 0;
    color: var(--fk-text-muted);
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
