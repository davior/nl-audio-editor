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
| `.vscode/` | tasks, debug configurations and recommended extensions for VS Code |

## Setting up a development machine (Linux)

The browser app is the Rust core compiled to WebAssembly, plus a TypeScript front end, so it
needs both tool chains:

| Tool | What it is for | Version |
|---|---|---|
| Rust, through rustup | the core and the command line | stable (1.80 or newer) |
| the `wasm32-unknown-unknown` target | compiling the core for the browser | — |
| a C compiler and linker | Rust links with it, and one dependency compiles C | any |
| wasm-pack | builds the core for the browser, with its JavaScript bindings | 0.13 or newer |
| Node.js | the front end's tools (Vite, TypeScript, Playwright) | 22 or newer |
| pnpm | the front end's package manager | 10.33.0, pinned in `frontend/package.json` |

On **Arch Linux** (and Manjaro):

```sh
# 1. Compiler and linker, Git, Node.js with npm, and rustup
sudo pacman -S --needed base-devel git nodejs npm rustup

# 2. Rust, and the target the browser build compiles to
rustup default stable
rustup target add wasm32-unknown-unknown

# 3. wasm-pack: compiled from source, a few minutes, once. Cargo puts programs in
#    ~/.cargo/bin, which Arch's rustup package leaves off PATH (use ~/.zshrc for zsh)
cargo install wasm-pack
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc && source ~/.bashrc

# 4. pnpm, through corepack. Node.js 25 and later no longer include corepack
sudo npm install -g corepack
sudo corepack enable pnpm
```

