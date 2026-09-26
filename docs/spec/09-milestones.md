# 09 — Milestones

Sequential. A milestone is complete when its acceptance criterion is met on the golden clips.
Rebased on the product owner's primary requirement: the stack record, recipes and dataset
export are pulled forward from M3, and a minimal plan (steps previewed and accepted as one unit)
from M2, because the reference workflow needs them.

| ID | Milestone | Contents | Acceptance |
|---|---|---|---|
| **M0** | Skeleton + stack engine | Import or record; waveform and spectrogram; transport; time and time × frequency selection; projects that save, reopen and clone; integrity badge. **Stack engine** (step object, event-sourced stack, plans, stack hashes, features v1), the registry with nine operations, the command-line workflow, **recipes** (exact and adaptive, dry-run diff), the built-in reference recipe, **dataset export** | A project round-trips with audio, analysis and view state intact. The reference workflow and the manual-process scenarios in `08-testing.md` pass |
| **M1** | The working loop | Console (typed; spoken, streamed to Deepgram), provider client, local routing of routine intents, preview / accept / reject / modify in the interface, residual monitor, render and export | One typed or spoken command produces a previewed, measured, accepted step and a rendered export; "clean this recording up" runs the reference plan |
| **M2** | Library and planning | Registry at scale, tiered tool exposure, model-planned composite goals, generated panels, parity tests for panels and tool schemas, more EQ, dynamics and authenticity analysis | A multi-step goal is applied as one plan whose steps can each be removed or edited; every descriptor has a working panel |
| **M3** | Provenance and reuse, complete | Capture bundles, ratings in the interface, control replay, adaptive-replay refinements, dataset tooling, separation (reconstruction class) | A job's chain replays on a different clip adaptively, with a dry-run diff and a rating recorded, from the interface |
| **M4** | Local and automated | Local model support, learned starting chains, proposal-first automated clean-up, batch processing with per-file review | A new file receives a proposed chain above the confidence threshold, reviewed before export |

**Status (2026-09-25).**
- M0 is built: the core, the command-line workflow, recipes and dataset export, the
  WebAssembly build and the browser interface, with the tests in `08-testing.md` passing.
- M1 is built:
  - the console, typed or spoken (streamed to Deepgram), with local routing and the model
    path;
  - previews with Original / Before / Processed / Residual;
  - accept, change and reject;
  - WAV export, undo and ratings;
  - `nlae ask` (typed).

  Its acceptance runs end to end against a stand-in provider and a stand-in recogniser
  (`08-testing.md`).
- Time edits (requested by the product owner, beyond the milestone plan) are built:
  removing a stretch and inserting silence, applied to the output, with cue markers in exports.
- Apply-first editing (chosen by the product owner, 2026-09-26, beyond the milestone plan):
  - a request applies at once, and exporting approves the stack as it stands;
  - any step can be removed (it stays in place and can be restored), restored or edited, with
    undo and redo;
  - the steps above a change keep their values, and a step measured on audio that has since
    changed is flagged, to be measured again if the user wants.

  Built in the core, the command line and the dataset export; the browser follows. It brings
  forward M2's generated panels (the step editor).
- M2 has begun with the catalogue (chosen by the product owner, 2026-09-25):
  - eight operations: `high_pass`, `low_pass`, `bell`, `shelf` and `tilt` (zero-phase EQ on
    the shared engine, which may now boost), `gate`, `loudness_normalise` (−23 LUFS by
    default) and `hum_reduce`;
  - tiered tool exposure: the model always has the core operations, sees the rest in an index,
    and asks for their schemas (`describe_operations`), one round at most;
  - router phrases for all eight, and a describe round in the console and `nlae ask`.

  Still to do in M2: generated panels with their parity tests, model-planned composite goals,
  level riding, voice-band EQ, de-click and de-clip, and authenticity analysis.
- Still open for M1, all needing the hosts allowed from the build environment, or a person
  with keys:
  - a manual run against DeepSeek, and whether it accepts browser requests (CORS);
  - a manual run against Deepgram: *Test connection*, then the routine requests spoken, whose
    transcripts become router test cases.
