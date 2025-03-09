//! Internal Zebra service for managing the Crosslink consensus protocol

use multiaddr::Multiaddr;
use rand::Rng;
use rand::SeedableRng;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{error, info, warn};

pub mod service;

use crate::service::{
    ReadStateServiceProcedure, TFLServiceCalls, TFLServiceError, TFLServiceHandle,
    TFLServiceRequest, TFLServiceResponse,
};

// TODO: do we want to start differentiating BCHeight/PoWHeight, BFTHeight/PoSHeigh etc?
use zebra_chain::block::{Hash as BlockHash, Height as BlockHeight, HeightDiff as BlockHeightDiff};
use zebra_state::{
    HashOrHeight, ReadRequest as ReadStateRequest, ReadResponse as ReadStateResponse,
};

/// Placeholder activation height for Crosslink functionality
pub const TFL_ACTIVATION_HEIGHT: BlockHeight = BlockHeight(2000);

#[derive(Debug)]
pub(crate) struct TFLServiceInternal {
    val: u64,
    latest_final_hash: BlockHash,
    latest_final_height: BlockHeight,
    is_tfl_activated: bool,
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

async fn tfl_final_block_hash(call: &TFLServiceCalls) -> Option<BlockHash> {
    use std::ops::Sub;
    use zebra_state::HashOrHeight;

    let locator = (call.read_state)(ReadStateRequest::BlockLocator).await;

    // NOTE: although this is a vector, the docs say it may skip some blocks
    // so we can't just `.get(MAX_BLOCK_REORG_HEIGHT)`
    if let Ok(ReadStateResponse::BlockLocator(hashes)) = locator {
        let result_1 = match hashes.last() {
            Some(hash) => Some(*hash),
            None => None,
        };

        let result_2 = match hashes.len() {
            0 => None,
            _ => {
                let tip_block_hdr_req =
                    ReadStateRequest::BlockHeader(HashOrHeight::Hash(*hashes.first().unwrap()));
                let tip_block_hdr = (call.read_state)(tip_block_hdr_req).await;

                if let Ok(ReadStateResponse::BlockHeader { height, .. }) = tip_block_hdr {
                    if height < BlockHeight(zebra_state::MAX_BLOCK_REORG_HEIGHT) {
                        // not enough blocks for any to be finalized
                        None // may be different from `locator.last()` in this case
                    } else {
                        let pre_reorg_height = height
                            .sub(BlockHeightDiff::from(zebra_state::MAX_BLOCK_REORG_HEIGHT))
                            .unwrap();
                        let final_block_req =
                            ReadStateRequest::BlockHeader(HashOrHeight::Height(pre_reorg_height));
                        let final_block_hdr = (call.read_state)(final_block_req).await;

                        match final_block_hdr {
                            Ok(ReadStateResponse::BlockHeader { hash, .. }) => Some(hash),
                            _ => None,
                        }
                    }
                } else {
                    None
                }
            }
        };

        let mut result_3 = None;
        if hashes.len() > 0 {
            let tip_block_hdr_req =
                ReadStateRequest::BlockHeader(HashOrHeight::Hash(*hashes.first().unwrap()));
            let tip_block_hdr = (call.read_state)(tip_block_hdr_req).await;

            if let Ok(ReadStateResponse::BlockHeader { height, .. }) = tip_block_hdr {
                if height >= BlockHeight(zebra_state::MAX_BLOCK_REORG_HEIGHT) {
                    // not enough blocks for any to be finalized
                    let pre_reorg_height = height
                        .sub(BlockHeightDiff::from(zebra_state::MAX_BLOCK_REORG_HEIGHT))
                        .unwrap();
                    let final_block_req =
                        ReadStateRequest::BlockHeader(HashOrHeight::Height(pre_reorg_height));
                    let final_block_hdr = (call.read_state)(final_block_req).await;

                    if let Ok(ReadStateResponse::BlockHeader { hash, .. }) = final_block_hdr {
                        result_3 = Some(hash)
                    }
                }
            }
        };
        let result_3 = result_3;

        assert_eq!(result_1, result_2); // NOTE: possible race condition: only for testing
        assert_eq!(result_1, result_3); // NOTE: possible race condition: only for testing
        result_1
    } else {
        None
    }
}

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

const MAIN_LOOP_SLEEP_INTERVAL: Duration = Duration::from_millis(125);
const MAIN_LOOP_INFO_DUMP_INTERVAL: Duration = Duration::from_millis(8000);

async fn tfl_service_main_loop(internal_handle: TFLServiceHandle) -> Result<(), String> {
    let call = internal_handle.call.clone();

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

    // TODO mutable state here in order to correctly respond to messages.

    let mut last_diagnostic_print = Instant::now();
    let mut run_instant = Instant::now();
    let mut current_bft_height = init_bft_height;
    let mut current_bft_round = Round::Nil;
    let mut current_bft_proposer = None;
    let mut current_bc_tip = None;
    let mut current_bc_final = None;
    let mut vote_extensions =
        std::collections::HashMap::<BFTHeight, VoteExtensions<TestContext>>::new();
    // let mut bft_values           = Vec::<Option<LocallyProposedValue>>::new();
    loop {
        // sleep if we are running ahead

        let task_periodic_run = tokio::time::sleep_until(run_instant);
        let task_new_malachite_event = channels.consensus.recv();
        tokio::select! {
            _ = task_periodic_run => {
                run_instant += MAIN_LOOP_SLEEP_INTERVAL;
            }
            ret = task_new_malachite_event => {
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
                            error!("Failed to send ConsensusReady reply");
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
                        //         error!("Failed to send GetValue reply");
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
                            error!("Failed to send GetValue reply");
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
                            error!("Failed to send ProcessSyncedValue reply");
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
                            error!("Failed to send GetValidatorSet reply");
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
                            error!("Failed to send Decided reply");
                        }
                    },

                    BFTAppMsg::GetHistoryMinHeight { reply } => {
                        // TODO: min height from DB
                        let min_height = init_bft_height;
                        info!("Reply min_height: {}", min_height);
                        if reply.send(min_height).is_err() {
                            error!("Failed to send GetHistoryMinHeight reply");
                        }
                    },

                    BFTAppMsg::GetDecidedValue { height, reply } => {
                        let decided_value = None; // state.get_decided_value(&height).cloned();

                        if reply.send(decided_value).is_err() {
                            error!("Failed to send GetDecidedValue reply");
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
                            error!("Failed to send ExtendVote reply");
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
                            error!("Failed to send VerifyVoteExtension reply");
                        }
                    },

                    _ => error!(?msg, "Unhandled message from Malachite"),
                }
            }
        }

        let new_bc_tip = if let Ok(ReadStateResponse::Tip(val)) =
            (call.read_state)(ReadStateRequest::Tip).await
        {
            val
        } else {
            None
        };

        let new_bc_final = if new_bc_tip != current_bc_tip {
            info!("tip changed to {:?}", new_bc_tip);
            // TODO: walk difference in height

            tfl_final_block_hash(&call).await
        } else {
            current_bc_final
        };

        #[allow(unused_mut)]
        let mut internal = internal_handle.internal.lock().await;
        // from this point onwards we must race to completion in order to avoid stalling incoming requests

        if new_bc_final != current_bc_final {
            info!("final changed to {:?}", new_bc_final);
            if let Some(hash) = new_bc_final {
                internal.final_change_tx.send(hash);
            }
            // TODO: walk difference in height & send all
        }

        if !internal.is_tfl_activated {
            if let Some((height, hash)) = new_bc_tip {
                if height < TFL_ACTIVATION_HEIGHT {
                    continue;
                } else {
                    internal.is_tfl_activated = true;
                    info!("activating TFL!");
                }
            }
        }

        if last_diagnostic_print.elapsed() >= MAIN_LOOP_INFO_DUMP_INTERVAL {
            last_diagnostic_print = Instant::now();
            info!(?internal.val, "TFL val is {}!!!", internal.val);
            if let Some((tip_height, _hash)) = current_bc_tip {
                if tip_height < internal.latest_final_height {
                    info!(
                        "Our PoW tip is {} blocks away from the latest final block.",
                        internal.latest_final_height - tip_height
                    );
                } else {
                    let behind = tip_height - internal.latest_final_height;
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

use crate::core::Round as BFTRound;
use async_trait::async_trait;
use malachitebft_app_channel::app::events::*;
use malachitebft_app_channel::app::types::codec::Codec;
use malachitebft_app_channel::app::types::core::*;
use malachitebft_app_channel::app::types::*;
use malachitebft_app_channel::app::*;
use malachitebft_app_channel::AppMsg as BFTAppMsg;
use malachitebft_test::codec::proto::ProtobufCodec;
use malachitebft_test::{
    Address, Ed25519Provider, Genesis, Height as BFTHeight, PrivateKey, PublicKey, TestContext,
    Validator, ValidatorSet,
};
use rand::{CryptoRng, RngCore};

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

async fn tfl_service_incoming_request(
    internal_handle: TFLServiceHandle,
    request: TFLServiceRequest,
) -> Result<TFLServiceResponse, TFLServiceError> {
    let call = internal_handle.call.clone();

    #[allow(unused_mut)]
    let mut internal = internal_handle.internal.lock().await;
    // from this point onwards we must race to completion in order to avoid stalling the main thread

    #[allow(unreachable_patterns)]
    match request {
        TFLServiceRequest::IncrementVal => {
            let ret = internal.val;
            internal.val += 1;
            Ok(TFLServiceResponse::ValIncremented(ret))
        }

        TFLServiceRequest::FinalBlockHash => Ok(TFLServiceResponse::FinalBlockHash(
            tfl_final_block_hash(&internal_handle.call).await,
        )),

        TFLServiceRequest::FinalBlockRx => Ok(TFLServiceResponse::FinalBlockRx(
            internal.final_change_tx.subscribe(),
        )),

        TFLServiceRequest::BlockFinalityStatus(hash) => {
            Ok(TFLServiceResponse::BlockFinalityStatus({
                let hash_h = HashOrHeight::Hash(hash);

                let block_hdr = (call.read_state)(ReadStateRequest::BlockHeader(hash_h));
                let final_block_hash = match tfl_final_block_hash(&internal_handle.call).await {
                    Some(v) => v,
                    None => {
                        return Err(TFLServiceError::Misc(
                            "There is no final block.".to_string(),
                        ));
                    }
                };
                let final_block_hdr = (call.read_state)(ReadStateRequest::BlockHeader(
                    HashOrHeight::Hash(final_block_hash),
                ));

                let block_hdr = block_hdr.await;
                let final_block_hdr = final_block_hdr.await;

                if let Ok(ReadStateResponse::BlockHeader {
                    height: final_height,
                    hash: final_hash,
                    ..
                }) = final_block_hdr
                {
                    if let Ok(ReadStateResponse::BlockHeader {
                        header: block_hdr,
                        height,
                        hash: mut check_hash,
                        ..
                    }) = block_hdr
                    {
                        assert_eq!(check_hash, block_hdr.hash());

                        if height > final_height {
                            Some(TFLBlockFinality::NotYetFinalized)
                        } else if check_hash == final_hash {
                            Some(TFLBlockFinality::Finalized)
                        } else {
                            // NOTE: option not available because KnownBlock is Request::, not ReadRequest::
                            // (despite all the current values being read from ReadStateService...)
                            // let known_block = (call.read_state)(ReadStateRequest::KnownBlock(hash_h));
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
                                        let check_block_hdr =
                                            (call.read_state)(ReadStateRequest::BlockHeader(
                                                HashOrHeight::Hash(check_hash),
                                            ))
                                            .await;

                                        if let Ok(ReadStateResponse::BlockHeader {
                                            height: check_height,
                                            ..
                                        }) = check_block_hdr
                                        {
                                            if check_height >= final_height {
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
                } else {
                    None
                }
            }))
        }

        _ => Err(TFLServiceError::NotImplemented),
    }
}

pub fn spawn_new_tfl_service(
    read_state_service_call: ReadStateServiceProcedure,
) -> (TFLServiceHandle, JoinHandle<Result<(), String>>) {
    let handle1 = TFLServiceHandle {
        internal: Arc::new(Mutex::new(TFLServiceInternal {
            val: 0,
            latest_final_height: BlockHeight(0),
            latest_final_hash: BlockHash([0_u8; 32]),
            is_tfl_activated: false,
            final_change_tx: broadcast::channel(16).0,
        })),
        call: TFLServiceCalls {
            read_state: read_state_service_call,
        },
    };
    let handle2 = handle1.clone();

    (
        handle1,
        tokio::spawn(async move { tfl_service_main_loop(handle2).await }),
    )
}

impl fmt::Display for TFLServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TFLServiceError: {:?}", self)
    }
}

impl std::error::Error for TFLServiceError {}
