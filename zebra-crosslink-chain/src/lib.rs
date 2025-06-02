//! Core Zcash Crosslink data structures
//!
//! This crate deals only with in-memory state validation and excludes I/O, tokio, services, etc...
//!
//! This crate is named similarly to [zebra_chain] since it has a similar scope. In a mature crosslink-enabled Zebra these two crates may be merged.
#![deny(unsafe_code, missing_docs)]

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use thiserror::Error;
use tracing::error;
use zebra_chain::block::Header as BcBlockHeader;

use zebra_chain::serialization::{SerializationError, ZcashDeserialize, ZcashSerialize};

/// The BFT block content for Crosslink
///
/// # Constructing [BftPayload]s
///
/// A [BftPayload] may be constructed from a node's local view in order to create a new BFT proposal, or they may be constructed from unknown sources across a network protocol.
///
/// To construct a [BftPayload] for a new BFT proposal, build a [Vec] of [BcBlockHeader] values, starting from the latest known PoW tip and traversing back in time (following [previous_block_hash](BcBlockHeader::previous_block_hash)) until exactly [bc_confirmation_depth_sigma](ZcashCrosslinkParameters::bc_confirmation_depth_sigma) headers are collected, then pass this to [BftPayload::try_from].
///
/// To construct from an untrusted source, call the same [BftPayload::try_from].
///
/// ## Validation and Limitations
///
/// The [BftPayload::try_from] method is the only way to construct [BftPayload] values and performs the following validation internally:
///
/// 1. The number of headers matches the expected protocol confirmation depth, [bc_confirmation_depth_sigma](ZcashCrosslinkParameters::bc_confirmation_depth_sigma).
/// 2. The [version](BcBlockHeader::version) field is a known expected value.
/// 3. The headers are in the correct order given the [previous_block_hash](BcBlockHeader::previous_block_hash) fields.
/// 4. The PoW solutions validate.
///
/// These validations use *immediate data* and are *stateless*, and in particular the following stateful validations are **NOT** performed:
///
/// 1. The [difficulty_threshold](BcBlockHeader::difficulty_threshold) is within correct bounds for the Difficulty Adjustment Algorithm.
/// 2. The [time](BcBlockHeader::time) field is within correct bounds.
/// 3. The [merkle_root](BcBlockHeader::merkle_root) field is sensible.
///
/// No other validations are performed.
///
/// **TODO:** Ensure deserialization delegates to [BftPayload::try_from].
///
/// ## Design Notes
///
/// This *assumes* is is more natural to fetch the latest BC tip in Zebra, then to iterate to parent blocks, appending each to the [Vec]. This means the in-memory header order is *reversed from the specification* [^1]:
///
/// > Each bft‑proposal has, in addition to origbft‑proposal fields, a headers_bc field containing a sequence of exactly σ bc‑headers (zero‑indexed, deepest first).
///
/// The [TryFrom] impl performs internal validations and is the only way to construct a [BftPayload], whether locally generated or from an unknown source. This is the safest design, though potentially less efficient.
///
/// # References
///
/// [^1]: [Zcash Trailing Finality Layer §3.3.3 Structural Additions](https://electric-coin-company.github.io/tfl-book/design/crosslink/construction.html#structural-additions)
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BftPayload {
    /// The Version Number
    pub version: u32,
    /// The Height of this BFT Payload
    pub height: u32,
    /// Hash of the previous BFT Block. Not the previous payload!
    pub previous_block_hash: zebra_chain::block::Hash,
    /// The PoW Headers
    pub headers: Vec<BcBlockHeader>,
}

impl ZcashSerialize for BftPayload {
    #[allow(clippy::unwrap_in_result)]
    fn zcash_serialize<W: std::io::Write>(&self, mut writer: W) -> Result<(), std::io::Error> {
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u32::<LittleEndian>(self.height)?;
        self.previous_block_hash.zcash_serialize(&mut writer)?;
        writer.write_u32::<LittleEndian>(self.headers.len().try_into().unwrap())?;
        for header in &self.headers {
            header.zcash_serialize(&mut writer)?;
        }
        Ok(())
    }
}

