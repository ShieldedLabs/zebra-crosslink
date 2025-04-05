//! Internal Zebra service for managing the Crosslink consensus protocol

#![allow(clippy::print_stdout)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::Instant;
use tracing::{error, info, warn};
/*
use rand::{CryptoRng, RngCore};
use crate::core::Round as BFTRound;
use std::str::FromStr;
use multiaddr::Multiaddr;
use rand::{SeedableRng, Rng};
use malachitebft_app_channel::app::types::codec::Codec;
use malachitebft_test::codec::proto::ProtobufCodec;
use malachitebft_app_channel::AppMsg as BFTAppMsg;
use async_trait::async_trait;
use malachitebft_app_channel::app::events::*;
use malachitebft_app_channel::app::types::core::*;
use malachitebft_app_channel::app::types::*;
use malachitebft_app_channel::app::*;
use malachitebft_test::{
    Address, Ed25519Provider, Genesis, Height as BFTHeight, PrivateKey, PublicKey, TestContext,
    Validator, ValidatorSet,
};
*/

pub mod service;

use crate::service::{
    TFLServiceCalls, TFLServiceError, TFLServiceHandle, TFLServiceRequest, TFLServiceResponse,
};

// TODO: do we want to start differentiating BCHeight/PoWHeight, BFTHeight/PoSHeigh etc?
use zebra_chain::block::{Block, Hash as BlockHash, Header as BlockHeader, Height as BlockHeight};
use zebra_state::{ReadRequest as ReadStateRequest, ReadResponse as ReadStateResponse};

/// Placeholder activation height for Crosslink functionality
pub const TFL_ACTIVATION_HEIGHT: BlockHeight = BlockHeight(2000);

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct TFLServiceInternal {
    latest_final_block: Option<(BlockHeight, BlockHash)>,
    tfl_is_activated: bool,

    stakers: Vec<TFLStaker>,

    // channels
    final_change_tx: broadcast::Sender<BlockHash>,
}

/// The finality status of a block
#[derive(Debug, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
pub enum TFLBlockFinality {
    // TODO: rename?
    /// The block height is above the finalized height, so it's not yet determined
    /// whether or not it will be finalized.
    NotYetFinalized,

    /// The block is finalized: it's height is below the finalized height and
    /// it is in the best chain.
    Finalized,

    /// The block cannot be finalized: it's height is below the finalized height and
    /// it is not in the best chain.
    CantBeFinalized,
}

/// Placeholder representation for entity staking on PoS chain.
// TODO: do we want to unify or separate staker/finalizer/delegator
#[derive(Debug, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
pub struct TFLStaker {
    id: u64, // TODO: IP/malachite identifier/...
    stake: u64, // TODO: do we want to store flat/tree delegators
             // ALT: delegate_stake_to_id
             // ...
}

/// Placeholder representation for group of stakers that are to be treated as finalizers.
#[derive(Debug, PartialEq, Eq, Clone, serde::Serialize, serde::Deserialize)]
pub struct TFLRoster {
    /// The list of stakers whose votes(?) will count. Sorted by weight(?)
    pub finalizers: Vec<TFLStaker>,
}

// TODO: Result?
async fn block_height_from_hash(call: &TFLServiceCalls, hash: BlockHash) -> Option<BlockHeight> {
    if let Ok(ReadStateResponse::BlockHeader { height, .. }) =
        (call.read_state)(ReadStateRequest::BlockHeader(hash.into())).await
    {
        Some(height)
    } else {
        None
    }
}

async fn block_height_hash_from_hash(
    call: &TFLServiceCalls,
    hash: BlockHash,
) -> Option<(BlockHeight, BlockHash)> {
    if let Ok(ReadStateResponse::BlockHeader {
        height,
        hash: check_hash,
        ..
    }) = (call.read_state)(ReadStateRequest::BlockHeader(hash.into())).await
    {
        assert_eq!(hash, check_hash);
        Some((height, hash))
    } else {
        None
    }
}

async fn _block_header_from_hash(
    call: &TFLServiceCalls,
    hash: BlockHash,
) -> Option<Arc<BlockHeader>> {
    if let Ok(ReadStateResponse::BlockHeader { header, .. }) =
        (call.read_state)(ReadStateRequest::BlockHeader(hash.into())).await
    {
        Some(header)
    } else {
        None
    }
}

async fn _block_prev_hash_from_hash(call: &TFLServiceCalls, hash: BlockHash) -> Option<BlockHash> {
    if let Ok(ReadStateResponse::BlockHeader { header, .. }) =
        (call.read_state)(ReadStateRequest::BlockHeader(hash.into())).await
    {
        Some(header.previous_block_hash)
    } else {
        None
    }
}

