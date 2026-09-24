# 04 — Operation registry

The registry is the keystone. Validation, panels, tool schemas, logging, replay and the
training corpus are all derived from it. Adding an operation means adding one descriptor and
its implementation — nothing else.

## Descriptor

One file per operation and version: `schemas/ops/<id>.v<N>.json`, validated against
`schemas/op-descriptor.schema.json`.

| Field | Meaning |
|---|---|
| `id`, `version` | stable identity; a behaviour change is a new version |
| `title`, `summary` | plain English, shown in panels and sent to the model |
| `category` | `gain_level`, `filters_eq`, `noise_restoration`, `separation`, `time_pitch`, `stereo`, `spectral`, `creative`, `analysis`, `utility`, `safety` |
| `class` | `analysis` (read-only), `corrective`, `attenuative` (can only reduce level), `reconstruction` (synthesises; labelled processed, not factual), `creative`, `safety` |
| `scopes` | which scope kinds it accepts |
| `params` | each with `id`, `type`, `title`, `help`, `default`, and type-specific limits |
| `identity` | parameter sets that must render bit-identical output (null tests) |
| `residual` | `exact`, `approximate` or `n/a` |
| `measurements` | what the DSP reports, with units |
| `binding_hints` | which analysis features the parameters naturally bind to |
| `system` | `true` for operations the application adds itself (the final limiter) |

### Parameter types

| Type | Limits | Notes |
|---|---|---|
| `number` | `min`, `max` (hard), `unit`, `step` | `auto_allowed` lets the value be `"auto"`, resolved on the clip |
| `integer` | `min`, `max` | |
| `enum` | `values` | |
| `bool` | | |
| `time_range` | | `"auto"` or `{t0, t1}` in seconds |
| `lines` | `max_items` | `"auto"` or a list of `{freq_hz, width_hz, depth_db}` |

## Validation

Every step is validated against its descriptor in Rust **before** any DSP runs:

- unknown parameters are rejected;
- missing parameters are filled with their defaults, so the recorded step always has every
  parameter written out;
- values outside the hard limits are **rejected** with a typed error naming the parameter and
  the limit — never silently clamped;
- `auto` is accepted only where the descriptor allows it.

Each operation's typed Rust parameter struct must accept the descriptor's defaults and
nothing else (tested), so the descriptor and the code cannot drift apart. A parity test fails
the build if a descriptor has no implementation or an implementation has no descriptor. When
panels are generated (M2), the parity test extends to panels and tool schemas.

## Tool schemas and panels (M1–M2)

- **Tool schema** for the model: generated from the descriptor — name, summary, JSON Schema of
  the parameters with limits and units. Tool exposure is tiered: a small always-present core
  set, a compact index of every operation, and full schemas retrieved on demand.
- **Panel** for the user: generated from the same descriptor — a control per parameter with its
  limits, units, default and help text, and an "auto" toggle where allowed.

## Current operations

| Operation | Class | Summary |
|---|---|---|
| `gain` v1 | corrective | Raise or lower the level by a fixed amount |
| `dc_remove` v1 | corrective | Remove a constant (or slowly drifting) offset |
| `normalise` v1 | corrective | Bring the peak to a target level |
| `compressor` v1 | corrective | Broadband speech compressor |
| `limiter` v1 | safety (system) | Always last; bit-exact below the ceiling |
| `noise_reduce` v1 | attenuative | Spectral gate against a noise profile |
| `line_reduce` v1 | attenuative | Find and flatten persistent tonal lines |
| `band_cut` v1 | attenuative | Manual narrow cut between two frequencies |
| `spectral_compressor` v1 | attenuative | Push down high points inside a time × frequency area |

Full parameter tables are in `05-operation-catalogue.md` and the descriptor files.
