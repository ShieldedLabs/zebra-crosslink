//! Internal Zebra service for managing the Crosslink consensus protocol

#![allow(clippy::print_stdout)]
#![allow(unexpected_cfgs, unused, missing_docs)]

#[macro_use]
extern crate lazy_static;

use color_eyre::install;

use async_trait::async_trait;
use strum::{EnumCount, IntoEnumIterator};
use strum_macros::{EnumCount, EnumIter};

use tenderlink::SortedRosterMember;
use zcash_primitives::transaction::{StakingAction, StakingActionKind};
use zebra_chain::serialization::{
    SerializationError, ZcashDeserialize, ZcashDeserializeInto, ZcashSerialize,
};
use zebra_state::crosslink::*;

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

use ed25519_zebra::SigningKey as MalPrivateKey;
use ed25519_zebra::VerificationKeyBytes as MalPublicKey;

pub mod chain;
use chain::*;

use std::sync::Mutex;
use tokio::sync::Mutex as TokioMutex;

pub static TEST_INSTR_C: Mutex<usize> = Mutex::new(0);
pub static TEST_MODE: Mutex<bool> = Mutex::new(false);
pub static TEST_FAILED: Mutex<i32> = Mutex::new(0);
pub static TEST_FAILED_INSTR_IDXS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
pub static TEST_CHECK_ASSERT: Mutex<bool> = Mutex::new(false);
pub static TEST_INSTR_PATH: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);
pub static TEST_INSTR_BYTES: Mutex<Vec<u8>> = Mutex::new(Vec::new());
pub static TEST_INSTRS: Mutex<Vec<test_format::TFInstr>> = Mutex::new(Vec::new());
pub static TEST_SHUTDOWN_FN: Mutex<fn()> = Mutex::new(|| ());
pub static TEST_PARAMS: Mutex<Option<ZcashCrosslinkParameters>> = Mutex::new(None);
pub static TEST_NAME: Mutex<&'static str> = Mutex::new("‰‰TEST_NAME_NOT_SET‰‰");

pub fn dump_test_instrs() {
    #![allow(clippy::print_stderr)]

    let failed_instr_idxs_lock = TEST_FAILED_INSTR_IDXS.lock();
    let failed_instr_idxs = failed_instr_idxs_lock.as_ref().unwrap();
    if failed_instr_idxs.is_empty() {
        eprintln!(
            "no failed instructions recorded. We should have at least 1 failed instruction here"
        );
    }

    let done_instr_c = *TEST_INSTR_C.lock().unwrap();

    let mut failed_instr_idx_i = 0;
    let instrs_lock = TEST_INSTRS.lock().unwrap();
    let instrs: &Vec<test_format::TFInstr> = instrs_lock.as_ref();
    let bytes_lock = TEST_INSTR_BYTES.lock().unwrap();
    let bytes = bytes_lock.as_ref();
    for instr_i in 0..instrs.len() {
        let col = if failed_instr_idx_i < failed_instr_idxs.len()
            && instr_i == failed_instr_idxs[failed_instr_idx_i]
        {
            failed_instr_idx_i += 1;
            "\x1b[91m F  " // red
        } else if instr_i < done_instr_c {
            "\x1b[92m P  " // green
        } else {
            "\x1b[37m    " // grey
        };
        eprintln!(
            "  {}{}\x1b[0;0m",
            col,
            &test_format::TFInstr::string_from_instr(bytes, &instrs[instr_i])
        );
    }
}

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
        /// temp seed for private/public key pair
        pub insecure_user_name: Option<String>,
        /// List of public IP addresses for peers, in the same format as `public_address`.
        pub malachite_peers: Vec<String>,
        /// Do not manipulate config
        pub do_not_manipulate_config: bool,
    }
    impl Default for Config {
        fn default() -> Self {
            Self {
                listen_address: None,
                public_address: None,
                insecure_user_name: None,
                malachite_peers: Vec::new(),
                do_not_manipulate_config: false,
            }
        }
    }
}

pub mod test_format;
#[cfg(feature = "viz_gui")]
pub mod viz;

pub mod viz2;

use crate::service::{TFLServiceCalls, TFLServiceHandle};

// TODO: do we want to start differentiating BCHeight/PoWHeight, MalHeight/PoSHeigh etc?
use zebra_chain::block::{
    Block, CountedHeader, Hash as BlockHash, Header as BlockHeader, Height as BlockHeight,
};
use zebra_node_services::mempool::{Request as MempoolRequest, Response as MempoolResponse};
use zebra_state::{crosslink::*, Request as StateRequest, Response as StateResponse};

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
    my_public_key: MalPublicKey,
    latest_final_block: Option<(BlockHeight, BlockHash)>,
    tfl_is_activated: bool,

    // channels
    final_change_tx: broadcast::Sender<(BlockHeight, BlockHash)>,

    bft_msg_flags: u64, // ALT: Vec of messages/combine flags
    bft_err_flags: u64,
    bft_blocks: Vec<BftBlock>,
    fat_pointer_to_tip: FatPointerToBftBlock2,
    our_set_bft_string: Option<String>,
    active_bft_string: Option<String>,

    // TODO: 2 versions of this: ever-added (in sequence) & currently non-0
    validators_keys_to_names: HashMap<MalPublicKey, String>,
    validators_at_current_height: Vec<MalValidator>,

    current_bc_final: Option<(BlockHeight, BlockHash)>,
}

// TODO: Result?
async fn block_height_from_hash(call: &TFLServiceCalls, hash: BlockHash) -> Option<BlockHeight> {
    if let Ok(StateResponse::KnownBlock(Some(known_block))) =
        (call.state)(StateRequest::KnownBlock(hash.into())).await
    {
        Some(known_block.height)
    } else {
        None
    }
}

async fn block_height_hash_from_hash(
    call: &TFLServiceCalls,
    hash: BlockHash,
) -> Option<(BlockHeight, BlockHash)> {
    if let Ok(StateResponse::BlockHeader {
        height,
        hash: check_hash,
        ..
    }) = (call.state)(StateRequest::BlockHeader(hash.into())).await
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
    if let Ok(StateResponse::BlockHeader { header, .. }) =
        (call.state)(StateRequest::BlockHeader(hash.into())).await
    {
        Some(header)
    } else {
        None
    }
}

