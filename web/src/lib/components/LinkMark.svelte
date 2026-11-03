<script lang="ts">
  /**
   * The mark of a linked clip (#473): a clip used in more than one place (placements, a launch-grid slot), so that
   * editing it changes them all. A copy is another clip and has none. Decorative: the accessible name of what carries
   * it says "linked". The same two links as the arrangement draws on its canvas.
   */
</script>

<svg class="link" width="12" height="8" viewBox="0 0 12 8" aria-hidden="true" data-testid="link-mark">
  <rect x="0.75" y="1.5" width="6" height="5" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.25" />
  <rect x="5.25" y="1.5" width="6" height="5" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.25" />
</svg>

<style>
  .link {
    flex: none;
    opacity: 0.8;
  }
</style>
