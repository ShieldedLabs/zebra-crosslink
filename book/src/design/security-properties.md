# Crosslink Security Properties

Crosslink is designed and can be analyzed with respect to the following security properties:

## _Irreversible Finality_

Transactions enter a finalized state, and finalized transactions cannot be rolled back. Failure of this security property is called “rollback attack”.

Irreversible finality cannot be provided by unalloyed Proof-of-Work, which provides only “probabilistic finality“. In probabilistic finality, the protocol provides no detection or prevention of rollback attack, so detection and prevention (and remediation) are left up to the users ("the social layer").

Importantly, with irreversible finality, all users who are sufficiently in sync with the consensus agree on finality status, so if there is a failure by the protocol to provide finality, all of those participants agree that this failure occurred. This contrasts with probabilistic finality—in that model, every user must select their own rollback threshold at which point they consider a protocol failure event to have occurred. Because these thresholds vary among users, rollbacks fragment the users into groups with competing interests.

## _Liveness/Availability_

 It is possible for any user to get at least some of their transactions paying sufficiently high fees to be included in one of the next few blocks. Failure of this security property is called a “denial-of-service for blockspace attack” (aka “blockspace DoS”). Two example attacks illustrate different attack approaches: in one case, an attacker has enough mining capacity to exclude all transactions (except perhaps their own); in another approach an attacker pays fees higher than any legitimate user for so many transactions that any other user cannot effectively get any of their transactions included.

## _Finality Progress_

Blocks become final or they are removed from the chain within a finite amount of time. A failure of “finality progress” is called a “finality stall attack”. This is subtly distinct from a general liveness failure: a finality stall may occur even while new blocks containing transactions are continually produced. The longer that finality stalls while new blocks arrive with new transactions, the more likely it is that those users who require finality and those users who do not rely on finality will diverge in their expectations and behavior, which is a growing economic and governance risk, so finality stalls may be especially pernicious.

## _Censorship-resistance_

It is possible to get new transactions processed even in the presence of an adversary who tries to prevent those specific transactions. Failure of this security property is called “censorship attack”. This kind of attack is distinct from blockspace DoS attacks because of the narrower targeting policy. In practice more general DoS attacks use different techniques than more targeted censorship.

Broadly speaking, an adversary may have a “censorship policy” based on a variety of criteria including both on-chain behavior and correlated metadata from many different domains.

Because censorship (and other privacy attacks) often rely on a wide array of data sources, practical protection also relies on a wide array of techniques and technologies out of scope for Crosslink’s design. One key area where we assume supporting defensive technologies (such as mixnets) is around network privacy for wallets (especially) and potentially other Crosslink infrastructure.

## _Supply-integrity_

The total supply of ZEC is correct and any user can verify that it is correct. Failure of this security property is called “counterfeiting attack”.

**TODO: Where does the following live?**

What is the difference between liveness and censorship-resistance? The difference is whether specific transactions are targeted (a censorship attack) or (a stall attack). An important fact is that a stall attack would be immediately noticed and a wide set of users would be able to agree that the stall attack is happening. In contrast, a censorship attack would tend to go unnoticed except by the victims, and it might be difficult to achieve widespread recognition that it had happened. For this reason, part of the design of Crosslink is intended to “convert censorship attacks into stall attacks” – make it impossible for an attacker to censor targeted transactions without causing the entire service to stall.
