---
type: Agent Configuration
title: "Domain docs"
description: "How the engineering skills read GLOSSARY.md and the ADRs, and why an ADR is only written after the code it records."
---

# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`GLOSSARY.md`** at the repo root.
- **`docs/adr/`**: read ADRs that touch the area you're about to work in.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill creates them lazily when terms or decisions actually get resolved. Until then, the vocabulary of `docs/product/01-vision.md` (mode Web, mode Boost, helper, Atelier, module, objet sonore) is the reference.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `GLOSSARY.md`. Don't drift to synonyms the glossary explicitly avoids.

The domain language is French in prose and English in identifiers. Write each `GLOSSARY.md` term in French with its code identifier (mode Boost → `NativeEngine`, module → `Module`, Atelier → `workshop`). Audio terms that are English in the trade (clip, track, send, sidechain) stay English everywhere.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## ADRs come after the code

An ADR records a decision the code already implements: it is written after the code, never before. A decision taken during a task goes into the plan (reviewed by Judge Dredd when its review is required) and the pull request; the ADR follows once that code exists.

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0002 (one engine, two targets), but worth reopening because…_
