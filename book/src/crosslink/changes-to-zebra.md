# Changes to Zebra

One goal while extending upstream [`zebra`](https://github.com/ZcashFoundation/zebra) was to minimize changes to existing code and introduce as much logic as possible into separate crates and `tower` services

## Crosslink Crates & Services

- `zebra-crosslink`: This crate provides the "TFL service" via `TFLServiceHandle` [^1].
- `zebra-crosslink-chain`: Contains Crosslink-specific protocol parameters and chain state types.

The TFL service is launched via the `zebrad start` subcommand along with other zebra internal services. This service is the connection between all of Zebra's pre-existing functionality and the BFT protocol, provided by the [Malachite BFT Engine](https://github.com/informalsystems/malachite).


**TODO:** Provide an overview of all "app-chain specific logic" here (using Cosmos-style terminology for "app-chain").

[^1] `TFL` stands for "Trailing Finality Layer", which was the original project name. Crosslink is a specific protocol which provides a "trailing finality layer": the finalization status of PoW blocks "trails behind" the most recent PoW blocks.

