//! Core Zcash Crosslink data structures
//!
//! This crate deals only with in-memory state validation and excludes I/O, tokio, services, etc...
#![deny(unsafe_code, missing_docs)]

mod bft;
pub mod params;
mod util;

pub use crate::bft::BftPayload;
