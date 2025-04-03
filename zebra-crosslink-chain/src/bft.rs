use std::marker::PhantomData;

use thiserror::Error;
use zebra_chain::block as bcblock;

use crate::params::ZcashCrosslinkParameters;

/// The BFT block content for Crosslink
///
/// Ref: [Zcash Trailing Finality Layer §3.3.3 Structural Additions](https://electric-coin-company.github.io/tfl-book/design/crosslink/construction.html#structural-additions)
#[derive(Debug)]
pub struct BftPayload<ZCP>
where
    ZCP: ZcashCrosslinkParameters,
{
    headers: Vec<bcblock::Header>,
    ph: PhantomData<ZCP>,
}

impl<ZCP> TryFrom<Vec<bcblock::Header>> for BftPayload<ZCP>
where
    ZCP: ZcashCrosslinkParameters,
{
    type Error = InvalidConfirmationDepth;

    fn try_from(headers: Vec<bcblock::Header>) -> Result<Self, Self::Error> {
        let expected = ZCP::BC_CONFIRMATION_DEPTH_SIGMA;
        let actual = headers.len();
        if actual == expected {
            Ok(BftPayload {
                headers,
                ph: PhantomData,
            })
        } else {
            Err(InvalidConfirmationDepth { expected, actual })
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid confirmation depth: Crosslink requires {expected} while {actual} were present")]
pub struct InvalidConfirmationDepth {
    expected: usize,
    actual: usize,
}
