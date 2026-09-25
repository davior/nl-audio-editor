# 06 — Reasoning layer (M1)

## Role

The reasoning layer turns plain-English requests into validated step descriptors and plans.
- The model **selects** from the registry; it never writes code.
- Everything it proposes is validated in Rust, previewed on a short window, and committed
  only when the user accepts it.

The core (`core/src/assistant/`) owns everything that must be the same in every front end and
must be recorded:
- what the model is shown;
- which requests are answered without it;
- how its answers are checked;
- what is logged.

The front ends only send the request: the browser (`frontend/src/assistant/provider.ts`) and
the command line (`nlae ask`). They hold the key, so the key never passes through the core.

## Provider client

- It is thin and provider-agnostic: any OpenAI-compatible `/chat/completions` endpoint with
  tool calls. The base URL, model and key are configurable.
- **Presets:**
  - **DeepSeek** (`https://api.deepseek.com`, `deepseek-chat`), the default;
  - **Ollama** on this computer (`http://localhost:11434/v1`, `qwen2.5`);
  - any other OpenAI-compatible address, with a provider name of your choosing.
- The provider and model are recorded on every step the assistant proposes (`actor.model`,
  `actor.provider`) and on every exchange.
- **Keys:**
  - The key is sent in the `Authorization` header only.
  - In the browser it is held in memory for the session. It goes into this browser's local
    storage only when *Remember on this device* is ticked, and the setting says so.
  - The command line reads it from `DEEPSEEK_API_KEY` or `NLAE_API_KEY`.
  - It is never part of a request body, a project file, a log, a bundle or an export. The
    end-to-end tests check the library, browser storage and a saved bundle for it.
- *Settings → Test connection* sends one short message that carries nothing from any project.
  It reports the host and latency, or the error.

`OPEN:` It is not yet known whether DeepSeek accepts requests from a web page (CORS).
- It could not be checked from the build environment, whose network policy denies
  api.deepseek.com.
- The browser's error message names CORS as a likely cause. *Test connection* answers the
  question in a minute from any browser.
- If browser calls are refused, a 127.0.0.1-only relay in the command line (`nlae relay`)
  will forward them, with the key read from the environment. It is not built until it is known
  to be needed.
- Ollama accepts browser calls when `OLLAMA_ORIGINS` includes the page's address.

## What the model receives

**Words and analysis numbers only — never audio.**

The request is built by `build_request` from an `AssistantContext`:
- the analysis summary of what the stack currently produces (the same numbers the dataset
  export uses);
- a summary of each step on the stack, with long lists counted rather than listed;
- the selection: time range and time × frequency area.

With those go:
- the user's words;
- the last eight turns of the conversation, as words;
- the tools.

No function on this path takes an audio buffer. As a tripwire, any numeric array longer than 64
entries is refused as data rather than description. The check runs three times:
1. on the context while it is still structured;
2. on the whole request;
3. again when the exchange is logged.

The system prompt is `SYSTEM_PROMPT`, versioned by `PROMPT_VERSION` (currently 1). The version
is recorded with every exchange, so a change of wording shows in the data. Dataset records use
the same prompt and the same user-message format.

**Tools.**
- One tool per operation: the latest version of every operation a user may apply, currently
  nine.
- The final limiter is applied automatically and is not offered.
- A `plan` tool proposes several operations to be previewed and accepted together.
- Tiered exposure, for when the catalogue grows, is M2.

## Routine requests are answered locally

`route` handles common intents without calling the model, which keeps cost and latency low and
predictable. It is deterministic and tested with a table of cases.

