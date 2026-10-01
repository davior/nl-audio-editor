# 07 — Interface

## Principles

The interface draws what the core returns and holds no audio logic. It must stay simple:
one project is one stack; different streams of work are clones, not layers.

## Screens

- **Library** — projects grouped by source, each source's clones shown as a family tree (fork
  step on each branch). Import (file picker or drag-and-drop, WAV or MP3), record, open a
  `.nlae` bundle.
- **Editor** — the lanes, transport, selection, the stack with the console at its foot (one
  list), log viewer, integrity badge and residual monitor.

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
- Either can be **labelled** (see *Labels*), and a label selects its place again.

## Transport

Play, pause, stop, click to seek, play selection, loop. Playback at the file's native sample
rate (the browser resamples only for output).

## The stack

**One list with the console** (chosen by the product owner, 2026-09-27): the console and the
stack showed the same steps twice, with different controls. They are one list, in the order
the requests were made, which is the stack's order, since a request's steps go on top:
- each request: the words, how it was answered (handled here or by which model; spoken or
  typed) and the answer, then the steps it put on the stack, numbered by their place;
- every control is on the step: select to change its values and hear it on its own, ×,
  *Restore*, *Clone here*; *Undo*, *Redo*, *Clone current* and the rating are the list's own;
- a request that put nothing on the stack (an answer in words, one that could not be used) is
  listed where it happened, and *Steps only* hides it;
- a clone's inherited steps come first, under the parent's name.

The requests are rebuilt from the log (`assistant::requests`): a step's event names the
exchange and the dictation it came from; an exchange names the one it corrects or follows, and
its dictation. So the list is the same when the project is opened again. What logged nothing
(listening, a typed *undo*, an error before anything was sent) lasts only for the session.

The stack's steps, projected from the log, each with:
- its operation and key values;
- its actor (for the assistant, its model and provider) and origin;
- the words that asked for it.

Plans are shown as a group. Rejected attempts are visible in the log viewer, flagged as not
applied. A 1–5 rating of the stack as it stands is logged as `stack.rated`.

**Changing the stack** (agreed with the product owner, 2026-09-26; see
`03-projects-clones-provenance.md`):
- Clicking a step opens its editor, generated from the operation's descriptor with its limits.
  *Apply* re-renders from that step up.
- × removes a step, with an optional reason (recorded). It stays in its place, greyed and
  struck through, with *Restore*.
- *Undo* and *Redo* in the panel's head walk back through the changes.
- A drifted step (measured before a change below it) shows a badge that opens *Measure again*,
  with what would change.
- With a step selected, the monitor offers *Before this step*, *After it* and *What it
  removed*, on the whole recording. This replaces cloning to listen at an earlier step.
- While the steps above a change are rendered again, the panel says how many.

## Apply-first editing

Chosen by the product owner, 2026-09-26: a request applies at once, and the stack is where
changes are reviewed, pruned and tuned. In the browser:
- A request applies its steps to the whole recording. The console shows an *Applied* card
  (since 2026-09-27, the request is listed in the stack with its steps under it):
  - each operation, its scope and key values;
  - its measurements over the whole recording;
  - the model's explanation;
  - *Undo this* and *Edit*.
- There is no preview window and nothing is set aside: the view and the playhead stay where
  they are, and the monitor switches to *Processed*.
- *Export WAV* says that it approves the stack as it stands, listing its steps and how many
  are excluded.
- "undo" and "redo" are handled here, without the model.

**As built** (in the core, the command line — `nlae ask --apply`, `remove`, `restore`, `edit`,
`remeasure`, `undo`, `redo` — and the browser):
- **The console** applies each request at once. The request is listed in the stack with its
  steps under it (see *One list with the console*); the *Applied* card, with its *change* and
  *Undo*, is gone.
- **The stack** lists every step in its place, under the request that made it:
  - × removes a step, with an optional reason; the step stays, greyed and struck through, and
    *Restore* brings it back;
  - *Undo* and *Redo* in the list's head name the change they take back or repeat;
  - an *edited* tag marks a changed step, and a *measured before a change* badge a drifted one.
- **The step editor** opens when a step's name is clicked. It is generated from the operation's
  descriptor, and *Apply* sends only the values that changed. For a drifted step it shows what
  measuring again would change, and offers *Measure again*.
- **The monitor**, with a step selected, adds *Before this step*, *After it* and *What it
  removed*, over the whole recording. Time edits have none (their marks are on the lanes), and
  these monitors are never saved as the project's view.
- **Busy:** while the steps above a change are rendered again, the banner says how many.
- **Approval:** under the stack, a note says that exporting approves it as it stands, and how
  many steps are removed.
- **Undo and redo** in words are handled here.
- **Preview mode is gone from the browser.** The command line keeps preview and accept.

## Labels

Requested by the product owner, 2026-10-01. A label is a one-line note on a selected stretch of
the recording, or on a selected time × frequency area, so that a place can be found again. It is
called a label because *annotation* already names a note on a step of the stack
(`step.annotated`).

- **Making one.** Select a stretch on the waveform, or an area on the spectrogram, then
  right-click that lane, or press *＋ Label* beside the selection's readout. A small form opens at
  the pointer, saying what will be labelled, with a one-line box: Enter adds the label; Escape, or
  a click elsewhere, closes the form. A right-click with nothing selected says what to select
  first; it selects nothing and does not move the playhead.
