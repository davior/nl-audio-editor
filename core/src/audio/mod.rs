//! Audio buffers, decoding and WAV writing.

mod buffer;
pub mod decode;
pub mod wav;

pub use buffer::AudioBuffer;
pub use decode::{decode, SourceInfo};
