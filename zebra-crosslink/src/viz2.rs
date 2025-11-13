
use visualizer_zcash::Hash32;
use std::cmp::{max, min};

use crate::*;

pub fn viz_main(tokio_root_thread_handle: Option<std::thread::JoinHandle<()>>) {
    // loop {
    //     if let Some(ref thread_handle) = tokio_root_thread_handle {
    //         if thread_handle.is_finished() {
    //             return;
    //         }
    //     }
    // }
    visualizer_zcash::main_thread_run_program();
}


/// Bridge between tokio & viz code
pub async fn service_viz_requests(
    tfl_handle: crate::TFLServiceHandle,
    params: &'static crate::ZcashCrosslinkParameters,
) {
    let call = tfl_handle.clone().call;

    loop {
        let request_queue = visualizer_zcash::REQUESTS_TO_ZEBRA.lock().unwrap();
        let response_queue = visualizer_zcash::RESPONSES_FROM_ZEBRA.lock().unwrap();
        if request_queue.is_none() || response_queue.is_none() { continue; }
        let request_queue = request_queue.as_ref().unwrap();
        let response_queue = response_queue.as_ref().unwrap();

        'main_loop: loop {
            let mempool_txs = if let Ok(MempoolResponse::FullTransactions { transactions, .. }) =
                (call.mempool)(MempoolRequest::FullTransactions).await
            {
                transactions
            } else {
                Vec::new()
            };
    
            let tip_height_hash: (BlockHeight, BlockHash) = {
                if let Ok(StateResponse::Tip(Some(tip_height_hash))) =
                    (call.state)(StateRequest::Tip).await
                {
                    tip_height_hash
                } else {
                    //error!("Failed to read tip");
                    continue;
                }
            };
            let bc_tip_height: u64 = tip_height_hash.0.0 as u64;

            let bc_req_h = (0, -1);

            #[allow(clippy::never_loop)]
            let (lo_height, bc_tip, height_hashes, seq_blocks) = loop {
                let (lo, hi) = (bc_req_h.0, bc_req_h.1);
                assert!(
                    lo <= hi || (lo >= 0 && hi < 0),
                    "lo ({}) should be below hi ({})",
                    lo,
                    hi
                );

                let tip_height_hash: (BlockHeight, BlockHash) = {
                    if let Ok(StateResponse::Tip(Some(tip_height_hash))) =
                        (call.state)(StateRequest::Tip).await
                    {
                        tip_height_hash
                    } else {
                        //error!("Failed to read tip");
                        break (BlockHeight(0), None, Vec::new(), Vec::new());
                    }
                };

                let (h_lo, h_hi) = (
                    min(
                        tip_height_hash.0,
                        abs_block_height(lo, Some(tip_height_hash)),
                    ),
                    min(
                        tip_height_hash.0,
                        abs_block_height(hi, Some(tip_height_hash)),
                    ),
                );
                // temp
                //assert!(h_lo.0 <= h_hi.0, "lo ({}) should be below hi ({})", h_lo.0, h_hi.0);

                async fn get_height_hash(
                    call: TFLServiceCalls,
                    h: BlockHeight,
                    existing_height_hash: (BlockHeight, BlockHash),
                ) -> Option<(BlockHeight, BlockHash)> {
                    if h == existing_height_hash.0 {
                        // avoid duplicating work if we've already got that value
                        Some(existing_height_hash)
                    } else if let Ok(StateResponse::BlockHeader { hash, .. }) =
                        (call.state)(StateRequest::BlockHeader(h.into())).await
                    {
                        Some((h, hash))
                    } else {
                        error!("Failed to read block header at height {}", h.0);
                        None
                    }
                }

                let hi_height_hash = if let Some(hi_height_hash) =
                    get_height_hash(call.clone(), h_hi, tip_height_hash).await
                {
                    hi_height_hash
                } else {
                    break (BlockHeight(0), None, Vec::new(), Vec::new());
                };

                let lo_height_hash = if let Some(lo_height_hash) =
                    get_height_hash(call.clone(), h_lo, hi_height_hash).await
                {
                    lo_height_hash
                } else {
                    break (BlockHeight(0), None, Vec::new(), Vec::new());
                };

                let (height_hashes, blocks) =
                    tfl_block_sequence(&call, lo_height_hash.1, Some(hi_height_hash), true, true).await;
                break (
                    lo_height_hash.0,
                    Some(tip_height_hash),
                    height_hashes,
                    blocks,
                );
            };

            for _ in 0..256 {
                if let Ok(request) = request_queue.try_recv() {
                    let mut internal = tfl_handle.internal.lock().await;
                    let mut response = visualizer_zcash::ResponseFromZebra::_0();
                    response.bc_tip_height = bc_tip_height;
                    response.bft_tip_height = (internal.bft_blocks.len() as u64).saturating_sub(1);
                    for (i, bc) in seq_blocks.iter().enumerate() {
                        if let Some(bc) = bc {
                            response.bc_blocks.push(visualizer_zcash::BcBlock {
                                this_hash: Hash32::from_bytes(bc.header.hash().0),
                                parent_hash: Hash32::from_bytes(bc.header.previous_block_hash.0),
                                this_height: lo_height.0 as u64 + i as u64,
                                is_best_chain: true,
                                points_at_bft_block: Hash32::from_bytes(bc.header.fat_pointer_to_bft_block.points_at_block_hash()),
                            });
                        }
                    }
                    response.bft_blocks = internal.bft_blocks.iter().enumerate().map(|(i, b)|
                        visualizer_zcash::BftBlock {
                            this_hash: Hash32::from_bytes(b.blake3_hash().0),
                            parent_hash: Hash32::from_bytes(b.previous_block_hash().0),
                            this_height: i as u64,
                            points_at_bc_block: Hash32::from_bytes(b.finalization_candidate().hash().0),
                        }
                    ).collect();
                    let _ = response_queue.try_send(response);
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    continue 'main_loop;
                }
            }
        }
    }
}

fn abs_block_height(height: i32, tip: Option<(BlockHeight, BlockHash)>) -> BlockHeight {
    if height >= 0 {
        BlockHeight(height.try_into().unwrap())
    } else if let Some(tip) = tip {
        tip.0.sat_sub(!height)
    } else {
        BlockHeight(0)
    }
}

fn abs_block_heights(
    heights: (i32, i32),
    tip: Option<(BlockHeight, BlockHash)>,
) -> (BlockHeight, BlockHeight) {
    (
        abs_block_height(heights.0, tip),
        abs_block_height(heights.1, tip),
    )
}
