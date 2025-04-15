//! Internal Zebra service for managing the Crosslink consensus protocol

#![allow(clippy::print_stdout)]

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::Instant;
use tracing::{error, info, warn};
use crate::core::Round as BFTRound;
use async_trait::async_trait;
use malachitebft_app_channel::app::config as mconfig;
use malachitebft_app_channel::app::events::*;
use malachitebft_app_channel::app::node::NodeConfig;
use malachitebft_app_channel::app::types::codec::Codec;
use malachitebft_app_channel::app::types::core::*;
use malachitebft_app_channel::app::types::*;
use malachitebft_app_channel::app::*;
use malachitebft_app_channel::AppMsg as BFTAppMsg;
use malachitebft_app_channel::NetworkMsg;
use malachitebft_test::codec::proto::ProtobufCodec;
use malachitebft_test::{
    Address, Ed25519Provider, Genesis, Height as BFTHeight, PrivateKey, ProposalData, ProposalFin,
    ProposalInit, ProposalPart, PublicKey, TestContext, Validator, ValidatorSet,
};
use multiaddr::Multiaddr;
use rand::{CryptoRng, RngCore};
use rand::{Rng, SeedableRng};
use sha3::Digest;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hasher};
use std::str::FromStr;
use sync::RawDecidedValue;
use tempdir::TempDir;

pub mod service;
pub mod config {
    use serde::{Deserialize, Serialize};

    /// Configuration for the state service.
    #[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
    #[serde(deny_unknown_fields, default)]
    pub struct Config {
        pub listen_address: Option<String>,
        pub public_address: Option<String>,
        pub malachite_peers: Vec<String>,
    }
    impl Default for Config {
        fn default() -> Self {
            Self {
                listen_address: None,
                public_address: None,
                malachite_peers: Vec::new(),
            }
        }
    }
}

