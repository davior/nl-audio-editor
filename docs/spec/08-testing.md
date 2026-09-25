# 08 — Testing and quality

## Golden set

Synthesised deterministically by `nlae golden` from separate, known components, so every test
can measure exactly what an operation did to each component (the voices, the noise, each line,
the whine, the bangs). Clip A resembles the reference material (very quiet, DC offset, lines at
750 Hz and 3,150 Hz); clip B has the same kinds of problem elsewhere (lines at 620 Hz and
2,450 Hz). See `tests/golden/README.md`. The expected SHA-256 of every generated file is
committed; generation is part of the determinism test.

Because mask operations are linear once their mask is fixed, the mask computed on the mixture
can be applied to each component separately to measure per-component effects exactly.

## Invariant tests

| Test | What it proves |
|---|---|
| STFT round trip | analysis + synthesis reconstructs below −120 dB |
| Null test per operation | every identity setting renders bit-identical output |
| Locality | samples outside a scope (plus one window) are bit-identical |
| Attenuation-only | every cell's gain ≤ 1 for `attenuative` operations |
| Preview = final | random preview windows equal the same span of the full render, bit for bit |
| Determinism | same source and stack → same render hash, twice; native = WebAssembly |
| Hard limits | out-of-range values are rejected with a typed error |
| Limiter passthrough | below the ceiling, output is bit-identical |
| Canonical JSON | RFC 8785 vectors |
| Hash chain | an edited byte, a deleted line, a reordered line and a re-hashed edit are each reported at the right line |
| Clone lineage | a clone bundle verifies back to import on its own |
| Bundle round trip | source bytes, analysis and view state survive save and reopen |
| Projection | the stack re-projected from the log equals the manifest's stack |
| Schemas | every event and dataset record validates against its JSON Schema |
| Path parity | the same step via the "panel" and "console" paths differs only in actor and origin |
| Registry parity | every descriptor has an implementation and vice versa; typed params accept the defaults |

## Acceptance scenarios

**Reference workflow, automatic** (clip A, built-in recipe `spoken-word-cleanup`, adaptive),
measured 2026-09-24:

| Criterion | Target | Measured |
|---|---|---|
| DC offset after | < 1e-4 | 1.45e-6 |
| Peak after (before the limiter) | −1 dBFS ± 0.1 | −1.000 dBFS |
| Final limiter | inactive | 0 samples touched |
| Noise (noise + rumble components) in the speech pause | ≥ 10 dB down | −10.4 dB |
| Every line's remaining prominence (50, 100, 750, 3,150 Hz) | ≤ 3 dB | ≤ 1.95 dB |
| Foreground / background voice energy | within ±2 dB | −0.3 / −1.2 dB |
| Previewed and accepted as one plan | yes | yes |

**Manual process made reusable** (measured 2026-09-24): the Audacity steps repeated by hand on
clip A — a rejected +40 dB, then +30 dB, DC removal, a normalise changed from −3 to −1 dBFS before
accepting (recorded as a modification), noise reduction with a hand-picked profile at
40.4–41.4 s, cuts at 740–760 Hz and 3,100–3,200 Hz with depths read off the analysis — saved as a
recipe. The inferred bindings point at the detected lines (750 Hz, 3,150 Hz), the profile at the
quietest region, the gain at a target peak. Replayed on clip B:

| | 620 Hz line | 2,450 Hz line |
|---|---|---|
| Adaptive: cut moved to | 610–630 Hz | 2,400–2,500 Hz |
| Adaptive: stood above surroundings / cut by | 7.5 / 7.2 dB | 7.7 / 7.4 dB |
| Exact: cut by / still stands | 0.0 / 10.1 dB | 0.0 / 10.9 dB |

