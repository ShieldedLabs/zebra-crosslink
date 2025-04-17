#[cfg(test)]
mod integration_tests {
    use std::sync::Arc;
    use zebra_chain::block::{Block, Hash as BlockHash, Height as BlockHeight};
    use zebra_chain::serialization::ZcashDeserialize;
    use zebra_crosslink::viz::VizState;
    use zebra_test::vectors::MAINNET_BLOCKS;

    fn get_test_block(height: u32) -> Arc<Block> {
        let bytes = MAINNET_BLOCKS.get(&height).unwrap();
        let block = Block::zcash_deserialize::<&[u8]>(bytes.as_ref()).expect("valid block");
        Arc::new(block)
    }

    #[test]
    fn crosslink_test_chain_growth_headless() {
        let blocks: Vec<Option<Arc<Block>>> = (0..5).map(|i| Some(get_test_block(i))).collect();
        let height_hashes: Vec<(BlockHeight, BlockHash)> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (BlockHeight(i as u32), b.as_ref().unwrap().hash()))
            .collect();

        let state = Arc::new(VizState {
            latest_final_block: Some(height_hashes[2]),
            bc_tip: Some(height_hashes[4]),
            height_hashes: height_hashes.clone(),
            blocks: blocks.clone(),
            internal_proposed_bft_string: Some("Genesis".into()),
            bft_blocks: vec![(0, "A:0".into()), (0, "B:1".into()), (1, "C:".into())],
        });

        assert_eq!(blocks.len(), 5);
        assert_eq!(state.height_hashes.len(), 5);

        #[cfg(feature = "viz_gui")]
        {
            zebra_crosslink::viz::serialization::write_to_file("viz_state.json", &state);

            eprintln!("🖼  Run `cargo run --bin viz_test_driver` to view visualization");
        }
    }
}
