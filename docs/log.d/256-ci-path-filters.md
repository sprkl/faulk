2026-10-18
* **Update** (#256): CI filters its jobs by changed paths on pull requests (the job always runs, its heavy steps are skipped when no relevant file changed), and the helper trial build no longer runs on a `Cargo.lock` change alone but runs on every push to `develop` and `main`; `docs/agents/verification.md` describes the filters.
