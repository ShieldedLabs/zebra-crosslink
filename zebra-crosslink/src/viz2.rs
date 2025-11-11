
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

            for _ in 0..256 {
                if let Ok(request) = request_queue.try_recv() {
                    let mut internal = tfl_handle.internal.lock().await;
                    let mut response = visualizer_zcash::ResponseFromZebra::_0();
                    response.bc_tip_height = bc_tip_height;
                    response.bft_tip_height = internal.bft_blocks.len() as u64;
                    let _ = response_queue.try_send(response);
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    continue 'main_loop;
                }
            }
        }
    }
}