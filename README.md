# nlae — natural-language audio editor (working name)

A non-destructive, provenance-first audio editor for spoken word, field recordings and
evidential material. You describe the result you want in plain English; the application
selects and configures the processing, previews it on a short window, and records every
action as a reusable, analysable stack.

The original recording is stored byte for byte and never changed. Every action — previews,
changes, rejections, manual work — goes into a hash-chained log, so a project can be verified,
replayed on another clip, and used as training data. Clones copy a project at any step to follow
a different stream of editing.

## Layout

| Path | What |
|---|---|
| `core/` | Rust core: decoding, analysis, the operations, projects, the log, recipes, dataset export |
| `cli/` | `nlae`, the command-line tool |
| `wasm/` | WebAssembly bindings used by the browser app |
| `frontend/` | the browser app (React + TypeScript + Vite; the core runs in a Web Worker) |
| `schemas/` | JSON Schemas for events, steps, recipes, dataset records and operation descriptors |
| `recipes/builtin/` | built-in recipes (`spoken-word-cleanup`, from the reference workflow) |
| `docs/spec/` | the parent spec; `docs/decisions.md` records every decision and its reason |

## Command line

```sh
cargo build --release -p nlae-cli          # → target/release/nlae
nlae golden --short out/                   # a 12 s synthetic test clip (omit --short for the full set)
nlae new out/golden_a_short.wav -o case    # a project: the source is stored byte for byte
nlae plan case --recipe builtin:spoken-word-cleanup --out p.wav --residual r.wav
nlae accept case <preview-id>              # commit the plan (ids are printed by `plan`)
nlae stack case                            # the stack, with what each step measured
nlae clone case --at <step-id> -o case-b   # follow another stream of editing
nlae render case -o cleaned.wav            # the stack, with the final limiter
nlae pack case -o case.nlae                # a portable bundle; `nlae verify case.nlae` checks it
nlae save-recipe case -o mine.recipe.json  # reuse the stack on other clips (`nlae replay`)
nlae dataset case case-b -o ds/            # training records (no audio unless --with-audio)
```

`nlae --help` lists every command.

## Browser app

Needs Rust with the `wasm32-unknown-unknown` target, `wasm-pack`, Node 22 and pnpm.

```sh
cd frontend
pnpm install
pnpm run wasm      # build the core for the browser
pnpm dev           # http://localhost:5173
```

Import or record a recording, or open a `.nlae` bundle (for example one made with
`nlae pack`). Projects are kept in the browser's private storage.

## Tests

```sh
cargo test --workspace                     # core and command line
wasm-pack test --node wasm                 # the WebAssembly build computes the native hashes
cd frontend && pnpm test                   # unit tests
pnpm build && pnpm e2e                     # the interface end to end in Chromium
```
