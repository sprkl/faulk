2026-10-31
* **Update** (#426): `merge-queue.sh` stops, before `gh pr update-branch`, when `develop` and the pull request both changed `web/src/lib/generated/`, and names the new `regen-generated.sh <pr>` (merge in a temporary worktree, `pnpm gen:protocol`, commit, push); the queue stays build-free, the new tool builds. Tests on throwaway repositories with stub `gh` and `pnpm`.
