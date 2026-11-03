#!/usr/bin/env bash
# Usage: bash .claude/tools/merge-queue.sh <pr> [<pr>...]   (lead only, gh + git, no build)
# Squash-merge PRs one at a time, one commit per task on develop (founder, 2026-11-03, #590). A PR is
# merged only once its CI ran on the current develop:
# if develop moved since, update the branch and wait for the PR's CI on the new head. No wait on
# develop's CI after the merge (founder, 2026-10-24: no test rerun after a push). Stop on conflict,
# on a failed check, and when no check is reported at all. When develop and the PR both changed the
# generated files (web/src/lib/generated/), a branch update would leave them stale or in conflict
# (#426): stop before updating, whatever the merge state, and name regen-generated.sh, the one tool
# of this folder that builds. This script itself never builds.
# The commit text is English (repo rule) while PR titles are French: the PR body carries a section
# `## Commit message` with one fenced block, written by the agent that opened the PR (first line: the
# subject, without the PR number; the rest: the body). The squash commit is that subject + " (#<pr>)",
# then that body, a "Closes #<n>" line per issue the PR closes, and the Co-Authored-By and
# Claude-Session trailers of the branch's commits. A PR without the section stops before anything moves.
cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)" || exit 1
gen=web/src/lib/generated
for pr in "$@"; do
  for i in $(seq 1 20); do
    st=$(gh pr view "$pr" --json mergeable,mergeStateStatus -q '.mergeable+" "+.mergeStateStatus')
    [ "$st" != "UNKNOWN UNKNOWN" ] && break; sleep 10
  done
  git fetch -q origin
  head=$(gh pr view "$pr" --json headRefName -q .headRefName)
  git rev-parse --verify -q "origin/$head" >/dev/null || { echo "STOP $pr: origin/$head not found (a fork branch?)"; exit 1; }
  # The first `## Commit message` section, its first fenced block (CRLF tolerated).
  msg=$(gh pr view "$pr" --json body -q .body | tr -d '\r' | awk '
    !sec && /^##+[ \t]+Commit message[ \t]*$/ { sec = 1; next }
    sec && !inb && /^```/ { inb = 1; next }
    sec && !inb && /^#/ { exit }
    inb && /^```/ { exit }
    inb { print }')
  subject=$(printf '%s\n' "$msg" | sed -n '1{s/^[[:space:]]*//;s/[[:space:]]*$//;p;}')
  if [ -z "$subject" ]; then
    echo "STOP $pr: no English commit message: add to the PR body a '## Commit message' section with one fenced block"
    echo "(first line: the commit subject, without the PR number; then a blank line and the body), e.g. gh pr edit $pr --body-file <file>."
    exit 1
  fi
  case "$subject" in *"(#$pr)") ;; *) subject="$subject (#$pr)" ;; esac
  # The body, built now from what was read once (an edit of the PR during the CI wait is not taken):
  # the section's body without trailer lines, the issues the PR closes, then every trailer once
  # (the section's and the branch's commits'), last so that git reads them as trailers.
  trailer='^(Co-Authored-By|Claude-Session):'
  text=$(printf '%s\n' "$msg" | sed '1d' | grep -viE "$trailer" | sed '/./,$!d')
  closes=$(gh pr view "$pr" --json closingIssuesReferences -q '.closingIssuesReferences[].number' | tr -d '\r' | while read -r n; do
    if [ -n "$n" ] && ! printf '%s\n' "$text" | grep -qiE "^(close[sd]?|fix(e[sd])?|resolve[sd]?) #$n([^0-9]|$)"; then echo "Closes #$n"; fi
  done)
  trailers=$( { printf '%s\n' "$msg" | grep -iE "$trailer"; git log --format='%(trailers:key=Co-Authored-By,key=Claude-Session)' "origin/develop..origin/$head"; } | tr -d '\r' | grep -iE "$trailer" | awk '!seen[tolower($0)]++')
  body=$(for part in "$text" "$closes" "$trailers"; do if [ -n "$part" ]; then printf '%s\n\n' "$part"; fi; done)
  if ! git merge-base --is-ancestor origin/develop "origin/$head"; then
    base=$(git merge-base origin/develop "origin/$head")
    if [ -n "$(git diff --name-only "$base" origin/develop -- "$gen")" ] && [ -n "$(git diff --name-only "$base" "origin/$head" -- "$gen")" ]; then
      echo "STOP $pr: $st; develop and the PR both changed $gen, a branch update would leave it stale or in conflict."
      echo "Regenerate (this builds Rust): bash .claude/tools/regen-generated.sh $pr"
      echo "then run this script again."
      exit 1
    fi
  fi
  # BLOCKED: develop's branch protection (required checks) blocks the merge while they run or fail;
  # the checks watch below decides, and a refused `gh pr merge` still stops (#458).
  case "$st" in "MERGEABLE CLEAN"|"MERGEABLE BEHIND"|"MERGEABLE UNSTABLE"|"MERGEABLE BLOCKED") ;; *) echo "STOP $pr: $st"; exit 1;; esac
  if ! git merge-base --is-ancestor origin/develop "origin/$head"; then
    echo "$pr: behind develop, updating the branch"
    old=$(gh pr view "$pr" --json headRefOid -q .headRefOid)
    gh pr update-branch "$pr" >/dev/null || { echo "STOP $pr: update-branch failed"; exit 1; }
    # Wait for the new head, then for a workflow run on it, so the watch below cannot accept the old run.
    for i in $(seq 1 30); do
      sha=$(gh pr view "$pr" --json headRefOid -q .headRefOid)
      [ "$sha" != "$old" ] && break; sleep 10
    done
    [ "$sha" != "$old" ] || { echo "STOP $pr: head did not move after update-branch"; exit 1; }
  fi
  # A workflow run on the current head, whether the branch was just updated here or pushed by regen-generated.sh a moment ago.
  sha=$(gh pr view "$pr" --json headRefOid -q .headRefOid)
  for i in $(seq 1 30); do
    runs=$(gh run list --commit "$sha" --json databaseId -q 'length')
    [ "${runs:-0}" -gt 0 ] && break; sleep 10
  done
  [ "${runs:-0}" -gt 0 ] || { echo "STOP $pr: no CI run started on $sha"; exit 1; }
  gh pr checks "$pr" --watch >/dev/null 2>&1
  checks=$(gh pr checks "$pr" --json name,bucket -q '.[] | .name + "\t" + .bucket') || { echo "STOP $pr: could not read checks"; exit 1; }
  if [ -z "$checks" ] || printf '%s\n' "$checks" | cut -f2 | grep -qvE '^(pass|skipping)$'; then
    echo "STOP $pr: PR checks not green (or none reported)"; gh pr checks "$pr"; exit 1
  fi
  # The checks branch protection requires on develop: they must have reported, not just be absent.
  # Protection does not bind admins (enforce_admins off), so this script checks them itself (#458).
  for req in rust web; do
    printf '%s\n' "$checks" | grep -qE "^$req	(pass|skipping)$" || { echo "STOP $pr: required check '$req' did not report (its workflow may not have started yet: run the queue again)"; exit 1; }
  done
  # --match-head-commit: the head whose checks were judged; a push since makes GitHub refuse the merge.
  printf '%s\n' "$body" | gh pr merge "$pr" --squash --match-head-commit "$sha" --subject "$subject" --body-file - >/dev/null 2>&1 || { echo "STOP $pr: merge failed (the head moved since its checks, or protection still blocks)"; exit 1; }
  echo "$pr merged -> $(git ls-remote origin refs/heads/develop | cut -c1-7)"
done
echo "ALL DONE"
