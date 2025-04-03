//! Core Zcash Crosslink data structures
//!
//! This crate deals only with in-memory state validation and excludes I/O, tokio, services, etc...
//!
//! This crate is named similarly to [zebra_chain] since it has a similar scope. In a mature crosslink-enabled Zebra these two crates may be merged.
#![deny(unsafe_code, missing_docs)]

mod bft;
pub mod params;

pub use crate::bft::{BftPayload, InvalidBftPayload};
