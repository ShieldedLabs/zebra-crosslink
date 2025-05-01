use color_eyre::eyre::Result;
use std::{sync::Arc, time::Duration};
use tower::{Service, ServiceExt};
use zebra_chain::{
    block::{genesis::regtest_genesis_block, Height},
    chain_sync_status::MockSyncStatus,
    parameters::{
        testnet::{self, ConfiguredActivationHeights, ConfiguredFundingStreams},
        NetworkUpgrade,
    },
    serialization::ZcashSerialize,
    work::difficulty::U256,
};
use zebra_consensus::router::init_test;
use zebra_crosslink::service::{spawn_new_tfl_service, TFLServiceRequest};
use zebra_crosslink_chain::params::PrototypeParameters;
use zebra_network::address_book_peers::MockAddressBookPeers;
use zebra_node_services::mempool;
use zebra_rpc::methods::GetBlockTemplateRpcServer;
use zebra_rpc::methods::{
    get_block_template_rpcs::{
        get_block_template::{proposal_block_from_template, GetBlockTemplateRequestMode},
        types::get_block_template,
        types::submit_block::{self, SubmitBlockChannel},
    },
    hex_data::HexData,
    GetBlockTemplateRpcImpl,
};
use zebra_state::{init_test_services, ReadRequest};
use zebra_test::mock_service::MockService;

#[tokio::test(flavor = "multi_thread")]
async fn crosslink_nu5_rollback() -> Result<()> {
    let _ = zebra_test::init();

    // Create Nu5 RegTest network
    let network = testnet::Parameters::build()
        .with_genesis_hash("029f11d80ef9765602235e1bc9727e3eb6ba20839319f761fee920d63401e327")
        .with_target_difficulty_limit(U256::from_big_endian(&[0x0f; 32]))
        .with_disable_pow(true)
        .with_slow_start_interval(Height::MIN)
        .with_activation_heights(ConfiguredActivationHeights {
            nu5: Some(1),
            ..Default::default()
        })
        .with_post_nu6_funding_streams(ConfiguredFundingStreams {
            height_range: Some(Height(1)..Height(100)),
            recipients: None,
        })
        .to_network();

    // Core zebra services, mempool, and Crosslink
    let (state, read_state, latest_chain_tip, _tip_change) = init_test_services(&network);

    let mempool = MockService::build()
        .with_max_request_delay(Duration::from_secs(5))
        .for_unit_tests();
    let read_state_for_tfl = read_state.clone();

    // Block Verifier
    let (verifier, _, _, _) =
        init_test(zebra_consensus::Config::default(), &network, state.clone()).await;

    // Create a Block with the genesis hash via get_block_template
    let genesis_hash = verifier
        .clone()
        .oneshot(zebra_consensus::Request::Commit(regtest_genesis_block()))
        .await
        .expect("should validate Regtest genesis block");

    let mut mempool_clone = mempool.clone();
    tokio::spawn(async move {
        loop {
            mempool_clone
                .expect_request_that(|req| matches!(req, mempool::Request::FullTransactions))
                .await
                .respond(mempool::Response::FullTransactions {
                    transactions: vec![],
                    transaction_dependencies: Default::default(),
                    last_seen_tip_hash: genesis_hash,
                });
        }
    });

    let submitblock_channel = SubmitBlockChannel::new();
    let mut mock_sync_status = MockSyncStatus::default();
    mock_sync_status.set_is_close_to_tip(true);

    let mut mining_config = zebra_rpc::config::mining::Config::default();
    mining_config.debug_like_zcashd = false;
    mining_config.miner_address = Some("tmTc6trRhbv96kGfA99i7vrFwb5p7BVFwc3".parse()?);

    let get_block_template = GetBlockTemplateRpcImpl::new(
        &network,
        mining_config,
        mempool,
        read_state,
        latest_chain_tip,
        verifier,
        mock_sync_status,
        MockAddressBookPeers::default(),
        Some(submitblock_channel.sender()),
    );

    let mut blocks = vec![];

    for _ in 0..5 {
        let block_template_resp = get_block_template.get_block_template(None).await?;
        let get_block_template::Response::TemplateMode(template) = block_template_resp else {
            panic!("Expected TemplateMode from GBT");
        };

        let block = proposal_block_from_template(&template, None, NetworkUpgrade::Nu5)?;
        let hex_block = HexData(block.zcash_serialize_to_vec()?);
        let proposal_check = get_block_template
            .get_block_template(Some(get_block_template::JsonParameters {
                mode: GetBlockTemplateRequestMode::Proposal,
                data: Some(hex_block.clone()),
                ..Default::default()
            }))
            .await?;

        let get_block_template::Response::ProposalMode(result) = proposal_check else {
            panic!("Expected ProposalMode from GBT::proposal");
        };
        assert!(result.is_valid(), "Block proposal must be valid");

        let submit_result = get_block_template.submit_block(hex_block, None).await?;
        assert_eq!(submit_result, submit_block::Response::Accepted);

        blocks.push(block);
    }

    let (tfl_service_handle, tfl_task) = spawn_new_tfl_service::<PrototypeParameters>(
        Arc::new(move |req: ReadRequest| {
            let mut read_state = read_state_for_tfl.clone();
            Box::pin(async move {
                read_state
                    .ready()
                    .await
                    .map_err(|e| eyre::eyre!("read_state not ready: {e}"))?
                    .call(req)
                    .await
            })
        }),
        zebra_crosslink::config::Config::default(),
    );

    for block in blocks {
        println!(
            "Finalizing block at height: {:?} with hash {:?}",
            block.coinbase_height(),
            block.hash()
        );

        // Feed it into Crosslink
        let block_hash = block.hash();
        let response = tfl_service_handle
            .clone()
            .ready()
            .await?
            .call(TFLServiceRequest::SetFinalBlockHash(block_hash))
            .await?;

        println!("Crosslink SetBlockHash response: {response:?}");
    }

    // Check current final hash
    let current_final = tfl_service_handle
        .clone()
        .ready()
        .await?
        .call(TFLServiceRequest::FinalBlockHash)
        .await?;

    println!("Crosslink finalization hash: {current_final:?}");

    let _ = tfl_task.await?;

    Ok(())
}
