//! Internal Zebra service for managing the Crosslink consensus protocol

#![allow(clippy::print_stdout)]
#![allow(unexpected_cfgs, unused, missing_docs)]

use color_eyre::install;

use async_trait::async_trait;
use strum::{EnumCount, IntoEnumIterator};
use strum_macros::EnumIter;

use zebra_chain::serialization::{ZcashDeserializeInto, ZcashSerialize};

use multiaddr::Multiaddr;
use rand::{CryptoRng, RngCore};
use rand::{Rng, SeedableRng};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hasher};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio::time::Instant;
use tracing::{error, info, warn};

use bytes::{Bytes, BytesMut};

pub mod malctx;
use malctx::*;
pub mod chain;
use chain::*;
pub mod mal_system;
use mal_system::*;

use std::sync::Mutex;
use tokio::sync::Mutex as TokioMutex;

pub static TEST_INSTR_I: Mutex<usize> = Mutex::new(0);
pub static TEST_MODE: Mutex<bool> = Mutex::new(false);
pub static TEST_FAILED: Mutex<i32> = Mutex::new(0);
pub static TEST_INSTR_PATH: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);
pub static TEST_INSTR_BYTES: Mutex<Vec<u8>> = Mutex::new(Vec::new());
pub static TEST_INSTRS: Mutex<Vec<test_format::TFInstr>> = Mutex::new(Vec::new());
pub static TEST_SHUTDOWN_FN: Mutex<fn()> = Mutex::new(|| ());
pub static TEST_PARAMS: Mutex<Option<ZcashCrosslinkParameters>> = Mutex::new(None);
pub static TEST_NAME: Mutex<&'static str> = Mutex::new("‰‰TEST_NAME_NOT_SET‰‰");

pub mod service;
/// Configuration for the state service.
pub mod config {
    use serde::{Deserialize, Serialize};

