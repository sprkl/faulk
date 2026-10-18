# Log fragments

A pull request that changes `docs/` does not edit `docs/log.md`: that file conflicted on almost every merge. It adds one file here instead, `docs/log.d/<issue>-<slug>.md`.

A fragment holds the dated log entry of the pull request, in the same format as a bullet of `docs/log.md`, with the date (`YYYY-MM-DD`) as the first line:

```
2026-10-14
* **Update** (#176): what changed and why, in one bullet.
```

The lead folds the fragments into `docs/log.md` (one dated section per day, newest first) and deletes them, when it prepares a release from `develop` to `main`, or whenever it wants, in a commit of its own and never in a task's pull request. A second pull request on the same issue uses another slug. Fragments and this README are exempt from the frontmatter rule. Until then, a fragment is the entry; do not add it to `docs/log.md` as well.
