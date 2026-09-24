---
title: "Build Brief — Natural Language Audio Editing Suite (for Claude Code)"
created: "September 24, 2026"
modified: "September 24, 2026"
---

# Build Brief — Natural Language Audio Editing Suite (for Claude Code)

# Build Brief — Natural Language Audio Editing Suite (for Claude Code)

## 1\. How to use this brief

This is a hand-off brief for building the application. It states requirements, constraints and intent, recommends an approach, and leaves engineering detail to you.

A much longer parent specification exists as a linked note. It is the full functional-requirement set, including the complete operation catalogue and the provenance model. Consult it for detail; do not treat this brief as a substitute for it.

Where this brief is silent, make your own decision and record it in the repository with a one-line reason. Where you disagree with a recommendation here, deviate — provided the reason is recorded.

The milestones in Section 8 are the definition of progress.

## 2\. What is being built

A tool in which the user records audio or loads a WAV or MP3 file, sees it immediately as a waveform and as a detailed spectrogram, plays it from any point, and edits it by describing the desired result in plain English — typed or spoken — rather than by operating controls.

The product principle is: **describe the goal, not the tool**. The user should never need to know what a filter order is, choose a plugin, or draw a curve. They describe a sound, a problem or an outcome; the application selects and configures the processing itself, previews it on a short window, and explains what it did and why.

There are two ways in, and both are first-class. A **conversation** — the assistant selects typed operations from a registry. **Manual panels** — the user turns dials generated from that same registry, with a per-tool assistant available inside each panel. Both paths must produce the **identical recorded step object**; only the recorded actor and provenance differ.

## 3\. Users and material

Primary material is spoken word and field recordings: interviews, testimony, dictation, phone and remote-call audio, and noisy outdoor recordings. A significant share of it carries **evidential value**, which is why the original file is never modified and why provenance is a core feature rather than a reporting afterthought. Do not optimise exclusively for music production.

## 4\. Recommended platform and why

**Recommendation: a Rust DSP and core engine behind a TypeScript web interface, delivered in two packaging modes from one codebase.**

*   **Browser build.** The Rust core compiled to WebAssembly; audio through the Web Audio API and AudioWorklet for glitch-free playback and processing off the main thread. Shareable by URL with no install.
*   **Desktop build.** Tauri for Linux, macOS and Windows, running the same frontend and the same Rust core natively. Real filesystem access, long renders, batch work, native speed.

**Linux is a first-class target, not an afterthought.**

**Justification.** One core and one codebase buy three operating systems and two distribution models cheaply. The native build gives performance exactly where browser limits bite — memory, large FFTs, long renders. Rust provides one fast DSP implementation shared by both, so the browser and native builds stay behaviourally identical. Tauri is preferred over Electron for binary size and resource use.

**Alternatives, and when they win**

| Option | When it wins | Cost |
| --- | --- | --- |

| Pure native Rust desktop app | Single-binary distribution matters most | Weakest for sharing; heaviest interactive UI work |
| Electron wrapping the web core | Speed of first build dominates everything | Larger, slower, higher memory |
| Pure web app | Sharing matters most and files stay small | Constrained by memory, filesystem and long renders |

**Prototyping.** A Python prototype using librosa, soundfile and pedalboard is a legitimate first step to validate algorithms quickly. Port anything validated that way to the Rust core, and keep a numerical comparison test proving the port matches.

**AI layer.** Deliberately thin and provider-agnostic. An OpenAI-compatible HTTP client is all that is required.

## 5\. Hard invariants

Non-negotiable. Do not trade these away for convenience.

*   The original audio is stored **byte-for-byte** and is never modified, transcoded, normalised, renamed or overwritten — including by housekeeping. All processing runs on a working copy.
*   Everything is **non-destructive**. Accepted edits form an ordered stack that is replayed over the immutable source.
*   **Nothing is ever executed as generated code.** The model may only select typed, schema-validated descriptors from a fixed registry.
*   Every operation is available **both conversationally and manually**, and both paths produce the identical step object.
*   Every operation **previews on a short window** — ten seconds by default — before it is committed anywhere.
*   **Nothing is committed without the user accepting it.**
*   Hard parameter limits are enforced **in the DSP layer**, never only in the prompt. The interface offers no bypass.
*   A **limiter is always last** in the chain.
*   Reconstruction-class work — separation, synthesis, heavy spectral repair — is **labelled processed, not factual** in file metadata, project record and any provenance report.
*   Every step is logged with its **parameters, measurements, rationale and actor**.

## 6\. Architecture

Six layers. Keep the boundaries clean; the interface must hold no audio logic.

