//! nlae core: DSP, analysis, the operation registry, the action stack and
//! provenance. Shared unchanged by the command-line tool, the WebAssembly build
//! and (later) the desktop build.

// Index loops are the clearest way to write most DSP kernels here.
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

pub mod analysis;
pub mod audio;
pub mod dsp;
pub mod engine;
pub mod golden;
pub mod hash;
pub mod math;
pub mod ops;
pub mod provenance;
pub mod scope;
