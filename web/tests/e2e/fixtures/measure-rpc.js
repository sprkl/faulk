/* eslint-disable @typescript-eslint/no-unused-expressions -- one arrow function, evaluated by the page */
// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-nocheck: plain JavaScript evaluated in the page by engine-mo2.spec.ts and engine-swap.bench.ts (#283).
// The request/reply link to a Worker or to the worklet's port, as an expression the page
// evaluates: `new Function('return (' + source + ')')()`. Replies are matched to requests in order.
// What it adds to the bare queue of callbacks the harness used to keep (#283):
//  - an `error` reply, or an error event of the Worker, rejects the pending request. The old
//    handler threw inside `onmessage`, where nothing catches it: the awaited promise never
//    settled and the test ran until its timeout, with no message;
//  - a request with no reply within `ms` rejects with the name of the link and of the request,
//    so that a stall names where it is instead of waiting for the test's timeout.
(target, name, ms) => {
  const waiting = []
  let failure = null
  const fail = (error) => {
    failure ??= error
    for (const w of waiting.splice(0)) w.reject(error)
  }
  target.onmessage = (event) => {
    if (event.data.type === 'error') return fail(new Error(`${name}: ${event.data.message}`))
    waiting.shift()?.resolve(event.data)
  }
  target.onerror = (event) => fail(new Error(`${name}: ${event.message ?? 'error event'}`))
  return (message) =>
    new Promise((resolve, reject) => {
      if (failure) return reject(failure)
      const entry = {
        resolve: (data) => (clearTimeout(entry.timer), resolve(data)),
        reject: (error) => (clearTimeout(entry.timer), reject(error)),
        timer: setTimeout(() => {
          waiting.splice(waiting.indexOf(entry), 1)
          reject(new Error(`${name}: no reply to "${message.type}" within ${ms} ms`))
        }, ms),
      }
      waiting.push(entry)
      target.postMessage(message)
    })
}
