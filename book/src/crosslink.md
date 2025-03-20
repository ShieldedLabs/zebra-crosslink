# Crosslink Functionality

This [zebra-crosslink](https://github.com/ShieldedLabs/zebra-crosslink) codebase is an in-progress implementation of the *Crosslink* hybrid Proof-of-Work / Proof-of-Stake consensus protocol described in [Zcash Trailing Finality Layer](https://electric-coin-company.github.io/tfl-book/).

**Status:** Early Prototype; see [Implementation Approach](#implementation-approach) below for definition.

## For Users

Presently, this implementation is not suitable for general users. There is no functioning testnet support. There is no guarantee Crosslink will activate on the Zcash network, although every indication we've seen is enthusiastic support from both the userbase and Zcash developer communities, so we expect it will be when the time is right.

## Implementation Approach

### Prototype Phase (You Are Here)

Our initial development effort will be "prototypical", producing "milestones" which have partial usable functionality which developers or motivated curious users can experiment with. However, during this prototype phase we make no guarantee about safety, correctness, performance, etc... of either the implementation or design.

During development of this working implementation, we will be filling in unspecific aspects of the protocol design, as well as making many implementation-specific decisions. During this prototype phase, our goal is to implement an _apparently_ working prototype rapidly, so to that end, we may make best-effort practical design or implementation decisions which may not prove to be safe enough or correct for mainnet production usage.

We will document all such decisions here, and for any significant decision we will provide an Architectural Design Record to follow this helpful practice [upstream has adopted](https://github.com/ZcashFoundation/zebra/pull/9310#issue-2886753962).

### Alpha & Beta Phase

Eventually, this prototype will reach a feature-complete "alpha" phase. At this phase we will begin refining both the code and the design to meet the high bar for rigor and safety for the Zcash mainnet.

It is during this phase that we will also be focused on merging the changes in earnest into upstream projects, including `zebrad`.

### Zcash Deployment Phase

Once the Beta phase reaches a sufficient level of maturity, there is widespread confidence in the design and implementation, and a clear mandate from Zcashers to activate it, we will begin the deployment phase.

During this phase, all Crosslink-specific functionality will be merged to upstream projects, or if it makes sense to the different teams involved to release differentiated first class products, we may also do that.

For example, it may make sense to have different specialized products such as a "PoW-full-node/PoS-light-client" that miners prefer, a distinct "finalizer node" that finalizer operators prefer, and a "light wallet backend service node" that wallet vendors or infrastructure providers prefer, and those might be supported by different teams. Or, it may make the most sense to support those different use cases in a unified codebase and product by a single team. We'll continue to engage with the other Zcash development teams early on to determine which approaches Shielded Labs can commit to supporting.

This phase is a success when the Crosslink functionality is released in mainnet production products (which is always well ahead of protocol activation).

### Activation and Follow-Through Phase

The final phase is crucial to success: we consider our effort successful only after the features have activated on mainnet, *and* we see successful up-take of those features, such as multiple functioning wallets, exchanges, DEXes, and finalizer provider services.

During this phase, we will be focused on ironing out any blockers to full successful adoption of the activated featureset. 

## Design & Architecture

TBD. Stay tuned.