| What you type | What happens |
|---|---|
| "clean this recording up", "tidy it up", "clean-up" | the built-in `spoken-word-cleanup` recipe, replayed adaptively as one plan (logged as `recipe.replayed`) |
| "undo", "remove the last step", "take that back" | the top step is removed (it stays in the log) |
| "play the residual", "compare with the original", "listen to the result" | the monitor switches (during a preview, to the preview's own monitors) |
| "cut 3,100 to 3,200 Hz by 12 dB" | `band_cut` with those values |
| "remove the 750 Hz line" | `line_reduce` in a band around 750 Hz |
| "normalise to −1 dB", "turn it up by 3 dB" | `normalise`, `gain` |
| "remove the hum", "reduce the noise", "remove the DC offset" | `line_reduce` (40–1,000 Hz), `noise_reduce`, `dc_remove` |
| "compress the bangs here", "tame the whine in this area" | `spectral_compressor` on the selected area (transient, tonal or level mode from the words) |
| "remove 12 to 15.5 seconds", "cut from 1:20 to 1:35", "trim the first 5 seconds" | `remove_time` for that stretch |
| "remove this part", "cut the selection out" (with a time selection, naming nothing to process) | `remove_time` for the selected stretch |
| "insert 2 seconds of silence at 30 s", "add a 500 ms gap at 1:05", "add 1 s of silence here" | `insert_silence` there (seconds, milliseconds, minutes and m:ss are understood) |

- "Here", "this part" or "the selection" makes the selection the scope.
- "Remove this part" removes time only when the words name nothing to process: "remove the
  hum here" cuts the hum in the selection.
- A named target beats the clean-up recipe: "clean up the hum" cuts the hum.
- Anything else goes to the model.

## What comes back

`parse_response` reads the tool calls:
- Every proposed operation must exist, must not be a system operation, must accept the scope
  given, and must have parameters within their limits.
- Out-of-range values are **refused, never clamped**.
- What passes becomes step drafts with:
  - the assistant as actor;
  - *console* as origin;
  - the user's words as `intent`;
  - the model's explanation as `rationale`.
- A reply in words only (a question, say) is shown as a reply.

When the answer cannot be used:
1. The model is asked **once** to correct itself (`build_correction`): the problems are
   returned as the result of each tool call.
2. If the second answer fails too, the problems are shown to the user.

## Every exchange is recorded

Each request and its answer is logged as `assistant.exchange` (actor: the assistant) with:
- provider, model, and the host the request went to (no path, no key);
- the request exactly as sent, its SHA-256 (`request_sha256`) and `prompt_version`;
- the response and the latency;
- `problems`, when the answer could not be used;
- `corrects`, the hash of the exchange this one corrects.

The preview made from an answer refers to its exchange (`PreviewRecord.exchange`). The steps
it produces carry the model and provider. The acceptance or rejection is the user's own event.

The dataset export (`10-learning-and-automation.md`) turns a logged exchange into a chat
record directly (`metadata.source: "exchange"`): exactly what was sent, what came back, and
what the user decided. Steps made without the model are reconstructed in the same format
(`"reconstructed"`).

## The command line

`nlae ask <project> "<words>"` runs the same loop:
1. it routes the words, or asks the model;
2. it logs the exchange;
3. it previews;
4. it prints the proposal and its measurements.

Accepting and rejecting are the existing `nlae accept` / `nlae reject`.
- `--provider deepseek|ollama|<https URL>`, `--model`.
- `--response-file` uses a saved model answer instead of calling a provider, so tests need no
  network or key.

## Plans

A goal no single operation covers becomes a **plan**:
- an ordered group of steps, previewed on one window and accepted as a unit;
- individual steps can be switched off before accepting.

Recipes are replayed as plans. That lets the model propose, for example, "the spoken-word
clean-up, but without compression" by starting from a recipe.

## Open questions from the brief

- `OPEN:` (13) May the per-tool panel assistant change scope or selection, or only
  parameters? Proposal: parameters only; a change of scope is a new request in the console.
- `OPEN:` (14) May a panel hold a short local chain? Proposal: no; composition happens in one
  place (plans in the console).
- `OPEN:` (15) How should it behave on a small-context local model? Proposal: a reduced mode
  with the core tool set and recipes only, and an honest statement when a request needs more.
- `OPEN:` (16) Speech-to-text for spoken commands. Chrome's Web Speech API sends voice to
  Google, so a local engine (e.g. Whisper compiled to WebAssembly) is preferred. Typed requests
  only so far.
