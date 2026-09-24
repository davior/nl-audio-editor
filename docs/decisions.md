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
- **Browser end-to-end tests run against the production build, with fixtures made by the command-line tool** — tests what ships, and proves the two front ends share one record format.
- **Working name `nlae`** [19] — from the repository name, until a product name is chosen and cleared.

## Operations

- **Audacity steps 2–3 become `dc_remove` → `normalise` to −1 dBFS** — in float the intermediate +30 dB changes nothing; removing DC first makes the peak exact; −1 dBFS leaves the final limiter inactive (≈1 dB quieter than the current final amplify).
- **The built-in recipe normalises after the reductions** (DC → noise → lines → normalise → optional compressor) — the reductions move the peak by hundredths of a dB; normalising last makes −1 dBFS exact and keeps the final limiter inactive. Manual order is recorded as performed. (`OPEN:` for the product owner.)
- **Steps 5–6 become `line_reduce`** — automatic line detection, each line cut by its measured prominence, re-measured until nothing stands out; the stop rule becomes a recorded measurement.
- **Noise-reduction parameter names follow Audacity's** — familiar to the product owner; results are not bit-identical to Audacity's.
- **Noise profile stored per bin in the resolved step** — a per-band profile would fail to gate steady lines present in the profile, as Audacity's does; about 1,000 numbers per step.
- **Out-of-range parameters are rejected, not clamped** — a silent clamp would record a value the user never chose.
- **Limiter always appended by the renderer, ceiling −1 dBFS** — required by the brief; bit-exact passthrough when nothing exceeds the ceiling.
- **Integer WAV export rounds to nearest without dither** — deterministic; float WAV is the default export.
- **Clipping counted as flat-topped runs (≥ 3 equal samples near the peak)** — quiet evidential recordings rarely reach full scale, so a full-scale threshold would miss clipping.
- **Quietest-region search tries 0.25, 0.5, 1, 1.5 and 2 s windows, shortest first, stable if its halves agree within 1 dB** — encodes "the quietest part, using the shortest sample that will work".

## Data, privacy and training

- **AI provider: cloud default (DeepSeek), Ollama local option** — chosen by the product owner; words and analysis numbers only, never audio, enforced by types.
- **Everything is recorded, always** — previews, tweaks, rejections, modifications, manual work; the richest training signal.
- [5] **Rejected attempts included in dataset exports, flagged** — they show what a person judged wrong.
- [12] **Manual work included in exports by default** — the product owner's own local tool; exporting is an explicit act.
- **Audio left out of dataset exports by default; `--with-audio` adds it** — voices from evidential recordings leave a bundle only deliberately; hashes join records back to the audio.
- [6] **Ratings: overall 1–5, optional per-dimension scores, optional note; per step, plan or stack** — quick by default, richer when wanted.
- [7] **Recipes are portable, versioned JSON files, also kept in a local library** — needed for reuse across clips and machines.
- [2] **A bundle embeds the source; clones in the local library share one stored copy** — self-contained sharing without duplicating audio locally.
- [3] **MP3 decoding yes; MP3 export deferred** — decoding is needed for the material; export licensing can wait, WAV is the evidential format.
- [16] **Speech-to-text deferred to M1; prefer a local engine** — Chrome's Web Speech API sends voice to a cloud service.
- [17] **Single-clip product** — multitrack is out of scope for the first build.
- [20] **Provisional performance targets** — 10-minute file shows both lanes < 3 s; playback < 100 ms; 10 s compressor preview < 1 s in WebAssembly.
