
1. Run a Zebra Crosslink node and have it sync fully to Mainnet and start the mempool. You need to enable the RPC interface in the config so that Zaino can connect to it.
2. Run our modified Zaino with the default config plus no_sync, no_db and no_state set to true. This avoids the need for Zaino to also sync its own database.
3. Use our modified zcash-devtools to set the trailing finality by fiat. Example command invokation, `cargo run --all-features -- set-finality 00000000007a76d2cf945a817101e5abad237ac3d9a7fcea6f33d937c5d0279e -s localhost:8137`
