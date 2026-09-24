//! nlae core: DSP, analysis, the operation registry, the action stack and
//! provenance. Shared unchanged by the command-line tool, the WebAssembly build
//! and (later) the desktop build.

// Index loops are the clearest way to write most DSP kernels here.
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

pub mod audio;
pub mod dsp;
pub mod hash;
pub mod math;
pub mod provenance;