async fn tfl_reorg_final_block_height_hash(
    call: &TFLServiceCalls,
) -> Option<(BlockHeight, BlockHash)> {
    let locator = (call.read_state)(ReadStateRequest::BlockLocator).await;

    // NOTE: although this is a vector, the docs say it may skip some blocks
    // so we can't just `.get(MAX_BLOCK_REORG_HEIGHT)`
    if let Ok(ReadStateResponse::BlockLocator(hashes)) = locator {
        let result_1 = match hashes.last() {
            Some(hash) => block_height_from_hash(call, *hash)
                .await
                .map(|height| (height, *hash)),
            None => None,
        };

        /* Alternative implementations:
        use std::ops::Sub;
        use zebra_chain::block::HeightDiff as BlockHeightDiff;

        let result_2 = if hashes.len() == 0 {
            None
        } else {
            let tip_block_height = block_height_from_hash(call, *hashes.first().unwrap()).await;

            if let Some(height) = tip_block_height {
                if height < BlockHeight(zebra_state::MAX_BLOCK_REORG_HEIGHT) {
                    // not enough blocks for any to be finalized
                    None // may be different from `locator.last()` in this case
                } else {
                    let pre_reorg_height = height
                        .sub(BlockHeightDiff::from(zebra_state::MAX_BLOCK_REORG_HEIGHT))
                        .unwrap();
                    let final_block_req = ReadStateRequest::BlockHeader(pre_reorg_height.into());
                    let final_block_hdr = (call.read_state)(final_block_req).await;

                    if let Ok(ReadStateResponse::BlockHeader { height, hash, .. }) = final_block_hdr
                    {
                        Some((height, hash))
                    } else {
                        None
                    }
                }
            } else {
                None
            }
        };

        let mut result_3 = None;
        if hashes.len() > 0 {
            let tip_block_hdr = block_height_from_hash(call, *hashes.first().unwrap()).await;

            if let Some(height) = tip_block_hdr {
                if height >= BlockHeight(zebra_state::MAX_BLOCK_REORG_HEIGHT) {
                    // not enough blocks for any to be finalized
                    let pre_reorg_height = height
                        .sub(BlockHeightDiff::from(zebra_state::MAX_BLOCK_REORG_HEIGHT))
                        .unwrap();
                    let final_block_req = ReadStateRequest::BlockHeader(pre_reorg_height.into());
                    let final_block_hdr = (call.read_state)(final_block_req).await;

                    if let Ok(ReadStateResponse::BlockHeader { height, hash, .. }) = final_block_hdr
                    {
                        result_3 = Some((height, hash))
                    }
                }
            }
        };
        let result_3 = result_3;

        //assert_eq!(result_1, result_2); // NOTE: possible race condition: only for testing
        //assert_eq!(result_1, result_3); // NOTE: possible race condition: only for testing
        // Sam: YES! Indeed there were race conditions.
        */

        result_1
    } else {
        None
    }
}

async fn tfl_final_block_height_hash(
    internal_handle: TFLServiceHandle,
) -> Option<(BlockHeight, BlockHash)> {
    #[allow(unused_mut)]
    let mut internal = internal_handle.internal.lock().await;

    if internal.latest_final_block.is_some() {
        internal.latest_final_block
    } else {
        drop(internal);
        tfl_reorg_final_block_height_hash(&internal_handle.call).await
    }
}

/*
/// Make up a new value to propose
/// A real application would have a more complex logic here,
/// typically reaping transactions from a mempool and executing them against its state,
/// before computing the merkle root of the new app state.
fn bft_make_value(
    rng: &mut rand::rngs::StdRng,
    vote_extensions: &mut std::collections::HashMap<BFTHeight, VoteExtensions<TestContext>>,
    height: BFTHeight,
    _round: BFTRound,
) -> malachitebft_test::Value {
    let value = rng.gen_range(100..=100000);

    // TODO: Where should we verify signatures?
    let extensions = vote_extensions
        .remove(&height)
        .unwrap_or_default()
        .extensions
        .into_iter()
        .map(|(_, e)| e.message)
        .fold(bytes::BytesMut::new(), |mut acc, e| {
            acc.extend_from_slice(&e);
            acc
        })
        .freeze();

    malachitebft_test::Value { value, extensions }
}
*/

const MAIN_LOOP_SLEEP_INTERVAL: Duration = Duration::from_millis(125);
const MAIN_LOOP_INFO_DUMP_INTERVAL: Duration = Duration::from_millis(8000);