    /// Configuration for the state service.
    #[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
    #[serde(deny_unknown_fields, default)]
    pub struct Config {
        /// Local address, e.g. "/ip4/0.0.0.0/udp/24834/quic-v1"
        pub listen_address: Option<String>,
        /// Public address for this node, e.g. "/ip4/127.0.0.1/udp/24834/quic-v1" if testing
        /// internally, or the public IP address if using externally.
        pub public_address: Option<String>,
        /// List of public IP addresses for peers, in the same format as `public_address`.
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

pub mod test_format;
#[cfg(feature = "viz_gui")]
pub mod viz;

use crate::service::{
    TFLServiceCalls, TFLServiceError, TFLServiceHandle, TFLServiceRequest, TFLServiceResponse,
};

// TODO: do we want to start differentiating BCHeight/PoWHeight, MalHeight/PoSHeigh etc?
use zebra_chain::block::{
    Block, CountedHeader, Hash as BlockHash, Header as BlockHeader, Height as BlockHeight,
};
use zebra_state::{ReadRequest as ReadStateRequest, ReadResponse as ReadStateResponse};

/// Placeholder activation height for Crosslink functionality
pub const TFL_ACTIVATION_HEIGHT: BlockHeight = BlockHeight(2000);

#[derive(Debug, Copy, Clone, EnumCount, EnumIter)]
enum BFTMsgFlag {
    ConsensusReady,
    StartedRound,
    GetValue,
    ProcessSyncedValue,
    GetValidatorSet,
    Decided,
    GetHistoryMinHeight,
    GetDecidedValue,
    ExtendVote,
    VerifyVoteExtension,
    ReceivedProposalPart,
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct TFLServiceInternal {
    latest_final_block: Option<(BlockHeight, BlockHash)>,
    tfl_is_activated: bool,

    stakers: Vec<TFLStaker>,

    // channels
    final_change_tx: broadcast::Sender<BlockHash>,

    bft_msg_flags: u64, // ALT: Vec of messages, Vec/flags of success/failure
    bft_blocks: Vec<BftBlock>,
    fat_pointer_to_tip: FatPointerToBftBlock,
    proposed_bft_string: Option<String>,

    malachite_watchdog: Instant,
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
    internal_handle: &TFLServiceHandle,
) -> Option<(BlockHeight, BlockHash)> {
    let mut internal = internal_handle.internal.lock().await;
    tfl_final_block_height_hash_pre_locked(internal_handle, &mut internal).await
}

async fn tfl_final_block_height_hash_pre_locked(
    internal_handle: &TFLServiceHandle,
    internal: &mut TFLServiceInternal,
) -> Option<(BlockHeight, BlockHash)> {
    #[allow(unused_mut)]
    if internal.latest_final_block.is_some() {
        internal.latest_final_block
    } else {
        tfl_reorg_final_block_height_hash(&internal_handle.call).await
    }
}

fn rng_private_public_key_from_address(
    addr: &str,
) -> (rand::rngs::StdRng, MalPrivateKey, MalPublicKey) {
    let mut hasher = DefaultHasher::new();
    hasher.write(addr.as_bytes());
    let seed = hasher.finish();
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let private_key = MalPrivateKey::new(&mut rng);
    let public_key = (&private_key).into();
    (rng, private_key, public_key)
}

async fn push_new_bft_msg_flags(tfl_handle: &TFLServiceHandle, bft_msg_flags: u64) {
    let mut internal = tfl_handle.internal.lock().await;
    internal.bft_msg_flags |= bft_msg_flags;
}

async fn propose_new_bft_block(
    tfl_handle: &TFLServiceHandle,
    my_public_key: &MalPublicKey,
) -> Option<BftBlock> {
    let call = tfl_handle.call.clone();
    let params = &PROTOTYPE_PARAMETERS;
    let (tip_height, tip_hash) =
        if let Ok(ReadStateResponse::Tip(val)) = (call.read_state)(ReadStateRequest::Tip).await {
            if val.is_none() {
                return None;
            }
            val.unwrap()
        } else {
            return None;
        };

    use std::ops::Sub;
    use zebra_chain::block::HeightDiff as BlockHeightDiff;

    let finality_candidate_height = tip_height.sub(BlockHeightDiff::from(
        params.bc_confirmation_depth_sigma as i64,
    ));

    let finality_candidate_height = if let Some(h) = finality_candidate_height {
        h
    } else {
        info!(
            "not enough blocks to enforce finality; tip height: {}",
            tip_height.0
        );
        return None;
    };

    let (latest_final_block, latest_bft_block_hash) = {
        let internal = tfl_handle.internal.lock().await;
        (
            internal.latest_final_block,
            internal
                .bft_blocks
                .last()
                .map_or(Blake3Hash([0u8; 32]), |b| b.blake3_hash()),
        )
    };
    let is_improved_final =
        latest_final_block.is_none() || finality_candidate_height > latest_final_block.unwrap().0;

    if !is_improved_final {
        info!(
            "candidate block can't be final: height {}, final height: {:?}",
            finality_candidate_height.0, latest_final_block
        );
        return None;
    }

    let resp = (call.read_state)(ReadStateRequest::BlockHeader(
        finality_candidate_height.into(),
    ))
    .await;

    let candidate_hash = if let Ok(ReadStateResponse::BlockHeader { hash, .. }) = resp {
        hash
    } else {
        // Error or unexpected response type:
        panic!("TODO: improve error handling.");
        return None;
    };

    // NOTE: probably faster to request 2x as many blocks as we need rather than have another async call
    let resp = (call.read_state)(ReadStateRequest::FindBlockHeaders {
        known_blocks: vec![candidate_hash],
        stop: None,
    })
    .await;

    let headers: Vec<BlockHeader> = if let Ok(ReadStateResponse::BlockHeaders(hdrs)) = resp {
        // TODO: do we want these in chain order or "walk-back order"
        hdrs.into_iter()
            .map(|ch| Arc::unwrap_or_clone(ch.header))
            .collect()
    } else {
        // Error or unexpected response type:
        panic!("TODO: improve error handling.");
    };

    let mut internal = tfl_handle.internal.lock().await;

    match BftBlock::try_from(
        params,
        internal.bft_blocks.len() as u32 + 1,
        internal.fat_pointer_to_tip.clone(),
        0,
        headers,
    ) {
        Ok(v) => {
            return Some(v);
        }
        Err(e) => {
            warn!("Unable to create BftBlock to propose, Error={:?}", e,);
            return None;
        }
    };
}

async fn new_decided_bft_block_from_malachite(
    tfl_handle: &TFLServiceHandle,
    new_block: &BftBlock,
    fat_pointer: &FatPointerToBftBlock,
) -> bool {
    let call = tfl_handle.call.clone();
    let params = &PROTOTYPE_PARAMETERS;

    let mut internal = tfl_handle.internal.lock().await;

    if fat_pointer.points_at_block_hash() != new_block.blake3_hash() {
        warn!("Fat Pointer hash does not match block hash.");
        return false;
    }
    // TODO: check public keys on the fat pointer against the roster
    if fat_pointer.validate_signatures() == false {
        warn!("Signatures are not valid. Rejecting block.");
        return false;
    }

    if validate_bft_block_from_malachite_already_locked(&tfl_handle, &mut internal, new_block).await
        == false
    {
        return false;
    }

    let new_final_hash = new_block.headers.first().expect("at least 1 header").hash();
    let new_final_height = block_height_from_hash(&call, new_final_hash).await.unwrap();
    // assert_eq!(new_final_height.0, new_block.finalization_candidate_height);
    let insert_i = new_block.height as usize - 1;

    // HACK: ensure there are enough blocks to overwrite this at the correct index
    for i in internal.bft_blocks.len()..=insert_i {
        let parent_i = i.saturating_sub(1); // just a simple chain
        internal.bft_blocks.push(BftBlock {
            version: 0,
            height: i as u32,
            previous_block_fat_ptr: FatPointerToBftBlock {
                vote_for_block_without_finalizer_public_key: [0u8; 76 - 32],
                signatures: Vec::new(),
            },
            finalization_candidate_height: 0,
            headers: Vec::new(),
        });
    }

    if insert_i > 0 {
        assert_eq!(
            internal.bft_blocks[insert_i - 1].blake3_hash(),
            new_block.previous_block_fat_ptr.points_at_block_hash()
        );
    }
    assert!(insert_i == 0 || new_block.previous_block_hash() != Blake3Hash([0u8; 32]));
    assert!(
        internal.bft_blocks[insert_i].headers.is_empty(),
        "{:?}",
        internal.bft_blocks[insert_i]
    );
    assert!(!new_block.headers.is_empty());
    // info!("Inserting bft block at {} with hash {}", insert_i, new_block.blake3_hash());
    internal.bft_blocks[insert_i] = new_block.clone();
    internal.fat_pointer_to_tip = fat_pointer.clone();
    internal.latest_final_block = Some((new_final_height, new_final_hash));
    // internal.malachite_watchdog = Instant::now(); // NOTE(Sam): We don't reset the watchdog in order to be always testing it for now.
    true
}

async fn validate_bft_block_from_malachite(
    tfl_handle: &TFLServiceHandle,
    new_block: &BftBlock,
) -> bool {
    let mut internal = tfl_handle.internal.lock().await;
    validate_bft_block_from_malachite_already_locked(tfl_handle, &mut internal, new_block).await
}
async fn validate_bft_block_from_malachite_already_locked(
    tfl_handle: &TFLServiceHandle,
    internal: &mut TFLServiceInternal,
    new_block: &BftBlock,
) -> bool {
    let call = tfl_handle.call.clone();
    let params = &PROTOTYPE_PARAMETERS;

    if new_block.previous_block_fat_ptr.points_at_block_hash()
        != internal.fat_pointer_to_tip.points_at_block_hash()
    {
        warn!(
            "Block has invalid previous block fat pointer hash: was {} but should be {}",
            new_block.previous_block_fat_ptr.points_at_block_hash(),
            internal.fat_pointer_to_tip.points_at_block_hash(),
        );
        return false;
    }

    let new_final_hash = new_block.headers.first().expect("at least 1 header").hash();
    let new_final_pow_height =
        if let Some(new_final_height) = block_height_from_hash(&call, new_final_hash).await {
            new_final_height.0
        } else {
            warn!(
                "Didn't have hash available for confirmation: {}",
                new_final_hash
            );
            return false;
        };
    true
}

fn fat_pointer_to_block_at_height(
    bft_blocks: &[BftBlock],
    fat_pointer_to_tip: &FatPointerToBftBlock,
    at_height: u64,
) -> Option<FatPointerToBftBlock> {
    if at_height == 0 || at_height as usize - 1 >= bft_blocks.len() {
        return None;
    }

    if at_height as usize == bft_blocks.len() {
        Some(fat_pointer_to_tip.clone())
    } else {
        Some(
            bft_blocks[at_height as usize]
                .previous_block_fat_ptr
                .clone(),
        )
    }
}

async fn get_historical_bft_block_at_height(
    tfl_handle: &TFLServiceHandle,
    at_height: u64,
) -> Option<(BftBlock, FatPointerToBftBlock)> {
    let mut internal = tfl_handle.internal.lock().await;
    if at_height == 0 || at_height as usize - 1 >= internal.bft_blocks.len() {
        return None;
    }
    let block = internal.bft_blocks[at_height as usize - 1].clone();
    Some((
        block,
        fat_pointer_to_block_at_height(
            &internal.bft_blocks,
            &internal.fat_pointer_to_tip,
            at_height,
        )
        .unwrap(),
    ))
}

const MAIN_LOOP_SLEEP_INTERVAL: Duration = Duration::from_millis(125);
const MAIN_LOOP_INFO_DUMP_INTERVAL: Duration = Duration::from_millis(8000);

async fn tfl_service_main_loop(internal_handle: TFLServiceHandle) -> Result<(), String> {
    let call = internal_handle.call.clone();
    let config = internal_handle.config.clone();
    let params = &PROTOTYPE_PARAMETERS;

    #[cfg(feature = "viz_gui")]
    {
        let rt = tokio::runtime::Handle::current();
        let viz_tfl_handle = internal_handle.clone();
        tokio::task::spawn_blocking(move || {
            rt.block_on(viz::service_viz_requests(viz_tfl_handle, params))
        });
    }

    if *TEST_MODE.lock().unwrap() {
        // ensure that tests fail on panic/assert(false); otherwise tokio swallows them
        std::panic::set_hook(Box::new(|panic_info| {
            #[allow(clippy::print_stderr)]
            {
                *TEST_FAILED.lock().unwrap() = -1;

                use std::backtrace::{self, *};
                let bt = Backtrace::force_capture();

                eprintln!("\n\n{panic_info}\n");

                // hacky formatting - BacktraceFmt not working for some reason...
                let str = format!("{bt}");
                let splits: Vec<_> = str.split("\n").collect();

                // skip over the internal backtrace unwind steps
                let mut start_i = 0;
                let mut i = 0;
                while i < splits.len() {
                    if splits[i].ends_with("rust_begin_unwind") {
                        i += 1;
                        if i < splits.len() && splits[i].trim().starts_with("at ") {
                            i += 1;
                        }
                        start_i = i;
                    }
                    if splits[i].ends_with("core::panicking::panic_fmt") {
                        i += 1;
                        if i < splits.len() && splits[i].trim().starts_with("at ") {
                            i += 1;
                        }
                        start_i = i;
                        break;
                    }
                    i += 1;
                }

                // print backtrace
                let mut i = start_i;
                let n = 80;
                while i < n {
                    let proc = if let Some(val) = splits.get(i) {
                        val.trim()
                    } else {
                        break;
                    };
                    i += 1;

                    let file_loc = if let Some(val) = splits.get(i) {
                        let val = val.trim();
                        if val.starts_with("at ") {
                            i += 1;
                            val
                        } else {
                            ""
                        }
                    } else {
                        break;
                    };

                    eprintln!(
                        "  {}{}    {}",
                        if i < 20 { " " } else { "" },
                        proc,
                        file_loc
                    );
                }
                if i == n {
                    eprintln!("...");
                }

                eprintln!("\n\nInstruction sequence:");
                let failed_instr_i = *TEST_INSTR_I.lock().unwrap();
                let instrs_lock = TEST_INSTRS.lock().unwrap();
                let instrs: &Vec<test_format::TFInstr> = instrs_lock.as_ref();
                let bytes_lock = TEST_INSTR_BYTES.lock().unwrap();
                let bytes = bytes_lock.as_ref();
                for instr_i in 0..instrs.len() {
                    let col = if instr_i < failed_instr_i {
                        "\x1b[92m" // green
                    } else if instr_i == failed_instr_i {
                        "\x1b[91m" // red
                    } else {
                        "\x1b[37m" // grey
                    };
                    eprintln!(
                        "  {}{}\x1b[0;0m",
                        col,
                        &test_format::TFInstr::string_from_instr(bytes, &instrs[instr_i])
                    );
                }

                #[cfg(not(feature = "viz_gui"))]
                std::process::abort();
            }
        }));

        tokio::task::spawn(test_format::instr_reader(internal_handle.clone()));
    }

    let public_ip_string = config
        .public_address
        .unwrap_or(String::from_str("/ip4/127.0.0.1/udp/45869/quic-v1").unwrap());

    let (mut rng, my_private_key, my_public_key) =
        rng_private_public_key_from_address(&public_ip_string);

    let initial_validator_set = {
        let mut array = Vec::with_capacity(config.malachite_peers.len());

        for peer in config.malachite_peers.iter() {
            let (_, _, public_key) = rng_private_public_key_from_address(peer);
            array.push(MalValidator::new(public_key, 1));
        }

        if array.is_empty() {
            let (_, _, public_key) = rng_private_public_key_from_address(&public_ip_string);
            array.push(MalValidator::new(public_key, 1));
        }

        MalValidatorSet::new(array)
    };

    let my_signing_provider = MalEd25519Provider::new(my_private_key.clone());
    let ctx = MalContext {};

    let codec = MalProtobufCodec;

    let init_bft_height = MalHeight::new(1);

    // let mut bft_config = config::load_config(std::path::Path::new("C:\\Users\\azmre\\.malachite\\config\\config.toml"), None)
    //     .expect("Failed to load configuration file");
    let mut bft_config: BFTConfig = Default::default(); // TODO: read from file?

    bft_config.logging.log_level = crate::mconfig::LogLevel::Error;

    for peer in config.malachite_peers.iter() {
        bft_config
            .consensus
            .p2p
            .persistent_peers
            .push(Multiaddr::from_str(&peer).unwrap());
    }
    if bft_config.consensus.p2p.persistent_peers.is_empty() {
        bft_config
            .consensus
            .p2p
            .persistent_peers
            .push(Multiaddr::from_str(&public_ip_string).unwrap());
    }
    bft_config.consensus.p2p.persistent_peers.remove(
        bft_config
            .consensus
            .p2p
            .persistent_peers
            .iter()
            .position(|x| *x == Multiaddr::from_str(&public_ip_string).unwrap())
            .unwrap(),
    );

    //bft_config.consensus.p2p.transport = mconfig::TransportProtocol::Quic;
    if let Some(listen_addr) = config.listen_address {
        bft_config.consensus.p2p.listen_addr = Multiaddr::from_str(&listen_addr).unwrap();
    } else {
        bft_config.consensus.p2p.listen_addr = Multiaddr::from_str(&format!(
            "/ip4/127.0.0.1/tcp/{}",
            45869 + rand::random::<u32>() % 1000
        ))
        .unwrap();
    }
    bft_config.consensus.p2p.discovery = mconfig::DiscoveryConfig {
        selector: mconfig::Selector::Random,
        bootstrap_protocol: mconfig::BootstrapProtocol::Full,
        num_outbound_peers: 10,
        num_inbound_peers: 30,
        ephemeral_connection_timeout: Duration::from_secs(10),
        enabled: true,
    };

    info!(?bft_config);

    let mut malachite_system = None;
    if !*TEST_MODE.lock().unwrap() {
        malachite_system = Some(
            start_malachite(
                internal_handle.clone(),
                1,
                initial_validator_set.validators.clone(),
                my_private_key,
                bft_config.clone(),
            )
            .await,
        );
    }

    let mut run_instant = Instant::now();
    let mut last_diagnostic_print = Instant::now();
    let mut current_bc_tip: Option<(BlockHeight, BlockHash)> = None;
    let mut current_bc_final: Option<(BlockHeight, BlockHash)> = None;

    {
        let mut internal = internal_handle.internal.lock().await;
        internal.malachite_watchdog = Instant::now();
    }

    loop {
        // Calculate this prior to message handling so that handlers can use it:
        let new_bc_tip = if let Ok(ReadStateResponse::Tip(val)) =
            (call.read_state)(ReadStateRequest::Tip).await
        {
            val
        } else {
            None
        };

        tokio::time::sleep_until(run_instant).await;
        run_instant += MAIN_LOOP_SLEEP_INTERVAL;

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

        if malachite_system.is_some() && internal.malachite_watchdog.elapsed().as_secs() > 120 {
            error!("Malachite Watchdog triggered, restarting subsystem...");
            let start_delay = Duration::from_secs(rand::rngs::OsRng.next_u64() % 10 + 20);
            malachite_system = Some(
                start_malachite_with_start_delay(
                    internal_handle.clone(),
                    internal.bft_blocks.len() as u64 + 1, // The next block malachite should produce
                    initial_validator_set.validators.clone(),
                    my_private_key,
                    bft_config.clone(),
                    start_delay,
                )
                .await,
            );
            internal.malachite_watchdog = Instant::now() + start_delay;
        }

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
                        // Note(Sam), very noisy and not connected to malachite right now.
                        // println!("Height change: {} => {}:", a, b);
                        // quiet = false;
                    }
                }
                if !quiet {
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
    private_key: MalPrivateKey,
}

#[derive(Clone, Copy, Debug)]
struct DummyHandle;

#[async_trait]
impl malachitebft_app_channel::app::node::NodeHandle<MalContext> for DummyHandle {
    fn subscribe(&self) -> RxEvent<MalContext> {
        panic!();
    }

