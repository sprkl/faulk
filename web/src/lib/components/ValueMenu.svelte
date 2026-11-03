<script lang="ts">
  /**
   * The small menu a right click opens on a continuous control: one item, "Default value", that shows the value it puts
   * back. It is the shared context menu (`ContextMenu.svelte`, #524) with that one entry, so it has the same keys: the
   * item takes the focus when the menu opens, Escape, a click elsewhere or losing the focus closes it.
   */
  import { useStores } from '../stores'
  import ContextMenu from './ContextMenu.svelte'

  let { x, y, value, onreset, onclose }: { x: number; y: number; value: string; onreset: () => void; onclose: () => void } = $props()

  const { ui } = useStores()
</script>

<ContextMenu
  {x}
  {y}
  testid="value-menu"
  {onclose}
  items={[{ id: 'default', label: ui.t('controls.default'), detail: value, testid: 'value-menu-default', run: onreset }]}
/>