async fn tfl_service_main_loop(internal_handle: TFLServiceHandle) -> Result<(), String> {
    let call = internal_handle.call.clone();

    /*
        let mut rng = rand::rngs::StdRng::seed_from_u64(5);
        let private_key = PrivateKey::generate(&mut rng);
        let public_key = private_key.public_key();
        let _address = Address::from_public_key(&public_key);
        let _signing_provider = Ed25519Provider::new(private_key.clone());
        let ctx = TestContext::new();

        let initial_validator_set = ValidatorSet::new(vec![Validator::new(public_key, 1)]);
        let genesis = Genesis {
            validator_set: initial_validator_set.clone(),
        };

        let codec = ProtobufCodec;

        let init_bft_height = BFTHeight::new(1);
        let bft_node_handle = BFTNode {
            private_key: private_key.clone(),
        };

        // let mut bft_config = config::load_config(std::path::Path::new("C:\\Users\\azmre\\.malachite\\config\\config.toml"), None)
        //     .expect("Failed to load configuration file");
        let mut bft_config: malachitebft_test_cli::config::Config = Default::default(); // TODO: read from file?
                                                                                        //
        bft_config.consensus.p2p.listen_addr = Multiaddr::from_str("/ip4/127.0.0.1/tcp/12345").unwrap();
        bft_config.consensus.p2p.discovery = config::DiscoveryConfig {
            selector: config::Selector::from_str("random").unwrap(), // TODO: Kademlia?
            bootstrap_protocol: config::BootstrapProtocol::from_str("full").unwrap(), // TODO: Kademlia?
            num_outbound_peers: 1,                                   // TODO: 20
            num_inbound_peers: 1,                                    // TODO: 20
            ephemeral_connection_timeout: Duration::from_secs(5),    // TODO: ?
            enabled: false,
        };

        bft_config.mempool.p2p.listen_addr = Multiaddr::from_str("/ip4/127.0.0.1/tcp/22345").unwrap();
        bft_config.mempool.p2p.discovery = config::DiscoveryConfig {
            selector: config::Selector::from_str("random").unwrap(), // TODO: Kademlia?
            bootstrap_protocol: config::BootstrapProtocol::from_str("full").unwrap(), // TODO: Kademlia?
            num_outbound_peers: 1,                                   // TODO: 20
            num_inbound_peers: 1,                                    // TODO: 20
            ephemeral_connection_timeout: Duration::from_secs(5),    // TODO: ?
            enabled: true,
        };

        info!(?bft_config);

        let (mut channels, _engine_handle) = malachitebft_app_channel::start_engine(
            ctx,
            codec,
            bft_node_handle,
            bft_config,
            Some(init_bft_height),
            initial_validator_set,
        )
        .await
        .unwrap();
    */

    let mut run_instant = Instant::now();
    let mut last_diagnostic_print = Instant::now();
    let mut current_bc_tip: Option<(BlockHeight, BlockHash)> = None;
    let mut current_bc_final: Option<(BlockHeight, BlockHash)> = None;

    /*
        // TODO mutable state here in order to correctly respond to messages.
        let mut current_bft_height = init_bft_height;
        let mut current_bft_round = Round::Nil;
        let mut current_bft_proposer = None;
        let mut vote_extensions =
            std::collections::HashMap::<BFTHeight, VoteExtensions<TestContext>>::new();
        // let mut bft_values           = Vec::<Option<LocallyProposedValue>>::new();
    */
    loop {
        tokio::select! {
            // sleep if we are running ahead
            _ = tokio::time::sleep_until(run_instant) => {
                run_instant += MAIN_LOOP_SLEEP_INTERVAL;
            }
        /* ret = channels.consensus.recv() => {
            let msg = ret.expect("Channel to Malachite has been closed.");
            match msg {
                // The first message to handle is the `ConsensusReady` message, signaling to the app
                // that Malachite is ready to start consensus
                BFTAppMsg::ConsensusReady { reply } => {
                    info!("BFT Consensus is ready");

                    if reply.send(malachitebft_app_channel::ConsensusMsg::StartHeight(
                            current_bft_height,
                            genesis.validator_set.clone(),
                    )).is_err() {
                        tracing::error!("Failed to send ConsensusReady reply");
                    }
                },

                // The next message to handle is the `StartRound` message, signaling to the app
                // that consensus has entered a new round (including the initial round 0)
                BFTAppMsg::StartedRound {
                    height,
                    round,
                    proposer,
                } => {
                    info!(%height, %round, %proposer, "Started round");

                    current_bft_height   = height;
                    current_bft_round    = round;
                    current_bft_proposer = Some(proposer);
                },

                // At some point, we may end up being the proposer for that round, and the engine
                // will then ask us for a value to propose to the other validators.
                BFTAppMsg::GetValue {
                    height,
                    round,
                    timeout: _,
                    reply,
                } => {
                    info!(%height, %round, "Consensus is requesting a value to propose");

                    // TODO: read from existing

                    // // Check if we have a previously built value for that height and round
                    // if let Some(proposal) = state.get_previously_built_value(height, round) {
                    //     info!(value = %proposal.value.id(), "Re-using previously built value");

                    //     if reply.send(proposal).is_err() {
                    //         tracing::error!("Failed to send GetValue reply");
                    //     }

                    //     return Ok(());
                    // }

                    // // Otherwise, propose a new value
                    // let proposal = state.propose_value(height, round);
                    //

                    tokio::time::sleep(Duration::from_millis(500)).await;

                    let proposal = LocallyProposedValue::<TestContext> {
                        value: bft_make_value(&mut rng, &mut vote_extensions, height, round),
                        height,
                        round,
                    };

                    // Send it to consensus
                    if reply.send(proposal.clone()).is_err() {
                        tracing::error!("Failed to send GetValue reply");
                    }

                    // // Decompose the proposal into proposal parts and stream them over the network
                    // for stream_message in state.stream_proposal(proposal) {
                    //     info!(%height, %round, "Streaming proposal part: {stream_message:?}");
                    //     channels
                    //         .network
                    //         .send(malachitebft_app_channel::NetworkMsg::PublishProposalPart(stream_message))
                    //         .await?;
                    // }
                },

                BFTAppMsg::ProcessSyncedValue {
                    height,
                    round,
                    proposer,
                    value_bytes,
                    reply,
                } => {
                    info!(%height, %round, "Processing synced value");

                    let value = codec.decode(value_bytes).unwrap();

                    if reply.send(ProposedValue {
                        height,
                        round,
                        valid_round: Round::Nil,
                        proposer,
                        value,
                        validity: Validity::Valid,
                        // extension: None, TODO? "does not have this field"
                    }).is_err() {
                        tracing::error!("Failed to send ProcessSyncedValue reply");
                    }
                },

                // In some cases, e.g. to verify the signature of a vote received at a higher height
                // than the one we are at (e.g. because we are lagging behind a little bit),
                // the engine may ask us for the validator set at that height.
                //
                // In our case, our validator set stays constant between heights so we can
                // send back the validator set found in our genesis state.
                BFTAppMsg::GetValidatorSet { height: _, reply } => {
                    // TODO: parameterize by height
                    if reply.send(genesis.validator_set.clone()).is_err() {
                        tracing::error!("Failed to send GetValidatorSet reply");
                    }
                },

                // After some time, consensus will finally reach a decision on the value
                // to commit for the current height, and will notify the application,
                // providing it with a commit certificate which contains the ID of the value
                // that was decided on as well as the set of commits for that value,
                // ie. the precommits together with their (aggregated) signatures.
                BFTAppMsg::Decided {
                    certificate,
                    extensions,
                    reply,
                } => {
                    info!(
                        height = %certificate.height,
                        round = %certificate.round,
                        value = %certificate.value_id,
                        "Consensus has decided on value"
                    );

                    // When that happens, we store the decided value in our store
                    // TODO: state.commit(certificate, extensions).await?;
                    current_bft_height = certificate.height.increment();
                    current_bft_round  = Round::new(0);

                    // And then we instruct consensus to start the next height
                    if reply.send(malachitebft_app_channel::ConsensusMsg::StartHeight(
                            current_bft_height,
                            genesis.validator_set.clone(),
                    )).is_err() {
                        tracing::error!("Failed to send Decided reply");
                    }
                },

                BFTAppMsg::GetHistoryMinHeight { reply } => {
                    // TODO: min height from DB
                    let min_height = init_bft_height;
                    info!("Reply min_height: {}", min_height);
                    if reply.send(min_height).is_err() {
                        tracing::error!("Failed to send GetHistoryMinHeight reply");
                    }
                },

                BFTAppMsg::GetDecidedValue { height, reply } => {
                    let decided_value = None; // state.get_decided_value(&height).cloned();

                    if reply.send(decided_value).is_err() {
                        tracing::error!("Failed to send GetDecidedValue reply");
                    }
                },

                BFTAppMsg::ExtendVote {
                    height: _,
                    round: _,
                    value_id: _,
                    reply,
                } => {
                    // TODO
                    if reply.send(None).is_err() {
                        tracing::error!("Failed to send ExtendVote reply");
                    }
                },

                BFTAppMsg::VerifyVoteExtension {
                    height: _,
                    round: _,
                    value_id: _,
                    extension: _,
                    reply,
                } => {
                    // TODO
                    if reply.send(Ok(())).is_err() {
                        tracing::error!("Failed to send VerifyVoteExtension reply");
                    }
                },

                _ => tracing::error!(?msg, "Unhandled message from Malachite"),
            }
        }*/
        }

        let new_bc_tip = if let Ok(ReadStateResponse::Tip(val)) =
            (call.read_state)(ReadStateRequest::Tip).await
        {
            val
        } else {
            None
        };

        let new_bc_final = {
            // partial dup of tfl_final_block_hash
            let internal = internal_handle.internal.lock().await;

            if internal.latest_final_block.is_some() {
                internal.latest_final_block
            } else {
                drop(internal);

                if new_bc_tip != current_bc_tip {
                    info!("tip changed to {:?}", new_bc_tip);
                    tfl_reorg_final_block_height_hash(&call).await
                } else {
                    current_bc_final
                }
            }
        };

        // from this point onwards we must race to completion in order to avoid stalling incoming requests
        // NOTE: split to avoid deadlock from non-recursive mutex - can we reasonably change type?
        #[allow(unused_mut)]
        let mut internal = internal_handle.internal.lock().await;

        if new_bc_final != current_bc_final {
            info!("final changed to {:?}", new_bc_final);
            if let Some((_, new_final_hash)) = new_bc_final {
                let start_hash = if let Some((_, prev_hash)) = current_bc_final {
                    prev_hash
                } else {
                    new_final_hash
                };

                let (new_final_blocks, infos) = tfl_block_sequence(
                    &call,
                    start_hash,
                    Some(new_final_hash),
                    /*include_start_hash*/ true,
                    true,
                )
                .await;

                if let (Some(Some(first_block)), Some(Some(last_block))) =
                    (infos.first(), infos.last())
                {
                    println!(
                        "Height change: {} => {}:",
                        first_block.coinbase_height().unwrap_or(BlockHeight(0)).0,
                        last_block.coinbase_height().unwrap_or(BlockHeight(0)).0
                    );
                }
                tfl_dump_blocks(&new_final_blocks[..], &infos[..]);

                // walk all blocks in newly-finalized sequence & broadcast them
                for new_final_block in new_final_blocks {
                    // skip repeated boundary blocks
                    if let Some((_, prev_hash)) = current_bc_final {
                        if prev_hash == new_final_block {
                            continue;
                        }
                    }

                    // We ignore the error because there will be one in the ordinary case
                    // where there are no receivers yet.
                    let _ = internal.final_change_tx.send(new_final_block);
                }
            }
        }

        if !internal.tfl_is_activated {
            if let Some((height, _hash)) = new_bc_tip {
                if height < TFL_ACTIVATION_HEIGHT {
                    continue;
                } else {
                    internal.tfl_is_activated = true;
                    info!("activating TFL!");
                }
            }
        }

        if last_diagnostic_print.elapsed() >= MAIN_LOOP_INFO_DUMP_INTERVAL {
            last_diagnostic_print = Instant::now();
            if let (Some((tip_height, _tip_hash)), Some((final_height, _final_hash))) =
                (current_bc_tip, internal.latest_final_block)
            {
                if tip_height < final_height {
                    info!(
                        "Our PoW tip is {} blocks away from the latest final block.",
                        final_height - tip_height
                    );
                } else {
                    let behind = tip_height - final_height;
                    if behind > 512 {
                        warn!("WARNING! BFT-Finality is falling behind the PoW chain. Current gap to tip is {:?} blocks.", behind);
                    }
                }
            }
        }

        current_bc_tip = new_bc_tip;
        current_bc_final = new_bc_final;
    }
}

