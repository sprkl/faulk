# Agent configuration

* [Domain docs](domain.md) - How the engineering skills read GLOSSARY.md and the ADRs, and why an ADR is only written after the code it records.
* [Issue tracker: GitHub](issue-tracker.md) - How the skills create, read and structure GitHub issues, and how a task is carried from plan to a reviewed pull request.
* [Judge Dredd](judge-dredd.md) - Impartial read-only reviewer subagent, run on the plan and once on the delivery from a brief built by `scripts/agent-review/brief.mjs`; the guard and the Stop hook that enforce it, and their limits.
* [Triage labels](triage-labels.md) - Mapping of the skills' triage roles to this repo's labels.
* [Verification](verification.md) - Checks required before a task is done (Rust, engine and modules, commands and agents, web UI, latency, agent scripts), how to record them with `scripts/agent-qa/record.mjs`, and the Stop hook.
