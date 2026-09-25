//! nlae core: DSP, analysis, the operation registry, the action stack and
//! provenance. Shared unchanged by the command-line tool, the WebAssembly build
//! and (later) the desktop build.

// Index loops are the clearest way to write most DSP kernels here.
#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

pub mod analysis;
pub mod assistant;
pub mod audio;
pub mod dataset;
pub mod dsp;
pub mod engine;
pub mod golden;
pub mod hash;
pub mod math;
pub mod ops;
pub mod parity;
pub mod project;
pub mod provenance;
pub mod recipe;
pub mod scope;
pub mod timeline;