- **The list.** *Labels* sits to the right of the stack (under it in a narrow window), in time
  order along the recording (labels at one place in the order they were made), each with its
  place and its text, scrolling by itself to the label just added.
- **Nothing is drawn on the lanes.** A label is not shown on the waveform or the spectrogram; the
  list is where they are.
- **Clicking a label** selects its place again, exactly: a stretch becomes the time selection and
  an area the area selection, and the other selection is cleared. The view moves to hold it:
  unchanged when it is all on screen already, otherwise centred at the same zoom, or widened when
  the view is too short to hold it. The spectrogram's top frequency rises when an area reaches
  above it. The label whose place is selected now is outlined.
- **Changing and removing.** ✎ changes the text in place (Enter, or leaving the box, keeps the
  change; Escape drops it); × at the end of the row removes the label. A label's place cannot be
  changed: remove it and make another.
- **Limits.** The text is one line of 1–200 characters, without spaces at either end; the place
  lies inside the recording. A project that did not verify takes no labels, and the form says so.
- **In the log.** `label.added`, `label.edited` and `label.removed`, as the user's actions
  (`03-projects-clones-provenance.md`). The list is rebuilt from the log by the core, so it is the
  same when the project is opened again and travels in a bundle. Removing a label keeps its words
  in the log.

`OPEN:` Should a clone inherit its parent's labels? Today it starts without them.
`OPEN:` Labels are not yet in the command line (`nlae label …`) or the dataset export.

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
spectrogram for a time × frequency area; right-click a lane to label its selection; wheel zooms
around the pointer, shift+wheel scrolls.
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

## M1 as built

The browser's preview and *Accept* described here were replaced by apply-first editing
(2026-09-26, above); the command line keeps them.

**Console.** A panel under the lanes. Say what should happen, in words, typed or spoken.
- Each turn shows whether it was handled here or which model answered it, and whether it was
  spoken.
- While the model is waiting to see an operation's parameters (`06-reasoning-layer.md`), the
  turn says "Looking up …" with the operations it asked for.
- A proposal is a card:
  - each operation, its scope and key values;
  - its measurements over the preview window;
  - the model's explanation.
- *change* opens an editor for that step's values, built from the operation's descriptor with
  its limits. Changed values are recorded as `step.modified` when the proposal is accepted.
- In a plan, each step can be switched off.
- *Accept*, or *Reject* with an optional reason; both are logged.
- A new request (other than one to listen) sets an open proposal aside, undecided; the log
  keeps it.
- The console waits for the analysis to finish, and is unavailable on a read-only project.

**Preview mode.** While a proposal is open:
- The lanes zoom to its window and draw the preview audio.
- The monitor offers **Original / Before / Processed / Residual** for that window:
  - Before is the stack without the proposal;
  - Residual is what the proposal would remove.
- *Play* plays the window, round and round with *Loop* on.
- "Play the residual" and the like switch among these.

When the proposal is accepted, rejected or set aside, the view returns to where it was:
- after accepting, to the processed stack;
- otherwise, to the monitor that was on.

The preview's zoom is never saved as the project's view.

**Time edits.**
- **Making them:**
  - *Remove this stretch* appears next to a time selection;
  - *Insert … s of silence at the playhead* is in the same bar;
  - both go through the console as if typed, so they are routed, previewed and logged with
    their words.
- **Previewing:** the preview opens on *Before*, the original's timeline, with the stretch or
  insertion point marked. *Processed* plays the joined result, and *Residual* the removed
  audio.
- **On the lanes:** the lanes keep the original's timeline. Removed stretches are hatched,
  and inserted silences are marked with their length.
- **Playback:** *Processed* plays what will be exported. It skips removed stretches and pauses
  the playhead during inserted silence.
- **In the stack panel:** the output's length is shown next to the original's.

**Settings.** Provider preset, address, model, provider name, key, *Remember on this device*,
and *Test connection* (see `06-reasoning-layer.md`). A **Speech (Deepgram)** section has the
streaming address, model, language, key, *Remember on this device*, *Let Deepgram keep my
dictation to improve its models* (ticked by default), and its own *Test connection*, which opens
a stream and closes it without sending audio.

**Dictation.** *Speak*, beside *Send*, streams the microphone to Deepgram
(`06-reasoning-layer.md`).
- While it listens, the box is read-only. It shows what was typed before, then the words
  recognised, then the words still being recognised.
- *Stop* or Enter ends listening without sending anything; Escape discards what was heard.
- Listening also ends when the speaker pauses, after 30 s, or after 8 s without speech.
- The words stay in the box until Enter sends them, and can be changed first.
- Playback pauses while the microphone is on, and *Play* waits.
- Without a Deepgram key, *Speak* opens the settings.

**Export WAV.** 32-bit float, 24-bit or 16-bit. It exports the stack with its time edits and the
final limiter, with a cue marker at each edit. It is logged as `render.exported` with:
- the stack hash (with the limiter);
- the output's render hash;
- the limiter's measurements;
- the file's SHA-256;
- the output's length and where the time edits are.

The file is identical to `nlae render` of the same stack, byte for byte (end-to-end test).

`OPEN:` (10) Is the operation catalogue browsed, searched, or found only conversationally?
Proposal: searchable list plus conversation; browsing by category later.
