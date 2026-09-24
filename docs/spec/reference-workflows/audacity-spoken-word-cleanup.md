# Reference workflow: spoken-word clean-up (current Audacity process)

Status: **reference** — the first workflow the application must reproduce, and then
improve on, from one goal-level command. It is the main acceptance scenario for the
action stack, adaptive replay and the built-in recipe
`recipes/builtin/spoken-word-cleanup.v1.json`.

## As provided (verbatim)

> **The Current Audacity Process**
>
> This is the manual procedure the user performs today in Audacity, recorded exactly as described. It is the reference workflow the application must be able to reproduce — and then improve on — with a single goal-level command. It is a spoken-word and field-recording clean-up chain, not a music-production chain.
>
> 1. Record. Capture roughly one minute of audio.
> 2. Amplify. Raise the whole clip by about +30 dB until the peaks reach full scale (1 and −1).
> 3. Normalise. Remove the DC offset and normalise the peaks to about −1 dB, then amplify so the material spans roughly −1 to +1.
> 4. Noise reduction. Take the quietest part of the clip as the noise profile, using the shortest sample that will work, and apply that profile across the entire clip.
> 5. Notch the lines. Locate the strongest spectral lines running through the whole clip (a line was found around 750 Hz) and notch-filter each one until its line strength matches the surrounding material.
> 6. Repeated narrow-band filtering / EQ. Work through the major lines one at a time with narrow adjustments — for example, inside a 3,000–4,000 Hz region, reduce only 3,100–3,200 Hz. Repeat for every prominent line until no peak stands out from the rest.
> 7. Compress (optional). Apply a compressor as a final stage.
>
> How this maps to the brief. Steps 2–3 belong to Gain and level; the optional compression of step 7 also sits under Gain and level. Step 4 belongs to Noise reduction and restoration. Steps 5–6 belong to Filters and EQ, but they depend on the Analysis category's tonal-line and prominent-band detection to find the lines in the first place. The natural-language target for the whole sequence is a single goal-level plan (M2) — roughly clean this recording up — previewed on a short window before anything is committed.
>
> What the workflow reveals. The human loop encoded here is: find the lines by eye, then reduce them by hand, one at a time. In the application the detection should be automatic and the reductions re-derived per clip (adaptive recipes, M3) rather than hard-coded to 750 Hz or 3,100–3,200 Hz, since an absolute notch is rarely right on a different recording. The repeated single-band moves are also evidence that narrow, sequential EQ steps compose well — and that the composite chain should be previewable and acceptable as one unit.

## Mapping to the application

| # | Audacity step | Application step | Found automatically | Recorded for reuse |
|---|---|---|---|---|
| 1 | Record ~1 min | Record (interface, M0) with echo cancellation, noise suppression and auto-gain **off** | — | the device settings the browser actually applied |
| 2–3 | Amplify +30 dB; remove DC; normalise −1 dB; amplify to ±1 | `dc_remove` → `normalise` (peak −1 dBFS) | DC offset, peak level | the gain applied (e.g. +29.4 dB), bound to "peak → −1 dBFS" |
| 4 | Noise profile from the quietest part, shortest sample that works; apply to the whole clip | `noise_reduce`, profile `auto` | the quietest *steady* region: the shortest window whose profile is stable | profile range and statistics, bound to "quietest region" |
| 5 | Notch the strongest lines (~750 Hz) until each matches its surroundings | `line_reduce`, lines `auto` | every persistent line across the whole clip: frequency, width, prominence | each line's frequency, width and depth, bound to "detected line #k" |
| 6 | Narrow cuts one line at a time (e.g. 3,100–3,200 Hz) until no peak stands out | the same `line_reduce`, which cuts every line by its measured prominence; or manual `band_cut` steps | the stop rule becomes a measurement: *largest remaining line prominence ≤ target* | manual cuts get inferred bindings to the nearest detected line |
| 7 | Compressor (optional) | `compressor`, optional step | dynamics and loudness | parameters, dynamics before and after |
| — | — | `limiter`, always last, ceiling −1 dBFS | — | how much it acted (usually 0 dB) |

### Why steps 2–3 become two steps

In 32-bit float processing, the intermediate "+30 dB until full scale" changes nothing
that the later normalisation does not redo: gain is linear and float does not clip.
Removing the DC offset *before* normalising makes the peak target exact (an offset
shifts the peaks). The target is −1 dBFS rather than full scale so that the
always-last limiter (ceiling −1 dBFS) has nothing to do; the result is about 1 dB
quieter than the current final amplify. `OPEN:` confirm −1 dBFS is acceptable, or
choose a different default target.

### Why steps 5–6 become one step

The manual loop — find a line by eye, cut it, look again, repeat until no peak stands
out — is exactly what `line_reduce` automates:

1. The long-term spectrum is measured at fine resolution (≈3 Hz bins at 48 kHz).
2. Every line that stands out from its neighbourhood and persists through the clip is
   found, with its frequency, width and prominence (how far it stands above the
   surrounding material).
3. Each line is cut by its prominence minus `target_excess_db` (0 = "match the
   surrounding material"), never more than `max_depth_db`.
4. The result is re-measured and any line still standing out is cut further — the
   "repeat until no peak stands out" loop — and the final largest prominence is
   recorded as the step's stop measurement.

Lines found this way are specific to the clip. On another recording the same step
finds that recording's lines, which is the point made above: an absolute notch at
750 Hz is rarely right on a different recording.

### When to use `line_reduce` and when the spectral compressor

- **`line_reduce`** — the line runs through the whole clip. Static, like the manual
  notches.
- **`spectral_compressor`** — the peak comes and goes (an intermittent whine, bangs, a
  loud foreground voice), or it should only be reduced inside a selected time ×
  frequency area.

## Automated form

The whole routine ships as the built-in recipe `spoken-word-cleanup` (v1): every value
is either `auto` or bound to the clip's own analysis, so it adapts to each recording.

```
nlae plan case/ --recipe builtin:spoken-word-cleanup
```

prints the derived plan — for example "+29.4 dB; profile 41.2–41.9 s; lines 750 Hz
(+14 dB), 3,150 Hz (+9 dB)" — renders a 10-second preview and its residual (what the
plan removes), and waits. `nlae accept case/ <plan-id>` commits the whole plan as one
unit; steps can be switched off before accepting. From M1 the console routes "clean
this recording up" to this recipe locally, without calling the model.

## Acceptance (see `08-testing.md`)

On golden clip A (built to resemble this material), the automatic plan must leave:
DC below 1e-4; peak at −1 dBFS ± 0.1 with the limiter inactive; noise in speech pauses
down ≥ 10 dB; every line's remaining prominence ≤ 3 dB; foreground and background
voice energy changed ≤ 2 dB.

The manual process is also reproduced by hand with the command-line tool — +30 dB, DC
removal, normalise, noise reduction with a hand-picked profile, cuts at 740–760 Hz and
3,100–3,200 Hz — saved as a recipe, and replayed on clip B (lines at 620 Hz and
2,450 Hz). Adaptive replay must move the cuts onto B's lines and flatten them;
exact replay must miss them.
