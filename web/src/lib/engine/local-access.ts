/**
 * Local Network Access: Chrome and Edge ask the user before a public page reaches an
 * app on this device, the helper included (`ws://127.0.0.1`). The prompt opens with the
 * first probe, which gives up before the user answers; this lets the page probe again
 * once the user allows.
 *
 * Permission names, newest first: `loopback-network` (Chrome and Edge 145 and later,
 * loopback only) and `local-network-access` (earlier versions). Browsers that know
 * neither name (Firefox, Safari) reject the query: nothing is watched.
 */
export const LOCAL_ACCESS_PERMISSIONS = ['loopback-network', 'local-network-access'] as const

type PermissionsLike = Pick<Permissions, 'query'>

/**
 * Calls `onGranted` each time the permission turns to `granted`, and `onState` with the
 * state once known and at each change. Returns a function that stops watching, safe to call before the query has answered.
 */
export function watchLocalAccess(
  onGranted: () => void,
  permissions: PermissionsLike | undefined = globalThis.navigator?.permissions,
  onState: (state: PermissionState) => void = () => {},
): () => void {
  let stopped = false
  let status: PermissionStatus | null = null
  const onChange = () => {
    if (stopped || !status) return
    onState(status.state)
    if (status.state === 'granted') onGranted()
  }
  void (async () => {
    if (!permissions) return
    for (const name of LOCAL_ACCESS_PERMISSIONS) {
      let found: PermissionStatus
      try {
        found = await permissions.query({ name: name as PermissionName })
      } catch {
        continue
      }
      if (stopped) return
      status = found
      status.addEventListener('change', onChange)
      onState(status.state)
      return
    }
  })()
  return () => {
    stopped = true
    status?.removeEventListener('change', onChange)
  }
}

/**
 * The message to show when no helper answered: the browser's permission explains the
 * failure when it is still to be asked (`prompt`, the prompt may be open) or refused
 * (`denied`). Anything else, or a browser without the permission (`null`), is a plain
 * absent helper.
 */
export function absentHelperMessages(access: PermissionState | null): ('helper.absent' | 'helper.localAccessPending' | 'helper.localAccessBlocked')[] {
  if (access === 'denied') return ['helper.localAccessBlocked']
  if (access === 'prompt') return ['helper.absent', 'helper.localAccessPending']
  return ['helper.absent']
}
