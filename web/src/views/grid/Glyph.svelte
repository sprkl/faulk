<script lang="ts">
  /**
   * The glyph of a cell state: its shape tells the state apart whatever the colour (GR-3). Idle is
   * an outlined triangle, queued a triangle in a dashed ring, playing a solid triangle, restarting (a playing clip launched again) a bar and a solid triangle, stopping a
   * square in a dashed ring, armed a clock face. `launch` and `stop` are the controls' own shapes.
   */
  export type GlyphKind = 'idle' | 'queued' | 'playing' | 'restarting' | 'stopping' | 'armed' | 'launch' | 'stop'
  let { kind, size = 14 }: { kind: GlyphKind; size?: number } = $props()
</script>

<svg class="glyph" data-glyph={kind} width={size} height={size} viewBox="0 0 16 16" aria-hidden="true" focusable="false">
  {#if kind === 'idle'}
    <path d="M5 3.5 L12.5 8 L5 12.5 Z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
  {:else if kind === 'playing' || kind === 'launch'}
    <path d="M5 3.5 L12.5 8 L5 12.5 Z" fill="currentColor" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
  {:else if kind === 'restarting'}
    <path d="M3.4 3.5 V12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
    <path d="M6.6 3.5 L13.4 8 L6.6 12.5 Z" fill="currentColor" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
  {:else if kind === 'queued'}
    <circle cx="8" cy="8" r="6.6" fill="none" stroke="currentColor" stroke-width="1.3" stroke-dasharray="2.4 2" />
    <path d="M6.3 5.2 L10.8 8 L6.3 10.8 Z" fill="currentColor" stroke="currentColor" stroke-width="1" stroke-linejoin="round" />
  {:else if kind === 'stopping'}
    <circle cx="8" cy="8" r="6.6" fill="none" stroke="currentColor" stroke-width="1.3" stroke-dasharray="2.4 2" />
    <rect x="5.4" y="5.4" width="5.2" height="5.2" fill="currentColor" />
  {:else if kind === 'armed'}
    <circle cx="8" cy="8" r="6.4" fill="none" stroke="currentColor" stroke-width="1.5" />
    <path d="M8 4.4 V8 L10.4 9.6" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
  {:else}
    <rect x="3.5" y="3.5" width="9" height="9" rx="1" fill="currentColor" />
  {/if}
</svg>

<style>
  .glyph {
    flex: none;
    display: block;
  }
</style>
