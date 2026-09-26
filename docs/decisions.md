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
- [16] **Spoken commands are streamed to Deepgram** — chosen by the product owner, 2026-09-25, in place of a local engine (Whisper): the user's dictated voice goes to Deepgram while the microphone is on; a project's audio never does.
- **Deepgram may keep dictation to improve its models unless the user opts out** — chosen by the product owner, 2026-09-25: Deepgram's default and lower price; the setting sends `mip_opt_out=true`, which costs more and needs a paid account.
- **Dictated words fill the console box, and the user sends them with Enter** — chosen by the product owner, 2026-09-25: a misheard "undo" cannot act on its own, and a correction is recorded next to what was heard.
- **Loudness normalisation aims at −23 LUFS by default (EBU R128)** — chosen by the product owner, 2026-09-25: the broadcast reference; −16 LUFS, common for spoken word online, is one parameter away.
- **A request applies at once; any step of the stack can be removed, restored or edited, with undo and redo** — chosen by the product owner, 2026-09-26: with the model planning several steps at a time, judging each proposal on a 10 s window is the bottleneck, and a live stack is reviewed faster by pruning and tuning it; editing a step in place also stops corrective steps piling up. Supersedes invariants 5 (preview before committing) and 6 (nothing committed without accepting) of the brief; `01-principles.md` has the new wording.
- **A removed step stays in its place, greyed, and can be restored** — chosen by the product owner, 2026-09-26: nothing is lost, even after later changes, and Undo and Redo move through it too.
- **The steps above a removed, restored or edited step keep their recorded values** — chosen by the product owner, 2026-09-26: predictable, and what the user typed stays as typed. A step measured on audio that has since changed (a normalise's gain, a noise profile) is flagged, and *Measure again* is the user's own edit.
- **Exporting approves the stack as it stands** — chosen by the product owner, 2026-09-26: without a per-step *Accept*, the export is the point where a person signs off the processing; `render.exported` already records the stack hash.
- **The command line keeps preview → accept** — chosen by the product owner, 2026-09-26: scripted and batch work (M4) benefits from a preview to a file before committing; it also applies at once with `ask --apply`, and gains `remove`, `restore`, `edit`, `remeasure`, `undo` and `redo`.

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
- **Dictation streams over Deepgram's WebSocket, with the key as the subprotocol** — its REST API refuses web pages (CORS), and a browser cannot set headers on a WebSocket; the key stays out of the address, the core and the log.
- **The core sets the stream's query and logs the transcript; the front end holds the key and streams** — the same split as the model's, so what is asked for and what is recorded are the same in every front end.
- **Raw 16-bit audio at the capture rate, about 100 ms at a time** — no container to wait for, and the format is stated in the query that is logged.
- **Echo cancellation, noise suppression and gain control on for dictation, off for recordings** — dictation is not evidence and they help recognition; echo cancellation also keeps speaker playback out.
- **Playback pauses while the microphone is open** — so a recording playing through the speakers is never streamed.
- **Spoken forms are routed like typed ones** ("minus 1 dB", "1 point 5 seconds", "kilohertz") — a recogniser writes some numbers and units in words, and routine requests should not cost a model call. "Kilohertz" had lost its unit in typed requests too.
- **Tool exposure is tiered by the descriptor's `tier`: `core` operations are always tools; each `on_demand` one is an index line, and its schema is sent when the model asks (`describe_operations`)** — the request stays small as the catalogue grows, and the model still sees everything it could use; the tier is registry data, so adding an operation changes no prompt code.
- **At most one describe round and one correction round per request, decided in the core (`next_round`)** — a second request to see operations means the model is lost rather than short of information; the browser and the command line behave alike.
- **The instructions with the index are prompt version 2** — the version recorded with each exchange keeps version-1 data distinguishable.
- **The `plan` tool names every offered operation, core or on demand** — a plan may use an operation once it has been described, and the tool does not change between rounds.
- **Every change to the stack records the steps above it again, in the same event (`chain`)** — their stack hashes chain through the changed step, so the new hashes, measurements and snapshots must be on record; the projection checks the chain and that no value changed except the edited step's.
- **A step keeps its id through edits and re-recordings** — the id names the stack item for ratings, annotations and clones; every version is in the log.
- **Undo and redo are logged as the inverse change, naming the change they undo or redo (`undoes`, `redoes`)** — nothing is deleted, and both lists are projected from the log, so they survive a reload. Undoing an edit brings back the earlier version exactly, not a re-measured one.
- **`step.removed` (written before 2026-09-26) is read as excluding the top step** — old projects keep verifying, and their removed steps become restorable.
- **A step whose input is unchanged is not rendered again when the stack changes** — determinism makes its output identical, so removing a time edit, or a step that changed nothing, costs almost nothing.
- **Recorded-again steps drop their inferred bindings and infer them on the new input; declared ones are kept** — inference lets a step's existing bindings win, so stale inferred ones would otherwise stick.
- **The render cache is bounded by size, least recently used first** — every change adds renders, and a 10-minute stereo render is about 230 MB; the current stack's render is kept.
- **Drift is recorded as `resolved_on`, only when it differs from the step's input** — steps that never drifted are unchanged in the log, and a step that is measured again, or whose input comes back, loses it.
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
- **Time edits (`remove_time`, `insert_silence`) are applied to the output, after all processing and before the final limiter, with every position in the original's time** — chosen by the product owner, 2026-09-24. Processing steps keep the whole timeline, so scopes never shift and residuals, previews and caches stay exact. A normalise measures the whole recording, including stretches that will be removed (a known limit).
- **Inserted time is digital silence** — chosen by the product owner, 2026-09-24. Nothing is invented in evidential material; generated room tone could come later, marked as synthetic.
- **Joins fade over 5 ms by default, and only kept audio is used** — a hard join clicks; a crossfade would blend in audio that was removed. `fade_ms` 0 gives an exact hard join.
- **Exported WAVs carry a cue marker at each time edit, labelled in the original's time** — chosen by the product owner, 2026-09-24. Anyone opening the file sees where it was edited. The log records the same.
- **A time edit is previewed on its own, opening on *Before*** — its result has a different length, so it can't be drawn on the original's timeline; *Processed* plays it.
- **Time edits are not saved in recipes** — they belong to one recording; replaying them on another clip would cut arbitrary stretches.
- **Limiter always appended by the renderer, ceiling −1 dBFS** — required by the brief; bit-exact passthrough when nothing exceeds the ceiling.
- **Integer WAV export rounds to nearest without dither** — deterministic; float WAV is the default export.
- **Clipping counted as flat-topped runs (≥ 3 equal samples near the peak)** — quiet evidential recordings rarely reach full scale, so a full-scale threshold would miss clipping.
- **Quietest-region search tries 0.25, 0.5, 1, 1.5 and 2 s windows, shortest first, stable if its halves agree within 1 dB** — encodes "the quietest part, using the shortest sample that will work".
- **EQ is static signed gains on the zero-phase engine, in a path of its own (`Reductions::Gains`)** — the EQs keep zero phase, locality and exact residuals; the cut-only operations keep a path that cannot boost at all.
- **High- and low-pass follow a Butterworth magnitude, −3 dB at the cutoff, with a depth cap** — what engineers expect from a cutoff and a slope, without the filter's phase shift; the cap keeps deep cuts finite and makes 0 dB the identity.
- **Bells and shelves are raised cosines in octaves** — smooth, and finite: a bell ends a whole width from its centre, and nothing beyond it is touched.
- **An EQ's analysis size follows the finest feature of its curve (at least 10 bins across it), 2048–32768 points at 48 kHz** — an 80 Hz high-pass needs fine bins; larger sizes only cost time and reach.
- **The gate's range defaults to 12 dB, not silence** — dead silence between words misrepresents a recording and sounds processed; some room tone keeps it honest. `auto` puts the threshold 6 dB above the scope's noise floor (the 10th percentile of its 10 ms levels).
- **The gate opens ahead of the level rising (`attack_ms`)** — it works on the whole recording, not in real time, so it can open before a word starts and never clips an onset.
- **Gate after noise reduction when faint voices matter** — on the raw golden mix it lowers the background voice by 1.3 dB, whose level sits near the noise.
- **`hum_reduce` is separate from `line_reduce`, and "remove the hum" stays `line_reduce`** — `line_reduce` cuts only lines that stand out, as far as they do; `hum_reduce` cuts the mains frequency and each harmonic by a set depth, even under speech. The agreed reference workflow uses `line_reduce`.
- **`hum_reduce` measures the mains frequency from the lines found; with `auto` and no hum found it cuts nothing** — the mains drifts from 50 or 60 Hz, and the error multiplies at each harmonic; cutting where there is no hum only removes signal.
- **`hum_reduce`'s analysis is fine enough for each cut to span four bins, up to 65536 points** — at 16384 points the golden hum came down only 16.6 dB; at 65536, 27.7 dB.
- **`loudness_normalise` is a fixed gain; peaks it pushes over the ceiling are left to the final limiter** — it resolves once, like `normalise`, and the limiter is always there.

## Data, privacy and training

- **AI provider: cloud default (DeepSeek), Ollama local option** — chosen by the product owner; words and analysis numbers only, never audio, enforced by types.
- **A request with a numeric array longer than 64 entries is refused, before sending and again before logging** — a tripwire behind the types: anything that long is data, not a description.
- **Every model exchange is logged in full (`assistant.exchange`), and previews refer to it** — the training record is exactly what was sent and what came back, not a reconstruction.
- **The key is held in memory unless "remember on this device" is ticked, and travels only in the request header** — a key is not evidence and must never reach a project, log, bundle or export; the tests look for it in all of them.
- **Everything is recorded, always** — previews, tweaks, rejections, modifications, manual work; the richest training signal.
- [5] **Rejected attempts included in dataset exports, flagged** — they show what a person judged wrong.
- **Steps that went onto the stack are exported with their fate: approved (the stack was exported as it stands), kept or removed, with every edit, removal and restoration** — without an *Accept*, what became of a step is the decision; an edit is the exact correction a person made. Dataset records are version 2.
- [12] **Manual work included in exports by default** — the product owner's own local tool; exporting is an explicit act.
- **Audio left out of dataset exports by default; `--with-audio` adds it** — voices from evidential recordings leave a bundle only deliberately; hashes join records back to the audio.
- [6] **Ratings: overall 1–5, optional per-dimension scores, optional note; per step, plan or stack** — quick by default, richer when wanted.
- [7] **Recipes are portable, versioned JSON files, also kept in a local library** — needed for reuse across clips and machines.
- [2] **A bundle embeds the source; clones in the local library share one stored copy** — self-contained sharing without duplicating audio locally.
- [3] **MP3 decoding yes; MP3 export deferred** — decoding is needed for the material; export licensing can wait, WAV is the evidential format.
- [16] **Speech-to-text deferred; the M1 console is typed; prefer a local engine** — Chrome's Web Speech API sends voice to a cloud service. *Superseded 2026-09-25: Deepgram, chosen by the product owner (see Product).*
- [17] **Single-clip product** — multitrack is out of scope for the first build.
- [20] **Provisional performance targets** — 10-minute file shows both lanes < 3 s; playback < 100 ms; 10 s compressor preview < 1 s in WebAssembly.
- **Dictation audio is never stored; what was heard and what was sent are logged (`speech.transcribed`) when the words are sent** — the words are the request; the voice is the user's, not evidence, and stays out of bundles. The preview refers to the event, and dataset records carry what was heard, so corrections become training data.
- **Dictation that is never sent is not logged** — nothing from it reached the project.
