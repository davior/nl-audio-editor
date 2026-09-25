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

The instructions are `SYSTEM_PROMPT` followed by the index of on-demand operations
(`system_message`), versioned by `PROMPT_VERSION` (currently 2: version 1 had no index and
offered every operation as a tool). The version is recorded with every exchange, so a change
of wording shows in the data. Dataset records use the same instructions and the same
user-message format.

**Tools.** Exposure is tiered by the descriptor's `tier` (`04-operation-registry.md`), so the
model keeps coping as the catalogue grows:
- **Core:** one tool per `core` operation, the latest version of each: `gain`, `normalise`,
  `dc_remove`, `compressor`, `noise_reduce`, `line_reduce`, `band_cut`,
  `spectral_compressor`, `remove_time`, `insert_silence`.
- **On demand:** the rest (currently the eight added in M2: `high_pass`, `low_pass`, `bell`,
  `shelf`, `tilt`, `gate`, `loudness_normalise`, `hum_reduce`) appear only in the index, one
  line each: id, title and summary. `describe_operations` (`ids`: one to eight of them) asks
  to see their parameters.
- A `plan` tool proposes several operations to be previewed and accepted together; its enum
  names every operation offered, core or on demand.
- The final limiter is applied automatically and is not offered.

**The describe round.** When the model calls `describe_operations`, `build_expansion` replays
the conversation, answers the call with those operations' full tool schemas and adds them to
the tools; any other call in the same answer is answered as not run, to be made again. The
model then answers as usual. The describe call wins over anything else in that answer. An
unknown or system id is a problem like any other; a core one is simply described again.

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
| "normalise to −16 LUFS", "normalise the loudness", "normalize the loudness to minus 19" | `loudness_normalise` (−23 LUFS when no target is given) |
| "remove the hum", "reduce the noise", "remove the DC offset" | `line_reduce` (40–1,000 Hz), `noise_reduce`, `dc_remove` |
| "dehum", "remove the 60 Hz hum and its harmonics", "remove the mains hum" | `hum_reduce` (50 or 60 Hz when named, otherwise found) |
| "high-pass at 80 Hz", "cut below 100 Hz", "low cut", "remove the rumble" | `high_pass` (80 Hz when no frequency is given) |
| "low-pass at 8 kHz", "roll off above 12 kHz", "high cut at 10 kHz" | `low_pass` |
| "boost 3 kHz by 3 dB", "dip 250 Hz by 4 dB" | `bell`, an octave wide |
| "boost the bass by 3 dB", "cut the treble above 5 kHz by 4 dB" | `shelf` (low at 200 Hz, high at 4 kHz, unless a corner is named) |
| "brighter by 1.5 dB per octave", "darker by 1 dB per octave" | `tilt` around 1 kHz |
| "gate the pauses", "gate below −50 dB" | `gate` (6 dB above the noise floor when no threshold is given) |
| "compress the bangs here", "tame the whine in this area" | `spectral_compressor` on the selected area (transient, tonal or level mode from the words) |
| "remove 12 to 15.5 seconds", "cut from 1:20 to 1:35", "trim the first 5 seconds" | `remove_time` for that stretch |
| "remove this part", "cut the selection out" (with a time selection, naming nothing to process) | `remove_time` for the selected stretch |
| "insert 2 seconds of silence at 30 s", "add a 500 ms gap at 1:05", "add 1 s of silence here" | `insert_silence` there (seconds, milliseconds, minutes and m:ss are understood) |

- "Here", "this part" or "the selection" makes the selection the scope.
- "Remove this part" removes time only when the words name nothing to process: "remove the
  hum here" cuts the hum in the selection.
- A named target beats the clean-up recipe: "clean up the hum" cuts the hum.
- "Remove the hum" stays `line_reduce`, which cuts only the lines that are there, as far as
  they stand out; `hum_reduce` is for the mains hum and every harmonic, asked for by name.
- "Below" or "above" a frequency means a filter only when no amount is given: "cut the treble
  above 5 kHz by 4 dB" is a shelf, "cut above 5 kHz" a low-pass.
- Short words ("gate", "dip", "eq", "hpf") count only as whole words, so "investigate" is
  not a gate.
- Too little to go on ("low-pass it", "make it brighter") goes to the model.
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