    async fn kill(&self, _reason: Option<String>) -> eyre::Result<()> {
        panic!();
    }
}

static TEMP_DIR_FOR_WAL: std::sync::Mutex<Option<TempDir>> = std::sync::Mutex::new(None);

#[async_trait]
impl malachitebft_app_channel::app::node::Node for BFTNode {
    type Context = MalContext;
    type Genesis = ();
    type PrivateKeyFile = ();
    type SigningProvider = MalEd25519Provider;
    type NodeHandle = DummyHandle;

    fn get_home_dir(&self) -> std::path::PathBuf {
        let mut td = TEMP_DIR_FOR_WAL.lock().unwrap();
        std::path::PathBuf::from(td.as_ref().unwrap().path())
    }

    fn get_address(&self, pk: &MalPublicKey) -> MalPublicKey2 {
        MalPublicKey2(pk.clone())
    }

    fn get_public_key(&self, pk: &MalPrivateKey) -> MalPublicKey {
        pk.into()
    }

    fn get_keypair(&self, pk: MalPrivateKey) -> MalKeyPair {
        MalKeyPair::ed25519_from_bytes(pk.as_ref().to_vec()).unwrap()
    }

    fn load_private_key(&self, _file: ()) -> MalPrivateKey {
        self.private_key.clone()
    }

    fn load_private_key_file(&self) -> Result<(), eyre::ErrReport> {
        Ok(())
    }

    fn get_signing_provider(&self, private_key: MalPrivateKey) -> Self::SigningProvider {
        MalEd25519Provider::new(private_key)
    }

    fn load_genesis(&self) -> Result<Self::Genesis, eyre::ErrReport> {
        Ok(())
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
            if let Some((_, hash)) = tfl_final_block_height_hash(&internal_handle).await {
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
                    match tfl_final_block_height_hash(&internal_handle).await {
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
                    match tfl_final_block_height_hash(&internal_handle).await {
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
