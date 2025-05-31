# Implementation Overview

## Components

The primary component to enable activating the Crosslink protocol on Zcash Mainnet is this `zebra-crosslink` full node. We anticipate this codebase being merged into upstream [`zebra`](https://github.com/ZcashFoundation/zebra) prior to the protocol activation if we are successful.

This codebase will be able to validate the network ledger by relying on both the Zcash PoW protocol and the BFT protocol which provides trailing finality.

Additionally, it will be possible to operate a _finalizer_ using this codebase, which will protect consensus finality for the Zcash network via the BFT protocol.

### Other Components

A successful deployment to Zcash Mainnet will include more than just a Crosslink-enabled full node: it would include wallets with PoS delegation support, finalizer tools, block explorer support, and potentially other products or tools.

However, [Shielded Labs](https://shieldedlabs.net) will only be directly creating a small subset of these products. We intend, at mimumum, to also produce PoS delegation support aimed at mobile wallet use case support. 