/*
struct BFTNode {
    private_key: PrivateKey,
}

#[derive(Clone, Copy, Debug)]
struct DummyHandle;

#[async_trait]
impl NodeHandle<TestContext> for DummyHandle {
    fn subscribe(&self) -> RxEvent<TestContext> {
        panic!();
    }

    async fn kill(&self, _reason: Option<String>) -> eyre::Result<()> {
        panic!();
    }
}

#[async_trait]
impl Node for BFTNode {
    type Context = TestContext;
    type Genesis = Genesis;
    type PrivateKeyFile = ();
    type SigningProvider = Ed25519Provider;
    type NodeHandle = DummyHandle;

    fn get_home_dir(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("aah_very_annoying_that_the_wal_is_required")
    }

    fn generate_private_key<R>(&self, rng: R) -> PrivateKey
    where
        R: RngCore + CryptoRng,
    {
        PrivateKey::generate(rng)
    }

    fn get_address(&self, pk: &PublicKey) -> Address {
        Address::from_public_key(pk)
    }

    fn get_public_key(&self, pk: &PrivateKey) -> PublicKey {
        pk.public_key()
    }

    fn get_keypair(&self, pk: PrivateKey) -> Keypair {
        Keypair::ed25519_from_bytes(pk.inner().to_bytes()).unwrap()
    }

    fn load_private_key(&self, _file: ()) -> PrivateKey {
        self.private_key.clone()
    }

    fn load_private_key_file(&self) -> std::io::Result<()> {
        Ok(())
    }

    fn make_private_key_file(&self, _private_key: PrivateKey) {
        panic!();
    }

    fn get_signing_provider(&self, private_key: PrivateKey) -> Self::SigningProvider {
        Ed25519Provider::new(private_key)
    }

    fn load_genesis(&self) -> std::io::Result<Self::Genesis> {
        panic!();
    }

    fn make_genesis(&self, validators: Vec<(PublicKey, VotingPower)>) -> Self::Genesis {
        let validators = validators
            .into_iter()
            .map(|(pk, vp)| Validator::new(pk, vp));

        let validator_set = ValidatorSet::new(validators);

        Genesis { validator_set }
    }

    async fn start(&self) -> eyre::Result<DummyHandle> {
        Ok(DummyHandle)
    }

    async fn run(self) -> eyre::Result<()> {
        Ok(())
    }
}
*/

