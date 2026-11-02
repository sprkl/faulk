<script lang="ts">
  import Journey from '../../lib/journey/Journey.svelte'
</script>

<!-- `/journey`: the English page, which moves French readers to `/fr/journey`. -->
<Journey lang="en" detect view={{ kind: 'home' }} />
