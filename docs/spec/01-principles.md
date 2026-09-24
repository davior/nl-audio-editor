# 01 — Principles

## Goal

Record audio or load a file, see it as a waveform and a detailed spectrogram, play it from
any point, and edit it by describing the result wanted in plain English — typed or spoken —
instead of operating controls. **Describe the goal, not the tool.** The application selects
and configures the processing, previews it on a short window, explains what it did and why,
and records it.

Two ways in, both first-class: the **conversation** (the assistant selects typed operations
from the registry) and **manual panels** (dials generated from the same registry). Both
produce the identical step object.

## Material and users

Spoken word and field recordings: interviews, testimony, dictation, phone and remote-call
audio, noisy outdoor recordings — often very quiet (peaks around −30 dBFS). A significant
share carries evidential value. The product's first targets, in priority order:

1. **Speech under noise** — hum, hiss, rumble, tonal lines, room, phone-line damage.
2. **Faint and background voices** — quieter voices under or behind the main content.
3. **Signals and authenticity** — tonal lines, the electrical network frequency (ENF)
   trace, discontinuities that may indicate edits, bandwidth history. Read-only analysis;
   results are *indicators, not proof*.

It is not a music-production tool.

## Hard invariants

From the brief (non-negotiable):

1. The original audio is stored **byte-for-byte** and is never modified, transcoded,
   normalised, renamed or overwritten — including by housekeeping. All processing runs on
   a working copy.
2. Everything is **non-destructive**: accepted steps form an ordered stack replayed over the
   immutable source.
3. **Nothing is ever executed as generated code.** The model only selects typed,
   schema-validated descriptors from a fixed registry.
4. Every operation is available **conversationally and manually**; both produce the
   identical step object.
5. Every operation **previews on a short window** (10 s by default) before it is committed.
6. **Nothing is committed without the user accepting it.**
7. Hard parameter limits are enforced **in the DSP layer**, never only in a prompt. Out-of-
   range values are rejected, not clamped. There is no bypass.
8. A **limiter is always last** in the chain.
9. Reconstruction-class work (separation, synthesis, heavy spectral repair) is labelled
   **processed, not factual** everywhere it is recorded or exported.
10. Every step is logged with its **parameters, measurements, rationale and actor**.

Added by this specification:

11. **The stack is the primary record.** The append-only, hash-chained event log is the
    source of truth; the edit stack is a projection of it. Every preview, tweak,
    acceptance, rejection, modification, removal, annotation and rating is an event.
12. **Every value is recorded in two forms**: exact (what ran) and adaptive (how it relates
    to the clip's own analysis), so any step can be replayed exactly or re-derived on
    another clip.
13. **Locality.** An operation scoped to a region leaves every sample outside the region
    (plus one analysis window) bit-identical to its input.
14. **Preview = final.** A preview is bit-identical to the same span of the final render.
    Every operation has a finite, declared reach; nothing depends on unbounded history.
15. **Determinism.** The same source and the same stack always render bit-identically, on
    every platform: native and WebAssembly builds use the same arithmetic (see
    `02-architecture.md`).
16. **Attenuation-only class.** Operations in the `attenuative` class can only reduce level,
    cell by cell. What they remove is exactly recoverable as the residual.
17. **No audio to the model.** The assistant receives words and analysis numbers only; this
    is enforced by the types the prompt builder accepts.
18. **Clones share, never alter.** A clone copies a project at any step; it shares the
    untouched source and records its lineage in both logs.