async fn _block_prev_hash_from_hash(call: &TFLServiceCalls, hash: BlockHash) -> Option<BlockHash> {
    if let Ok(StateResponse::BlockHeader { header, .. }) =
        (call.state)(StateRequest::BlockHeader(hash.into())).await
    {
        Some(header.previous_block_hash)
    } else {
        None
    }
}

async fn tfl_reorg_final_block_height_hash(
    call: &TFLServiceCalls,
) -> Option<(BlockHeight, BlockHash)> {
    let locator = (call.state)(StateRequest::BlockLocator).await;

    // NOTE: although this is a vector, the docs say it may skip some blocks
    // so we can't just `.get(MAX_BLOCK_REORG_HEIGHT)`
    if let Ok(StateResponse::BlockLocator(hashes)) = locator {
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
                    let final_block_req = StateRequest::BlockHeader(pre_reorg_height.into());
                    let final_block_hdr = (call.state)(final_block_req).await;

                    if let Ok(StateResponse::BlockHeader { height, hash, .. }) = final_block_hdr
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
                    let final_block_req = StateRequest::BlockHeader(pre_reorg_height.into());
                    let final_block_hdr = (call.state)(final_block_req).await;

                    if let Ok(StateResponse::BlockHeader { height, hash, .. }) = final_block_hdr
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

pub fn rng_private_public_key_from_address(
    addr: &[u8],
) -> (rand::rngs::StdRng, MalPrivateKey, MalPublicKey) {
    let mut hasher = DefaultHasher::new();
    hasher.write(addr);
    let seed = hasher.finish();
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let private_key = MalPrivateKey::new(&mut rng);
    let public_key = (&private_key).into();
    (rng, private_key, public_key)
}

async fn push_new_bft_msg_flags(
    tfl_handle: &TFLServiceHandle,
    bft_msg_flags: u64,
    bft_err_flags: u64,
) {
    let mut internal = tfl_handle.internal.lock().await;
    internal.bft_msg_flags |= bft_msg_flags;
    internal.bft_err_flags |= bft_err_flags;
}

async fn propose_new_bft_block(tfl_handle: &TFLServiceHandle) -> Option<BftBlock> {
    #[cfg(feature = "viz_gui")]
    if let Some(state) = viz::VIZ_G.lock().unwrap().as_ref() {
        if state.bft_pause_button {
            return None;
        }
    }

    let call = tfl_handle.call.clone();
    let params = &PROTOTYPE_PARAMETERS;
    let (tip_height, tip_hash) =
        if let Ok(StateResponse::Tip(val)) = (call.state)(StateRequest::Tip).await {
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

    let resp = (call.state)(StateRequest::BlockHeader(finality_candidate_height.into())).await;

    let candidate_hash = if let Ok(StateResponse::BlockHeader { hash, .. }) = resp {
        hash
    } else {
        // Error or unexpected response type:
        panic!("TODO: improve error handling.");
        return None;
    };

    // NOTE: probably faster to request 2x as many blocks as we need rather than have another async call
    let resp = (call.state)(StateRequest::FindBlockHeaders {
        known_blocks: vec![candidate_hash],
        stop: None,
    })
    .await;

    let headers: Vec<BlockHeader> = if let Ok(StateResponse::BlockHeaders(hdrs)) = resp {
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
        Ok(v) => Some(v),
        Err(e) => {
            warn!("Unable to create BftBlock to propose, Error={:?}", e,);
            None
        }
    }
}

async fn malachite_wants_to_know_what_the_current_validator_set_is(
    tfl_handle: &TFLServiceHandle,
) -> Vec<MalValidator> {
    let internal = tfl_handle.internal.lock().await;

    let mut return_validator_list_because_of_malachite_bug =
        internal.validators_at_current_height.clone();
    if return_validator_list_because_of_malachite_bug
        .iter()
        .position(|v| v.address.0 == internal.my_public_key)
        .is_none()
    {
        return_validator_list_because_of_malachite_bug.push(MalValidator {
            address: MalPublicKey2(internal.my_public_key),
            public_key: internal.my_public_key,
            voting_power: 0,
        });
    }
    let finalizers = return_validator_list_because_of_malachite_bug;
    if true {
        let mut total_voting_power = 0;
        let mut non_0_members = 0;
        for finalizer_i in 0..finalizers.len() {
            let finalizer = &finalizers[finalizer_i];
            if finalizer.voting_power == 0 {
                continue;
            }

            non_0_members += 1;
            total_voting_power += finalizer.voting_power;
        }

        info!(
            "Giving malachite roster with {} voting power between {} non-0 members",
            total_voting_power, non_0_members
        );
    }

    finalizers
}

async fn new_decided_bft_block_from_malachite(
    tfl_handle: &TFLServiceHandle,
    new_block: &BftBlock,
    fat_pointer: &FatPointerToBftBlock2,
) -> Vec<tenderlink::SortedRosterMember> {
    let call = tfl_handle.call.clone();
    let params = &PROTOTYPE_PARAMETERS;

    let mut internal = tfl_handle.internal.lock().await;

    let mut return_validator_list_because_of_malachite_bug =
        internal.validators_at_current_height.clone();
    if return_validator_list_because_of_malachite_bug
        .iter()
        .position(|v| v.address.0 == internal.my_public_key)
        .is_none()
    {
        return_validator_list_because_of_malachite_bug.push(MalValidator {
            address: MalPublicKey2(internal.my_public_key),
            public_key: internal.my_public_key,
            voting_power: 0,
        });
    }

    if fat_pointer.points_at_block_hash() != new_block.blake3_hash() {
        error!(
            "Fat Pointer hash does not match block hash. fp: {} block: {}",
            fat_pointer.points_at_block_hash(),
            new_block.blake3_hash()
        );
        panic!();
    }
    // TODO: check public keys on the fat pointer against the roster
    if fat_pointer.validate_signatures() == false {
        error!("Signatures are not valid. Rejecting block.");
        panic!();
    }

    assert_eq!(
        validate_bft_block_from_malachite_already_locked(&tfl_handle, &mut internal, new_block)
            .await,
        tenderlink::TMStatus::Pass
    );

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
            previous_block_fat_ptr: FatPointerToBftBlock2 {
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

    match (call.state)(zebra_state::Request::CrosslinkFinalizeBlock(new_final_hash)).await {
        Ok(zebra_state::Response::CrosslinkFinalized(hash)) => {
            info!("Successfully crosslink-finalized {}", hash);
            assert_eq!(
                hash, new_final_hash,
                "PoW finalized hash should now match ours"
            );
        }
        Ok(_) => unreachable!("wrong response type"),
        Err(err) => {
            error!(?err);
        }
    }

    {
        let new_bc_final = internal.latest_final_block;

        // info!("final changed to {:?}", new_bc_final);
        if let Some(new_final_height_hash) = new_bc_final {
            let start_hash = if let Some(prev_height_hash) = internal.current_bc_final {
                prev_height_hash.1
            } else {
                new_final_height_hash.1
            };

            let (new_final_height_hashes, new_final_blocks) = tfl_block_sequence(
                &call,
                start_hash,
                Some(new_final_height_hash),
                /*include_start_hash*/ true,
                true,
            )
            .await;

            let mut quiet = true;
            if let (Some(Some(first_block)), Some(Some(last_block))) =
                (new_final_blocks.first(), new_final_blocks.last())
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
                tfl_dump_blocks(&new_final_height_hashes[..], &new_final_blocks[..]);
            }

            let pos_total_reward: u64 = 6000; // arbitrary scale

            // walk all blocks in newly-finalized sequence; handle rewards & broadcast them
            for i in 0..new_final_height_hashes.len() {
                // skip repeated boundary blocks
                let new_final_height_hash = &new_final_height_hashes[i];
                if let Some((_, prev_hash)) = internal.current_bc_final {
                    if prev_hash == new_final_height_hash.1 {
                        continue;
                    }
                }

                // Divide the reward between finalizers. Any rounding errors are intended to be
                // accounted for here by giving those zats to the finalizer with the largest
                // stake. As a result, this should always *exactly* apportion the entire
                // reward.
                // TODO: is there a standardised way of doing this?
                {
                    // NOTE: recalculating here means the *compounding* is per PoW block, as well
                    // as the value.
                    // It also means that finalizers added in the same PoS will get rewards for PoW
                    // blocks they couldn't have actually voted on.

                    let finalizers = &mut internal.validators_at_current_height;
                    let mut total_voting_power = 0;
                    let mut max_power_finalizer_i: Option<usize> = None;

                    // determine total_voting_power & max_power_finalizer_i
                    for finalizer_i in 0..finalizers.len() {
                        let finalizer = &finalizers[finalizer_i];
                        if finalizer.voting_power == 0 {
                            continue;
                        }

                        if max_power_finalizer_i.is_none()
                            || finalizers[max_power_finalizer_i.unwrap()].voting_power
                                < finalizer.voting_power
                        {
                            max_power_finalizer_i = Some(finalizer_i);
                        }

                        total_voting_power += finalizer.voting_power;
                    }

                    // Give share of block reward to all finalizers based on their stake
                    // (except for max staker).
                    // We make use of the fact that integer division always rounds down such
                    // that sum_reward <= the infinite-precision sum.
                    let mut sum_reward = 0_u64;
                    for finalizer_i in 0..finalizers.len() {
                        let finalizer = &mut finalizers[finalizer_i];
                        if finalizer.voting_power == 0
                            || finalizer_i
                                == max_power_finalizer_i
                                    .expect("there must be a max finalizer if at least 1 is non-0")
                        {
                            continue;
                        }

                        // NOTE: total_voting_power must be non-0 if we have any non-0 roster
                        // members
                        // TODO: most numerically stable version of this that won't overflow
                        let mul: u128 =
                            (finalizer.voting_power as u128) * (pos_total_reward as u128);
                        let reward = (mul / (total_voting_power as u128)) as u64;
                        sum_reward += reward;
                        finalizer.voting_power += reward;
                    }

                    // Give remaining block reward (including any accumulated rounding errors)
                    // to max staker
                    if let Some(finalizer_i) = max_power_finalizer_i {
                        let finalizer = &mut finalizers[finalizer_i];
                        let mul: u128 =
                            (finalizer.voting_power as u128) * (pos_total_reward as u128);
                        let reward = (mul / (total_voting_power as u128)) as u64;
                        let rem_reward = pos_total_reward - sum_reward;
                        assert!(reward <= rem_reward,
                            "should be at least the expected share remaining. Any discrepancy should be in this finalizer's favor.\n\
                            expected reward: {}, remaining reward: {}",
                            reward, rem_reward);

                        if reward != rem_reward {
                            info!("Max finalizer given rounding error: expected {}, got {}, bonus: {}",
                                reward, rem_reward, rem_reward - reward);
                        }

                        finalizer.voting_power += rem_reward;
                    }
                }

                // Modify the stake for members
                if let Some(new_final_block) = &new_final_blocks[i] {
                    let cmd_c = update_roster_for_block(&mut internal, new_final_block);
                    if cmd_c > 0 {
                        info!(
                            "Applied {} commands to roster from PoW height {}",
                            cmd_c, new_final_height_hashes[i].0 .0,
                        );
                    }
                } else {
                    error!(
                        "failed to get known block at {:?}",
                        new_final_height_hashes[i]
                    );
                    debug_assert!(false, "this shouldn't happen");
                }

                // We ignore the error because there will be one in the ordinary case
                // where there are no receivers yet.
                let _ = internal.final_change_tx.send(*new_final_height_hash);
            }
        }
        internal.current_bc_final = new_bc_final;
    }

    tenderlink_roster_from_internal(&internal.validators_at_current_height)
}

// TODO: collapse away Malachite roster
fn tenderlink_roster_from_internal(vals: &[MalValidator]) -> Vec<SortedRosterMember> {
    let mut ret: Vec<SortedRosterMember> = vals
        .iter()
        .map(|v| SortedRosterMember {
            pub_key: tenderlink::PubKeyID(v.public_key.into()),
            stake: v.voting_power,
            cumulative_stake: 0,
        })
        .collect();

    // Roster needs to be sorted for various reasons, including determining who is under max, and
    // giving a consistent index that can be used to represent a roster member.
    // Needs to uniquely & stably tie-break members with the same stake, so that everyone has
    // exactly the same view regardless of whether they found out about members in different
    // orders... so we use pub_key.
    //
    // We separately keep track of cumulative stake to make weighted round-robin easy.
    // fn prepare_roster(roster: &mut [SortedRosterMember])
    {
        ret.sort_by_key(|m: &SortedRosterMember| std::cmp::Reverse((m.stake, m.pub_key)));
        debug_assert!(ret.is_sorted_by(|a, b| a.stake >= b.stake)); // descending

        let mut cumulative_stake = 0;
        for m in &mut ret {
            cumulative_stake += m.stake;
            m.cumulative_stake = cumulative_stake;
        }
    }

    ret
}

async fn validate_bft_block_from_malachite(
    tfl_handle: &TFLServiceHandle,
    new_block: &BftBlock,
) -> tenderlink::TMStatus {
    let mut internal = tfl_handle.internal.lock().await;
    validate_bft_block_from_malachite_already_locked(tfl_handle, &mut internal, new_block).await
}
async fn validate_bft_block_from_malachite_already_locked(
    tfl_handle: &TFLServiceHandle,
    internal: &mut TFLServiceInternal,
    new_block: &BftBlock,
) -> tenderlink::TMStatus {
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
        return tenderlink::TMStatus::Fail;
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
            return tenderlink::TMStatus::Indeterminate;
        };
    return tenderlink::TMStatus::Pass;
}

fn fat_pointer_to_block_at_height(
    bft_blocks: &[BftBlock],
    fat_pointer_to_tip: &FatPointerToBftBlock2,
    at_height: u64,
) -> Option<FatPointerToBftBlock2> {
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
) -> Option<(BftBlock, FatPointerToBftBlock2)> {
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
pub fn run_tfl_test(internal_handle: TFLServiceHandle) {
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
            dump_test_instrs();

            #[cfg(not(feature = "viz_gui"))]
            std::process::abort();
        }
    }));

    tokio::task::spawn(test_format::instr_reader(internal_handle));
}

