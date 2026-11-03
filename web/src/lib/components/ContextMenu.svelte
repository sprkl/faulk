<script module lang="ts">
  /** Where the focus was before the menu that is open (or was just replaced by a second list) took it. */
  let lastOpener: Element | null = null
</script>

<script lang="ts">
  /**
   * The menu a right click, Shift+F10 or the Menu key opens on a target (#524). One component for the whole app: it sits at
   * the pointer and is kept inside the window; the arrows, Home and End move through the entries, Enter or Space picks one,
   * Escape or Tab closes it, a press elsewhere or losing the window's focus closes it too. The first entry takes the focus
   * when it opens, and the focus goes back to where it was when it closes. Each entry has its shortcut written at the
   * right. It only reports the pick: the owner's `run` sends the command.
   */
  import { onMount, tick } from 'svelte'
  import { clampMenu, nextEnabled, type MenuItem } from '../context-menu'

  let {
    x,
    y,
    items,
    label,
    testid = 'context-menu',
    onclose,
  }: {
    x: number
    y: number
    items: readonly MenuItem[]
    label?: string
    testid?: string
    onclose: () => void
  } = $props()

  let root: HTMLDivElement | undefined = $state()
  let at = $state({ x: 0, y: 0 })
  let placed = $state(false)
  let buttons = $state<HTMLButtonElement[]>([])
  let opener: Element | null = null

  const first = () => nextEnabled(items, -1, 1)

  onMount(() => {
    // A second list opened from the first one inherits its opener: the focus was on the first menu's entry.
    const active = document.activeElement
    opener = active?.closest('[data-context-menu]') ? lastOpener : active
    lastOpener = opener
    // Measured once it is in the page, so that it never leaves the window.
    const box = root!.getBoundingClientRect()
    at = clampMenu(x, y, box.width, box.height, window.innerWidth, window.innerHeight)
    placed = true
    void tick().then(() => buttons[first()]?.focus())
    // A press anywhere else closes it (capture: a control that stops the event must not keep it open).
    const away = (event: PointerEvent) => {
      if (!root?.contains(event.target as Node)) close(false)
    }
    const lost = () => close(false)
    window.addEventListener('pointerdown', away, true)
    window.addEventListener('blur', lost)
    window.addEventListener('resize', lost)
    return () => {
      window.removeEventListener('pointerdown', away, true)
      window.removeEventListener('blur', lost)
      window.removeEventListener('resize', lost)
    }
  })

  /** Closes the menu; `restore` gives the focus back to the element that had it. */
  function close(restore: boolean) {
    const back = opener
    onclose()
    // Unless a second menu took its place (an entry that leads to another list): that one has the focus.
    if (restore && back instanceof HTMLElement && back.isConnected) void tick().then(() => !document.querySelector('[data-context-menu]') && back.focus())
  }

  function pick(item: MenuItem) {
    if (item.disabled) return
    // The menu goes first: the command may rebuild the very element the focus returns to.
    close(true)
    item.run()
  }

  function onKey(event: KeyboardEvent) {
    const current = buttons.findIndex((b) => b === document.activeElement)
    let next: number
    switch (event.key) {
      case 'ArrowDown':
        next = nextEnabled(items, current, 1)
        break
      case 'ArrowUp':
        next = nextEnabled(items, current < 0 ? 0 : current, -1)
        break
      case 'Home':
        next = nextEnabled(items, -1, 1)
        break
      case 'End':
        next = nextEnabled(items, 0, -1)
        break
      case 'Escape':
        event.preventDefault()
        event.stopPropagation()
        close(true)
        return
      case 'Tab':
        // Leaves the menu like any popup; the key keeps its own move.
        close(true)
        return
      default:
        return
    }
    // The menu holds the keys it uses: the page behind does not scroll or move a note.
    event.preventDefault()
    event.stopPropagation()
    if (next >= 0) buttons[next]?.focus()
  }
</script>

<!-- A second right click on the menu opens nothing. -->
<div
  bind:this={root}
  class="menu"
  role="menu"
  aria-label={label}
  data-context-menu
  data-testid={testid}
  style:left="{at.x}px"
  style:top="{at.y}px"
  style:visibility={placed ? 'visible' : 'hidden'}
  tabindex="-1"
  onkeydown={onKey}
  oncontextmenu={(event) => event.preventDefault()}
>
  {#each items as item, i (item.id)}
    {#if item.separator && i > 0}<div class="sep" role="separator"></div>{/if}
    <button
      bind:this={buttons[i]}
      role="menuitem"
      type="button"
      disabled={item.disabled}
      data-testid={item.testid ?? `ctx-${item.id}`}
      onclick={() => pick(item)}
    >
      <span class="label"
        >{#if item.swatch}<span class="swatch" style:background={item.swatch} aria-hidden="true"></span>{/if}{item.label}</span
      >
      {#if item.detail}<span class="detail">{item.detail}</span>{:else if item.shortcut}<kbd>{item.shortcut}</kbd>{/if}
    </button>
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 70;
    min-width: 180px;
    max-width: min(320px, calc(100vw - 16px));
    padding: var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    box-shadow: var(--fk-shadow-float);
    max-height: calc(100vh - 8px);
    overflow-y: auto;
  }
  .menu:focus {
    outline: none;
  }
  button {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: var(--fk-space-4);
    width: 100%;
    border: 0;
    border-radius: var(--fk-radius-panel);
    text-align: left;
  }
  button:disabled {
    opacity: 0.45;
  }
  .label {
    display: inline-flex;
    align-items: center;
    gap: var(--fk-space-2);
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .detail,
  kbd {
    color: var(--fk-text-muted);
    font-family: var(--fk-font-mono);
    font-size: 0.85em;
  }
  .sep {
    height: 1px;
    margin: var(--fk-space-1) 0;
    background: var(--fk-line);
  }
</style>
