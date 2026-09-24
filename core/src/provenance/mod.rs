//! Provenance: canonical JSON (RFC 8785), the hash-chained append-only event
//! log, and the environment (clock, identifiers) events are stamped with.

pub mod env;
pub mod jcs;
pub mod log;

pub use env::{Env, FixedEnv};
pub use log::{Actor, ActorKind, AppInfo, EventLog, Verification, VerifyFailure};
