# Terminology

Here we introduce terminology specific to this design, especially when it may deviate from other consensus systems or common terminology used in the consensus protocol field.

## Finality

We use the term _finality_ explicitly to mean an [objectively verifiable](#FIXME) property of a given local node's [block](#FIXME) which has these characteristics:

- It is _accountable_ because the chain contains explicit voting records which indicate each participating [finalizer](#FIXME)'s on-chain behavior.
- It is _irreversible_ within the protocol scope. The only way to "undo" a final block is to modify the software to do so, which requires intervention by the user. Note that because a [finality safety violation](#FIXME) is always theoretically possible, this implies the only recovery from such a violation is out-of-band to the protocol and requires human intervention.
- It is _in consensus_ meaning that all nodes which have received a sufficient number of valid blocks agree on the finality status, and also can rely on all other nodes in this set arriving at the same conclusion.
- It provides [assymetric cost-of-attack defense](#FIXME).
