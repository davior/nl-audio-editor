# Decisions

One line each, with the reason. Newest decisions at the bottom of each section. Numbers in
brackets refer to the open questions in section 11 of `docs/build-brief.md`.

## Product (agreed with the product owner, 2026-09-24)

- **The action stack is the primary requirement** — every action is recorded so it can be analysed, reused on other clips and used to train an AI; this is the product owner's route to automation.
- **Clones instead of in-project layers or branches** — keeps the interface to a single stack; different streams of work are clones with recorded lineage.
- **Residual listening** (Original / Processed / Residual) — hearing what was removed is the quickest check that only the intended material went.
- **Standalone clone export with lineage** — a clone bundle verifies back to the original import on its own.
- **A spectral compressor** that pushes down high points in a selected time × frequency area — requested; detection modes cover the three peak types named: loud foreground voice (`level`), tonal whines (`tonal`), bangs (`transient`).
- **Targets, in order: speech under noise, faint/background voices, signals & authenticity** — as prioritised by the product owner; music production is out of scope.
- **The Audacity spoken-word clean-up is the reference workflow** — it is the process used today; see `docs/spec/reference-workflows/`.
- **Parent spec written together, in `docs/spec/`** — the product owner asked to co-write it; `OPEN:` markers hold their decisions.

## Platform and architecture

- [1] **Browser-first**, Tauri desktop later — testable end to end in CI and cloud sessions; same core and frontend reused by the desktop shell.
- **Command-line tool as the first actor** — drives the stack engine headlessly now, and is the base for batch automation (M4).
- **Own f64 radix-2 FFT instead of a crate** — identical arithmetic on every target, so native and WebAssembly renders are bit-identical; small enough to audit.
- **All transcendental maths through `libm`**, enforced by `clippy.toml` — the platform maths library differs in the last bit between targets.
- **Finite-support smoothing everywhere on render paths** — recursive smoothing has unbounded memory, which would make previews differ from final renders.
- **Zero-phase STFT-mask engine for spectral operations, with a subtractive render** — no phase shift or timing smear, exact residual, provable locality and bit-exact identity settings.
- **Frame grid anchored to the clip start** — results never depend on where a preview window or selection begins.
- **Strict decoding: a corrupt packet is an error** — a silently skipped packet would shift every later timestamp.
- **Own decoder (symphonia), never the browser's `decodeAudioData`** — the browser resamples and differs between browsers.
- **Parameters resolved on the clip before rendering; renders use only resolved values** — resolved values are the exact form of a step, and make replays reproducible.
- **Project directory as the working form, `.nlae` ZIP as the portable form** — true append-only logging while working; one file to share.
- **Own minimal stored-only ZIP writer/reader** — reproducible bundles (sorted entries, fixed timestamps), no compression of evidence audio, few dependencies.
- **Source stored under its original filename** — the invariant forbids renaming; its hash is recorded alongside.
- **No `wasm-opt` on the WebAssembly module** — the parity test verifies exactly what the compiler produced; the optimiser rewrites the binary and is not needed at ~2 MB.
- **React + TypeScript + Vite for the frontend** — widely known, adequate for canvas-heavy views and generated panels.
- **TypeScript types generated from the Rust types (ts-rs)** — the frontend cannot drift from the record formats; CI fails if they are stale.
- **A project that does not verify is read-only, enforced in the core** — nothing can be chained onto a broken log, cloned from it or exported from it; problems are reported, never repaired.
- **A bundle never replaces a library copy unless its log continues the copy's log exactly; an unverified bundle is not added to the library** — opening a file must never lose or overwrite recorded history.
- **Interface state (zoom, selection, colour range, monitor) lives in the manifest, not the log** — it is not evidence; logging every zoom would bury the actions.
- **Creating a project does not analyse it; `analysis.computed` is logged when the analysis is done** — the interface shows a recording before its analysis finishes. In the browser the analysis runs in a separate worker, and the core checks the result's render hash against its source before logging it.
- **Imports are saved to the library in the background, the recording first and straight from the file** — the editor opens without waiting for storage; a project directory is only written once its recording is stored.
- **Hashing is fed in 64 KiB chunks** — the same digest; a WebAssembly engine only switches to optimised code between calls, and one long call hashed a 10-minute file 5× slower.
- **Browser end-to-end tests run against the production build, with fixtures made by the command-line tool** — tests what ships, and proves the two front ends share one record format.
- **The assistant's logic is in the Rust core; the front ends only send the request** — prompt, routing, validation and logging are then identical in the browser and the command line, and the key stays with whatever sends the request.
- **Routine requests are routed locally and deterministically** — lower cost and latency, the same answer every time; the model is kept for descriptive or ambiguous requests.
- **A model answer that cannot be used gets one correction round, then the problems are shown** — a model usually fixes a stated range error at once; more rounds cost time and hide a real misunderstanding.
- **Accepting is the user's event; the steps keep the assistant as their actor** — both who proposed and who decided are on record.
- **A new request sets an open proposal aside, undecided, instead of rejecting it** — a rejection is a judgement the user did not make; the log still shows a preview that was never decided.
- **Exports log the file's SHA-256, from the browser and the command line alike** — an exported file can be matched to its record byte for byte, and the two front ends are shown to produce the same file.
- **The console tests use a stand-in OpenAI-compatible provider started with the test run** — the model path, CORS included, is tested without a network or a key.
- **`nlae relay` only if browsers are refused** — CORS for api.deepseek.com could not be checked from the build environment; *Test connection* settles it, and the relay is built only if needed.
- **Working name `nlae`** [19] — from the repository name, until a product name is chosen and cleared.

