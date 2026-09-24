# 09 — Milestones

Sequential. A milestone is complete when its acceptance criterion is met on the golden clips.
Rebased on the product owner's primary requirement: the stack record, recipes and dataset
export are pulled forward from M3, and a minimal plan (steps previewed and accepted as one unit)
from M2, because the reference workflow needs them.

| ID | Milestone | Contents | Acceptance |
|---|---|---|---|
| **M0** | Skeleton + stack engine | Import or record; waveform and spectrogram; transport; time and time × frequency selection; projects that save, reopen and clone; integrity badge. **Stack engine** (step object, event-sourced stack, plans, stack hashes, features v1), the registry with nine operations, the command-line workflow, **recipes** (exact and adaptive, dry-run diff), the built-in reference recipe, **dataset export** | A project round-trips with audio, analysis and view state intact. The reference workflow and the manual-process scenarios in `08-testing.md` pass |
| **M1** | The working loop | Console (typed; spoken via local speech-to-text), provider client, local routing of routine intents, preview / accept / reject / modify in the interface, residual monitor, render and export | One typed or spoken command produces a previewed, measured, accepted step and a rendered export; "clean this recording up" runs the reference plan |
| **M2** | Library and planning | Registry at scale, tiered tool exposure, model-planned composite goals, generated panels, parity tests for panels and tool schemas, more EQ, dynamics and authenticity analysis | A multi-step goal completes as one previewable plan; every descriptor has a working panel |
| **M3** | Provenance and reuse, complete | Capture bundles, ratings in the interface, control replay, adaptive-replay refinements, dataset tooling, separation (reconstruction class) | A job's chain replays on a different clip adaptively, with a dry-run diff and a rating recorded, from the interface |
| **M4** | Local and automated | Local model support, learned starting chains, proposal-first automated clean-up, batch processing with per-file review | A new file receives a proposed chain above the confidence threshold, reviewed before rendering |

**Status (2026-09-24).** M0 is built: the core, the command-line workflow, recipes and dataset
export, the WebAssembly build and the browser interface, with the tests in `08-testing.md`
passing. The residual monitor for the whole stack, planned for M1, is already in the M0
interface.
