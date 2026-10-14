<script lang="ts">
  import Landing from '../../lib/landing/Landing.svelte'
  import type { PageProps } from './$types'

  let { data }: PageProps = $props()
</script>

<Landing lang={data.lang} />
