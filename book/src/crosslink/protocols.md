# Protocols Overview

The Crosslink Protocol is a meta-protocol which ties together two subprotocols, a so-called "best chain protocol", and a "finalizing protocol". Crosslink is designed to be as modular and agnostic about those subprotocols as possible to ensure we have flexibility as protocol designers in adapting the subprotocols.

## Relationship Between Design and Implementation

This implementation of the Crosslink protocol is aimed at deploying a practical safe instantiation of the more general Crosslink protocol. Our goal is to follow the design specified at (FIXME link to Design chapters), while we will also fill in gaps or make modifications we believe are safe (especially during the prototype phase). The Crosslink section in this book will evolve to document all such details so that a protocol design and security expert familiar with the Crosslink design would be well equiped to evaluate the safety of this implementation.

This implementation uses Zcash PoW as the best chain protocol in order to provide a conservatively safe upgrade step from the current Zcash Mainnet. For the finalizing protocol, this implementation uses [MalachiteBFT]FIXME LINK for the BFT protocol.

Crosslink does not combine the subprotocols entirely agnostically, and they must be altered to meet specific security requirements.

## Modifications to BFT

The BFT subprotocol is not modified, per-se, but proposals must refer to known PoW blocks as candidates for finality, and a "PoW attestation" of a given number of PoW headers must be within the expected bounds of difficulty adjustment.

## Modifications to PoW Consensus

**Prototype Status:** The details of this section are not yet well-specified or implemented.

The PoW consensus protocol is modified in two ways:

- First, every block starting with the Crosslink activation height must refer to a final BFT block (with other additional constraints on that choice TODO: which?).
- Second, the chain selection rule forbids ever selecting a chain which excludes a known final BFT block.

A notable detail which is _not_ modified from Zcash Mainnet is that PoW blocks are the primary reference to transactions and the corresponding ledger state. However, the ledger state is modified:

## Modifications to Ledger State

**Prototype Status:** The details of this section are not yet well-specified or implemented.

In addition to the modifications to PoW consensus itself, the ledger state of Zcash is modified in these ways:

- The ledger state now includes a _candidate roster_ and an _active roster_ for every height. These are tables with entries consisting of finalizer signing keys and associated delegated ZEC weights.
- The roster state is updated for each PoW block (similar to all other ledger state). However, a nuance is that some consensus rules (particularly for selecting active BFT finalizers) rely only on _finalized_ PoW Blocks.
- A new transaction version is acceptable in PoW Blocks (starting at the activation height) which includes new fields which enable finalizers to enter the candidate roster, to be selected for the candidate roster, or to alter the delegated staking weight for finalizers. Like all transactions, these are created and authorized by user wallets.