Line reductions are measured per component (the line's own attenuation across its cut) rather
than as prominence in the result: a 100 Hz-wide cut also lowers part of the ±150 Hz neighbourhood
that prominence is measured against, which understates the cut. The dataset export over five
projects (A, B adaptive, B exact, two clones of B) validates against the schemas; rejected and
modified steps are flagged; the clones are linked as siblings and to their parent; there are no
keys, and no audio unless `--with-audio` is given.

**Spectral compressor** (clip A), measured per component:

| Mode | Scope | Target | Measured (2026-09-24) |
|---|---|---|---|
| `tonal` (defaults) | 9.5–14.5 s × 4.5–6 kHz | whine ≥ 15 dB down; background ≤ 2 dB | whine −17.4 dB; background −0.0 dB |
| `transient` (defaults) | whole clip | bangs ≥ 10 dB down; background ≤ 2 dB | bangs −12.0 dB; background −0.5 dB; foreground −0.9 dB |
| `level` (ratio 8, attack 5 ms, release 50 ms) | 200–4000 Hz | foreground ≥ **5** dB down relative to background; background ≤ 2 dB | foreground −7.2 dB; background −1.8 dB; relative −5.4 dB |

The `level` target was 6 dB. The two synthetic voices overlap in time–frequency (moving pitch
smears their partials across cells), which caps what per-cell processing can separate at about
5.5 dB on this material; the target is recorded at 5 dB (accepted by the product owner,
2026-09-24; to be checked against real recordings). Tuning that produced these defaults:
the `transient` reference moved from the median to the upper quartile (speech onsets were
being treated as transients), and `threshold_db` gained a mode-dependent `auto` default.

**Line detection on a short clip** (found 2026-09-24, fixed in `line_reduce` v2 and features
v2): on the 12-second variant of clip A (`nlae golden --short`), version 1 of the detection
reported the real lines (50, 100, 150 Hz hum; 750 and 3,150 Hz) at widths of 5.9–8.8 Hz, present
in 100% of segments — and also two voice harmonics, 583.7 Hz and 1,861.5 Hz (widths 17.6 and
20.5 Hz, present in 75% and 67% of segments), which the built-in recipe then cut by about 8.8 dB.
Both are multiples of a ~116.5 Hz pitch the synthetic voice holds near phrase ends, lifted by
formants. With the pause rule (agreed with the product owner) a line must also stand out in the
speech pauses; detection now returns exactly 50, 100, 150, 750 and 3,150 Hz on that clip, and a
synthetic case (a tone that sounds only while "talking", 70% of the time) is rejected while the
steady line under it is kept (`tonal` and `ops` unit tests). Version 1 still resolves the seven
lines, so recipes recorded with it replay as they did.

## Browser end to end (Playwright, headless Chromium, fake microphone)

Run against the production build; fixtures are made by the command-line tool (a 12 s golden
clip, a project processed with the built-in recipe and packed, and a copy of that bundle with
one logged value changed and both ZIP checksums fixed, as a careful forger would).
All pass (2026-09-24, about 21 s):

| # | Scenario | Checks |
|---|---|---|
| 1 | Import a recording | both lanes draw; the source hash equals the file's SHA-256; badge verified |
| 2 | Seek and play | click seeks; playback advances; pause holds; stop returns to the start point |
| 3 | Select | a time range on the waveform and a time × frequency area on the spectrogram land at the right seconds and hertz |
| 4 | Save, reload, reopen | after a page reload, and in a fresh browser profile from the downloaded bundle: the same view (zoom, scale, range, monitor, both selections), hashes and project; the chain verifies; the export itself is logged |
| 5 | Clone | cloning after step 2 gives a two-step clone with the parent's step-2 stack hash, lineage shown, nested under its parent in the library; after a reload the clone's log and its parent's log verify; the parent's log records the clone |
| 6 | Tampered bundle | the badge names the edited event (line 5, seq 4); the project is read-only; it is not added to the library; the log stops at the edit |
| 7 | Bundle from the command line | its four steps show in order; the stack hash matches; the browser's render of the stack has the **same render hash the command line recorded** (native and browser produce the same samples) |
| 8 | Record from the microphone | echo cancellation, noise suppression and gain control requested off and reported off; the `source.recorded` event is in the log |
| — | Core parity in the browser | the parity chain computes the pinned hashes in Chromium, as it does natively and in Node |

Worth knowing from scenario 8: Chromium's fake microphone delivered **2 channels although 1
was requested**. The capture record shows it (requested 1, applied 2), which is the reason the
applied settings are logged.

### The console (M1)

A stand-in OpenAI-compatible provider is started with the test run (`tests/e2e/mock-model.ts`).
- It answers from the user's words with tool calls, as a model would.
- It allows browser requests (CORS) and keeps what it was sent.
- It is reached at a different origin from the app, so the browser's CORS path is exercised.

The command line also renders its processed fixture to a WAV file, which the export scenario
compares with the browser's. All pass (2026-09-24, with the M0 scenarios, 16 tests in about
70 s).

| # | Scenario | Checks |
|---|---|---|
| 1 | "clean this recording up" | Handled locally: a four-step plan preview, the same operations as the command line's. The lanes zoom to the preview window and draw the preview audio. "Play the residual" switches to the preview's residual and keeps the proposal open. Accepting gives **the command line's stack hash**, and the view returns. `recipe.replayed` and `plan.accepted` are logged, with the words on every step. The 32-bit float export is **byte-identical to `nlae render`**, and `render.exported` logs the file's SHA-256, stack hash, output hash and limiter measurements, equal to the command line's |
| 2 | "the hum is distracting" | It goes to the model: `line_reduce`, with the model's explanation and the preview measurements. A value changed before accepting is logged as `step.modified` and used. The logged exchange's request **is exactly what the provider received**, and the preview refers to it. The step's actor is the model and provider; the acceptance is the user's |
| 3 | Reject with a reason | "cut 3,100 to 3,200 Hz by 12 dB" gives a `band_cut`. The rejection is logged with its reason. The stack and view are unchanged |
| 4 | Area + "compress the peaks here" | A `spectral_compressor` scoped to the dragged area exactly, with preview measurements, accepted |
| 5 | Correction and a question | The model first proposes +90 dB. That is refused, the model is asked once to correct itself, and it proposes +6 dB. Both exchanges are logged, the second marked as correcting the first. A question gets a reply in words, and the open proposal is set aside |
| 6 | Selection, audio and key | *Test connection* reaches the provider. A model-path request scoped to the selection gets that area as its scope. The provider received the key in the header only, and no numeric array longer than 64. The key is **not in any library file, browser storage or the saved bundle** |
| 7 | Undo and rating | Undo removes the top step, both from the stack panel and by typing "undo"; each is logged as `step.removed`, and the stack hash returns to the empty stack's. A rating is logged as `stack.rated` |

