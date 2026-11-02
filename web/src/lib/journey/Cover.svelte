<script lang="ts">
  import { ART_SIZE, artOf } from './art'
  import type { JourneyEntry } from './types'

  /** The entry's generated cover (`art.ts`), prerendered as SVG; `id` names its stage for the player. */
  let { entry, id, label }: { entry: JourneyEntry; id: string; label: string } = $props()

  const art = $derived(artOf(entry))
  const o = $derived(art.object)
  /** The foci, for the visualiser that takes over the cover while the track plays (`live/viz.ts`). */
  const foci = $derived(JSON.stringify(art.foci.map((f) => ({ x: f.x, y: f.y, r: f.r, c: f.color }))))
  const tilePath = $derived.by(() => {
    const s = o.size * 0.92
    return `M ${o.x - s} ${o.y - s} h ${s * 2} v ${s * 2} h ${-s * 2} Z`
  })
</script>

<div class="cover lp-grain" {id} data-fk-stage data-foci={foci}>
  <svg viewBox="0 0 {ART_SIZE} {ART_SIZE}" role="img" aria-label={label}>
    <defs>
      <filter id="{id}-blur" x="-50%" y="-50%" width="200%" height="200%">
        <feGaussianBlur stdDeviation="28" />
      </filter>
      {#each art.foci as f, i (i)}
        <radialGradient id="{id}-f{i}">
          <stop offset="0" stop-color={f.color} stop-opacity={f.opacity} />
          <stop offset="1" stop-color={f.color} stop-opacity="0" />
        </radialGradient>
      {/each}
      <radialGradient id="{id}-orb" cx="0.42" cy="0.4" r="0.62">
        <stop offset="0" stop-color={o.colors[0]} />
        <stop offset="0.35" stop-color={o.colors[1]} />
        <stop offset="1" stop-color={o.colors[2]} />
      </radialGradient>
      <linearGradient id="{id}-tile" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color={o.colors[0]} />
        <stop offset="0.5" stop-color={o.colors[1]} />
        <stop offset="1" stop-color={o.colors[2]} />
      </linearGradient>
    </defs>
    <rect width={ART_SIZE} height={ART_SIZE} fill="#131211" />
    <g filter="url(#{id}-blur)">
      {#each art.foci as f, i (i)}
        <circle cx={f.x} cy={f.y} r={f.r} fill="url(#{id}-f{i})" />
      {/each}
    </g>
    <g class="rings" fill="none">
      {#each art.rings as ring, i (i)}
        <circle cx={o.x} cy={o.y} r={ring.r} stroke="#ece9e6" stroke-opacity={ring.opacity} />
      {/each}
    </g>
    <g class="object" transform="rotate({o.rotation} {o.x} {o.y})">
      {#if o.kind === 'orb'}
        <circle cx={o.x} cy={o.y} r={o.size} fill="url(#{id}-orb)" />
      {:else if o.kind === 'ring'}
        {#each [0, 1, 2, 3, 4] as k (k)}
          <circle cx={o.x} cy={o.y} r={o.size * (1 - k * 0.19)} fill="none" stroke={o.colors[1]} stroke-width={7 - k} stroke-opacity={1 - k * 0.16} />
        {/each}
        <circle cx={o.x} cy={o.y} r={o.size * 0.16} fill={o.colors[0]} />
      {:else}
        <path d={tilePath} fill="url(#{id}-tile)" stroke-linejoin="round" stroke={o.colors[1]} stroke-width="22" />
      {/if}
    </g>
    <g class="form" fill="#ece9e6">
      {#each art.form as d, i (i)}
        <circle cx={d.x} cy={d.y} r={d.r} fill-opacity={d.opacity} />
      {/each}
    </g>
  </svg>
</div>

<style>
  .cover {
    aspect-ratio: 1;
    border-radius: 20px;
    background: var(--fk-ink);
  }
  svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  /* The live visualiser covers the drawing while it plays, under the grain. */
  .cover :global(.fk-live-canvas) {
    position: absolute;
    inset: 0;
    z-index: 1;
    width: 100%;
    height: 100%;
    opacity: 0;
    transition: opacity 600ms cubic-bezier(0.2, 0.7, 0, 1);
    pointer-events: none;
  }
  .cover:global(.is-live) :global(.fk-live-canvas) {
    opacity: 1;
  }
</style>
