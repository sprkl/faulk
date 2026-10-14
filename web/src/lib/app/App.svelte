<script lang="ts">
  // The DAW's shell: shows the current view of the registry. Views live in `src/views/`.
  import { DEFAULT_VIEW, viewById } from '../../views/registry'

  const View = viewById(DEFAULT_VIEW)!.component
</script>

<View />