impl ZcashDeserialize for BftPayload {
    fn zcash_deserialize<R: std::io::Read>(mut reader: R) -> Result<Self, SerializationError> {
        let version = reader.read_u32::<LittleEndian>()?;
        let height = reader.read_u32::<LittleEndian>()?;
        let previous_block_hash = zebra_chain::block::Hash::zcash_deserialize(&mut reader)?;
        let header_count = reader.read_u32::<LittleEndian>()?;
        if header_count > 2048 {
            // Fail on unreasonably large number.
            return Err(SerializationError::Parse(
                "header_count was greater than 2048.",
            ));
        }
        let mut array = Vec::new();
        for i in 0..header_count {
            array.push(zebra_chain::block::Header::zcash_deserialize(&mut reader)?);
        }

        Ok(BftPayload {
            version,
            height,
            previous_block_hash,
            headers: array,
        })
    }
}

/// The wrapper around the BftPayload that also contains the finalizer signatures allowing the Block to be
/// validated offline. The signatures are not included yet! D:
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BftBlock {
    /// The inner payload
    pub payload: BftPayload,
}

impl BftPayload {
    /// Refer to the [BcBlockHeader] that is the finalization candidate for this payload
    ///
    /// **UNVERIFIED:** The finalization_candidate of a final [BftPayload] is finalized.
    pub fn finalization_candidate(&self) -> &BcBlockHeader {
        &self.headers.last().expect("Vec should never be empty")
    }

    /// Attempt to construct a [BftPayload] from headers while performing immediate validations; see [BftPayload] type docs
    pub fn try_from(
        params: &ZcashCrosslinkParameters,
        height: u32,
        previous_block_hash: zebra_chain::block::Hash,
        headers: Vec<BcBlockHeader>,
    ) -> Result<Self, InvalidBftPayload> {
        let expected = params.bc_confirmation_depth_sigma + 1;
        let actual = headers.len() as u64;
        if actual != expected {
            return Err(InvalidBftPayload::IncorrectConfirmationDepth { expected, actual });
        }

        error!("not yet implemented: all the documented validations");

        Ok(BftPayload {
            version: 0,
            height,
            previous_block_hash,
            headers,
        })
    }
}

/// Validation error for [BftPayload]
#[derive(Debug, Error)]
pub enum InvalidBftPayload {
    /// An incorrect number of headers was present
    #[error(
        "invalid confirmation depth: Crosslink requires {expected} while {actual} were present"
    )]
    IncorrectConfirmationDepth {
        /// The expected number of headers, as per [bc_confirmation_depth_sigma](ZcashCrosslinkParameters::bc_confirmation_depth_sigma)
        expected: u64,
        /// The number of headers present
        actual: u64,
    },
}

/// Zcash Crosslink protocol parameters
///
/// This is provided as a trait so that downstream users can define or plug in their own alternative parameters.
///
/// Ref: [Zcash Trailing Finality Layer §3.3.3 Parameters](https://electric-coin-company.github.io/tfl-book/design/crosslink/construction.html#parameters)
#[derive(Debug)]
pub struct ZcashCrosslinkParameters {
    /// The best-chain confirmation depth, `σ`
    ///
    /// At least this many PoW blocks must be atop the PoW block used to obtain a finalized view.
    pub bc_confirmation_depth_sigma: u64,

    /// The depth of unfinalized PoW blocks past which "Stalled Mode" activates, `L`
    ///
    /// Quoting from [Zcash Trailing Finality Layer §3.3.3 Stalled Mode](https://electric-coin-company.github.io/tfl-book/design/crosslink/construction.html#stalled-mode):
    ///
    /// > In practice, L should be at least 2σ.
    pub finalization_gap_bound: u64,
}

/// Crosslink parameters chosed for prototyping / testing
///
/// <div class="warning">No verification has been done on the security or performance of these parameters.</div>
pub const PROTOTYPE_PARAMETERS: ZcashCrosslinkParameters = ZcashCrosslinkParameters {
    bc_confirmation_depth_sigma: 3,
    finalization_gap_bound: 7,
};
