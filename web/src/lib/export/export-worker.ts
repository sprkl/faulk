// The export's Worker (issue #297): runs `renderExport` off the page's thread. The page cancels by
// terminating it.
import { renderExport, type FromExport, type ToExport } from './export-render'

declare const self: {
  onmessage: ((event: MessageEvent<ToExport>) => void) | null
  postMessage(message: FromExport, transfer?: Transferable[]): void
}

self.onmessage = (event) => {
  if (event.data.type === 'start') renderExport(event.data, (message, transfer) => self.postMessage(message, transfer))
}