The rounds that may follow an answer are decided in the core (`next_round`), so the browser
and the command line follow the same policy:
- **Describe**, at most once: the model asked to see on-demand operations, so it is sent their
  schemas (above). A second `describe_operations` call is treated as an unusable answer.
- **Correct**, at most once: when the answer cannot be used, the problems are returned as the
  result of each tool call (`build_correction`) and the model is asked to correct itself.
- **Done:** a usable answer is previewed, or a reply in words is shown; if the corrected answer
  fails too, the problems are shown to the user.

A request therefore makes at most three exchanges with the model.

## Every exchange is recorded

Each request and its answer is logged as `assistant.exchange` (actor: the assistant) with:
- provider, model, and the host the request went to (no path, no key);
- the request exactly as sent, its SHA-256 (`request_sha256`) and `prompt_version`;
- the response and the latency;
- `problems`, when the answer could not be used;
- `corrects`, the hash of the exchange this one corrects;
- `describes`, the hash of the exchange whose `describe_operations` call this one answers.

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
  network or key. It can be given once per round, used in order: a saved describe call, then
  the answer made with the schemas.

## Spoken commands

*Speak* in the console streams the microphone to **Deepgram** (chosen by the product owner,
2026-09-25), which sends the words back as it recognises them. The core decides what the stream
asks for and records what came back (`core/src/assistant/speech.rs`); the browser holds the key
and streams (`frontend/src/assistant/dictation.ts`).

- **The stream.** Deepgram's live endpoint (`wss://api.deepgram.com/v1/listen`), over a
  WebSocket, because its REST API refuses web pages (CORS). The core builds the query
  (`listen_params`):
  - model `nova-3` and language `en` (both can be changed in the settings);
  - raw 16-bit mono audio (`linear16`) at the capture rate, sent about 100 ms at a time;
  - interim results, smart formatting, and `UtteranceEnd` after 1 s without words;
  - key terms for the editor's vocabulary (spectrogram, DC offset, sibilance, kilohertz…),
    which only Nova-3 models accept.
- **The key** travels as the WebSocket subprotocol (`token`, then the key), because a browser
  cannot set headers on a WebSocket. It is held like the model's key: in memory for the
  session, or in this browser's local storage when *Remember on this device* is ticked. It is
  never in the address, the core, a project, a log or an export.
- **Only dictation is streamed.** The microphone is opened with the browser's echo
  cancellation, noise suppression and gain control on (recordings have them off). Playback
  pauses while it is open, so a project's audio is never sent.
- **Model improvement.** Deepgram may keep dictation to improve its models: its default, at a
  lower price. Unticking *Let Deepgram keep my dictation* in the settings adds
  `mip_opt_out=true`, which costs more and needs a paid account.
- *Settings → Speech → Test connection* opens a stream and closes it at once, without audio. It
  reports the host, the time it took and Deepgram's request id, or that the connection was
  refused (a browser is not told why; usually the key).

**The words go in the box.** Interim words appear as they are recognised and are replaced as
they firm up. Listening stops:
- when Deepgram reports a pause after speech;
- on *Stop*, or on Enter;
- after 30 s, or after 8 s without speech.

Escape discards what was heard instead. Nothing is sent until the user presses Enter again, so a
misheard "undo" cannot act on its own, and the words can be corrected first.

**What is recorded.** When the words are sent, and before anything is done with them, the core
logs `speech.transcribed`, with the user as actor:
- provider, model, host, and the query exactly as sent;
- Deepgram's request id for each time the microphone was opened;
- what was heard (the final results, each with its confidence) and the words sent, and whether
  they differ (`edited`);
- how many seconds of audio were streamed, and how long the last results took after listening
  stopped.

The preview made from the words refers to that event (`PreviewRecord.dictation`), and the
dataset's step and chat records carry `spoken`: what was heard, and whether the user changed
it. The audio itself is not stored. Dictation that is never sent is not logged.

**Routing.** Spoken words are routed like typed ones. The router also reads the forms a
recogniser writes: "minus 1 dB", "1 point 5 seconds", "3.1 to 3.2 kilohertz".

- **Not yet run against Deepgram itself.** The build environment's network policy denies
  api.deepgram.com, so the tests use a stand-in. The live check: *Test connection* with a key,
  then the routine requests in the table above, spoken. Their transcripts become router test
  cases.
- The command line stays typed.

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
  Tiered exposure already keeps the request small: the on-demand operations cost one line each
  until they are asked for.
