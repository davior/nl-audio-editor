# nlae parent specification (draft)

This is the parent specification the build brief (`docs/build-brief.md`) refers to. It is
being written together with the product owner: this is the first draft, produced alongside
the first build. Anything marked **`OPEN:`** needs a decision from the product owner;
everything else is a proposal that stands until someone changes it. Decisions, with their
one-line reasons, are logged in `docs/decisions.md`.

## Primary requirement

Every action taken on a recording is recorded as a **stack** of self-describing steps so it
can be **analysed**, **reused on other clips**, and used to **train an AI**. This is the
path to automating the clean-up process. The stack record is designed first; interfaces
are built on top of it.

## Contents

| File | What it covers |
|---|---|
| [01-principles.md](01-principles.md) | Goal, material, users, hard invariants |
| [02-architecture.md](02-architecture.md) | Layers, builds, data flow, determinism |
| [03-projects-clones-provenance.md](03-projects-clones-provenance.md) | Projects, clones, event log, hash chain, bundles, residuals |
| [04-operation-registry.md](04-operation-registry.md) | Descriptors, classes, validation, tool schemas and panels |
| [05-operation-catalogue.md](05-operation-catalogue.md) | Operations, ordered by the product's targets |
| [06-reasoning-layer.md](06-reasoning-layer.md) | The assistant, provider client, privacy |
| [07-interface.md](07-interface.md) | Lanes, transport, selection, console, stack, clones |
| [08-testing.md](08-testing.md) | Golden set and invariant tests |
| [09-milestones.md](09-milestones.md) | M0–M4, rebased on the stack-first work |
| [10-learning-and-automation.md](10-learning-and-automation.md) | Stack records as training data; the path to automation |
| [reference-workflows/](reference-workflows/) | Real workflows the product must reproduce |

## How to review

Read a file, then either comment on the pull request or answer the `OPEN:` items in chat.
Accepted answers move into `docs/decisions.md` and the `OPEN:` marker is removed.
