use std::marker::PhantomData;

use thiserror::Error;
use zebra_chain::block::Header as BcBlockHeader;

use crate::params::ZcashCrosslinkParameters;

/// The BFT block content for Crosslink
///
/// # Constructing [BftPayload]s
///
/// A [BftPayload] may be constructed from a node's local view in order to create a new BFT proposal, or they may be constructed from unknown sources across a network protocol.
///
/// To construct a [BftPayload] for a new BFT proposal, build a [Vec] of [BcBlockHeader] values, starting from the latest known PoW tip and traversing back in time (following [previous_block_hash](BcBlockHeader::previous_block_hash)) until exactly [BC_CONFIRMATION_DEPTH_SIGMA](ZcashCrosslinkParameters::BC_CONFIRMATION_DEPTH_SIGMA) headers are collected, then pass this to [BftPayload::try_from].
///
/// To construct from an untrusted source, call the same [BftPayload::try_from].
///
/// ## Validation and Limitations
///
/// The [BftPayload::try_from] method is the only way to construct [BftPayload] values and performs the following validation internally:
///
/// 1. The number of headers matches the expected protocol confirmation depth, [BC_CONFIRMATION_DEPTH_SIGMA](ZcashCrosslinkParameters::BC_CONFIRMATION_DEPTH_SIGMA).
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
/// # Refences
///
/// [^1]: [Zcash Trailing Finality Layer §3.3.3 Structural Additions](https://electric-coin-company.github.io/tfl-book/design/crosslink/construction.html#structural-additions)
#[derive(Debug)]
pub struct BftPayload<ZCP>
where
    ZCP: ZcashCrosslinkParameters,
{
    headers: Vec<BcBlockHeader>,
    ph: PhantomData<ZCP>,
}

impl<ZCP> BftPayload<ZCP>
where
    ZCP: ZcashCrosslinkParameters,
{
    /// Refer to the [BcBlockHeader] that is the finalization candidate for this payload
    ///
    /// **UNVERIFIED:** The finalization_candidate of a final [BftPayload] is finalized.
    pub fn finalization_candidate(&self) -> &BcBlockHeader {
        &self.headers[ZCP::BC_CONFIRMATION_DEPTH_SIGMA - 1]
    }
}

/// Attempt to construct a [BftPayload] from headers while performing immediate validations; see [BftPayload] type docs
impl<ZCP> TryFrom<Vec<BcBlockHeader>> for BftPayload<ZCP>
where
    ZCP: ZcashCrosslinkParameters,
{
    type Error = InvalidBftPayload;

    fn try_from(headers: Vec<BcBlockHeader>) -> Result<Self, Self::Error> {
        use InvalidBftPayload::*;

        let expected = ZCP::BC_CONFIRMATION_DEPTH_SIGMA;
        let actual = headers.len();
        if actual != expected {
            return Err(IncorrectConfirmationDepth { expected, actual });
        }

        todo!("all the documented validations");

        // Ok(BftPayload {
        //     headers,
        //     ph: PhantomData,
        // })
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
        /// The expected number of headers, as per [BC_CONFIRMATION_DEPTH_SIGMA](ZcashCrosslinkParameters::BC_CONFIRMATION_DEPTH_SIGMA)
        expected: usize,
        /// The number of headers present
        actual: usize,
    },
}
