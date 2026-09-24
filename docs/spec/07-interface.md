# 07 — Interface

## Principles

The interface draws what the core returns and holds no audio logic. It must stay simple:
one project is one stack; different streams of work are clones, not layers.

## Screens

- **Library** — projects grouped by source, each source's clones shown as a family tree (fork
  step on each branch). Import (file picker or drag-and-drop, WAV or MP3), record, open a
  `.nlae` bundle.
- **Editor** — the lanes, transport, selection, stack panel, log viewer, integrity badge, and
  (M1) the console and residual monitor.

## Lanes

- **Waveform** — min/max peaks at every zoom level, with a view-only vertical zoom so a very
  quiet clip is visible without amplifying it (amplifying is a step; looking is not).
- **Spectrogram** — computed in the core (worker), max-pooled when zoomed out so short events
  are never hidden; linear or log frequency axis; adjustable dB range; perceptual colour map;
  hover readout of time, frequency and level.
- Shared time axis; zoom and scroll.

## Selection

- A **time range** on the waveform.
- A **time × frequency rectangle** on the spectrogram — the scope for `tf_patch` operations
  such as the spectral compressor.
- Both are saved in the project's view state.

## Transport

Play, pause, stop, click to seek, play selection, loop. Playback at the file's native sample
rate (the browser resamples only for output).

## Stack panel

The accepted steps, projected from the log, each with its operation, key values, actor and
origin, and (M1) toggles for Original / Processed / Residual listening at that step. Plans are
shown as a group. Rejected attempts are visible in the log viewer, flagged as not applied.

## Recording

`getUserMedia` with echo cancellation, noise suppression and automatic gain control **off**;
captured through an AudioWorklet as 32-bit float and written as a WAV, which becomes the
source. The settings the browser actually applied are logged with the recording, so it is
documented whether the browser honoured the request.

## Integrity badge

Shown for every open project: source hash verified, chain verified (or the first failing
event), stack projection consistent with the manifest.

## M0 as built

**Layout.** The library is a sidebar; the editor fills the rest: header (name, source details,
lineage, integrity badge, *Save bundle*, *Close*), two tool rows, the lanes, the selection
readouts, then three panels — stack, record (hashes, capture settings), event log.

**Mouse and keys.** Click a lane to seek; drag on the waveform for a time range, on the
spectrogram for a time × frequency area; wheel zooms around the pointer, shift+wheel scrolls.
Space plays and pauses, Home returns to the start. *Stop* returns to where playback started;
*Loop* with a selection loops the selection.

**Monitor.** *Original / Processed / Residual* switches what the lanes draw and what plays, for
the whole stack. (Switching per step while previewing comes with previews in the interface, M1.)
The first render of a stack can take seconds; each lane says *computing…* until its new data
arrives, and meanwhile keeps the last image, placed where it now falls on the time axis. The
level readout is shown only when the drawn data match the view exactly.

**View state** — `t0`, `t1`, `scale`, `fMax`, `dbRange`, `vZoom`, `monitor`, `selection`,
`tfSelection` — is written to the manifest 300 ms after it stops changing, and at once when
another project is opened. It is not evidence and is not logged. Saved values are checked when
restored; anything missing or out of range falls back to the default.

**Loading.**
- The lanes come first.
- The analysis (features) runs afterwards in a worker of its own, with an *Analysing…* badge; the core logs it as `analysis.computed` once it has checked it describes the source.
- Imports are saved to the library in the background (*Saving to the library…*): the recording first, straight from the file, then the project.
- Long views arrive in parts, and each lane says *computing…* until its view is complete.

**Browser storage (OPFS).** `nlae/projects/<id>/` holds each project's files;
`nlae/blobs/<sha256>/<original filename>` holds each source recording once, shared by its
clones. Renders are never stored (they are recomputed and cached in memory). The last open
project is reopened when the page loads.

**Opening a bundle.**

| The bundle… | What happens |
|---|---|
| does not verify | opens read-only, is **not** added to the library |
| is new to the library | is added |
| has the same log as the library copy | the library copy is kept; the bundle is shown |
| continues the library copy's log exactly | the library copy is updated |
| is an earlier state of the library copy | the library copy is opened instead |
| has a different history from the library copy | opens without replacing it; nothing is saved |

A bundle can never overwrite a library copy with a shorter or different history.

**A project that does not verify is read-only** (enforced by the core): no event can be
appended to a broken chain, the view is not saved, and it cannot be cloned or exported. The
banner and the badge say why; the log viewer lists the verified events and marks the line
where verification stopped.

**Render hash.** The record panel computes, on request, the render hash of what the monitor
plays: SHA-256 over the samples, the same hash native builds record as a step's
`output_hash`. Equal hashes mean the browser and the command line produced the same audio,
sample for sample.

**Core parity check.** A footer button runs the native/WebAssembly parity chain in the
browser and compares it with the pinned hashes.

**Test hooks.** The page publishes its state as `window.__nlae` (project summary, view, saved
view, playhead, what the lanes drew). End-to-end tests read it; nothing reads it back.

## Console (M1)

Typed or spoken requests; the assistant's proposal is shown as a previewable step or plan with
its explanation; accept, reject, or change values before accepting (recorded as a
modification).

## Residual monitor

A three-way switch — **Original / Processed / Residual** — for the whole stack (M0) and for the
step being previewed (M1).

`OPEN:` (10) Is the operation catalogue browsed, searched, or found only conversationally?
Proposal: searchable list plus conversation; browsing by category later.