1.  **Presentation.** Waveform and spectrogram rendering, transport, region and band selection, the command console, the edit stack, and generated operation panels.
2.  **DSP and analysis core.** STFT, feature extraction and operation implementations. Deterministic, testable, driven only by typed descriptors.
3.  **Operation registry.** One typed, versioned, schema-validated descriptor per operation. From each descriptor both the tool schema for the model and the manual panel for the user are generated.
4.  **Reasoning layer.** Turns language into validated descriptors, plans and groups via an OpenAI-compatible provider.
5.  **Project and provenance layer.** Project bundles, the append-only event log, capture bundles, recipes and hash chains.
6.  **Provider client.** Thin, OpenAI-compatible, configurable base URL, key and model.

**The registry is the keystone.** Panels, validation, logging, replay and the training corpus are all derived from it. Adding an operation must mean adding one descriptor and nothing else.

**Suggested source layout** (by purpose, not by exact filename):

```text
core/          Rust: DSP, analysis, operation implementations, registry
schemas/       Machine-readable operation descriptors + generation of tool schemas
frontend/      TypeScript: UI, waveform/spectrogram rendering, transport, panels, console
desktop/       Tauri shell and native sidecar
shared/        Types, units, validation shared across core and frontend
tests/         Golden set, null tests, determinism and hash-chain tests
```

**Operation categories** (full catalogue lives in the parent specification):

| Category | Description |
| --- | --- |

| Gain and level | Gain, normalisation, compression, gating, limiting, level riding |
| Filters and EQ | High pass, low pass, shelf, bell, notch, tilt, graphic, dynamic EQ, hum removal |
| Noise reduction and restoration | Noise profiling, spectral subtraction, click and crackle repair, de-reverb, de-click |
| Separation and remix | Stem separation, voice isolation, speaker separation, stem blending, ducking |
| Time and pitch | Time stretch, pitch shift, formant shift, gap compression, align, crossfade |
| Stereo and spatial | Downmix, mid/side, width, balance, channel alignment, polarity correction |
| Spectral-domain surgical | Two-dimensional time–frequency patching, masking, spectral repair |
| Creative and gated | Saturation, tape, reverb, delays, lo-fi, and the separately gated identity-altering operations |
| Analysis (read-only) | Spectral statistics, tonal line and prominent band detection, loudness, voice activity, noise floor |
| Selection, scope and utility | Region, band and time–frequency patch selection, rendering, export, provenance, capability discovery |

**Tool exposure is tiered.** Send a small always-present core set; send a compact index of every operation; retrieve full schemas on demand. The catalogue is expected to grow into the hundreds, and sending it whole would be slow, expensive and less accurate.

## 7\. Data and provenance

Describe the shapes; implement the detail.

