<script lang="ts">
  /**
   * The field a continuous control opens to type a value (`Slider`): a text input pre-filled with the
   * displayed value and selected, placed under the control. Enter applies, Escape cancels, losing the
   * focus applies. An unreadable text changes nothing and the field says so: it stays open, marked
   * invalid, until a readable text, Escape or a press elsewhere. The field takes every key, like the
   * other text fields (the page's shortcuts ignore it, #361). It reads nothing itself: `commit` gets
   * the text and answers whether it was accepted (and sends the command).
   */
  import { onMount } from 'svelte'
  import { useStores } from '../stores'

  let {
    anchor,
    initial,
    label,
    commit,
    onclose,
  }: {
    /** The control the field belongs to: it sits under it. */
    anchor: HTMLElement
    /** The text the field opens with: the displayed value, with its unit. */
    initial: string
    /** What the control is called, for the field's accessible name. */
    label: string
    /** True when the text was accepted. Not called when the text is the one the field opened with. */
    commit: (text: string) => boolean
    /** The field is done; `refocus` when the control should take the focus back (a key ended it, not a click elsewhere). */
    onclose: (refocus: boolean) => void
  } = $props()

  const { ui } = useStores()

  // The field opens with `initial` for good: a value that changes meanwhile (undo, a command from elsewhere) does not rewrite what is typed.
  // svelte-ignore state_referenced_locally
  let text = $state(initial)
  let invalid = $state(false)
  let message = $state(false)
  let messageTimer: ReturnType<typeof setTimeout> | undefined
  let input: HTMLInputElement | undefined = $state()
  let done = false

  // The field is placed once, under the control as it is when the field opens.
  // svelte-ignore state_referenced_locally
  const rect = anchor.getBoundingClientRect()
  const WIDTH = 104
  const left = Math.max(4, Math.min(rect.left + rect.width / 2 - WIDTH / 2, window.innerWidth - WIDTH - 4))
  const top = Math.max(4, Math.min(rect.bottom + 4, window.innerHeight - 64))

  function finish(refocus: boolean) {
    if (done) return
    done = true
    onclose(refocus)
  }

  /** Applies the text; false when it is unreadable (the field stays open and says so). */
  function apply(): boolean {
    // The text the field opened with is no entry: the displayed value is rounded, applying it would overwrite a finer value.
    if (text === initial) return true
    if (commit(text)) return true
    invalid = true
    // The message goes after about two seconds (or at the next keystroke); the field stays marked invalid.
    clearTimeout(messageTimer)
    messageTimer = setTimeout(() => (message = false), 2000)
    message = true
    return false
  }

  function onKeyDown(event: KeyboardEvent) {
    // Every key stays in the field: no shortcut, no ancestor handler (the track row's Enter, the list's Escape). Except with Ctrl or
    // Cmd: Save and the history belong to the page even in a text field (`keyboard.ts`), so they pass.
    if (event.ctrlKey || event.metaKey) return
    event.stopPropagation()
    // A key of an input method's composition (confirming a candidate with Enter) is not the field's.
    if (event.isComposing) return
    if (event.key === 'Enter') {
      event.preventDefault()
      if (apply()) finish(true)
    } else if (event.key === 'Escape') {
      event.preventDefault()
      finish(true)
    }
  }

  function onBlur() {
    if (!done && apply()) finish(false)
  }

  onMount(() => {
    input?.focus()
    input?.select()
    // An unreadable text keeps the field open past the blur; a press elsewhere then drops it.
    const away = (event: PointerEvent) => {
      if (invalid && !(event.target as Element | null)?.closest('[data-value-field]')) finish(false)
    }
    // The same for a key pressed outside the field: an unfocused field must not stay up while the page's keys play notes under it.
    const awayKey = (event: KeyboardEvent) => {
      if (invalid && event.target !== input) finish(false)
    }
    window.addEventListener('pointerdown', away, true)
    window.addEventListener('keydown', awayKey, true)
    return () => {
      clearTimeout(messageTimer)
      window.removeEventListener('pointerdown', away, true)
      window.removeEventListener('keydown', awayKey, true)
    }
  })
</script>

<div class="field" data-value-field data-testid="value-field" style:left="{left}px" style:top="{top}px" style:width="{WIDTH}px">
  <input
    bind:this={input}
    bind:value={text}
    type="text"
    inputmode="decimal"
    autocomplete="off"
    spellcheck="false"
    aria-label={ui.t('controls.field', { name: label })}
    aria-invalid={invalid}
    data-testid="value-input"
    oninput={() => {
      invalid = false
      message = false
    }}
    onkeydown={onKeyDown}
    onblur={onBlur}
  />
  {#if invalid && message}
    <p class="error" role="alert" data-testid="value-error">{ui.t('controls.invalid')}</p>
  {/if}
</div>

<style>
  .field {
    position: fixed;
    z-index: 20;
    padding: var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    box-shadow: var(--fk-shadow-float);
  }
  input {
    width: 100%;
    box-sizing: border-box;
    font-family: var(--fk-font-mono);
    text-align: center;
  }
  input[aria-invalid='true'] {
    outline: 2px solid var(--fk-danger, #d33);
  }
  .error {
    margin: var(--fk-space-1) 0 0;
    font-size: 11px;
    color: var(--fk-danger, #d33);
  }
</style>
