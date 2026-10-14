<script lang="ts">
  /**
   * Faulk's flat monogram (logo track A, DS-0.1). Decorative unless `title` is given.
   * Below 20 px the lower stroke starts lower ("espace élargi") so the gap stays visible.
   */
  let { size = 24, color = 'currentColor', title }: { size?: number; color?: string; title?: string } = $props()

  const lower = $derived(size < 20 ? 'M0 220 H338 C228 270 92 340 4 435 H0 Z' : 'M0 185 H338 C228 245 92 330 4 435 H0 Z')
</script>

<svg
  width={size}
  height={Math.round((size * 435) / 430)}
  viewBox="0 0 430 435"
  role={title ? 'img' : undefined}
  aria-label={title}
  aria-hidden={title ? undefined : 'true'}
>
  <path d="M0 0 H430 C392 82 330 157 245 157 H0 Z" fill={color} />
  <path d={lower} fill={color} />
</svg>
