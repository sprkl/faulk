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
if ! git rev-parse --verify -q "origin/$head" >/dev/null; then
  echo "STOP $pr: origin/$head not found (a fork branch? only same-repository branches are supported)"; exit 1
fi
if [ -z "${CARGO_TARGET_DIR:-}" ] && [ "${FAULK_SHARED_TARGET:-}" != 0 ]; then
  # The module path is passed as an argument and turned into a URL: a raw `C:/...` path is not an ES module specifier.
  CARGO_TARGET_DIR=$(node --input-type=module -e "
    import { homedir } from 'node:os'
    import { pathToFileURL } from 'node:url'
    const { sharedTargetDir } = await import(pathToFileURL(process.argv[1]).href)
    console.log(sharedTargetDir(process.env, process.platform, homedir()) ?? '')" "$root/scripts/agent/verify-core.mjs")
  if [ -n "$CARGO_TARGET_DIR" ]; then export CARGO_TARGET_DIR; else unset CARGO_TARGET_DIR; fi
fi
export CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-0}
wt=$(mktemp -d)
cleanup() { git -C "$root" worktree remove --force "$wt" >/dev/null 2>&1 || true; rm -rf "$wt"; git -C "$root" worktree prune; }
trap cleanup EXIT
trap 'echo "STOP $pr: a step failed (line $LINENO: $BASH_COMMAND); the branch was not pushed"' ERR
git worktree add -q --detach "$wt" "origin/$head"
cd "$wt"
if ! git merge --no-ff --no-commit origin/develop >/dev/null; then
  conflicts=$(git diff --name-only --diff-filter=U)
  if [ -z "$conflicts" ] || echo "$conflicts" | grep -qv "^$gen/"; then
    echo "STOP $pr: conflicts outside $gen, resolve them by hand:"; echo "$conflicts"
    git merge --abort || true; exit 1
  fi
  for f in $conflicts; do
    git checkout --ours -- "$f" && git add -- "$f" || {
      echo "STOP $pr: cannot take the branch's copy of $f (deleted or renamed on one side?), resolve by hand"
      git merge --abort || true; exit 1
    }
  done
fi
(cd web && pnpm install --frozen-lockfile && pnpm gen:protocol)
git add -A -- "$gen"
git commit -q -m "Merge branch 'develop' into $head (regenerated protocol files)"
git push -q origin "HEAD:refs/heads/$head" || { echo "STOP $pr: push refused (the branch moved?); run the tool again"; exit 1; }
echo "$pr: $head updated with develop and regenerated -> $(git rev-parse --short HEAD)"
