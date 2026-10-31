#!/usr/bin/env bash
# Usage: bash .claude/tools/regen-generated.sh <pr>   (lead only; BUILDS: pnpm install + a Rust build)
# Brings a PR branch up to date with develop when merge-queue.sh stopped because both sides changed
# web/src/lib/generated/ (#426): in a temporary worktree, merges origin/develop into the PR branch,
# takes the branch's copy of the generated files where they conflict (any other conflict stops the
# tool), regenerates them (`pnpm gen:protocol`, built into the shared target directory of
# scripts/agent/verify-core.mjs so a cold cargo build is avoided), commits the merge with the hooks
# and pushes it to the PR branch without force. Then run merge-queue.sh again: the branch is no longer
# behind, so it waits for the CI of the new head. Same-repository branches only.
set -euo pipefail
pr=${1:?usage: regen-generated.sh <pr>}
root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cd "$root"
gen=web/src/lib/generated
head=$(gh pr view "$pr" --json headRefName -q .headRefName)
git fetch -q origin
if git merge-base --is-ancestor origin/develop "origin/$head"; then
  echo "$pr: $head already contains develop, nothing to do"; exit 0
fi
if [ -z "${CARGO_TARGET_DIR:-}" ]; then
  CARGO_TARGET_DIR=$(node --input-type=module -e "
    import { homedir } from 'node:os'
    import { sharedTargetDir } from '$root/scripts/agent/verify-core.mjs'
    console.log(sharedTargetDir(process.env, process.platform, homedir()) ?? '')")
  [ -n "$CARGO_TARGET_DIR" ] && export CARGO_TARGET_DIR || unset CARGO_TARGET_DIR
fi
export CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-0}
wt=$(mktemp -d)
cleanup() { git -C "$root" worktree remove --force "$wt" >/dev/null 2>&1 || true; rm -rf "$wt"; git -C "$root" worktree prune; }
trap cleanup EXIT
git worktree add -q --detach "$wt" "origin/$head"
cd "$wt"
if ! git merge --no-ff --no-commit origin/develop >/dev/null; then
  conflicts=$(git diff --name-only --diff-filter=U)
  if [ -z "$conflicts" ] || echo "$conflicts" | grep -qv "^$gen/"; then
    echo "STOP $pr: conflicts outside $gen, resolve them by hand:"; echo "$conflicts"
    git merge --abort || true; exit 1
  fi
  echo "$conflicts" | while read -r f; do git checkout --ours -- "$f" && git add -- "$f"; done
fi
(cd web && pnpm install --frozen-lockfile && pnpm gen:protocol)
git add -A -- "$gen"
git commit -q -m "Merge branch 'develop' into $head (regenerated protocol files)"
git push -q origin "HEAD:refs/heads/$head" || { echo "STOP $pr: push refused (the branch moved?); run the tool again"; exit 1; }
echo "$pr: $head updated with develop and regenerated -> $(git rev-parse --short HEAD)"
