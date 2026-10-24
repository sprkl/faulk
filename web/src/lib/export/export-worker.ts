// The export's Worker (issue #297): runs `renderExport` off the page's thread, as fast as the page
// writes what it makes. The page cancels by terminating it.
import { CHUNKS_IN_FLIGHT, renderExport, type FromExport, type ToExport } from './export-render'

declare const self: {
  onmessage: ((event: MessageEvent<ToExport>) => void) | null
  postMessage(message: FromExport, transfer?: Transferable[]): void
}

let free = CHUNKS_IN_FLIGHT
let waiting: (() => void) | null = null

/** One more chunk is in flight; waits while too many are. */
async function room() {
  free--
  while (free <= 0) await new Promise<void>((resolve) => (waiting = resolve))
}

self.onmessage = (event) => {
  const message = event.data
  if (message.type === 'ack') {
    free++
    waiting?.()
    waiting = null
  } else {
    void renderExport(message, (out, transfer) => self.postMessage(out, transfer), room)
  }
}