async fn tfl_service_incoming_request(
    internal_handle: TFLServiceHandle,
    request: TFLServiceRequest,
) -> Result<TFLServiceResponse, TFLServiceError> {
    let call = internal_handle.call.clone();

    // from this point onwards we must race to completion in order to avoid stalling the main thread

    #[allow(unreachable_patterns)]
    match request {
        TFLServiceRequest::FinalBlockHash => Ok(TFLServiceResponse::FinalBlockHash(
            if let Some((_, hash)) = tfl_final_block_height_hash(internal_handle.clone()).await {
                Some(hash)
            } else {
                None
            },
        )),

        TFLServiceRequest::FinalBlockRx => {
            let internal = internal_handle.internal.lock().await;
            Ok(TFLServiceResponse::FinalBlockRx(
                internal.final_change_tx.subscribe(),
            ))
        }

        TFLServiceRequest::SetFinalBlockHash(hash) => Ok(TFLServiceResponse::SetFinalBlockHash(
            tfl_set_finality_by_hash(internal_handle.clone(), hash).await,
        )),

        TFLServiceRequest::BlockFinalityStatus(hash) => {
            Ok(TFLServiceResponse::BlockFinalityStatus({
                let block_hdr = (call.read_state)(ReadStateRequest::BlockHeader(hash.into()));
                let (final_height, final_hash) =
                    match tfl_final_block_height_hash(internal_handle.clone()).await {
                        Some(v) => v,
                        None => {
                            return Err(TFLServiceError::Misc(
                                "There is no final block.".to_string(),
                            ));
                        }
                    };

                if let Ok(ReadStateResponse::BlockHeader {
                    header: block_hdr,
                    height,
                    hash: mut check_hash,
                    ..
                }) = block_hdr.await
                {
                    assert_eq!(check_hash, block_hdr.hash());

                    if height > final_height {
                        Some(TFLBlockFinality::NotYetFinalized)
                    } else if check_hash == final_hash {
                        assert_eq!(height, final_height);
                        Some(TFLBlockFinality::Finalized)
                    } else {
                        // NOTE: option not available because KnownBlock is Request::, not ReadRequest::
                        // (despite all the current values being read from ReadStateService...)
                        // let known_block = (call.read_state)(ReadStateRequest::KnownBlock(hash.into()));
                        // let known_block = known_block.await.map_misc_error();
                        //
                        // if let Ok(ReadStateResponse::KnownBlock(Some(known_block))) = known_block {
                        //     match known_block {
                        //         BestChain => Some(TFLBlockFinality::Finalized),
                        //         SideChain => Some(TFLBlockFinality::CantBeFinalized),
                        //         Queue     => { debug!("Block in queue below final height"); None },
                        //     }
                        // } else {
                        //     None
                        // }

                        loop {
                            let hdrs = (call.read_state)(ReadStateRequest::FindBlockHeaders {
                                known_blocks: vec![check_hash],
                                stop: Some(final_hash),
                            })
                            .await;

                            if let Ok(ReadStateResponse::BlockHeaders(hdrs)) = hdrs {
                                let hdr = &hdrs
                                    .last()
                                    .expect("This case should be handled above")
                                    .header;
                                check_hash = hdr.hash();

                                if check_hash == final_hash {
                                    // is in best chain
                                    break Some(TFLBlockFinality::Finalized);
                                } else {
                                    let check_height =
                                        block_height_from_hash(&call, check_hash).await;

                                    if let Some(check_height) = check_height {
                                        if check_height >= final_height {
                                            // TODO: may not actually be possible to hit this without
                                            // caching non-final blocks ourselves, given that most
                                            // things get thrown away if not in the final chain.

                                            // is not in best chain
                                            break Some(TFLBlockFinality::CantBeFinalized);
                                        } else {
                                            // need to look at next batch of block headers
                                            assert!(hdrs.len() == (zebra_state::constants::MAX_FIND_BLOCK_HEADERS_RESULTS as usize));
                                            continue;
                                        }
                                    }
                                }
                            }

                            break None;
                        }
                    }
                } else {
                    None
                }
            }))
        }

        TFLServiceRequest::TxFinalityStatus(hash) => Ok(TFLServiceResponse::TxFinalityStatus({
            if let Ok(ReadStateResponse::Transaction(Some(tx))) =
                (call.read_state)(ReadStateRequest::Transaction(hash)).await
            {
                let (final_height, _final_hash) =
                    match tfl_final_block_height_hash(internal_handle.clone()).await {
                        Some(v) => v,
                        None => {
                            return Err(TFLServiceError::Misc(
                                "There is no final block.".to_string(),
                            ));
                        }
                    };

                if tx.height <= final_height {
                    // TODO: CantBeFinalized
                    Some(TFLBlockFinality::Finalized)
                } else {
                    Some(TFLBlockFinality::NotYetFinalized)
                }
            } else {
                None
            }
        })),

        TFLServiceRequest::UpdateStaker(staker) => {
            let mut internal = internal_handle.internal.lock().await;
            if let Some(staker_i) = internal.stakers.iter().position(|x| x.id == staker.id) {
                // existing staker: modify or remove
                if staker.stake == 0 {
                    internal.stakers.remove(staker_i);
                } else {
                    internal.stakers[staker_i] = staker;
                }
            } else if staker.stake != 0 {
                // new staker: insert
                internal.stakers.push(staker);
            }

            internal.stakers.sort_by(|a, b| b.stake.cmp(&a.stake)); // sort descending order
            Ok(TFLServiceResponse::UpdateStaker)
        }

        TFLServiceRequest::Roster => Ok(TFLServiceResponse::Roster({
            let internal = internal_handle.internal.lock().await;
            let mut roster = TFLRoster {
                finalizers: internal.stakers.clone(),
            };
            roster.finalizers.truncate(3);
            roster
        })),

        _ => Err(TFLServiceError::NotImplemented),
    }
}

