# 03 — Projects, clones and provenance

## Project

A project is one source recording plus one linear stack of steps, each active or excluded,
with everything that happened recorded in its event log.

### Layout (working directory = portable bundle contents)

```
manifest.json                  pointers and caches (rewritable; not evidence)
source/<original filename>     the original bytes, byte-for-byte, never renamed
events.jsonl                   the hash-chained event log (source of truth)
lineage/<ancestor id>.jsonl    ancestor logs up to the fork (clones only)
analysis/                      derived caches, keyed by version; safe to delete
renders/                       derived render cache, keyed by stack hash; safe to delete
```

`manifest.json` holds: format version; project id and name; the source reference (SHA-256,
original filename, size); lineage; the view state (zoom, scroll, frequency scale, colour
range, selection); the log head (seq and hash); and the cached stack projection. On open the
source hash is re-computed, the chain verified, the stack re-projected from the log and
compared with the manifest. Any disagreement is reported, never silently repaired.

## Event log

JSON Lines. Each line is one event in RFC 8785 canonical JSON:

```json
{"actor":{"kind":"user"},"app":{"build":"cli","name":"nlae","version":"0.1.0"},
 "data":{…},"hash":"sha256:…","prev":"sha256:…","seq":7,"ts":"2026-09-24T07:12:03.123Z",
 "type":"step.accepted","v":1}
```

- `hash` = SHA-256 of the canonical form of the event without `hash`.
- `prev` = the previous event's `hash`; the first event's `prev` is the all-zero hash for an
  original project, or the parent's `project.clone_made` hash for a clone.
- Verification walks the log and reports the **first** failing line and why: not JSON,
  sequence break (line removed, inserted or reordered), broken link (an edited event was
  re-hashed), or hash mismatch (an event was altered).
- Timestamps come from the device clock and are not trusted time. `OPEN:` whether to add
  RFC 3161 trusted timestamps for evidential exports.
- API keys are never written to the log, the manifest or any export.

### Event types

