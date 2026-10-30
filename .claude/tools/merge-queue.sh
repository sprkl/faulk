#!/usr/bin/env bash
# Usage: bash .claude/tools/merge-queue.sh <pr> [<pr>...]   (lead only, gh + git, no build)
# Merge PRs one at a time. A PR is merged only once its CI ran on the current develop:
# if develop moved since, update the branch and wait for the PR's CI. No wait on develop's CI
# after the merge (founder, 2026-10-24: no test rerun after a push). Stop on conflict or failure.
cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)" || exit 1
for pr in "$@"; do
  for i in $(seq 1 20); do
    st=$(gh pr view $pr --json mergeable,mergeStateStatus -q '.mergeable+" "+.mergeStateStatus')
    [ "$st" != "UNKNOWN UNKNOWN" ] && break; sleep 10
  done
  case "$st" in "MERGEABLE CLEAN"|"MERGEABLE BEHIND"|"MERGEABLE UNSTABLE") ;; *) echo "STOP $pr: $st"; exit 1;; esac
  git fetch -q origin
  head=$(gh pr view $pr --json headRefName -q .headRefName)
  if ! git merge-base --is-ancestor origin/develop "origin/$head"; then
    echo "$pr: behind develop, updating the branch"
    gh pr update-branch $pr >/dev/null || { echo "STOP $pr: update-branch failed"; exit 1; }
    sleep 30
  fi
  gh pr checks $pr --watch >/dev/null 2>&1
  if gh pr checks $pr | awk -F'\t' '{print $2}' | grep -qvE '^(pass|skipping)$'; then
    echo "STOP $pr: PR checks not green"; gh pr checks $pr; exit 1
  fi
  gh pr merge $pr --merge >/dev/null 2>&1 || { echo "STOP $pr: merge failed"; exit 1; }
  echo "$pr merged -> $(git ls-remote origin refs/heads/develop | cut -c1-7)"
done
echo "ALL DONE"
