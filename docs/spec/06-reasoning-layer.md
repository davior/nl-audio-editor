# 06 — Reasoning layer (M1)

## Role

Turn plain-English requests into validated step descriptors and plans. The model **selects**
from the registry; it never writes code, and everything it proposes is validated in Rust,
previewed on a short window and accepted by the user before it is committed.

## Provider client

- Thin and provider-agnostic: any OpenAI-compatible chat-completions endpoint with tool calls.
  Configurable base URL, key and model.
- **Default: DeepSeek** (cloud). **Local option: Ollama.** The provider in use is recorded on
  every assistant step (`actor.model`, `actor.provider`).
- Keys: in the browser build, held in memory for the session by default, with an explicit
  "remember on this device" option; in the desktop build, the OS keychain. Never in a project
  file, bundle, log or export.
- `OPEN:` browser calls to cloud providers may be blocked by CORS. If so, the browser build
  needs a small local relay or the desktop build; to be checked at the start of M1.

## What the model receives

**Words and analysis numbers only — never audio.** The prompt builder accepts only
`AnalysisSummary`-style types (features, measurements, the stack's step summaries, the
registry index), so sample data cannot be serialised into a prompt by accident. This is the
privacy posture for evidential material with a cloud default.

## Routine requests are answered locally

Common intents are routed without calling the model, which keeps cost and latency low and
predictable:

- "clean this recording up" → the built-in recipe `spoken-word-cleanup`, adaptive;
- "undo", "remove the last step", "play the residual", "compare with the original";
- direct commands with explicit numbers ("cut 3,100 to 3,200 Hz by 12 dB").

The model is reserved for descriptive or ambiguous requests.

## Tiered tool exposure

A small always-present core set (gain, normalise, noise reduction, line reduction, spectral
compressor, plan), a compact index of every operation (id, title, one-line summary), and full
schemas retrieved on demand. The catalogue is expected to grow into the hundreds.

## Plans

A goal no single operation covers becomes a **plan**: an ordered group of steps previewed on
one window and accepted as a unit, with individual steps switchable off before acceptance.
Plans are how recipes are replayed, so the model can propose "the spoken-word clean-up, but
without compression" by starting from a recipe.

## Open questions from the brief

- `OPEN:` (13) May the per-tool panel assistant change scope or selection, or only parameters?
  Proposal: parameters only; a change of scope is a new request in the console.
- `OPEN:` (14) May a panel hold a short local chain? Proposal: no; composition happens in one
  place (plans in the console).
- `OPEN:` (15) Behaviour on a small-context local model? Proposal: a reduced mode with the core
  tool set and recipes only, and an honest statement when a request needs more.
- `OPEN:` (16) Speech-to-text for spoken commands: Chrome's Web Speech API sends voice to Google,
  so a local engine (e.g. Whisper compiled to WebAssembly) is preferred.