#[cfg(feature = "viz_gui")]
pub mod viz;

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

    bft_block_strings: Vec<String>,
    proposed_bft_string: Option<String>,
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
    tracing::error!("bft_make_value");
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
    let config = internal_handle.config.clone();

    #[cfg(feature = "viz_gui")]
    {
        let rt = tokio::runtime::Handle::current();
        let viz_tfl_handle = internal_handle.clone();
        tokio::task::spawn_blocking(move || rt.block_on(viz::service_viz_requests(viz_tfl_handle)));
    }

    fn rng_private_public_key_from_address(addr: &str) -> (rand::rngs::StdRng, PrivateKey, PublicKey) {
        let mut hasher = DefaultHasher::new();
        hasher.write(addr.as_bytes());
        let seed = hasher.finish();
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        let private_key = PrivateKey::generate(&mut rng);
        let public_key = private_key.public_key();
        (rng, private_key, public_key)
    }

    let (mut rng, my_private_key, my_public_key) = if let Some(ref address) = config.public_address {
        rng_private_public_key_from_address(&address)
    } else {
        let mut rng = rand::rngs::StdRng::seed_from_u64(0);
        let private_key = PrivateKey::generate(&mut rng);
        let public_key = private_key.public_key();
        (rng, private_key, public_key)
    };

    let initial_validator_set = {
        let mut array = Vec::with_capacity(config.public_address.is_some() as usize + config.malachite_peers.len());

        if config.public_address.is_some() {
            array.push(Validator::new(my_public_key, 1));
        }

        for peer in config.malachite_peers.iter() {
            let (_, _, public_key) = rng_private_public_key_from_address(&peer);
            array.push(Validator::new(public_key, 1));
        }

        ValidatorSet::new(array)
    };

    let my_address = Address::from_public_key(&my_public_key);
    let my_signing_provider = Ed25519Provider::new(my_private_key.clone());
    let ctx = TestContext::new();

    let genesis = Genesis {
        validator_set: initial_validator_set.clone(),
    };

    let codec = ProtobufCodec;

    let init_bft_height = BFTHeight::new(1);
    let bft_node_handle = BFTNode {
        private_key: my_private_key.clone(),
    };

    // let mut bft_config = config::load_config(std::path::Path::new("C:\\Users\\azmre\\.malachite\\config\\config.toml"), None)
    //     .expect("Failed to load configuration file");
    let mut bft_config: BFTConfig = Default::default(); // TODO: read from file?


    if let Some(address) = config.public_address {
        bft_config
            .consensus
            .p2p
            .persistent_peers
            .push(Multiaddr::from_str(&address).unwrap());
    }

    for peer in config.malachite_peers.iter() {
        bft_config
            .consensus
            .p2p
            .persistent_peers
            .push(Multiaddr::from_str(&peer).unwrap());
    }

    //bft_config.consensus.p2p.transport = mconfig::TransportProtocol::Quic;
    if let Some(listen_addr) = config.listen_address {
        bft_config.consensus.p2p.listen_addr = Multiaddr::from_str(&listen_addr).unwrap();
    }
    bft_config.consensus.p2p.discovery = mconfig::DiscoveryConfig {
        selector: mconfig::Selector::Kademlia,
        bootstrap_protocol: mconfig::BootstrapProtocol::Kademlia,
        num_outbound_peers: 10,
        num_inbound_peers: 30,
        ephemeral_connection_timeout: Duration::from_secs(5),
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

    let mut run_instant = Instant::now();
    let mut last_diagnostic_print = Instant::now();
    let mut current_bc_tip: Option<(BlockHeight, BlockHash)> = None;
    let mut current_bc_final: Option<(BlockHeight, BlockHash)> = None;

    // TODO mutable state here in order to correctly respond to messages.
    let mut current_bft_height = init_bft_height;
    let mut current_bft_round = Round::Nil;
    let mut current_bft_proposer = None;
    let mut vote_extensions =
        std::collections::HashMap::<BFTHeight, VoteExtensions<TestContext>>::new();

    let mut prev_bft_values = HashMap::<(u64, i64), ProposedValue<TestContext>>::new();
    let mut decided_bft_values = HashMap::<u64, RawDecidedValue<TestContext>>::new();

    let mut streams_map = strm::PartStreamsMap::new();

    loop {
        tokio::select! {
                    // sleep if we are running ahead
                    _ = tokio::time::sleep_until(run_instant) => {
                        run_instant += MAIN_LOOP_SLEEP_INTERVAL;
                    }
                    ret = channels.consensus.recv() => {
                        let msg = ret.expect("Channel to Malachite has been closed.");
                        match msg {
                            // The first message to handle is the `ConsensusReady` message, signaling to the app
                            // that Malachite is ready to start consensus
                            BFTAppMsg::ConsensusReady { reply } => {
                                info!("BFT Consensus is ready");

                                if reply.send((current_bft_height, genesis.validator_set.clone())).is_err() {
                                    tracing::error!("Failed to send ConsensusReady reply");
                                }
                            },

                            // The next message to handle is the `StartRound` message, signaling to the app
                            // that consensus has entered a new round (including the initial round 0)
                            BFTAppMsg::StartedRound {
                                height,
                                round,
                                proposer,
                                reply_value,
                            } => {
                                info!(%height, %round, %proposer, "Started round");

                                current_bft_height   = height;
                                current_bft_round    = round;
                                current_bft_proposer = Some(proposer);

                                // If we have already built or seen a value for this height and round,
                                // send it back to consensus. This may happen when we are restarting after a crash.
                                if let Some(proposal) = prev_bft_values.get(&(height.as_u64(), round.as_i64())) {
                                    info!(%height, %round, "Replaying already known proposed value: {}", proposal.value.id());

                                    if reply_value.send(Some(proposal.clone())).is_err() {
                                        error!("Failed to send undecided proposal");
                                    }
                                } else {
                                    let _ = reply_value.send(None);
                                }
                            },

                            // At some point, we may end up being the proposer for that round, and the engine
                            // will then ask us for a value to propose to the other validators.
                            BFTAppMsg::GetValue {
                                height,
                                round,
                                timeout,
                                reply,
                            } => {
                                info!(%height, %round, "Consensus is requesting a value to propose. Timeout = {} ms.", timeout.as_millis());

                                if let Some(propose_string) = internal_handle.internal.lock().await.proposed_bft_string.take() {
                                    // Here it is important that, if we have previously built a value for this height and round,
                                    // we send back the very same value.
                                    let proposal = if let Some(val) = prev_bft_values.get(&(height.as_u64(), round.as_i64())) {
                                        info!(value = %val.value.id(), "Re-using previously built value");
                                        val.clone()
                                    } else {
                                        let val = ProposedValue {
                                            height,
                                            round,
                                            valid_round: Round::Nil,
                                            proposer: my_address,
                                            value: malachitebft_test::Value::new(propose_string.parse().unwrap_or(rng.gen_range(100..=100000))),
                                            validity: Validity::Valid,
                                            // extension: None, TODO? "does not have this field"
                                        };
                                        prev_bft_values.insert((height.as_u64(), round.as_i64()), val.clone());
                                        val
                                    };
                                    if reply.send(LocallyProposedValue::<TestContext>::new(
                                            proposal.height,
                                            proposal.round,
                                            proposal.value.clone(),
                                        )).is_err() {
                                        error!("Failed to send GetValue reply");
                                    }

                                    // The POL round is always nil when we propose a newly built value.
                                    // See L15/L18 of the Tendermint algorithm.
                                    let pol_round = Round::Nil;

                                    // NOTE(Sam): I have inlined the code from the example so that we
                                    // can actually see the functionality. I am not sure what the purpose
                                    // of this circus is. Why not just send the value with a simple signature?
                                    // I am sure there is a good reason.

                                    let mut hasher = sha3::Keccak256::new();
                                    let mut parts = Vec::new();

                                    // Init
                                    // Include metadata about the proposal
                                    {
                                        parts.push(ProposalPart::Init(ProposalInit {
                                            height: proposal.height,
                                            round: proposal.round,
                                            pol_round,
                                            proposer: my_address,
                                        }));

                                        hasher.update(proposal.height.as_u64().to_be_bytes().as_slice());
                                        hasher.update(proposal.round.as_i64().to_be_bytes().as_slice());
                                    }

            fn factor_value(value: malachitebft_test::Value) -> Vec<u64> {
                let mut factors = Vec::new();
                let mut n = value.value;

                let mut i = 2;
                while i * i <= n {
                    if n % i == 0 {
                        factors.push(i);
                        n /= i;
                    } else {
                        i += 1;
                    }
                }

                if n > 1 {
                    factors.push(n);
                }

                factors
            }

                                    // Data
                                    // Include each prime factor of the value as a separate proposal part
                                    {
                                        for factor in factor_value(proposal.value) {
                                            parts.push(ProposalPart::Data(ProposalData::new(factor)));

                                            hasher.update(factor.to_be_bytes().as_slice());
                                        }
                                    }

                                    // Fin
                                    // Sign the hash of the proposal parts
                                    {
                                        let hash = hasher.finalize().to_vec();
                                        let signature = my_signing_provider.sign(&hash);
                                        parts.push(ProposalPart::Fin(ProposalFin::new(signature)));
                                    }

                                    let stream_id = {
                                        let mut bytes = Vec::with_capacity(size_of::<u64>() + size_of::<u32>());
                                        bytes.extend_from_slice(&height.as_u64().to_be_bytes());
                                        bytes.extend_from_slice(&round.as_u32().unwrap().to_be_bytes());
                                        malachitebft_app_channel::app::types::streaming::StreamId::new(bytes.into())
                                    };

                                    let mut msgs = Vec::with_capacity(parts.len() + 1);
                                    let mut sequence = 0;

                                    for part in parts {
                                        let msg = malachitebft_app_channel::app::types::streaming::StreamMessage::new(stream_id.clone(), sequence, malachitebft_app_channel::app::streaming::StreamContent::Data(part));
                                        sequence += 1;
                                        msgs.push(msg);
                                    }

                                    msgs.push(malachitebft_app_channel::app::types::streaming::StreamMessage::new(stream_id, sequence, malachitebft_app_channel::app::streaming::StreamContent::Fin));

                                    for stream_message in msgs {
                                        info!(%height, %round, "Streaming proposal part: {stream_message:?}");
                                        channels
                                            .network
                                            .send(NetworkMsg::PublishProposalPart(stream_message))
                                            .await.unwrap();
                                    }
                                }
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
                                let proposed_value = ProposedValue {
                                    height,
                                    round,
                                    valid_round: Round::Nil,
                                    proposer,
                                    value,
                                    validity: Validity::Valid,
                                };

                                prev_bft_values.insert((height.as_u64(), round.as_i64()), proposed_value.clone());

                                if reply.send(proposed_value).is_err() {
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

                                let decided_value = prev_bft_values.get(&(certificate.height.as_u64(), certificate.round.as_i64())).unwrap();

                                let raw_decided_value = RawDecidedValue {
                                    certificate: certificate.clone(),
                                    value_bytes: ProtobufCodec.encode(&decided_value.value).unwrap(),
                                };

                                decided_bft_values.insert(certificate.height.as_u64(), raw_decided_value);

                                let mut internal = internal_handle.internal.lock().await;
                                internal.bft_block_strings.insert(certificate.height.as_u64() as usize - 1, format!("{:?}", decided_value.value.value));

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
                                if reply.send(min_height).is_err() {
                                    tracing::error!("Failed to send GetHistoryMinHeight reply");
                                }
                            },

                            BFTAppMsg::GetDecidedValue { height, reply } => {
                                let raw_decided_value = decided_bft_values.get(&height.as_u64()).cloned();

                                if reply.send(raw_decided_value).is_err() {
                                    tracing::error!("Failed to send GetDecidedValue reply");
                                }
                            },

                            BFTAppMsg::ExtendVote {
                                height: _,
                                round: _,
                                value_id: _,
                                reply,
                            } => {
        tracing::error!("extend vote");
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
        tracing::error!("verify vote extension");
                                if reply.send(Ok(())).is_err() {
                                    tracing::error!("Failed to send VerifyVoteExtension reply");
                                }
                            },

                            // On the receiving end of these proposal parts (ie. when we are not the proposer),
                            // we need to process these parts and re-assemble the full value.
                            // To this end, we store each part that we receive and assemble the full value once we
                            // have all its constituent parts. Then we send that value back to consensus for it to
                            // consider and vote for or against it (ie. vote `nil`), depending on its validity.
                            BFTAppMsg::ReceivedProposalPart { from, part, reply } => {
                                let part_type = match &part.content {
                                    malachitebft_app_channel::app::streaming::StreamContent::Data(part) => part.get_type(),
                                    malachitebft_app_channel::app::streaming::StreamContent::Fin => "end of stream",
                                };

                                info!(%from, %part.sequence, part.type = %part_type, "Received proposal part");

                                let sequence = part.sequence;

                                // Check if we have a full proposal
                                if let Some(parts) = streams_map.insert(from, part) {

                                    // NOTE(Sam): It seems VERY odd that we don't drop individual stream parts for being too
                                    // old. Why assemble something that might never finish and is known to be stale?

                                    // Check if the proposal is outdated
                                    if parts.height < current_bft_height {
                                        info!(
                                            height = %current_bft_height,
                                            round = %current_bft_round,
                                            part.height = %parts.height,
                                            part.round = %parts.round,
                                            part.sequence = %sequence,
                                            "Received outdated proposal part, ignoring"
                                        );
                                    } else {

                                        // signature verification
                                        {
                                            let mut hasher = sha3::Keccak256::new();

                                            let init = parts.init().unwrap();
                                            let fin = parts.fin().unwrap();

                                            let hash = {
                                                hasher.update(init.height.as_u64().to_be_bytes());
                                                hasher.update(init.round.as_i64().to_be_bytes());

                                                // The correctness of the hash computation relies on the parts being ordered by sequence
                                                // number, which is guaranteed by the `PartStreamsMap`.
                                                for part in parts.parts.iter().filter_map(|part| part.as_data()) {
                                                    hasher.update(part.factor.to_be_bytes());
                                                }

                                                hasher.finalize()
                                            };

                                            // TEMP get the proposers key
                                            let mut proposer_public_key = None;
                                            for peer in config.malachite_peers.iter() {
                                                let (_, _, public_key) =
                                                    rng_private_public_key_from_address(&peer);
                                                let address = Address::from_public_key(&public_key);
                                                if address == parts.proposer {
                                                    proposer_public_key = Some(public_key);
                                                    break;
                                                }
                                            }

                                            // Verify the signature
                                            assert!(my_signing_provider.verify(&hash, &fin.signature, &proposer_public_key.expect("proposer not found")));
                                        }

                                        // Re-assemble the proposal from its parts
                                        let value : ProposedValue::<TestContext> = {
                                            let init = parts.init().unwrap();

                                            let value = parts
                                                .parts
                                                .iter()
                                                .filter_map(|part| part.as_data())
                                                .fold(1, |acc, data| acc * data.factor);

                                            ProposedValue {
                                                height: parts.height,
                                                round: parts.round,
                                                valid_round: init.pol_round,
                                                proposer: parts.proposer,
                                                value: malachitebft_test::Value::new(value),
                                                validity: Validity::Valid,
                                            }
                                        };


                                        info!(
                                            "Storing undecided proposal {} {}",
                                            value.height, value.round
                                        );

                                        prev_bft_values.insert((value.height.as_u64(), value.round.as_i64()), value.clone());


                                        if reply.send(Some(value)).is_err() {
                                            error!("Failed to send ReceivedProposalPart reply");
                                        }
                                    }
                                }
                            },

                            _ => tracing::error!(?msg, "Unhandled message from Malachite"),
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

        let new_bc_final = {
            // partial dup of tfl_final_block_hash
            let internal = internal_handle.internal.lock().await;

            if internal.latest_final_block.is_some() {
                internal.latest_final_block
            } else {
                drop(internal);

                if new_bc_tip != current_bc_tip {
                    //info!("tip changed to {:?}", new_bc_tip);
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
            // info!("final changed to {:?}", new_bc_final);
            if let Some(new_final_height_hash) = new_bc_final {
                let start_hash = if let Some(prev_height_hash) = current_bc_final {
                    prev_height_hash.1
                } else {
                    new_final_height_hash.1
                };

                let (new_final_blocks, infos) = tfl_block_sequence(
                    &call,
                    start_hash,
                    Some(new_final_height_hash),
                    /*include_start_hash*/ true,
                    true,
                )
                .await;

                let mut quiet = true;
                if let (Some(Some(first_block)), Some(Some(last_block))) =
                    (infos.first(), infos.last())
                {
                    let a = first_block.coinbase_height().unwrap_or(BlockHeight(0)).0;
                    let b = last_block.coinbase_height().unwrap_or(BlockHeight(0)).0;
                    if a != b {
                        println!("Height change: {} => {}:", a, b);
                        quiet = false;
                    }
                }
                if quiet == false {
                    tfl_dump_blocks(&new_final_blocks[..], &infos[..]);
                }

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

struct BFTNode {
    private_key: PrivateKey,
}

#[derive(Clone, Copy, Debug)]
struct DummyHandle;

#[async_trait]
impl malachitebft_app_channel::app::node::NodeHandle<TestContext> for DummyHandle {
    fn subscribe(&self) -> RxEvent<TestContext> {
        panic!();
    }

    async fn kill(&self, _reason: Option<String>) -> eyre::Result<()> {
        panic!();
    }
}

static temp_dir_for_wal: std::sync::Mutex<Option<TempDir>> = std::sync::Mutex::new(None);

#[async_trait]
impl malachitebft_app_channel::app::node::Node for BFTNode {
    type Context = TestContext;
    type Genesis = Genesis;
    type PrivateKeyFile = ();
    type SigningProvider = Ed25519Provider;
    type NodeHandle = DummyHandle;

    fn get_home_dir(&self) -> std::path::PathBuf {
        let mut td = temp_dir_for_wal.lock().unwrap();
        if td.is_none() {
            *td = Some(
                TempDir::new(&format!(
                    "aah_very_annoying_that_the_wal_is_required_id_is_{}",
                    rand::random::<u32>()
                ))
                .unwrap(),
            );
        }
        std::path::PathBuf::from(td.as_ref().unwrap().path())
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

    fn load_private_key_file(&self) -> Result<(), eyre::ErrReport> {
        Ok(())
    }

    fn get_signing_provider(&self, private_key: PrivateKey) -> Self::SigningProvider {
        Ed25519Provider::new(private_key)
    }

    fn load_genesis(&self) -> Result<Self::Genesis, eyre::ErrReport> {
        panic!();
    }

    async fn start(&self) -> eyre::Result<DummyHandle> {
        Ok(DummyHandle)
    }

    async fn run(self) -> eyre::Result<()> {
        Ok(())
    }
    type Config = BFTConfig;
    fn load_config(&self) -> eyre::Result<Self::Config> {
        panic!()
    }
}

async fn tfl_service_incoming_request(
    internal_handle: TFLServiceHandle,
    request: TFLServiceRequest,
) -> Result<TFLServiceResponse, TFLServiceError> {
    let call = internal_handle.call.clone();

    // from this point onwards we must race to completion in order to avoid stalling the main thread

    #[allow(unreachable_patterns)]
    match request {
        TFLServiceRequest::IsTFLActivated => Ok(TFLServiceResponse::IsTFLActivated(
            internal_handle.internal.lock().await.tfl_is_activated,
        )),

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

trait SatSubAffine<D> {
    fn sat_sub(&self, d: D) -> Self;
}

/// Saturating subtract: goes to 0 if self < d
impl SatSubAffine<i32> for BlockHeight {
    fn sat_sub(&self, d: i32) -> BlockHeight {
        use std::ops::Sub;
        use zebra_chain::block::HeightDiff as BlockHeightDiff;
        self.sub(BlockHeightDiff::from(d)).unwrap_or(BlockHeight(0))
    }
}

// TODO: handle headers as well?
// NOTE: this is currently best-chain-only due to request/response limitations
// TODO: add more request/response pairs directly in zebra-state's ReadStateService
/// always returns block hashes. If read_extra_info is set, also returns Blocks, otherwise returns an empty vector.
async fn tfl_block_sequence(
    call: &TFLServiceCalls,
    start_hash: BlockHash,
    final_height_hash: Option<(BlockHeight, BlockHash)>,
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
                (Some(height.sat_sub(1)), Some(header.previous_block_hash))
            } else {
                (Some(height), Some(start_hash))
            }
        } else {
            (None, None)
        }
    };
    let (final_height, final_hash) = if let Some((height, hash)) = final_height_hash {
        (Some(height), Some(hash))
    } else if let Ok(ReadStateResponse::Tip(val)) = (call.read_state)(ReadStateRequest::Tip).await {
        val.unzip()
    } else {
        (None, None)
    };

    // check validity //////////////////////////////
    if start_height.is_none() {
        error!(?start_hash, "start_hash has invalid height");
        return (Vec::new(), Vec::new());
    }
    let start_height = start_height.unwrap();
    let init_hash = init_hash.unwrap();

    if final_height.is_none() {
        error!(?final_height, "final_hash has invalid height");
        return (Vec::new(), Vec::new());
    }
    let final_height = final_height.unwrap();

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
            let chunk_start_hash = if chunk.is_empty() {
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

trait HasBlockHash {
    fn get_hash(&self) -> Option<BlockHash>;
}
impl HasBlockHash for BlockHash {
    fn get_hash(&self) -> Option<BlockHash> {
        Some(*self)
    }
}

/// "How many little-endian chars are needed to uniquely identify any of the blocks in the given
/// slice"
fn block_hash_unique_chars_n<T>(hashes: &[T]) -> usize
where
    T: HasBlockHash,
{
    let is_unique = |prefix_len: usize, hashes: &[T]| -> bool {
        let mut prefixes = HashSet::<BlockHash>::with_capacity(hashes.len());

        // NOTE: characters correspond to nibbles
        let bytes_n = prefix_len / 2;
        let is_nib = (prefix_len % 2) != 0;

        for hash in hashes {
            if let Some(hash) = hash.get_hash() {
                let mut subhash = BlockHash([0; 32]);
                subhash.0[..bytes_n].clone_from_slice(&hash.0[..bytes_n]);

                if is_nib {
                    subhash.0[bytes_n] = hash.0[bytes_n] & 0xf;
                }

                if !prefixes.insert(subhash) {
                    return false;
                }
            }
        }

        true
    };

    let mut unique_chars_n: usize = 1;
    while !is_unique(unique_chars_n, hashes) {
        unique_chars_n += 1;
        assert!(unique_chars_n <= 64);
    }

    unique_chars_n
}

fn tfl_dump_blocks(blocks: &[BlockHash], infos: &[Option<Arc<Block>>]) {
    let highlight_chars_n = block_hash_unique_chars_n(blocks);

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
                " - {}, height: {}, work: {:?}, {:3} transactions ({} shielded)",
                block.header.time,
                block.coinbase_height().unwrap_or(BlockHeight(0)).0,
                block.header.difficulty_threshold.to_work().unwrap(),
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
    final_height_hash: Option<(BlockHeight, BlockHash)>,
    include_start_hash: bool,
) {
    let (blocks, infos) = tfl_block_sequence(
        call,
        start_hash,
        final_height_hash,
        include_start_hash,
        true,
    )
    .await;
    tfl_dump_blocks(&blocks[..], &infos[..]);
}

mod strm {
    use std::cmp::Ordering;
    use std::collections::{BTreeMap, BinaryHeap, HashSet};

    use malachitebft_app_channel::app::consensus::PeerId;
    use malachitebft_app_channel::app::streaming::{Sequence, StreamId, StreamMessage};
    use malachitebft_app_channel::app::types::core::Round;
    use malachitebft_test::{Address, Height, ProposalFin, ProposalInit, ProposalPart};

    struct MinSeq<T>(StreamMessage<T>);

    impl<T> PartialEq for MinSeq<T> {
        fn eq(&self, other: &Self) -> bool {
            self.0.sequence == other.0.sequence
        }
    }

    impl<T> Eq for MinSeq<T> {}

    impl<T> Ord for MinSeq<T> {
        fn cmp(&self, other: &Self) -> Ordering {
            other.0.sequence.cmp(&self.0.sequence)
        }
    }

    impl<T> PartialOrd for MinSeq<T> {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    struct MinHeap<T>(BinaryHeap<MinSeq<T>>);

    impl<T> Default for MinHeap<T> {
        fn default() -> Self {
            Self(BinaryHeap::new())
        }
    }

    impl<T> MinHeap<T> {
        fn push(&mut self, msg: StreamMessage<T>) {
            self.0.push(MinSeq(msg));
        }

        fn len(&self) -> usize {
            self.0.len()
        }

        fn drain(&mut self) -> Vec<T> {
            let mut vec = Vec::with_capacity(self.0.len());
            while let Some(MinSeq(msg)) = self.0.pop() {
                if let Some(data) = msg.content.into_data() {
                    vec.push(data);
                }
            }
            vec
        }
    }

    #[derive(Default)]
    struct StreamState {
        buffer: MinHeap<ProposalPart>,
        init_info: Option<ProposalInit>,
        seen_sequences: HashSet<Sequence>,
        total_messages: usize,
        fin_received: bool,
    }

    impl StreamState {
        fn is_done(&self) -> bool {
            self.init_info.is_some()
                && self.fin_received
                && self.buffer.len() == self.total_messages
        }

        fn insert(&mut self, msg: StreamMessage<ProposalPart>) -> Option<ProposalParts> {
            if msg.is_first() {
                self.init_info = msg.content.as_data().and_then(|p| p.as_init()).cloned();
            }

            if msg.is_fin() {
                self.fin_received = true;
                self.total_messages = msg.sequence as usize + 1;
            }

            self.buffer.push(msg);

            if self.is_done() {
                let init_info = self.init_info.take()?;

                Some(ProposalParts {
                    height: init_info.height,
                    round: init_info.round,
                    proposer: init_info.proposer,
                    parts: self.buffer.drain(),
                })
            } else {
                None
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ProposalParts {
        pub height: Height,
        pub round: Round,
        pub proposer: Address,
        pub parts: Vec<ProposalPart>,
    }

    impl ProposalParts {
        pub fn init(&self) -> Option<&ProposalInit> {
            self.parts.iter().find_map(|p| p.as_init())
        }

        pub fn fin(&self) -> Option<&ProposalFin> {
            self.parts.iter().find_map(|p| p.as_fin())
        }
    }

    #[derive(Default)]
    pub struct PartStreamsMap {
        streams: BTreeMap<(PeerId, StreamId), StreamState>,
    }

    impl PartStreamsMap {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn insert(
            &mut self,
            peer_id: PeerId,
            msg: StreamMessage<ProposalPart>,
        ) -> Option<ProposalParts> {
            let stream_id = msg.stream_id.clone();
            let state = self
                .streams
                .entry((peer_id, stream_id.clone()))
                .or_default();

            if !state.seen_sequences.insert(msg.sequence) {
                // We have already seen a message with this sequence number.
                return None;
            }

            let result = state.insert(msg);

            if state.is_done() {
                self.streams.remove(&(peer_id, stream_id));
            }

            result
        }
    }
}

/// Malachite configuration options
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BFTConfig {
    /// A custom human-readable name for this node
    pub moniker: String,

    /// Log configuration options
    pub logging: mconfig::LoggingConfig,

    /// Consensus configuration options
    pub consensus: mconfig::ConsensusConfig,

    /// ValueSync configuration options
    pub value_sync: mconfig::ValueSyncConfig,

    /// Metrics configuration options
    pub metrics: mconfig::MetricsConfig,

    /// Runtime configuration options
    pub runtime: mconfig::RuntimeConfig,
}

impl NodeConfig for BFTConfig {
    fn moniker(&self) -> &str {
        &self.moniker
    }

    fn consensus(&self) -> &mconfig::ConsensusConfig {
        &self.consensus
    }

    fn value_sync(&self) -> &mconfig::ValueSyncConfig {
        &self.value_sync
    }
}