async fn push_staking_action_from_cmd_str(
    call: &TFLServiceCalls,
    cmd_str: &str,
) -> Result<(), String> {
    use zebra_chain::transaction::{LockTime, Transaction, UnminedTx};
    let staking_action = zcash_primitives::transaction::StakingAction::parse_from_cmd(cmd_str)?;
    let tx: UnminedTx = Transaction::VCrosslink {
        // TODO(@prod): determine from network/height
        network_upgrade: zebra_chain::parameters::NetworkUpgrade::Nu6,
        lock_time: LockTime::unlocked(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        sapling_shielded_data: None,
        orchard_shielded_data: None,
        expiry_height: BlockHeight(0), // "don't expire"
        staking_action,
    }
    .into();

    if let Ok(MempoolResponse::Queued(receivers)) =
        (call.mempool)(MempoolRequest::Queue(vec![tx.into()])).await
    {
        for receiver in receivers {
            match receiver {
                Err(err) => return Err(format!("tried to send command transaction: {err}")),
                Ok(receiver) => match receiver.await {
                    Err(err) => return Err(format!("tried to await command transaction: {err}")),
                    Ok(result) => match result {
                        Err(err) => {
                            return Err(format!("unsuccessfully mempooled transaction: {err}"))
                        }
                        Ok(()) => {}
                    },
                },
            }
        }
    }
    Ok(())
}

fn update_roster_for_cmd(
    roster: &mut Vec<MalValidator>,
    validators_keys_to_names: &mut HashMap<MalPublicKey, String>,
    action: &StakingAction,
) -> usize {
    // TODO: what is allowed in terms of multiple staking action in 1 command?
    // Any subtract is serially dependent

    let (has_add, sub_key_name, is_clear) = match action.kind {
        StakingActionKind::Add => (true, None, false),
        StakingActionKind::Sub => (
            false,
            Some((action.target, &action.insecure_target_name)),
            false,
        ),
        StakingActionKind::Clear => (
            false,
            Some((action.target, &action.insecure_target_name)),
            true,
        ),
        StakingActionKind::Move => (
            true,
            Some((action.source, &action.insecure_source_name)),
            false,
        ),
        StakingActionKind::MoveClear => (
            true,
            Some((action.source, &action.insecure_source_name)),
            true,
        ),
    };

    let mut amount = action.val;
    if let Some((sub_key, sub_name)) = sub_key_name {
        let sub_key = MalPublicKey2(sub_key.into());
        let Some(member) = roster.iter_mut().find(|cmp| cmp.public_key == sub_key.0) else {
            warn!(
                "Roster command invalid: can't subtract from non-present finalizer \"{}\"",
                sub_name
            );
            return 0;
        };

        if member.voting_power < action.val {
            if is_clear {
                warn!("Roster command invalid: can't clear the finalizer to a higher current value \"{}\"/{}: {} => {}",
                    sub_name, sub_key, member.voting_power, action.val);
            } else {
                warn!("Roster command invalid: can't subtract more from the finalizer than their current value \"{}\"/{}: {} - {}",
                    sub_name, sub_key, member.voting_power, action.val);
            }
            return 0;
        }

        if is_clear {
            amount = member.voting_power - action.val
        };

        member.voting_power -= amount;
    }

    if has_add {
        // NOTE: all adds are to action.target
        let add_key = MalPublicKey2(action.target.into());
        if let Some(member) = roster.iter_mut().find(|cmp| cmp.public_key == add_key.0) {
            member.voting_power += amount;
        } else {
            roster.push(MalValidator::new(add_key.0, amount));
            validators_keys_to_names.insert(add_key.0, action.insecure_target_name.clone());
        }
    }

    1
}

fn update_roster_for_block(internal: &mut TFLServiceInternal, block: &Block) -> usize {
    let roster = &mut internal.validators_at_current_height;
    let validators_keys_to_names = &mut internal.validators_keys_to_names;
    let mut cmd_c = 0;

    for tx in &block.transactions {
        let mut is_cmd = 0;
        if let zebra_chain::transaction::Transaction::VCrosslink { staking_action, .. } =
            tx.as_ref()
        {
            if let Some(staking_action) = staking_action {
                cmd_c += update_roster_for_cmd(roster, validators_keys_to_names, &staking_action);
            }
        };
    }

    // TODO: retain stake > 0?
    cmd_c
}

async fn tfl_service_main_loop(internal_handle: TFLServiceHandle) -> Result<(), String> {
    let call = internal_handle.call.clone();
    let config = internal_handle.config.clone();
    let params = &PROTOTYPE_PARAMETERS;

    #[cfg(feature = "viz_gui")]
    {
        let rt = tokio::runtime::Handle::current();
        let viz_tfl_handle = internal_handle.clone();
        tokio::task::spawn_blocking(move || {
            rt.block_on(viz2::service_viz_requests(viz_tfl_handle, params))
        });
    }

    if *TEST_MODE.lock().unwrap() {
        run_tfl_test(internal_handle.clone());
    }

    let public_ip_string = config
        .public_address
        .unwrap_or(String::from_str("/ip4/127.0.0.1/tcp/45869").unwrap());
    info!("public IP: {}", public_ip_string);

    let user_name = config
        .insecure_user_name
        .unwrap_or(public_ip_string.clone());
    // .unwrap_or(String::from_str("tester").unwrap());
    info!("user_name: {}", user_name);

    let (mut rng, my_private_key, my_public_key) =
        rng_private_public_key_from_address(&user_name.as_bytes());
    internal_handle.internal.lock().await.my_public_key = my_public_key;

    {
        use tenderlink::{SecureUdpEndpoint, StaticDHKeyPair};

        use std::net::{Ipv6Addr, SocketAddr};

        /// Parses "IP[:port]" (IPv4 or bracketed IPv6 with port) into (16-byte IPv6, port)
        fn parse_to_ipv6_bytes(s: &str) -> Result<([u8; 16], u16), std::net::AddrParseError> {
            let sa: SocketAddr = s.parse()?;

            let (ip6, port) = match sa {
                SocketAddr::V4(v4) => {
                    // Map IPv4 to IPv6-mapped ::ffff:a.b.c.d
                    (v4.ip().to_ipv6_mapped(), v4.port())
                    // (Alternatively on newer Rust: (v4.ip().to_ipv6_mapped(), v4.port()))
                }
                SocketAddr::V6(v6) => (*v6.ip(), v6.port()),
            };

            Ok((ip6.octets(), port))
        }

        fn addr_string_to_stuff(addr: &str) -> (StaticDHKeyPair, SecureUdpEndpoint) {
            let mut hasher = DefaultHasher::new();
            hasher.write(addr.as_bytes());
            let seed = hasher.finish();

            let kp = snow::Builder::with_resolver(
                "Noise_IK_25519_ChaChaPoly_BLAKE2s".parse().unwrap(),
                Box::new(tenderlink::SnowRngResolver::seed_from_u64(seed)),
            )
            .generate_keypair()
            .unwrap();
            let static_keypair = tenderlink::StaticDHKeyPair {
                private: kp.private.try_into().unwrap(),
                public: kp.public.try_into().unwrap(),
            };
            let (ip, port) = parse_to_ipv6_bytes(addr).unwrap();
            (
                static_keypair,
                SecureUdpEndpoint {
                    public_key: static_keypair.public,
                    ip_address: ip,
                    port,
                },
            )
        }

        let mut static_keypair_maybe = None;
        let mut endpoint_maybe = None;
        if let Some(listen_addr) = &config.listen_address {
            let (a, b) = addr_string_to_stuff(&listen_addr);
            static_keypair_maybe = Some(a);
            endpoint_maybe = Some(b);
        };

        let unsorted_roster = internal_handle
            .internal
            .lock()
            .await
            .validators_at_current_height
            .clone();
        let roster = tenderlink_roster_from_internal(&unsorted_roster);

        // Note(Sam): We do not support human names in the start config for now.
        let evidence = unsorted_roster
            .iter()
            .enumerate()
            .map(|(i, m)| {
                use tenderlink::EndpointEvidence;

                let string = format!("{:?}", m);
                let mut hasher = DefaultHasher::new();
                hasher.write(string.as_bytes());
                let seed = hasher.finish();
                let string = format!("127.0.0.1:{}", seed % 4000);
                let (a, b) =
                    addr_string_to_stuff(&config.malachite_peers.get(i).unwrap_or_else(|| &string));
                EndpointEvidence {
                    endpoint: b,
                    root_public_key: m.public_key.into(),
                }
            })
            .collect();

        let tfl_handle1 = internal_handle.clone();
        let tfl_handle2 = internal_handle.clone();
        let tfl_handle3 = internal_handle.clone();
        let tfl_handle4 = internal_handle.clone();

        tokio::spawn(tenderlink::entry_point(
            my_private_key,
            static_keypair_maybe,
            endpoint_maybe,
            roster,
            evidence,
            None,
            tenderlink::ClosureToProposeNewBlock(Arc::new(move || {
                let tfl_handle1 = tfl_handle1.clone();
                Box::pin(async move {
                    propose_new_bft_block(&tfl_handle1).await.map(|block| {
                        tenderlink::BlockValue(block.zcash_serialize_to_vec().unwrap())
                    })
                })
            })),
            tenderlink::ClosureToValidateProposedBlock(Arc::new(move |block| {
                let tfl_handle2 = tfl_handle2.clone();
                Box::pin(async move {
                    use bytes::Buf;
                    use zebra_chain::serialization::ZcashDeserialize;

                    if let Ok(bft_block) = BftBlock::zcash_deserialize(block.0.reader()) {
                        validate_bft_block_from_malachite(&tfl_handle2, &bft_block).await
                    } else {
                        error!("Failed to deserialize Tenderlink payload.");
                        tenderlink::TMStatus::Fail
                    }
                })
            })),
            tenderlink::ClosureToPushDecidedBlock(Arc::new(move |block, fat_pointer| {
                let tfl_handle3 = tfl_handle3.clone();
                Box::pin(async move {
                    use bytes::Buf;
                    use zebra_chain::serialization::ZcashDeserialize;

                    new_decided_bft_block_from_malachite(
                        &tfl_handle3,
                        &BftBlock::zcash_deserialize(block.0.reader()).unwrap(),
                        &fat_pointer.into(),
                    )
                    .await
                })
            })),
            tenderlink::ClosureToGetHistoricalBlock(Arc::new(move |height| {
                Box::pin(async move {
                    panic!();
                })
            })),
            tenderlink::ClosureToUpdateRosterCmd(Arc::new(move |str| {
                let tfl_handle = tfl_handle4.clone();
                Box::pin(async move {
                    let mut internal = tfl_handle.internal.lock().await;
                    // ours overrides
                    if internal.our_set_bft_string.is_some() {
                        internal.our_set_bft_string.take()
                    } else {
                        if let Some(str) = str {
                            internal.active_bft_string = Some(str)
                        }
                        None
                    }
                })
            })),
        ));
    }

    let mut run_instant = Instant::now();
    let mut last_diagnostic_print = Instant::now();
    let mut current_bc_tip: Option<(BlockHeight, BlockHash)> = None;

    loop {
        // Calculate this prior to message handling so that handlers can use it:
        let new_bc_tip = if let Ok(StateResponse::Tip(val)) = (call.state)(StateRequest::Tip).await
        {
            val
        } else {
            None
        };

        tokio::time::sleep_until(run_instant).await;
        run_instant += MAIN_LOOP_SLEEP_INTERVAL;

        // from this point onwards we must race to completion in order to avoid stalling incoming requests
        // NOTE: split to avoid deadlock from non-recursive mutex - can we reasonably change type?
        #[allow(unused_mut)]
        let mut internal = internal_handle.internal.lock().await;

        // Check TFL is activated before we do anything that assumes it
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
    }
}

async fn tfl_block_finality_from_height_hash(
    internal_handle: TFLServiceHandle,
    height: BlockHeight,
    hash: BlockHash,
) -> Result<Option<TFLBlockFinality>, TFLServiceError> {
    // TODO: None is no longer ever returned
    let call = internal_handle.call.clone();
    let block_hdr = (call.state)(StateRequest::BlockHeader(hash.into()));
    let (final_height, final_hash) = match tfl_final_block_height_hash(&internal_handle).await {
        Some(v) => v,
        None => {
            return Err(TFLServiceError::Misc(
                "There is no final block.".to_string(),
            ));
        }
    };

    if height > final_height {
        // N.B. this may be invalidated by the time it is received
        Ok(Some(TFLBlockFinality::NotYetFinalized))
    } else {
        let cmp_hash = if height == final_height {
            final_hash // we already have the hash at the final height, no point in re-getting it
        } else {
            match (call.state)(StateRequest::BlockHeader(height.into())).await {
                Ok(StateResponse::BlockHeader { hash, .. }) => hash,

                Err(err) => return Err(TFLServiceError::Misc(err.to_string())),

                _ => {
                    return Err(TFLServiceError::Misc(
                        "Invalid BlockHeader response type".to_string(),
                    ))
                }
            }
        };

        // We have the hash of the block at the given height from the best chain.
        // If it matches the queried hash then our block is on the best chain under the finalization
        // height & is thus finalized.
        // Otherwise it can't be finalized.
        Ok(Some(if hash == cmp_hash {
            TFLBlockFinality::Finalized
        } else {
            TFLBlockFinality::CantBeFinalized
        }))
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

        TFLServiceRequest::FinalBlockHeightHash => Ok(TFLServiceResponse::FinalBlockHeightHash(
            tfl_final_block_height_hash(&internal_handle).await,
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

        TFLServiceRequest::BlockFinalityStatus(height, hash) => {
            match tfl_block_finality_from_height_hash(internal_handle.clone(), height, hash).await {
                Ok(val) => Ok(TFLServiceResponse::BlockFinalityStatus({ val })), // N.B. may still be None
                Err(err) => Err(err),
            }
        }

        TFLServiceRequest::TxFinalityStatus(hash) => Ok(TFLServiceResponse::TxFinalityStatus({
            if let Ok(StateResponse::Transaction(Some(tx))) =
                (call.state)(StateRequest::Transaction(hash)).await
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

        TFLServiceRequest::Roster => Ok(TFLServiceResponse::Roster({
            let internal = internal_handle.internal.lock().await;
            internal
                .validators_at_current_height
                .iter()
                .map(|v| (<[u8; 32]>::from(v.public_key), v.voting_power))
                .collect()
        })),

        TFLServiceRequest::FatPointerToBFTChainTip => {
            let internal = internal_handle.internal.lock().await;
            Ok(TFLServiceResponse::FatPointerToBFTChainTip(
                internal.fat_pointer_to_tip.clone().to_non_two(),
            ))
        }

        TFLServiceRequest::StakingCmd(cmd) => {
            match push_staking_action_from_cmd_str(&internal_handle.call, &cmd).await {
                Ok(()) => Ok(TFLServiceResponse::StakingCmd),
                Err(err) => Err(TFLServiceError::Misc(format!("{err}"))),
            }
        }

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

// TODO: can we change the signature to unwrap the block options? The blocks must exist if the
// hashes do
// NOTE: this is currently best-chain-only due to request/response limitations
// TODO: add more request/response pairs directly in zebra-state's StateService
/// always returns block hashes. If read_extra_info is set, also returns Blocks, otherwise returns an empty vector.
async fn tfl_block_sequence(
    call: &TFLServiceCalls,
    start_hash: BlockHash,
    final_height_hash: Option<(BlockHeight, BlockHash)>,
    include_start_hash: bool,
    read_extra_info: bool, // NOTE: done here rather than on print to isolate async from sync code
) -> (Vec<(BlockHeight, BlockHash)>, Vec<Option<Arc<Block>>>) {
    // get "real" initial values //////////////////////////////
    let (start_height, init_hash) = {
        if let Ok(StateResponse::BlockHeader { height, header, .. }) =
            (call.state)(StateRequest::BlockHeader(start_hash.into())).await
        {
            if include_start_hash {
                // NOTE: BlockHashes does not return the first hash provided, so we move back 1.
                //       We would probably also be fine to just push it directly.
                (Some(height), Some(header.previous_block_hash))
            } else {
                (Some(BlockHeight(height.0 + 1)), Some(start_hash))
            }
        } else {
            (None, None)
        }
    };
    let (final_height, final_hash) = if let Some((height, hash)) = final_height_hash {
        (Some(height), Some(hash))
    } else if let Ok(StateResponse::Tip(val)) = (call.state)(StateRequest::Tip).await {
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

            let res = (call.state)(StateRequest::FindBlockHashes {
                known_blocks: vec![*chunk_start_hash],
                stop: final_hash,
            })
            .await;

            if let Ok(StateResponse::BlockHashes(chunk_hashes)) = res {
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
            let height = BlockHeight(
                start_height.0 + <u32>::try_from(hashes.len()).expect("should fit in u32"),
            );
            // debug_assert!(if let Some(h) = block_height_from_hash(call, *val).await {
            //     if h != height {
            //         error!("expected: {:?}, actual: {:?}", height, h);
            //     }
            //     h == height
            // } else {
            //     true
            // });
            hashes.push((height, *val));
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
                if let Ok(StateResponse::Block(block)) =
                    (call.state)(StateRequest::Block((hash.1).into())).await
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
impl HasBlockHash for (BlockHeight, BlockHash) {
    fn get_hash(&self) -> Option<BlockHash> {
        Some(self.1)
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

fn tfl_dump_blocks(blocks: &[(BlockHeight, BlockHash)], infos: &[Option<Arc<Block>>]) {
    let highlight_chars_n = block_hash_unique_chars_n(blocks);

    let print_color = true;

    for (block_i, (_, hash)) in blocks.iter().enumerate() {
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

/// A validator is a public key and voting power
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MalValidator {
    pub address: MalPublicKey2,
    pub public_key: MalPublicKey,
    pub voting_power: u64,
}

impl MalValidator {
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn new(public_key: MalPublicKey, voting_power: u64) -> Self {
        Self {
            address: MalPublicKey2(public_key),
            public_key,
            voting_power,
        }
    }
}

impl PartialOrd for MalValidator {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MalValidator {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.address.cmp(&other.address)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MalPublicKey2(pub MalPublicKey);

impl std::fmt::Display for MalPublicKey2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0.as_ref() {
            write!(f, "{:02X}", byte)?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for MalPublicKey2 {
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

/// A bundle of signed votes for a block
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)] //, serde::Serialize, serde::Deserialize)]
pub struct FatPointerToBftBlock2 {
    pub vote_for_block_without_finalizer_public_key: [u8; 76 - 32],
    pub signatures: Vec<FatPointerSignature2>,
}

impl std::fmt::Display for FatPointerToBftBlock2 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{{hash:")?;
        for b in &self.vote_for_block_without_finalizer_public_key[0..32] {
            write!(f, "{:02x}", b)?;
        }
        write!(f, " ovd:")?;
        for b in &self.vote_for_block_without_finalizer_public_key[32..] {
            write!(f, "{:02x}", b)?;
        }
        write!(f, " signatures:[")?;
        for (i, s) in self.signatures.iter().enumerate() {
            write!(f, "{{pk:")?;
            for b in s.public_key {
                write!(f, "{:02x}", b)?;
            }
            write!(f, " sig:")?;
            for b in s.vote_signature {
                write!(f, "{:02x}", b)?;
            }
            write!(f, "}}")?;
            if i + 1 < self.signatures.len() {
                write!(f, " ")?;
            }
        }
        write!(f, "]}}")?;
        Ok(())
    }
}

impl From<tenderlink::FatPointerToBftBlock3> for FatPointerToBftBlock2 {
    fn from(fat_pointer: tenderlink::FatPointerToBftBlock3) -> FatPointerToBftBlock2 {
        FatPointerToBftBlock2 {
            vote_for_block_without_finalizer_public_key: fat_pointer
                .vote_for_block_without_finalizer_public_key,
            signatures: fat_pointer
                .signatures
                .into_iter()
                .map(|s| FatPointerSignature2 {
                    public_key: s.public_key,
                    vote_signature: s.vote_signature,
                })
                .collect(),
        }
    }
}

impl FatPointerToBftBlock2 {
    pub fn to_non_two(self) -> zebra_chain::block::FatPointerToBftBlock {
        zebra_chain::block::FatPointerToBftBlock {
            vote_for_block_without_finalizer_public_key: self
                .vote_for_block_without_finalizer_public_key,
            signatures: self
                .signatures
                .into_iter()
                .map(|two| zebra_chain::block::FatPointerSignature {
                    public_key: two.public_key,
                    vote_signature: two.vote_signature,
                })
                .collect(),
        }
    }

    pub fn null() -> FatPointerToBftBlock2 {
        FatPointerToBftBlock2 {
            vote_for_block_without_finalizer_public_key: [0_u8; 76 - 32],
            signatures: Vec::new(),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.vote_for_block_without_finalizer_public_key);
        buf.extend_from_slice(&(self.signatures.len() as u16).to_le_bytes());
        for s in &self.signatures {
            buf.extend_from_slice(&s.to_bytes());
        }
        buf
    }
    #[allow(clippy::reversed_empty_ranges)]
    pub fn try_from_bytes(bytes: &Vec<u8>) -> Option<FatPointerToBftBlock2> {
        if bytes.len() < 76 - 32 + 2 {
            return None;
        }
        let vote_for_block_without_finalizer_public_key = bytes[0..76 - 32].try_into().unwrap();
        let len = u16::from_le_bytes(bytes[76 - 32..2].try_into().unwrap()) as usize;

        if 76 - 32 + 2 + len * (32 + 64) > bytes.len() {
            return None;
        }
        let rem = &bytes[76 - 32 + 2..];
        let signatures = rem
            .chunks_exact(32 + 64)
            .map(|chunk| FatPointerSignature2::from_bytes(chunk.try_into().unwrap()))
            .collect();

        Some(Self {
            vote_for_block_without_finalizer_public_key,
            signatures,
        })
    }

    pub fn get_vote_template(&self) -> MalVote {
        let mut vote_bytes = [0_u8; 76];
        vote_bytes[32..76].copy_from_slice(&self.vote_for_block_without_finalizer_public_key);
        MalVote::from_bytes(&vote_bytes)
    }
    pub fn inflate(&self) -> Vec<(MalVote, ed25519_zebra::ed25519::SignatureBytes)> {
        let vote_template = self.get_vote_template();
        self.signatures
            .iter()
            .map(|s| {
                let mut vote = vote_template.clone();
                vote.validator_address = MalPublicKey2(MalPublicKey::from(s.public_key));
                (vote, s.vote_signature)
            })
            .collect()
    }
    pub fn validate_signatures(&self) -> bool {
        let mut batch = ed25519_zebra::batch::Verifier::new();
        for (vote, signature) in self.inflate() {
            let vk_bytes = ed25519_zebra::VerificationKeyBytes::from(vote.validator_address.0);
            let sig = ed25519_zebra::Signature::from_bytes(&signature);
            let msg = vote.to_bytes();

            batch.queue((vk_bytes, sig, &msg));
        }
        batch.verify(rand::thread_rng()).is_ok()
    }
    pub fn points_at_block_hash(&self) -> Blake3Hash {
        Blake3Hash(
            self.vote_for_block_without_finalizer_public_key[0..32]
                .try_into()
                .unwrap(),
        )
    }
}

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io;

impl ZcashSerialize for FatPointerToBftBlock2 {
    fn zcash_serialize<W: io::Write>(&self, mut writer: W) -> Result<(), io::Error> {
        writer.write_all(&self.vote_for_block_without_finalizer_public_key)?;
        writer.write_u16::<LittleEndian>(self.signatures.len() as u16)?;
        for signature in &self.signatures {
            writer.write_all(&signature.to_bytes())?;
        }
        Ok(())
    }
}

impl ZcashDeserialize for FatPointerToBftBlock2 {
    fn zcash_deserialize<R: io::Read>(mut reader: R) -> Result<Self, SerializationError> {
        let mut vote_for_block_without_finalizer_public_key = [0u8; 76 - 32];
        reader.read_exact(&mut vote_for_block_without_finalizer_public_key)?;

        let len = reader.read_u16::<LittleEndian>()?;
        let mut signatures: Vec<FatPointerSignature2> = Vec::with_capacity(len.into());
        for _ in 0..len {
            let mut signature_bytes = [0u8; 32 + 64];
            reader.read_exact(&mut signature_bytes)?;
            signatures.push(FatPointerSignature2::from_bytes(&signature_bytes));
        }

        Ok(FatPointerToBftBlock2 {
            vote_for_block_without_finalizer_public_key,
            signatures,
        })
    }
}

/// A vote signature for a block
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)] //, serde::Serialize, serde::Deserialize)]
pub struct FatPointerSignature2 {
    pub public_key: [u8; 32],
    pub vote_signature: [u8; 64],
}

impl FatPointerSignature2 {
    pub fn to_bytes(&self) -> [u8; 32 + 64] {
        let mut buf = [0_u8; 32 + 64];
        buf[0..32].copy_from_slice(&self.public_key);
        buf[32..32 + 64].copy_from_slice(&self.vote_signature);
        buf
    }
    pub fn from_bytes(bytes: &[u8; 32 + 64]) -> FatPointerSignature2 {
        Self {
            public_key: bytes[0..32].try_into().unwrap(),
            vote_signature: bytes[32..32 + 64].try_into().unwrap(),
        }
    }
}

/// A vote for a value in a round
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MalVote {
    pub validator_address: MalPublicKey2,
    pub value: Blake3Hash,
    pub height: u64,
    pub typ: bool, // true is commit
    pub round: i32,
}

/*
DATA LAYOUT FOR VOTE
32 byte ed25519 public key of the finalizer who's vote this is
32 byte blake3 hash of value, or all zeroes to indicate Nil vote
8 byte height
4 byte round where MSB is used to indicate is_commit for the vote type. 1 bit is_commit, 31 bits round index

TOTAL: 76 B

A signed vote will be this same layout followed by the 64 byte ed25519 signature of the previous 76 bytes.
*/

impl MalVote {
    pub fn to_bytes(&self) -> [u8; 76] {
        let mut buf = [0_u8; 76];
        buf[0..32].copy_from_slice(self.validator_address.0.as_ref());
        buf[32..64].copy_from_slice(&self.value.0);
        buf[64..72].copy_from_slice(&self.height.to_le_bytes());

        let mut merged_round_val: u32 = (self.round & 0x7fff_ffff) as u32;
        if self.typ {
            merged_round_val |= 0x8000_0000;
        }
        buf[72..76].copy_from_slice(&merged_round_val.to_le_bytes());
        buf
    }
    pub fn from_bytes(bytes: &[u8; 76]) -> MalVote {
        let validator_address =
            MalPublicKey2(From::<[u8; 32]>::from(bytes[0..32].try_into().unwrap()));
        let value_hash_bytes = bytes[32..64].try_into().unwrap();
        let value = Blake3Hash(value_hash_bytes);
        let height = u64::from_le_bytes(bytes[64..72].try_into().unwrap());

        let merged_round_val = u32::from_le_bytes(bytes[72..76].try_into().unwrap());

        let typ = merged_round_val & 0x8000_0000 != 0;
        let round = (merged_round_val & 0x7fff_ffff) as i32;

        MalVote {
            validator_address,
            value,
            height,
            typ,
            round,
        }
    }
}
