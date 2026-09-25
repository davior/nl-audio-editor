# 02 — Architecture

## Layers

Six layers with clean boundaries. The interface holds no audio logic.

1. **Presentation** (`frontend/`, TypeScript + React) — waveform and spectrogram lanes,
   transport, time and time × frequency selection, console, stack panel, clone family view,
   log viewer, generated operation panels. Draws only what the core returns.
2. **DSP and analysis core** (`core/src/dsp`, `core/src/analysis`, `core/src/ops`) — STFT,
   features, operation implementations. Deterministic, driven only by typed, validated
   descriptors.
3. **Operation registry** (`schemas/ops/`, `core/src/ops/registry.rs`) — one versioned JSON
   descriptor per operation. Validation, panels, tool schemas, logging, replay and the
   training corpus all derive from it.
4. **Reasoning layer** (M1) — turns language into validated descriptors and plans.
5. **Project and provenance layer** (`core/src/project`, `core/src/provenance`,
   `core/src/recipe`, `core/src/dataset`) — projects, the event log, clones, bundles,
   recipes, dataset export.
6. **Provider clients** (M1) — thin: OpenAI-compatible for the model, and a stream to Deepgram
   for spoken requests. They hold the keys; the core builds what they send and logs what comes
   back.

## Builds

One Rust core, one TypeScript frontend, several packagings:

| Build | Status | Notes |
|---|---|---|
| Command-line tool `nlae` (`cli/`) | this session | The first actor on the stack; the base for batch automation |
| Browser (`wasm/` + `frontend/`) | this session (M0) | Core compiled to WebAssembly, run in a Web Worker |
| Desktop (Tauri, `desktop/`) | later | Same frontend, core native; filesystem, keychain, long renders |

Linux is a first-class target.

## Data flow

```
source bytes ──decode (symphonia, strict)──► working copy (f32, planar)
                                                  │
                    stack of resolved steps ──────┤  render engine
                                                  ▼
                               output  +  residual (= input − output)
                                                  │
              features before/after, measurements, hashes ──► event log (hash chain)
```

- **Resolution.** When a step is previewed, its parameters are *resolved* on its actual
  input: `auto` values are measured (e.g. the peak for `normalise`, the lines for
  `line_reduce`, the quietest region for `noise_reduce`). Rendering only ever uses resolved
  values. Resolved values are the step's exact form.
- **Rendering.** Each operation declares its reach (how many input samples either side an
  output sample depends on). A preview of a window renders only that window plus the
  accumulated reach of the stack. Full renders run step by step over the whole clip; the
  heavy spectral operations process in bounded chunks internally, which exercises the same
  reach guarantee.
- **Caching.** Full renders are cached by stack hash; features by render hash.

## Determinism across targets

Native and WebAssembly renders must be bit-identical, so:

- The FFT is our own (`core/src/dsp/fft.rs`): radix-2, f64, fixed operation order, no SIMD,
  no fused multiply-add. Twiddles come from `libm`.
- Every transcendental function (`pow`, `exp`, `log10`, `sin`, `cos`, …) goes through
  `core/src/math.rs`, which calls the pure-Rust `libm` crate. `clippy.toml` forbids the
  standard-library versions, whose results differ in the last bit between platforms.
- Basic IEEE-754 arithmetic and `sqrt` are correctly rounded everywhere and need nothing
  special.
- Decoding is pure Rust (symphonia with SIMD features off).
- Tests compare native and WebAssembly render hashes on the golden clips.

## Storage

- **Working form** — a project directory (native) or an OPFS directory (browser) with the
  layout in `03-projects-clones-provenance.md`. The log is appended in place; the source is
  written once.
- **Portable form** — a `.nlae` file: a ZIP of the same layout, audio stored uncompressed,
  entries in sorted order with fixed timestamps so packing is reproducible.

## Source layout

```
core/      Rust: DSP, analysis, operations, registry, stack, provenance, recipes, dataset
wasm/      wasm-bindgen API over the core (runs in a Web Worker)
cli/       nlae command-line tool
schemas/   operation descriptors and JSON Schemas for every record
recipes/   built-in recipes
shared/    TypeScript types generated from the Rust types (ts-rs)
frontend/  TypeScript interface
desktop/   Tauri shell (planned)
tests/     golden-set notes and expected hashes
docs/      this specification, decisions, the build brief
```
