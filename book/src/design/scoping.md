Scope for Crosslink
===

# What are our goals for the first deployment of Crosslink?

# Rationale

## Why add a Proof-of-Stake finality gadget?

* The Proof-of-Stake finality gadget adds [_crosslink finality_](./terminology.md#crosslink_finality), which protects users from being robbed, improves the safety and efficiency of bridges, and enables centralized exchanges, decentralized exchanges, and other services to reduce and unify [deposit times](https://zechub.wiki/using-zcash/custodial-exchanges).
* Proof-of-Stake provides a different and complementary kind of security that pure Proof-of-Work doesn't, by allowing the value at stake (which inventivizes defenders to protect users) to be cumulative instead of transient. This makes Zcash more secure—it provides better security for the same security budget. It also makes Zcash more sustainable—it provides long-term security even as the rate of issuance shrinks, which strengthens assurance that the 21M supply cap will be maintained.
* Proof-of-Stake replaces some of the sell pressure from miners with buy pressure from finalizers. This is good because the price of ZEC is the fuel for the mission and attracts users.
* Proof-of-Stake allows a larger number of users to participate actively in the network than mining does, and to become direct recipients of ZEC issued by the blockchain. This increases the size and diversity of the set of users and stakeholders [^economies_of_scale].

[^economies_of_scale]: In Proof-of-Work, smaller miners have a substantial economic disadvantage compared to larger mining operations, resulting in almost all hashpower being controlled by a few large miners. This economic disadvantage is reduced in Proof-of-Stake finalization—smaller finalizers have less of an economic disadvantage compared to larger finalizers. The economic disadvantage is almost eliminated in Proof-of-Stake delegation—smaller delegators compete on a level playing field with larger delegators, earning roughly the same reward with roughly the same risk.

## Why Keep the Proof-of-Work blockchain?

* Proof-of-Work provides a different and complementary kind of security that pure Proof-of-Stake doesn’t, by requiring a recurring, non-reusable, and non-recoverable expenditure (by defenders and potential attackers). This prevents attackers from re-using resources in attacks (whether simultaneous or successive attacks). Crosslink security is therefore more robust and self-healing against large-scale attacks than pure Proof-of-Stake security.
* Proof-of-Work provides excellent liveness, availability, and censorship-resistance (which complements the censorship-resistance provided by using end-to-end-encryption whenever possible).
* Proof-of-Work allows people to earn ZEC by mining, even if they don’t already own any ZEC and they can’t acquire ZEC any other way.
* Keeping Proof-of-Work in addition to adding Proof-of-Stake means that in addition to adding the stakers, we also keep miners as active participants in the network and recipients of ZEC, increasing the size and diversity of the Zcash network and the ZEC economy.


UX goals
---

This list of [UX Goals](https://github.com/ShieldedLabs/zebra-crosslink/labels/UX%20Goal) is tracked on GitHub.

* CEXes ([GH #131](https://github.com/ShieldedLabs/zebra-crosslink/issues/131)) and decentralized services like DEXes/bridges ([GH #128](https://github.com/ShieldedLabs/zebra-crosslink/issues/128)) are willing to rely on Zcash transaction finality. E.g. Coinbase reduces its required confirmations to no longer than Crosslink finality. All or most services rely on the same canonical (protocol-provided) finality instead of enforcing [their own additional delays or conditions](https://zechub.wiki/using-zcash/custodial-exchanges), so users get predictable transaction finality across services.
* [GH #127](https://github.com/ShieldedLabs/zebra-crosslink/issues/127): Casual users (who understand little about crypto and do not use specialized tools such as a Linux user interface) delegate ZEC and get rewards from their mobile wallet. [GH #124](https://github.com/ShieldedLabs/zebra-crosslink/issues/124) They have to learn a minimal set of new concepts in order to do this.
* [GH #158](https://github.com/ShieldedLabs/zebra-crosslink/issues/158): Users can get compounding returns by leaving their stake plus their rewards staked, without them or their wallet having to take action.
* [GH #126](https://github.com/ShieldedLabs/zebra-crosslink/issues/126): Skilled users (who understand more and can use specialized tools) run finalizers and get rewards.
* [GH #214](https://github.com/ShieldedLabs/zebra-crosslink/issues/214): Skilled users can easily collect records of finalizer behavior and performaance.

_Shielded Labs’s First Deployment of Crosslink is not done until substantial numbers of real users are actually gaining these six benefits._

Deployment Goals
---

This list of [Deployment Goals](https://github.com/ShieldedLabs/zebra-crosslink/labels/Deployment%20Goals) is tracked on GitHub.

* [GH #125](https://github.com/ShieldedLabs/zebra-crosslink/issues/125): Zcash transactions come with a kind of finality which protects the users as much as possible against all possible attacks, and is sufficient for services such as cross-chain bridges and centralized exchanges.
* [GH #124](https://github.com/ShieldedLabs/zebra-crosslink/issues/124): Users can delegate their ZEC and earn rewards, safely and while needing to learn only the minimal number of new concepts.
    * Delegating to a finalizer does not enable the finalizer to steal your funds.
    * Delegating to a finalizer does not leak information that links the user's action to other information about them, such as their IP address, their other ZEC holdings that they are choosing not to stake, or their previous or future transactions.
* [GH #123](https://github.com/ShieldedLabs/zebra-crosslink/issues/123): The time-to-market and the risk of Shielded Labs's First Deployment of Crosslink is minimized: the benefits listed above start accruing to users as soon as safely possible.
* [GH #122](https://github.com/ShieldedLabs/zebra-crosslink/issues/122): Activating Crosslink on Zcash mainnet retains as much as possible of Zcash users' safety, security, privacy, and availability guarantees.

Trade-offs
---

Goals which we currently believe are not safely achievable in this first deployment without losing some of the above goals:
* Improving Zcash's bandwidth (number of transactions per time) or latency (time for a transaction).
* Deploying cross-chain interoperation such as the Inter-Blockchain Communication protocol (IBC) or Cross-Chain Interoperability Protocol (CCIP).
* Supporting a large number of finalizers so that any user who wants to run their own finalizer can do so.
* Supporting casual users, with little computer expertise, running finalizers.

Non-Goals
---

* The consensus mechanism is needed primarily to prevent double-spending/multi-spending/rollback attacks. It is not needed for censorship-resistance, since that is provided by end-to-end encryption, and it is not needed for counterfeiting-resistance, since that is provided by two layers of defense: proofs and turnstiles. So censorship-resistance and counterfeiting-resistance are non-goals for (at least) the first deployment of Crosslink. XXX actually we _do_ need censorship-resistance for transactions that have transparent components or that a would-be censor would have out-of-band knowledge
* This is neutral with regard to Zcash governance —-- it doesn't change anything about Zcash governance.

See also [Zebra Crosslink Implementation Requirements](https://docs.google.com/document/d/1YXalTGoezGH8GS1dknO8aK6eBFRq_Pq8LvDeho1KVZ8/edit?usp=sharing).