| Type | Data |
|---|---|
| `project.created` | project id, name |
| `source.imported` | SHA-256, original filename, size, last-modified time (if known), container, codec, rate, channels, frames, decoder version |
| `source.recorded` | as above, plus the capture settings the browser actually applied |
| `analysis.computed` | features version, render hash of the source, features hash, summary. Once per features version; it may follow creation later (the browser shows the recording first) |
| `assistant.exchange` | provider, model, host; the request exactly as sent, its SHA-256 and the prompt version; the response; latency; the problems found; the exchange it corrects, or whose request to see operations it answers (`describes`), if any. Actor: the assistant. Never the key |
| `speech.transcribed` | a spoken request, logged when it is sent: provider, model, host; the stream's query exactly as sent; the recogniser's request ids; what was heard (final results with their confidences), the words sent, and whether they differ; seconds of audio streamed; latency. Actor: the user. Never the key or the audio |
| `step.previewed` | preview id, candidate step(s), window, window measurements, base stack hash; `kind` = `step` or `plan`; the recipe it replays, the exchange it came from and the spoken request it came from, if any |
| `step.modified` | preview id, proposed parameters, changed parameters (a change made in a preview, before accepting it) |
| `step.accepted` | the full step object |
| `plan.accepted` | plan id, preview id, the accepted steps, the steps switched off |
| `step.rejected` | preview id, reason |
| `step.applied` | `kind` (`step` or `plan`), plan id, the full step objects, measured over the whole clip; the recipe, the exchange and the spoken request they came from, if any. A request applied at once, without a preview |
| `step.excluded` | step ids, optional reason, `chain`, `undoes` / `redoes` if it is an undo or a redo |
| `step.restored` | step ids, `chain`, `undoes` / `redoes` |
| `step.edited` | step id, parameters before and after, `remeasured` (measured again on the current input, the parameters unchanged), `chain`, `undoes` / `redoes` |
| `step.removed` | step id. Written before 2026-09-26 for undoing the top step; read as `step.excluded` of that step |
| `step.annotated` | step id, note, labels |
| `stack.rated` | target (step, plan or stack), overall 1–5, optional dimensions, note |
| `recipe.saved` / `recipe.replayed` | recipe hash, range, mode, dry-run diff |
| `render.exported` | file name, format, stack hash (with the final limiter), output hash, limiter measurements, the file's SHA-256, the output's length, and where time was removed or silence inserted (in the original's and the output's time) |
| `dataset.exported` | options, manifest hash |
| `project.clone_made` | child project id, fork step (in the parent's log) |
| `project.cloned_from` | parent id, parent head, fork step and its event hash, inherited steps (first event of the clone) |
| `bundle.exported` | bundle manifest hash |

## The step object

The same object is produced by the console, panels, the command-line tool, recipes and
automation; only `actor` and `origin` differ.

| Field | Meaning |
|---|---|
| `step_id`, `plan_id` | identity; `plan_id` groups steps previewed and accepted as one unit |
| `op`, `op_version` | the registry descriptor |
| `params` | the parameters as given, every one written out (defaults included); may contain `auto` |
| `resolved` | the concrete values used on this clip — the **exact form** |
| `scope` | `clip`, `time_range {t0,t1}`, `band {f_lo,f_hi}` or `tf_patch {t0,t1,f_lo,f_hi}` |
| `bindings` | the **adaptive form**: how values relate to the clip's analysis; `declared` or `inferred` |
| `actor`, `origin` | who (user, assistant with model/provider, automation) and through what (console, panel, cli, recipe, automation) |
| `intent`, `rationale`, `note` | the user's words verbatim; the assistant's explanation; a free note |
| `state_before`, `state_after` | features snapshots of the input and output |
| `measurements` | what the DSP measured while doing it |
| `class`, `label` | registry class; `processed` or `reconstruction (processed, not factual)` |
| `input_hash`, `output_hash`, `stack_hash` | render hashes and the stack hash after this step |
| `resolved_on` | the render hash of the input `resolved` was measured on, when that is no longer the step's input (a step below was removed, restored or edited since) |
| `inherited_from` | for steps inherited by a clone: parent project, step id, event hash |

### Stack hash

`stack_hash_0` = the source SHA-256. `stack_hash_n` = SHA-256 over `stack_hash_{n−1}` and
the canonical step core (`op`, `op_version`, `resolved`, `scope`). It is the render-cache
key, and a replay that reproduces it has reproduced the render.

## Changing the stack

A request applies its steps at once (`step.applied`); the command line can still preview and
accept (`step.previewed`, then `step.accepted` or `plan.accepted`). After that, any step can be
changed:

- **Excluded** (`step.excluded`): it stops playing a part in the render but keeps its place in
  the stack, so it can be **restored** (`step.restored`) to the same place at any time. Nothing
  is deleted.
- **Edited** (`step.edited`): its parameters change and it is resolved again on its input. A
  step keeps its id through edits; every version of it is in the log.
- **Measured again** (`step.edited` with `remeasured`): resolved again on its current input with
  the same parameters, after a change below it.

**The steps above keep their values.** Changing a step changes the input of every active step
above it, so each of them is recorded again in the same event, in `chain`: the active steps
from the lowest changed position up, with new measurements, snapshots, render hashes and stack
hashes, and their inferred bindings worked out again (declared ones are kept). Their `op`,
`params`, `resolved` and `scope` do not change. A step whose values were measured on its input
(a normalise's gain, a noise profile, the lines found) and whose input has changed since is
**drifted**: `resolved_on` records the input it was measured on. Measuring it again is the
user's decision.

The projection checks every such event: the ids in `chain` are exactly the active steps from
the lowest changed position up, in order; every `stack_hash` chains on from the step below;
and no value changed except the edited step's. A step whose input is unchanged has the same
output, so it is not rendered again.

**Undo and redo** walk back through the changes to the stack (applied, accepted, excluded,
restored, edited), last first. Each is logged as the inverse change, with `undoes` naming the
event it undoes; a redo repeats it, with `redoes`. A new change clears what can be redone. The
lists are projected from the log, so they survive a reload. Undoing an edit brings back the
previous version exactly, not a re-measured one.

**Approval.** Exporting approves the stack as it stands: `render.exported` records its stack
hash, and when the stack at the end is the one exported, the dataset export marks its steps as
approved.

## Clones

A clone copies a project at any step into a new project to follow a different stream of
edits. There are no in-project branches or layers; the interface stays a single stack.

1. The parent's log gets `project.clone_made` (child id, fork step).
2. The clone's first event is `project.cloned_from`; its `prev` is the hash of that
   `clone_made` event, so the clone's history cryptographically extends the parent's.
3. Steps up to the fork are copied into the clone with `inherited_from`.
4. The clone stores the parent's log up to the `clone_made` event (and the parent's own
   lineage) in `lineage/`, so a clone bundle verifies all the way back to import on its own.
5. In the browser library, clones share the one stored copy of the source; a bundle always
   embeds it.

Clones forked from the same step are natural side-by-side comparisons, and dataset exports
link them (see `10-learning-and-automation.md`).

## Projects that do not verify

Opening a project checks the source hash, the hash chain, the lineage and the stack
projection. If anything fails, the problems are reported and the project is **read-only**: the
core refuses to append an event or rewrite the manifest, so nothing can be chained onto a
broken log and nothing can be cloned from or exported out of it. Problems are reported, never
repaired. In the browser, a bundle that does not verify is shown but not added to the library,
and a bundle never replaces a library copy unless its log continues the copy's log exactly
(`07-interface.md`).

## Residuals

- **Step residual** = step input − step output: exactly what that step removed.
- **Project residual** = the source passed through the stack's level steps only (DC removal,
  gain, normalise, compressor) − the current output: what the attenuating steps removed, at the
  level it would have had. (A plain source − output would be dominated by any gain change.)
- Each descriptor declares its residual kind: `exact`, `approximate` or `n/a` (e.g. a time
  stretch has no sample-aligned residual).
- For the STFT-mask operations the residual is computed directly and the output is
  `input − residual`, so the residual is exact by construction.

The interface offers a three-way monitor: **Original / Processed / Residual** (M1). The
command-line tool writes residual files now. Listening to the residual is the quickest check
that an operation removed only what it should have.

## Capture bundles (M3)

One per job: the source SHA-256 and a copy of the source; every rendered output with its
hash; full-resolution before and after spectrogram and waveform images; the machine-readable
step chain; a human-readable transcript of the same chain; a per-step metrics file.