async fn tfl_set_finality_by_hash(
    internal_handle: TFLServiceHandle,
    hash: BlockHash,
) -> Option<BlockHeight> {
    // ALT: Result with no success val?
    let mut internal = internal_handle.internal.lock().await;

    if internal.tfl_is_activated {
        // TODO: sanity checks
        let new_height = block_height_from_hash(&internal_handle.call, hash).await;

        if let Some(height) = new_height {
            internal.latest_final_block = Some((height, hash));
        }

        new_height
    } else {
        None
    }
}

// TODO: handle headers as well?
// NOTE: this is currently best-chain-only due to request/response limitations
// TODO: add more request/response pairs directly in zebra-state's ReadStateService
/// always returns block hashes. If read_extra_info is set, also returns Blocks, otherwise returns an empty vector.
async fn tfl_block_sequence(
    call: &TFLServiceCalls,
    start_hash: BlockHash,
    final_hash: Option<BlockHash>,
    include_start_hash: bool,
    read_extra_info: bool, // NOTE: done here rather than on print to isolate async from sync code
) -> (Vec<BlockHash>, Vec<Option<Arc<Block>>>) {
    // get "real" initial values //////////////////////////////
    let (start_height, init_hash) = {
        if let Ok(ReadStateResponse::BlockHeader { height, header, .. }) =
            (call.read_state)(ReadStateRequest::BlockHeader(start_hash.into())).await
        {
            if include_start_hash {
                // NOTE: BlockHashes does not return the first hash provided, so we move back 1.
                //       We would probably also be fine to just push it directly.
                (Some(height), Some(header.previous_block_hash))
            } else {
                (Some(height), Some(start_hash))
            }
        } else {
            (None, None)
        }
    };
    let final_height_hash = if let Some(hash) = final_hash {
        block_height_hash_from_hash(call, hash).await
    } else if let Ok(ReadStateResponse::Tip(val)) = (call.read_state)(ReadStateRequest::Tip).await {
        val
    } else {
        None
    };

    // check validity //////////////////////////////
    if start_height.is_none() {
        error!(?start_hash, "start_hash has invalid height");
        return (Vec::new(), Vec::new());
    }
    let start_height = start_height.unwrap();
    let init_hash = init_hash.unwrap();

    if final_height_hash.is_none() {
        error!(?final_height_hash, "final_hash has invalid height");
        return (Vec::new(), Vec::new());
    }
    let final_height = final_height_hash.unwrap().0;

    if final_height < start_height {
        error!(?final_height, ?start_height, "final_height < start_height");
        return (Vec::new(), Vec::new());
    }

    // build vector //////////////////////////////
    let mut hashes = Vec::with_capacity((final_height - start_height + 1) as usize);
    let mut chunk_i = 0;
    let mut chunk =
        Vec::with_capacity(zebra_state::constants::MAX_FIND_BLOCK_HASHES_RESULTS as usize);
    // NOTE: written as if for iterator
    let mut c = 0;
    loop {
        if chunk_i >= chunk.len() {
            let chunk_start_hash = if !chunk.is_empty() {
                &init_hash
            } else {
                // NOTE: as the new first element, this won't be repeated
                chunk.last().expect("should have chunk elements by now")
            };

            let res = (call.read_state)(ReadStateRequest::FindBlockHashes {
                known_blocks: vec![*chunk_start_hash],
                stop: final_hash,
            })
            .await;

            if let Ok(ReadStateResponse::BlockHashes(chunk_hashes)) = res {
                if c == 0 && include_start_hash && !chunk_hashes.is_empty() {
                    assert_eq!(
                        chunk_hashes[0], start_hash,
                        "first hash is not the one requested"
                    );
                }

                chunk = chunk_hashes;
            } else {
                break; // unexpected
            }

            chunk_i = 0;
        }

        if let Some(val) = chunk.get(chunk_i) {
            hashes.push(*val);
        } else {
            break; // expected
        };
        chunk_i += 1;
        c += 1;
    }

    let mut infos = Vec::with_capacity(if read_extra_info { hashes.len() } else { 0 });
    if read_extra_info {
        for hash in &hashes {
            infos.push(
                if let Ok(ReadStateResponse::Block(block)) =
                    (call.read_state)(ReadStateRequest::Block((*hash).into())).await
                {
                    block
                } else {
                    None
                },
            )
        }
    }

    (hashes, infos)
}

