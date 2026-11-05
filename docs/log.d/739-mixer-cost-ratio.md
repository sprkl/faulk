2026-11-05
* **Update** (#739): the `@full` render-cost test of `mixer.spec.ts` no longer reads the machine's load: it counts the style writes per second of the moving mixer (load-independent) and keeps a wide ceiling on the renderer's CPU time against the grid view, instead of an absolute figure in reference units (`agents/verification.md`).
