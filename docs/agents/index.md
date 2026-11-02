# Agent configuration

* [Domain docs](domain.md) - How the engineering skills read GLOSSARY.md and the ADRs, and why an ADR is only written after the code it records.
* [Issue tracker: GitHub](issue-tracker.md) - How the skills create, read and structure GitHub issues, and how a task is carried from plan to a reviewed pull request.
* [Journey: adding a day](journey.md) - How a product-owner run's track is published on the public Journey page: one folder in `sprkl/faulk-journey` (MP3, MIDI, sheet), one entry file in `web/src/lib/journey/entries/`.
* [Judge Dredd](judge-dredd.md) - Impartial read-only reviewer subagent, run on the delivery of every task and, for a task that touches a contract or many files, on the plan from a brief built by `scripts/agent-review/brief.mjs`; the guard and the Stop hook that enforce it, and their limits.
* [Triage labels](triage-labels.md) - Mapping of the skills' triage roles to this repo's labels.
* [Verification](verification.md) - Checks required before a task is done (Rust, engine and modules, commands and agents, web UI, latency, agent scripts), how to record them with `scripts/agent-qa/record.mjs`, and the Stop hook.
