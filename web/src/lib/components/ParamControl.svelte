<script lang="ts">
  /**
   * The control of one device parameter, generated from its descriptor: a choice among named values
   * (a segmented control) or a knob. It names no device and no parameter: the label and the tooltip
   * come from the catalogue (`<kind>.<key>`, `<kind>.<key>.hint`), and stand in with the parameter's
   * key and the descriptor's description when the catalogue has none.
   */
  import { choices, deviceText, formatValue, isChoice, labelKey, positionOf, valueAt, type DeviceDescriptor, type ParamDescriptor } from '../devices'
  import { useStores } from '../stores'
  import { parseParam as parseValue } from '../value-entry'
  import Knob from './Knob.svelte'

  let {
    device,
    param,
    value,
    onchange,
    onbegin,
    onend,
  }: {
    device: DeviceDescriptor
    param: ParamDescriptor
    value: number
    /** The new value in the parameter's unit. */
    onchange: (value: number) => void
    /** A drag starts and ends (knobs only): the owner groups it as one history entry. */
    onbegin?: () => void
    onend?: () => void
  } = $props()

  const { ui } = useStores()

  /**
   * Glyphs of the common waveform values, by the value's key; a value without one shows its name.
   * Display only: nothing here knows a device.
   */
  const ICONS: Record<string, string> = {
    sine: 'M2 10 C 6 0, 10 0, 12 10 S 18 20, 22 10',
    saw: 'M2 16 L 12 4 L 12 16 L 22 4',
    square: 'M2 16 L 2 4 L 12 4 L 12 16 L 22 16 L 22 4',
    triangle: 'M2 16 L 7 4 L 12 16 L 17 4 L 22 16',
  }

  // The value the entry field opened on.
  // svelte-ignore state_referenced_locally
  let openedAt = value

  const key = $derived(labelKey(device, param.key))
  const label = $derived(deviceText(ui.lang, key) ?? param.key)
  const hint = $derived(deviceText(ui.lang, `${key}.hint`) ?? param.description)
  const valueLabel = (valueKey: string) => deviceText(ui.lang, labelKey(device, valueKey)) ?? valueKey
</script>

{#if isChoice(param)}
  <div class="choice" data-testid="param-{param.key}" title={hint}>
    <div role="radiogroup" aria-label={label} class="segmented">
      {#each choices(param) as choice (choice.value)}
        <button
          role="radio"
          aria-checked={value === choice.value}
          aria-label={valueLabel(choice.key)}
          title={valueLabel(choice.key)}
          onclick={() => onchange(choice.value)}
        >
          {#if ICONS[choice.key]}
            <svg width="24" height="20" viewBox="0 0 24 20" aria-hidden="true"
              ><path d={ICONS[choice.key]} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" /></svg
            >
          {:else}
            {valueLabel(choice.key)}
          {/if}
        </button>
      {/each}
    </div>
  </div>
{:else}
  <div data-testid="param-{param.key}">
    <Knob
      {label}
      {hint}
      {onbegin}
      {onend}
      position={positionOf(param, value)}
      display={formatValue(param, value)}
      home={positionOf(param, param.default)}
      defaultDisplay={formatValue(param, param.default)}
      entry={{
        text: () => {
          // A bare number is in the unit the field opened with, even if the value moves while it is open.
          openedAt = value
          return formatValue(param, value)
        },
        commit: (text) => {
          const next = parseValue(param, text, openedAt)
          if (next === null) return false
          // The same value: no command, no empty history entry.
          if (next !== value) onchange(next)
          return true
        },
      }}
      onchange={(p) => {
        const next = p === positionOf(param, param.default) ? param.default : valueAt(param, p)
        if (next !== value) onchange(next)
      }}
      onreset={() => {
        // Already at the default: no command, no empty history entry.
        if (value !== param.default) onchange(param.default)
      }}
    />
  </div>
{/if}

<style>
  .choice {
    display: flex;
  }
  .segmented {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 2px;
    padding: 2px;
    border-radius: var(--fk-radius-pill);
    background: var(--fk-bg);
    border: 1px solid var(--fk-line);
  }
  button {
    min-height: var(--fk-control);
    border: 0;
    padding: 0 var(--fk-space-3);
    border-radius: var(--fk-radius-pill);
    background: transparent;
    color: var(--fk-text-muted);
    cursor: pointer;
  }
  button:hover {
    background: var(--fk-surface-2);
  }
  button[aria-checked='true'] {
    background: var(--fk-accent-tint);
    color: var(--fk-accent-fg);
  }
</style>
