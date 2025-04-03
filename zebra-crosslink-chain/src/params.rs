//! Zcash Crosslink protocol parameter definitions and values

/// Zcash Crosslink protocol parameters
///
/// This is provided as a trait so that downstream users can define or plug in their own alternative parameters.
///
/// Ref: [Zcash Trailing Finality Layer §3.3.3 Parameters](https://electric-coin-company.github.io/tfl-book/design/crosslink/construction.html#parameters)
pub trait ZcashCrosslinkParameters {
    /// The best-chain confirmation depth, `σ`
    ///
    /// At least this many PoW blocks must be atop the PoW block used to obtain a finalized view.
    const BC_CONFIRMATION_DEPTH_SIGMA: usize;

    /// The depth of unfinalized PoW blocks past which "Stalled Mode" activates, `L`
    ///
    /// Quoting from [Zcash Trailing Finality Layer §3.3.3 Stalled Mode](https://electric-coin-company.github.io/tfl-book/design/crosslink/construction.html#stalled-mode):
    ///
    /// > In practice, L should be at least 2σ.
    const FINALIZATION_GAP_BOUND: usize;
}

/// Crosslink parameters chosed for prototyping / testing
///
/// <div class="warning">No verification has been done on the security or performance of these parameters.</div>
pub struct PrototypeParameters;

impl ZcashCrosslinkParameters for PrototypeParameters {
    const BC_CONFIRMATION_DEPTH_SIGMA: usize = 3;
    const FINALIZATION_GAP_BOUND: usize = 7;
}
