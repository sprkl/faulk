<script lang="ts">
  import Journey from '../../../../lib/journey/Journey.svelte'
  import type { PageProps } from './$types'

  let { data }: PageProps = $props()
</script>

<Journey lang={data.lang} view={{ kind: 'day', day: data.day }} />
