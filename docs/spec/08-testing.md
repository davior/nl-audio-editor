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

**Reference workflow, automatic** (clip A, built-in recipe): DC below 1e-4; peak −1 dBFS ± 0.1
with the limiter inactive; noise in speech pauses down ≥ 10 dB; every line's remaining
prominence ≤ 3 dB; foreground and background voice energy changed ≤ 2 dB; previewed and accepted
as one plan.

**Manual process made reusable**: the Audacity steps repeated by hand on clip A; saved as a
recipe; inferred bindings point at the detected lines; adaptive replay on clip B moves the cuts
to 620 Hz and 2,450 Hz and leaves B's lines ≤ 3 dB; exact replay leaves them ≥ 8 dB; a rating is
recorded; the dataset export validates, with rejected and modified steps flagged and clone pairs
linked, and without keys or audio unless requested.

**Spectral compressor** (clip A): `tonal` takes the intermittent whine down ≥ 15 dB; `transient`
takes the bangs down ≥ 10 dB; `level` takes the foreground down ≥ 6 dB relative to the background;
in each, the background voice changes ≤ 2 dB. (Starting targets; tuned values are recorded here
when they change.)

## Proposed: control replay (M3)

To show that something "brought out" by processing is in the recording rather than made by the
processing: replay the same chain on noise synthesised to match the clip's long-term spectrum
and level. Anything that appears in the control as well was produced by the processing. The
control render and its comparison are stored with the capture bundle.

## Performance targets (provisional, brief question 20)

- A 10-minute 48 kHz mono file shows both lanes in under 3 s in the browser.
- Playback starts within 100 ms of pressing play.
- A 10 s preview of the spectral compressor renders in under 1 s in WebAssembly.