## Operations

- **Audacity steps 2–3 become `dc_remove` → `normalise` to −1 dBFS** — in float the intermediate +30 dB changes nothing; removing DC first makes the peak exact; −1 dBFS leaves the final limiter inactive (≈1 dB quieter than the current final amplify). Agreed with the product owner, 2026-09-24.
- **The built-in recipe normalises after the reductions** (DC → noise → lines → normalise → optional compressor) — the reductions move the peak by hundredths of a dB; normalising last makes −1 dBFS exact and keeps the final limiter inactive. Manual order is recorded as performed. Agreed with the product owner, 2026-09-24.
- **Steps 5–6 become `line_reduce`** — automatic line detection, each line cut by its measured prominence, re-measured until nothing stands out; the stop rule becomes a recorded measurement.
- **Noise-reduction parameter names follow Audacity's** — familiar to the product owner; results are not bit-identical to Audacity's.
- **Noise profile stored per bin in the resolved step** — a per-band profile would fail to gate steady lines present in the profile, as Audacity's does; about 1,000 numbers per step.
- **The spectral compressor's `level` mode is held to 5 dB of foreground reduction relative to the background, not 6 dB** — the two synthetic voices overlap in time and frequency, which caps per-cell separation near 5.5 dB on the golden clip; to be checked against real recordings. Agreed with the product owner, 2026-09-24.
- **`line_reduce` v2: a detected line must also stand out in the speech pauses** (or, with under 1 s of pause, be present in ≥ 90% of the clip) — hum and whines continue through pauses, voice harmonics do not; found on the 12 s golden clip, agreed with the product owner 2026-09-24.
- **A change to what `auto` resolves to is a new operation version; the old version stays** — a recorded recipe must replay as it did. `line_reduce` v1 remains registered; the built-in recipe moves to v2 (`builtin:spoken-word-cleanup`, with v1 as `builtin:spoken-word-cleanup@1`); features move to version 2.
- **Out-of-range parameters are rejected, not clamped** — a silent clamp would record a value the user never chose.
- **Limiter always appended by the renderer, ceiling −1 dBFS** — required by the brief; bit-exact passthrough when nothing exceeds the ceiling.
- **Integer WAV export rounds to nearest without dither** — deterministic; float WAV is the default export.
- **Clipping counted as flat-topped runs (≥ 3 equal samples near the peak)** — quiet evidential recordings rarely reach full scale, so a full-scale threshold would miss clipping.
- **Quietest-region search tries 0.25, 0.5, 1, 1.5 and 2 s windows, shortest first, stable if its halves agree within 1 dB** — encodes "the quietest part, using the shortest sample that will work".

## Data, privacy and training

- **AI provider: cloud default (DeepSeek), Ollama local option** — chosen by the product owner; words and analysis numbers only, never audio, enforced by types.
- **A request with a numeric array longer than 64 entries is refused, before sending and again before logging** — a tripwire behind the types: anything that long is data, not a description.
- **Every model exchange is logged in full (`assistant.exchange`), and previews refer to it** — the training record is exactly what was sent and what came back, not a reconstruction.
- **The key is held in memory unless "remember on this device" is ticked, and travels only in the request header** — a key is not evidence and must never reach a project, log, bundle or export; the tests look for it in all of them.
- **Everything is recorded, always** — previews, tweaks, rejections, modifications, manual work; the richest training signal.
- [5] **Rejected attempts included in dataset exports, flagged** — they show what a person judged wrong.
- [12] **Manual work included in exports by default** — the product owner's own local tool; exporting is an explicit act.
- **Audio left out of dataset exports by default; `--with-audio` adds it** — voices from evidential recordings leave a bundle only deliberately; hashes join records back to the audio.
- [6] **Ratings: overall 1–5, optional per-dimension scores, optional note; per step, plan or stack** — quick by default, richer when wanted.
- [7] **Recipes are portable, versioned JSON files, also kept in a local library** — needed for reuse across clips and machines.
- [2] **A bundle embeds the source; clones in the local library share one stored copy** — self-contained sharing without duplicating audio locally.
- [3] **MP3 decoding yes; MP3 export deferred** — decoding is needed for the material; export licensing can wait, WAV is the evidential format.
- [16] **Speech-to-text deferred; the M1 console is typed; prefer a local engine** — Chrome's Web Speech API sends voice to a cloud service.
- [17] **Single-clip product** — multitrack is out of scope for the first build.
- [20] **Provisional performance targets** — 10-minute file shows both lanes < 3 s; playback < 100 ms; 10 s compressor preview < 1 s in WebAssembly.
