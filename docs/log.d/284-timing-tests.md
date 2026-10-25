2026-10-24
* **Update** (#284): the e2e frame-rate checks of the launch grid, the mixer and the piano roll hold the main-thread time per frame, in units of a fixed reference workload and as the best of three windows, instead of wall-clock frame intervals (strict mode only); the keyboard test keeps a drum hit's crest; method in `agents/verification.md` ("Timing budgets of the e2e specs").
