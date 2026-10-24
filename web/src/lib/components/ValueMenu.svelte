<script lang="ts">
  /**
   * The small menu a right click opens on a continuous control: one item, "Default value", that
   * shows the value it puts back. Escape, a click elsewhere or losing the focus closes it. The
   * item takes the focus when the menu opens, so the keyboard (the context menu key) works.
   */
  import { onMount } from 'svelte'
  import { useStores } from '../stores'

  let { x, y, value, onreset, onclose }: { x: number; y: number; value: string; onreset: () => void; onclose: () => void } = $props()

  const { ui } = useStores()
  let item: HTMLButtonElement | undefined = $state()

  onMount(() => {
    item?.focus()
    // A press anywhere else closes it (capture: a control that stops the event must not keep it open).
    const away = (event: PointerEvent) => {
      if (!(event.target as Element | null)?.closest('[data-value-menu]')) onclose()
    }
    window.addEventListener('pointerdown', away, true)
    window.addEventListener('blur', onclose)
    return () => {
      window.removeEventListener('pointerdown', away, true)
      window.removeEventListener('blur', onclose)
    }
  })

  function onKey(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      onclose()
    } else if (event.key === 'Tab') {
      onclose()
    }
  }
</script>

<div class="menu" role="menu" data-value-menu data-testid="value-menu" style:left="{x}px" style:top="{y}px" tabindex="-1" onkeydown={onKey}>
  <button bind:this={item} role="menuitem" type="button" data-testid="value-menu-default" onclick={onreset}>
    <span>{ui.t('controls.default')}</span>
    <span class="value">{value}</span>
  </button>
</div>

<style>
  .menu {
    position: fixed;
    z-index: 20;
    min-width: 160px;
    padding: var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    box-shadow: var(--fk-shadow-float);
  }
  button {
    display: flex;
    justify-content: space-between;
    gap: var(--fk-space-3);
    width: 100%;
    border: 0;
    border-radius: var(--fk-radius-panel);
    text-align: left;
  }
  .value {
    color: var(--fk-text-muted);
    font-family: var(--fk-font-mono);
  }
</style>