Check (each should print a version, or the target's name):

```sh
cargo --version
rustup target list --installed | grep wasm32
wasm-pack --version
node --version                  # v22 or newer
cd frontend && pnpm --version   # 10.33.0: corepack takes it from package.json
```

The first time pnpm runs, corepack asks whether to download that version: answer yes.

- **Other distributions:** install a C compiler and Git (`build-essential git` on Debian and
  Ubuntu, `gcc git` on Fedora), get Rust from <https://rustup.rs> (it adds `~/.cargo/bin` to
  `PATH` itself), then continue from step 2. If your packaged Node.js is older than 22 (Ubuntu
  24.04's is 18), install Node 22 with [nvm](https://github.com/nvm-sh/nvm): `nvm install` in
  this repository reads `.nvmrc`.
- **Node.js from nvm or fnm:** run `corepack enable pnpm` without `sudo`. Node 22 and 24
  include corepack, so the `npm install -g corepack` line is only needed from Node 25.
- **Without corepack:** `npm install -g pnpm` works too; pnpm switches itself to the pinned
  version.

## Running the browser app while developing

```sh
cd frontend
pnpm install      # once, and again when package.json changes
pnpm run wasm     # build the core for the browser: under a minute the first time, ~20 s after a change
pnpm dev          # http://localhost:5173
```

Open <http://localhost:5173> in Chrome or Chromium, the browser the tests use. The first
`pnpm run wasm` also downloads a matching `wasm-bindgen` tool, once.

What to run again after a change:

| You changed | Run |
|---|---|
| front-end code (`frontend/src`) | nothing: the page updates by itself |
| Rust code in `core/` or `wasm/` | `pnpm run wasm`; the page reloads when it finishes |
| a Rust type the front end uses | `cargo test -p nlae-core --features ts export_bindings` (regenerates `shared/types`) |
| pulled new commits | `pnpm install`, then `pnpm run wasm` |

- `pnpm build && pnpm preview` serves the production build on <http://localhost:4173>, which
  is what the end-to-end tests run against.
- `pnpm run wasm:dev` builds the core with debug information, but the audio processing then
  runs far slower. Use it only to debug inside the WebAssembly.

**Using the app.** Import or record a recording, or open a `.nlae` bundle (for example one made
with `nlae pack`). Projects are kept in the browser's private storage.

Say what should happen in the console under the lanes: "clean this recording up", "cut 3,100
to 3,200 Hz by 12 dB", or, with an area selected on the spectrogram, "compress the bangs here".
Each proposal is previewed on a short window (Original / Before / Processed / Residual) and
waits for *Accept* or *Reject*.

To take a stretch out of the recording, select it on the waveform and use *Remove this
stretch*; to add a pause, click where it should go and use *Insert … s of silence* (or type
"remove 12 to 15.5 seconds", "insert 2 s of silence at 30 s"). The original is untouched: the
edits apply to the output, the lanes keep the original's timeline with removed stretches
hatched, *Processed* plays the result, and the exported WAV has a cue marker at each edit.

**The model.** Routine requests like those are handled without a model and need no setup.
Anything else goes to the provider chosen in the console's settings, with your words and
analysis numbers only, never audio. The key stays in the browser and is never written to a
project.
- **DeepSeek** (the default) needs a key. Whether DeepSeek accepts requests from a web page
  is not confirmed yet: *Test connection* in the settings tells you.
- **Ollama** must allow the app's addresses:
  `OLLAMA_ORIGINS=http://localhost:5173,http://localhost:4173 ollama serve`.
- **On the command line**, `nlae ask` reads the key from `DEEPSEEK_API_KEY`.

## Command line

```sh
cargo build --release -p nlae-cli          # → target/release/nlae
nlae golden --short out/                   # a 12 s synthetic test clip (omit --short for the full set)
nlae new out/golden_a_short.wav -o case    # a project: the source is stored byte for byte
nlae ask case "clean this recording up"    # routine requests are handled locally…
nlae ask case "the hum is distracting"     # …the rest go to the model (DEEPSEEK_API_KEY; --provider ollama)
nlae ask case "remove 2 to 3 seconds"      # time edits apply to the output; the original is untouched
nlae plan case --recipe builtin:spoken-word-cleanup --out p.wav --residual r.wav
nlae accept case <preview-id>              # commit a preview (ids are printed by `ask` and `plan`)
nlae stack case                            # the stack, with what each step measured
nlae clone case --at <step-id> -o case-b   # follow another stream of editing
nlae render case -o cleaned.wav            # the stack, with the final limiter
nlae pack case -o case.nlae                # a portable bundle; `nlae verify case.nlae` checks it
nlae save-recipe case -o mine.recipe.json  # reuse the stack on other clips (`nlae replay`)
nlae dataset case case-b -o ds/            # training records (no audio unless --with-audio)
```

`nlae --help` lists every command. `cargo run --release -p nlae-cli -- <command>` builds and
runs it in one step.

## VS Code

Open the repository's **root** folder, not `frontend/`: the Rust workspace and the
`.vscode/` files live there.

- **Extensions.** VS Code offers the recommended ones: rust-analyzer, CodeLLDB (debugger),
  Prettier, Playwright and Vitest.
  - Arch's `code` package is *Code – OSS*, which installs extensions from Open VSX.
  - `visual-studio-code-bin` (AUR) uses Microsoft's marketplace.
  - If one isn't found, only that editor feature is missing: everything works from the
    terminal.
- **Tasks** (Terminal → Run Task…):
  - *Build the core for the browser*;
  - *Dev server*, which builds the core first if it has never been built;
  - *Production preview*.
- **Debugging** (Run and Debug, or F5):
  - ***Browser app (Chrome)*** and ***Browser app (Chromium)*** start the dev server if it isn't
    running, then open the browser with the debugger attached. Breakpoints work in the
    TypeScript, including the workers that run the audio core. The debug browser keeps its own
    profile, so its library is separate from your everyday browser's.
  - ***nlae command line*** runs the command-line tool under CodeLLDB. Edit its `args` in
    `.vscode/launch.json`. Debug builds of the core are unoptimised, so step through a short
    clip.
  - **Rust tests**: click *Debug* above any test (rust-analyzer).
- **Formatting.** Saving formats Rust with rustfmt, and the front end's TypeScript and CSS with
  Prettier, the way CI checks them.

## Tests

```sh
cargo test --workspace                     # core and command line
wasm-pack test --node wasm                 # the WebAssembly build computes the native hashes
cd frontend && pnpm test                   # unit tests
pnpm exec playwright install chromium      # once: the browser the end-to-end tests drive
pnpm build && pnpm e2e                     # the interface end to end (with a stand-in model)
```

**Also run by CI:**
- `cargo fmt --all --check`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `pnpm format:check` and `pnpm typecheck`;
- a check that the TypeScript types generated from Rust are current:
  `cargo test -p nlae-core --features ts export_bindings`, then `git diff shared/types`.

Playwright's `--with-deps` option installs system libraries on Debian and Ubuntu only. On
Arch, the downloaded Chromium normally runs as is; if it reports a missing library, install
that library with pacman.

## Troubleshooting

| What you see | What to do |
|---|---|
| `pnpm: command not found` | Step 4: `sudo npm install -g corepack && sudo corepack enable pnpm` |
| `corepack enable` fails with `EACCES` (permission denied) | Node.js is installed system-wide: use `sudo`, or install Node.js with nvm |
| `wasm-pack: command not found` just after installing it | `~/.cargo/bin` is not on `PATH`: see step 3, then open a new terminal |
| `` the `wasm32-unknown-unknown` target may not be installed ``, after ``can't find crate for `core` `` (or `` `std` ``) | `rustup target add wasm32-unknown-unknown` |
| `rustup: command not found` | Rust came from Arch's `rust` package: `sudo pacman -S rustup` replaces it, then `rustup default stable` |
| ``error: linker `cc` not found`` | `sudo pacman -S --needed base-devel` |
| `Vite requires Node.js version 20.19+ or 22.12+` | Node.js is too old: install Node 22 or newer (with nvm, if your distribution's is older) |
| The page opens, but a red banner says *The audio core did not load*; the terminal shows `Failed to resolve import "../core-wasm/nlae.js"` | The core hasn't been built: `pnpm run wasm`, then reload the page |
| `Error: Port 5173 is already in use` | A dev server is already running, in another terminal or as VS Code's *Dev server* task |
| A change to the Rust code doesn't show up | `pnpm run wasm` again; the page reloads when it finishes |
| Saving or recording doesn't work when the page is opened from another computer | Browsers only allow storage and the microphone on `localhost` or HTTPS. Forward the port (`ssh -L 5173:localhost:5173 <host>`, or VS Code's port forwarding) and open <http://localhost:5173> |
| End-to-end tests: `Executable doesn't exist at …` | `pnpm exec playwright install chromium` |