fn dump_hash_highlight_lo(hash: &BlockHash, highlight_chars_n: usize) {
    let hash_string = hash.to_string();
    let hash_str = hash_string.as_bytes();
    let bgn_col_str = "\x1b[90m".as_bytes(); // "bright black" == grey
    let end_col_str = "\x1b[0m".as_bytes(); // "reset"
    let grey_len = hash_str.len() - highlight_chars_n;

    let mut buf: [u8; 64 + 9] = [0; 73];
    let mut at = 0;
    buf[at..at + bgn_col_str.len()].copy_from_slice(bgn_col_str);
    at += bgn_col_str.len();

    buf[at..at + grey_len].copy_from_slice(&hash_str[..grey_len]);
    at += grey_len;

    buf[at..at + end_col_str.len()].copy_from_slice(end_col_str);
    at += end_col_str.len();

    buf[at..at + highlight_chars_n].copy_from_slice(&hash_str[grey_len..]);
    at += highlight_chars_n;

    let s = std::str::from_utf8(&buf[..at]).expect("invalid utf-8 sequence");
    print!("{}", s);
}

fn tfl_dump_blocks(blocks: &[BlockHash], infos: &[Option<Arc<Block>>]) {
    let is_unique = |prefix_len: usize, hashes: &[BlockHash]| -> bool {
        let mut prefixes = HashSet::with_capacity(hashes.len());

        // NOTE: characters correspond to nibbles
        let bytes_n = prefix_len / 2;
        let is_nib = (prefix_len % 2) != 0;

        for hash in hashes {
            let nib = if is_nib {
                Some(hash.0[bytes_n] & 0xf)
            } else {
                None
            };

            if !prefixes.insert((&hash.0[..bytes_n], nib)) {
                return false;
            }
        }

        true
    };

    let mut highlight_chars_n: usize = 1;
    while !is_unique(highlight_chars_n, blocks) {
        highlight_chars_n += 1;
        assert!(highlight_chars_n <= 64);
    }

    let print_color = true;

    for (block_i, hash) in blocks.iter().enumerate() {
        print!("  ");
        if print_color {
            dump_hash_highlight_lo(hash, highlight_chars_n);
        } else {
            print!("{}", hash);
        }

        if let Some(Some(block)) = infos.get(block_i) {
            let shielded_c = block
                .transactions
                .iter()
                .filter(|tx| tx.has_shielded_data())
                .count();
            print!(
                " - {}, height: {}, difficulty: {}, {:3} transactions ({} shielded)",
                block.header.time,
                block.coinbase_height().unwrap_or(BlockHeight(0)).0,
                block.header.difficulty_threshold,
                block.transactions.len(),
                shielded_c
            );
        }

        println!();
    }
}

async fn _tfl_dump_block_sequence(
    call: &TFLServiceCalls,
    start_hash: BlockHash,
    final_hash: Option<BlockHash>,
    include_start_hash: bool,
) {
    let (blocks, infos) =
        tfl_block_sequence(call, start_hash, final_hash, include_start_hash, true).await;
    tfl_dump_blocks(&blocks[..], &infos[..]);
}
