2026-10-26
* **Update** (#359): `agents/verification.md` describes the clock check of the render-thread timings. In strict mode a run whose counter changed pace is taken again (4 attempts), then fails by name. Outside strict mode it is taken again once, then the last run is kept with a note and held to the 10x ceilings, because shared CI runners flip pace on every attempt. Budgets untouched.
