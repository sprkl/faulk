<script lang="ts">
  /** Two octaves of piano from `low`, lighting the notes that play. Display only. */
  let { low, active }: { low: number; active: number[] } = $props()

  const BLACK = new Set([1, 3, 6, 8, 10])
  const keys = $derived(Array.from({ length: 25 }, (_, i) => low + i))
  const whites = $derived(keys.filter((n) => !BLACK.has(n % 12)))
</script>

<div class="piano" aria-hidden="true">
  {#each whites as note (note)}
    <div class="white" class:on={active.includes(note)}>
      {#if BLACK.has((note + 1) % 12) && note + 1 <= low + 24}
        <div class="black" class:on={active.includes(note + 1)}></div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .piano {
    display: flex;
    height: 64px;
    gap: 2px;
  }
  .white {
    position: relative;
    flex: 1;
    background: var(--fk-surface-2);
    border-radius: 0 0 var(--fk-radius-work) var(--fk-radius-work);
  }
  .black {
    position: absolute;
    right: -30%;
    width: 60%;
    height: 60%;
    background: var(--fk-bg);
    border: 1px solid var(--fk-line);
    z-index: 1;
  }
  .on {
    background: var(--fk-accent);
  }
</style>
