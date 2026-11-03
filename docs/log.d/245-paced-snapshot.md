2026-11-03
* **Update**: a big project's snapshot is sent in pieces as the engine consumes it, on the web (`push-queue.ts`) and natively (`engine::host::StreamQueue`, the helper and the render harness); both bound what waits behind the stream in progress. `specs/02-decoupage-mvp.md` (PR-3 note) and `agents/verification.md` follow (#245).
