# The `zebra-crosslink-chain` Crate

The `zebra-crosslink-chain` crate distills the essential protocol parameters and blockchain state for Crosslink, while excluding all other considerations such as networking, BFT-specific details, POW-specific details, configuration, and so forth.

This code organization is intended to provide coordination for these different audiences:

- `zebra-crosslink` implementors,
- protocol experts focused on `Crosslink 2` specifically, such as protocol engineers, security auditors, etc...,
- -and developers of other services or tools that are aware of Crosslink (such as perhaps block explorers, or other infrastructure tooling providers).

In particular, this crate has nothing specific to the BFT protocol or Proof-of-Stake itself (accounting, finalizer rosters, etc...). It's theoretically possible that Zcash could upgrade or change between different BFT protocols without altering the contents of this crate.

This crate is named similarly to `zebra-chain` which has a similar scope for the current Zcash mainnet: ie, protocol types and parameters without all of the complexity of networking, processing, generating, etc... Once `zebra-crosslink` activates, code which needs to fully validate Zcash consensus will necessarily rely on both crates. Alternatively, at that time, it may make sense to merge these into a single crate (or introduce a new "container crate") to codify the scope of "all parameters and types necessary for validating consensus, without I/O, networking, processing logic...").

## The `ZcashCrosslinkParameters` Struct

This struct contains the protocol parameters (i.e. constant tunable values) defined by the Crosslink 2 construction.

Presently we only have semi-arbitrary values chosen as test parameters.

## The `BftPayload` Struct

The primary type in this crate is `BftPayload`. This type contains everything necessary for a Crosslink BFT _proposal_ (without BFT signatures).

**Design Note:** This type currently contains $\sigma$ PoW block headers in full, where $\sigma$ is the "best-chain confirmation depth" in [Zcash Trailing Finality Layer §3.3.3 Parameters](https://electric-coin-company.github.io/tfl-book/design/crosslink/construction.html#parameters). An alternative which should be equivalently validatable / secure but has performance trade-offs would be to include only $\sigma$ block hashes. This trades-off "immediate verifiability" versus redundant storage.

