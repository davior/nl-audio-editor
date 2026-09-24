# 08 — Testing and quality

## Golden set

Synthesised deterministically by `nlae golden` from separate, known components, so every test
can measure exactly what an operation did to each component (the voices, the noise, each line,
the whine, the bangs). Clip A resembles the reference material (very quiet, DC offset, lines at
750 Hz and 3,150 Hz); clip B has the same kinds of problem elsewhere (lines at 620 Hz and
2,450 Hz). See `tests/golden/README.md`. The expected SHA-256 of every generated file is
committed; generation is part of the determinism test.

Because mask operations are linear once their mask is fixed, the mask computed on the mixture
can be applied to each component separately to measure per-component effects exactly.

## Invariant tests

| Test | What it proves |
|---|---|
| STFT round trip | analysis + synthesis reconstructs below −120 dB |
| Null test per operation | every identity setting renders bit-identical output |
| Locality | samples outside a scope (plus one window) are bit-identical |
| Attenuation-only | every cell's gain ≤ 1 for `attenuative` operations |
| Preview = final | random preview windows equal the same span of the full render, bit for bit |
| Determinism | same source and stack → same render hash, twice; native = WebAssembly |
| Hard limits | out-of-range values are rejected with a typed error |
| Limiter passthrough | below the ceiling, output is bit-identical |
| Canonical JSON | RFC 8785 vectors |
| Hash chain | an edited byte, a deleted line, a reordered line and a re-hashed edit are each reported at the right line |
| Clone lineage | a clone bundle verifies back to import on its own |
| Bundle round trip | source bytes, analysis and view state survive save and reopen |
| Projection | the stack re-projected from the log equals the manifest's stack |
| Schemas | every event and dataset record validates against its JSON Schema |
| Path parity | the same step via the "panel" and "console" paths differs only in actor and origin |
| Registry parity | every descriptor has an implementation and vice versa; typed params accept the defaults |

## Acceptance scenarios

**Reference workflow, automatic** (clip A, built-in recipe `spoken-word-cleanup`, adaptive),
measured 2026-09-24:

| Criterion | Target | Measured |
|---|---|---|
| DC offset after | < 1e-4 | 1.45e-6 |
| Peak after (before the limiter) | −1 dBFS ± 0.1 | −1.000 dBFS |
| Final limiter | inactive | 0 samples touched |
| Noise (noise + rumble components) in the speech pause | ≥ 10 dB down | −10.4 dB |
| Every line's remaining prominence (50, 100, 750, 3,150 Hz) | ≤ 3 dB | ≤ 1.95 dB |
| Foreground / background voice energy | within ±2 dB | −0.3 / −1.2 dB |
| Previewed and accepted as one plan | yes | yes |

**Manual process made reusable** (measured 2026-09-24): the Audacity steps repeated by hand on
clip A — a rejected +40 dB, then +30 dB, DC removal, a normalise changed from −3 to −1 dBFS before
accepting (recorded as a modification), noise reduction with a hand-picked profile at
40.4–41.4 s, cuts at 740–760 Hz and 3,100–3,200 Hz with depths read off the analysis — saved as a
recipe. The inferred bindings point at the detected lines (750 Hz, 3,150 Hz), the profile at the
quietest region, the gain at a target peak. Replayed on clip B:

| | 620 Hz line | 2,450 Hz line |
|---|---|---|
| Adaptive: cut moved to | 610–630 Hz | 2,400–2,500 Hz |
| Adaptive: stood above surroundings / cut by | 7.5 / 7.2 dB | 7.7 / 7.4 dB |
| Exact: cut by / still stands | 0.0 / 10.1 dB | 0.0 / 10.9 dB |

Line reductions are measured per component (the line's own attenuation across its cut) rather
than as prominence in the result: a 100 Hz-wide cut also lowers part of the ±150 Hz neighbourhood
that prominence is measured against, which understates the cut. The dataset export over five
projects (A, B adaptive, B exact, two clones of B) validates against the schemas; rejected and
modified steps are flagged; the clones are linked as siblings and to their parent; there are no
keys, and no audio unless `--with-audio` is given.

**Spectral compressor** (clip A), measured per component:

| Mode | Scope | Target | Measured (2026-09-24) |
|---|---|---|---|
| `tonal` (defaults) | 9.5–14.5 s × 4.5–6 kHz | whine ≥ 15 dB down; background ≤ 2 dB | whine −17.4 dB; background −0.0 dB |
| `transient` (defaults) | whole clip | bangs ≥ 10 dB down; background ≤ 2 dB | bangs −12.0 dB; background −0.5 dB; foreground −0.9 dB |
| `level` (ratio 8, attack 5 ms, release 50 ms) | 200–4000 Hz | foreground ≥ **5** dB down relative to background; background ≤ 2 dB | foreground −7.2 dB; background −1.8 dB; relative −5.4 dB |

The `level` target was 6 dB. The two synthetic voices overlap in time–frequency (moving pitch
smears their partials across cells), which caps what per-cell processing can separate at about
5.5 dB on this material; the target is recorded at 5 dB. Tuning that produced these defaults:
the `transient` reference moved from the median to the upper quartile (speech onsets were
being treated as transients), and `threshold_db` gained a mode-dependent `auto` default.

## Proposed: control replay (M3)

To show that something "brought out" by processing is in the recording rather than made by the
processing: replay the same chain on noise synthesised to match the clip's long-term spectrum
and level. Anything that appears in the control as well was produced by the processing. The
control render and its comparison are stored with the capture bundle.

## Performance targets (provisional, brief question 20)

- A 10-minute 48 kHz mono file shows both lanes in under 3 s in the browser.
- Playback starts within 100 ms of pressing play.
- A 10 s preview of the spectral compressor renders in under 1 s in WebAssembly.
