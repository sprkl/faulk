2026-11-01

- **Update** (#432): the history panel draws the 200 newest rows and adds 200 per click on "Show older entries"; on a 10 000-entry history with the tab open an edit cost a median 217 ms per frame (drawing the rows; `pw_history` itself is 3.5 ms) and now costs 16.8 ms. The history keeps no cap and no `pw_*` export changes. Test: a group join keeps an entry `id` and `at`.
- **Note** (#432): the history itself stays unbounded in memory (no cap: product decision); only the panel is windowed, and "Show older entries" is an explicit request for more rows.
