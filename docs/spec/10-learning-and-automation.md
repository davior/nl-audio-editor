# 10 — Learning and automation

The product owner's goal is to automate the clean-up process. The route there is data: every
action is recorded in a form that can be analysed, replayed on other clips, and used to train
a model. This section describes that data and how it leads to automation.

## State → action → outcome

Every decided step is a training example:

- **State** — the features snapshot of the step's input, for the whole clip and for the
  step's scope (`state_before`), plus the stack so far.
- **Action** — the operation and its values in both forms: exact (`resolved`) and adaptive
  (`params` + `bindings`), plus who proposed it and the user's words (`intent`).
- **Decision** — accepted, rejected (with reason), or modified (the proposal and the change,
  side by side). The number of previews and tweaks before the decision.
- **Outcome** — `state_after`, the DSP's measurements, and any rating.

Rejected attempts are recorded and exported (flagged as not applied): they are the richest
signal about what a person judged wrong. Manual work is recorded and exported by default.

## Exact and adaptive forms

An absolute value is rarely right on another recording. Each value is therefore recorded with
how it relates to the clip:

- **`auto` values** are measured on the clip when the step is resolved (the peak for
  `normalise`, the lines for `line_reduce`, the quietest region for `noise_reduce`).
- **Declared bindings** come from the assistant or a recipe ("the band around the strongest
  line").
- **Inferred bindings** are computed when a step is accepted, so that manual work becomes
  reusable without anyone explaining it:
  - a band containing, or close to, a detected tonal line binds to that line (by nearest
    frequency within ±30 %, falling back to the same prominence rank);
  - a cut's depth binds to the line's prominence (as an offset);
  - a fixed gain binds to the resulting peak level;
  - a hand-picked noise-profile range that is within 6 dB of the quietest region binds to
    "quietest steady region";
  - a time range covering ≥ 95 % of the clip binds to "whole clip".

## Recipes

Any contiguous part of a stack can be saved as a recipe file (portable, versioned JSON, also
kept in a local library). Replay is `exact` (resolved values as recorded) or `adaptive` (the
default: bindings and `auto` values re-derived on the new clip). Every replay starts with a
dry-run diff listing, for each value, what was recorded, what it will be, and the binding that
explains the change. A replay becomes a plan, previewed and accepted as one unit.

## Dataset export

`nlae dataset export <projects…> -o <dir>`:

| File | One record per | Contents |
|---|---|---|
| `steps.jsonl` | decided step | state → action (both forms) → actor, origin, intent → decision → outcome → rating → lineage, hashes |
| `episodes.jsonl` | project | source features, final stack, stack and output hashes, ratings, lineage; clones forked from the same step are linked as comparison pairs |
| `chat.jsonl` | step with an intent | OpenAI chat fine-tuning format: the user's words (plus the analysis summary the assistant would see) → a tool call against the registry's schema |
| `manifest.json` | export | schema, feature and registry versions, options, counts, SHA-256 of each file |

Audio is **not** included by default: records carry the source and render hashes, which join
back to the project bundles. `--with-audio full` adds the sources. API keys never appear.

## Clones as comparisons

Two clones forked from the same step start from the same state and diverge. With ratings, they
form preference pairs ("from here, B was better than A"), which is exactly the data needed to
learn which chain to propose.

## Path to automation (M4)

1. **Retrieval.** Compute features for a new clip; find the most similar past episodes; propose
   their chain as an adaptive replay; show the dry-run diff and a preview; the user reviews.
2. **Learned starting chains.** Train a small model on the steps dataset to propose the
   operation sequence and values from the features, with a confidence score; propose only above
   a threshold.
3. **Batch.** Apply proposals across a folder with per-file review before rendering.

The model never acts unreviewed: proposal first, acceptance by a person, everything recorded.

## Open

- `OPEN:` also record listening and selection behaviour (what was played, looped, compared) as
  extra context? Useful for learning where people look for problems; off until decided.
- `OPEN:` (8) retention policy for logs and captures, and whether the user is asked before
  anything is discarded. Proposal: keep indefinitely; derived caches may be purged, never logs
  or sources.
- `OPEN:` (9) whether capture bundles travel when a project is shared.
