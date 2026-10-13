---
type: Agent Configuration
title: "Issue tracker: GitHub"
description: "How the skills create, read and structure GitHub issues, and how a task is carried from plan to a reviewed pull request."
---

# Issue tracker: GitHub

Issues for this repo live as GitHub issues in `sprkl/faulk`. Use the `gh` CLI for all operations. Write issue titles, bodies and comments in French.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --json number,title,body,labels,comments`.
- **List issues**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label` and `--state` filters.
- **Make an issue a sub-issue of a parent**: `gh issue create --parent <parent> ...`, or `gh issue edit <parent> --add-sub-issue <child>` afterwards (`gh` 2.94+). Older `gh`: `gh api --method POST repos/sprkl/faulk/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>`.
- **Comment on an issue**: `gh issue comment <number> --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

## Structure: epics, tasks, phases

Structure lives in GitHub's native fields; labels carry triage state only.

- **Issue type**: `Epic` for a body of work that groups tasks, `Task` for a ticket an agent or a human picks up, `Bug` for a defect. Set it at creation: `gh issue create --type Task`.
- **Hierarchy**: every task is a sub-issue of its epic: `gh issue create --type Task --parent <epic>`.
- **Phase**: a milestone titled `Phase <N>`, with the phase goal in the milestone description. Every task carries the milestone of the phase it ships in. Create a missing phase with `gh api repos/sprkl/faulk/milestones -f title="Phase 2" -f description="<goal>"`.
- **Order inside a phase**: native blocking links (`gh issue create --blocked-by 12,13`, or `gh issue edit <n> --add-blocked-by 12`). The next task is any open, unassigned task of the current phase whose blockers are all closed.

## Working on a task

The lead carries a task from plan to a reviewed pull request on its own (`CLAUDE.md` > Working on an issue). It stops for the user only on a decision that is theirs, and never merges.

- **Start**: assign the task (`gh issue edit <n> --add-assignee @me`) and remove `ready-for-agent`. Create the task's branch from an up-to-date `develop`: `feat/<n>-<slug>` (`fix/` for a bug, `spike/` for a spike).
- **Plan**: write the plan to `.scratch/judge-dredd/<n>-plan.md`, run Judge Dredd's `plan` review (`judge-dredd.md`), report its findings with the decision taken on each, and start the work.
- **Deliver**: implement, verify (`verification.md`, `/code-review`), then run Judge Dredd's `delivery` review.
- **Pull request**: push the branch (`git push -u origin <branch>`) and open the pull request against `develop` with `gh pr create --base develop`. Title and description in French: why, what changes, verification (including the reviews and what became of their findings), out of scope, `Closes #<n>`, then the Claude Code attribution.
- **Copilot review**, when enabled on the repository:
  1. Request it: `gh pr edit <pr> --add-reviewer @copilot`.
  2. Wait for it without ending the turn: poll `gh api repos/sprkl/faulk/pulls/<pr>/reviews` in the background until a review by `copilot-pull-request-reviewer[bot]` newer than the last push arrives. Its findings are the inline comments: `gh api repos/sprkl/faulk/pulls/<pr>/comments`.
  3. Settle each finding: fix it with a test that fails without the fix, or reject it with a reason. Reply on its thread in French: `gh api repos/sprkl/faulk/pulls/<pr>/comments/<id>/replies -f body="..."`.
  4. Push the fixes as new commits, update the pull request description, request a new review.
  5. Stop when a round brings nothing to fix, or after three rounds.
- **End**, before telling the user the task is finished:
  1. Tick in the issue body every acceptance criterion that is met and verified. Leave the others unticked and explain why.
  2. Post a report comment in French: branch, commits and pull request, decisions taken, verification, review findings and what became of them, deviations from the ticket, out-of-scope defects with the issues created for them.
- **Close** on merge into `develop` through `Closes #<n>`. The user merges.

## Pull requests as a triage surface

**PRs as a request surface: no.** _(Set to `yes` if this repo treats external PRs as feature requests; `/triage` reads this flag.)_

When set to `yes`, PRs run through the same labels and states as issues, using the `gh pr` equivalents:

- **Read a PR**: `gh pr view <number> --comments` and `gh pr diff <number>` for the diff.
- **List external PRs for triage**: `gh api --paginate 'repos/{owner}/{repo}/pulls?state=open' --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not) | {number, title, author: .user.login, author_association, labels: [.labels[].name]}'`.
- **Comment / label / close**: `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`.

GitHub shares one number space across issues and PRs, so a bare `#42` may be either: resolve with `gh pr view 42` and fall back to `gh issue view 42`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Read it as in **Read an issue** above.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes / Decisions-so-far / Fog body. `gh issue create --label wayfinder:map`.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue. Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/`task`). Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: GitHub's native issue dependencies. Add an edge with `gh api --method POST repos/sprkl/faulk/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker's database id (`gh api repos/sprkl/faulk/issues/<n> --jq .id`). A ticket is unblocked when every blocker is closed.
- **Frontier query**: list the map's open children, drop any with an open blocker or an assignee; first in map order wins.
- **Claim**: `gh issue edit <n> --add-assignee @me`, the session's first write.
- **Resolve**: `gh issue comment <n> --body "<answer>"`, then `gh issue close <n>`, then append a context pointer (gist + link) to the map's Decisions-so-far.
