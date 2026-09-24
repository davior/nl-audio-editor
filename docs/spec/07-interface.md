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

## Console (M1)

Typed or spoken requests; the assistant's proposal is shown as a previewable step or plan with
its explanation; accept, reject, or change values before accepting (recorded as a
modification).

## Residual monitor (M1)

A three-way switch — **Original / Processed / Residual** — for the whole stack and for the
step being previewed.

`OPEN:` (10) Is the operation catalogue browsed, searched, or found only conversationally?
Proposal: searchable list plus conversation; browsing by category later.