In Rust:
- the router cases;
- the request tripwire (sample data refused in the context and in the request);
- the parser (valid, out of range, unknown, system, plans, questions);
- a logged exchange: correction, preview link, chain verification after reopening, and the
  dataset chat record taken from the exchange;
- `nlae ask`, both locally routed and through a saved model answer.

### Time edits

**End to end** (with the M0 and M1 scenarios, 17 tests; all pass, 2026-09-24):
1. Select 4–6 s of the 12 s clip on the waveform and use *Remove this stretch*.
   - The proposal previews on *Before*, with the stretch marked.
   - Once accepted, the stretch is hatched and the output is 12 s minus the selection.
2. On *Processed*, play from 3.8 s: the playhead passes 6.2 s within 1.8 s (playing through
   would take 2.4 s), so playback skips the removed stretch.
3. Insert 0.5 s of silence at the playhead (9 s), and accept.
4. Export the WAV:
   - its length is the original minus the removal plus 0.5 s;
   - it has two cue markers, labelled in the original's time;
   - its SHA-256 is the one logged, and the log lists both edits.
5. Undo twice: the original length again, and nothing marked.

**In Rust:**
- **Layout:** overlapping removals merge; a silence inside a removed stretch goes at its join;
  the order of edits doesn't matter; kept audio is exact outside the fades; fades are
  symmetric at a join; removing everything leaves an empty output.
- **Project:**
  - processing is unchanged by the edits;
  - the output is exact outside the fades, with the silence where it was inserted;
  - the preview's residual is the removed audio, where it was;
  - a reopened project gives the same export hash;
  - undo restores the length;
  - recipes refuse edit-only ranges;
  - a time edit is previewed alone, and an insertion must fall inside the recording.
- **WAV cue markers:** chunk sizes, the pad byte after an odd 24-bit data chunk, and
  byte-identical files when there are no edits. Our own decoder still reads a file that has
  markers.
- **Router:** seconds, milliseconds, minutes and m:ss are understood. "Remove this part"
  removes the selection; "remove the hum here" still cuts the hum; "cut 3,100 to 3,200 Hz"
  stays a band cut.
- **Command line:** `nlae ask "remove 2 to 3 seconds"` and "insert 0.5 s of silence at 6 s",
  then `nlae render`: the length, the cue chunk, the printed marks and the logged edits are
  checked.
- **Parity:** the chain now removes and inserts time before the final limiter. The re-pinned
  hashes match natively and in WebAssembly.

## Proposed: control replay (M3)

To show that something "brought out" by processing is in the recording rather than made by the
processing: replay the same chain on noise synthesised to match the clip's long-term spectrum
and level. Anything that appears in the control as well was produced by the processing. The
control render and its comparison are stored with the capture bundle.

## Performance targets (provisional, brief question 20)

- A 10-minute 48 kHz mono file shows both lanes in under 3 s in the browser.
- Playback starts within 100 ms of pressing play.
- A 10 s preview of the spectral compressor renders in under 1 s in WebAssembly.

**Measured (2026-09-24, headless Chromium on a 4-core cloud container, 48 kHz mono float WAV).**
First in M0, then after the loading work in this milestone:

| File | Import → both lanes | Reopen → both lanes | Analysis (in the background) |
|---|---|---|---|
| 1 minute, M0 | 3.0 s | 1.1 s | before the editor opened |
| 1 minute, now | 0.6 s | 0.6 s | 1.9 s |
| 10 minutes, M0 | 28.8 s | 8.6 s | before the editor opened |
| 10 minutes, now | 3.1 s (whole view 5.4 s) | 3.1 s (whole view 5.3 s) | 20.8 s |

What changed:
- Creating a project no longer analyses it. The browser analyses in a second worker once the
  lanes are drawn, and the core logs the result after checking it describes its source.
- Playback and analysis get their copy of the audio after the first view is drawn, not before.
- Imports are saved to the library in the background. The recording is written straight from
  the file the user chose, then the project.
- Renders are borrowed rather than copied for each redraw.
- Long views arrive in parts, left to right, and finished views are cached.
- Files are handed to the workers without copying.
- The file hash is fed in 64 KiB chunks. A WebAssembly engine can only switch to its optimised
  code between calls, so hashing a 10-minute file in one call took 4.6 s; in chunks it takes
  about 0.8 s, with the same digest.

For ten minutes, first paint is about the 3 s target. What remains is decoding and hashing
the file in WebAssembly: about 2.4 s on import and 2.3 s on reopen, when the source hash is
verified. A warm-up run at start-up made no measurable difference and was dropped. The
background analysis takes 20.8 s (12.5 s natively); the editor is usable meanwhile, and zooming
redraws in about 1.5 s while it runs.