*   **Project bundle** — one portable file holding the manifest, the source audio, the analysis cache, the event log, the renders and the capture bundles. Reopening restores the full editing context, not just the audio.
*   **Event log** — append-only, JSON Lines, one self-describing object per line, from first import to final export. Never edited in place.
*   **Capture bundle, one per job** — the source SHA-256 plus a copy of the source, every rendered output with its own hash, full-resolution before-and-after spectrogram and waveform images, the machine-readable step chain, a human-readable transcript of the same chain, and a per-step metrics file.
*   **Rejected attempts are logged too**, explicitly flagged as not applied. The canonical chain is what takes the starting file to the ending file; rejections sit alongside it as the richest available signal about what a human judged wrong.
*   **Hash chain** — each log entry contains the hash of the previous entry, so later alteration is detectable.
*   **Recipes** — any step chain, or portion of one, is saveable and replayable on another clip in two modes: **exact** (parameters unchanged) and **adaptive** (parameters re-derived from the new clip's own analysis). **Adaptive is the default**, because an absolute correction is rarely right on a different recording. Every replay is preceded by a dry-run diff.

## 8\. Feature scope and milestones

Sequential. A milestone is complete only when its acceptance criterion is met on the golden test clip.

| ID | Milestone | Goal | Acceptance criterion | Demonstrable |
| --- | --- | --- | --- | --- |

| **M0** | Skeleton | Import or record; waveform and spectrogram; transport; a project that saves and reopens | A project round-trips with audio, analysis and view state intact | Load a file, see both lanes, play it, close, reopen |
| **M1** | The working loop | One natural-language command end to end: preview window, accept and reject, edit stack, render and export, event log | One spoken or typed command produces a previewed, measured, accepted step and a rendered export | Ask for a gain change, hear it, accept it, export the result |
| **M2** | The library and planning | Operation registry at scale, tiered tool exposure, goal-level planning with composite chains, manual panels generated from the registry | A multi-step goal completes as one previewable plan; every descriptor has a working panel | Ask for a goal no single tool covers; then reproduce it by hand |
| **M3** | Provenance and reuse | Capture bundles, recipes, adaptive replay, ratings and dataset export | A job's chain replays on a different clip adaptively, with a dry-run diff and a rating recorded | Replay a chain on a new file and export the dataset record |
| **M4** | Local and automated | Local model support, learned starting chains, proposal-first automated clean-up, batch processing | A new file receives a proposed chain above the confidence threshold, reviewed before rendering | Batch-clean a folder with per-file review |

## 9\. Testing and quality

*   **Golden set** — short clips with known problems (hum, hiss, rumble, harsh sibilance, clipping, reverberation) with expected outcomes. Prompt and algorithm changes are tested against it.
*   **Null test per operation** — an identity parameter must render bit-identical output.
*   **Numerical comparison test** wherever an algorithm was prototyped elsewhere, proving the port matches.
*   **Determinism test** — the same source and the same step chain always render bit-identical output.
*   **Hash-chain integrity test** — tamper detection works and reports the first failure.
*   **Build-time parity test** — the build fails if a descriptor has a tool schema without a panel, or the reverse.

## 10\. Provider and cost strategy

OpenAI-compatible endpoints only, so any conforming provider works. Ship **DeepSeek** as the default. Support **local models through Ollama** as the zero-cost, offline option. Keys live in the operating system keychain and are never written into the project file. Cache the analysis per clip and reuse it for every subsequent command. Answer routine intents locally without calling the model at all. Reserve the model for genuinely descriptive or ambiguous requests, so per-command cost and latency stay low and predictable.

## 11\. Open questions to resolve

1.  Browser-first or desktop-first for the first usable build? — Determines what M0 has to prove and where the effort goes early.
2.  Does a capture embed a copy of the source audio, or only reference it with a hash? — Embedding is self-contained; referencing is light but breaks when the original moves.
3.  How is MP3 and other codec licensing handled in a distributed build? — Determines whether MP3 export ships at all, and how the build is configured.
4.  Which separation and voice-isolation models are built on, and under what licences? — They must run locally, so size, speed and licence all constrain the choice.
5.  Are rejected attempts included in exported datasets? — Affects the training corpus and the privacy and evidentiary posture.
6.  Is the rating a single score or per-dimension? — Richer training data against slower feedback.
7.  Are recipes shared as files, or held only in a local library? — Determines whether a shared recipe format and its versioning are needed now.
8.  What is the retention policy for logs and captures? — Kept indefinitely, pruned by age, or capped by size — and is the user asked before anything is discarded.
9.  Does the capture bundle travel when a project is shared? — Determines what a recipient receives, and how much voice data leaves the machine.
10.  Is the operation catalogue browsed, searched, or found only conversationally? — Determines how much interface work the breadth of the library demands.
11.  How much of the creative and identity-altering catalogue is exposed at all, and how is it gated? — A policy question with evidentiary consequences, not a UI preference.
12.  Does manual work enter the training corpus by default, or only on opt-in? — Weighs the value of the data against privacy and consent.
13.  May the per-tool panel assistant change scope or selection, or only parameters? — A change of scope is arguably a different request.
14.  May a panel hold a short local chain, or does all multi-step intent escalate to the console? — Determines whether composition happens in exactly one place.
15.  How should the assistant behave on a small-context local model? — Reduced modes, or an honest statement that assistance is unavailable.
16.  Which speech-to-text engine is used for dictation, and must it run locally? — Determines offline capability and whether voice leaves the machine.
17.  Is multitrack ever genuinely wanted, or is the product fundamentally single-clip? — Shapes the data model if the answer is yes.
18.  Are third-party plugins in scope at all? — If so, under what sandbox and licence constraints.
19.  What is the product called, and is the name clear of trademarks? — Needed before any distribution.
20.  What are the target performance numbers for preview latency and full-clip render time? — Turns responsiveness from an aspiration into a test.

Unresolved questions may be answered provisionally by the implementer, with the decision and its reasoning recorded in the repository, rather than blocking progress.

## 12\. Out of scope for the first build

*   A full multitrack digital audio workstation with mixing, routing and automation.
*   MIDI sequencing or virtual instrument hosting.
*   Real-time collaborative editing by multiple users.
*   Hosting third-party audio plugins.
*   Cloud processing of audio.

The parent specification note should be read in full before any part of the operation library or the provenance model is implemented.