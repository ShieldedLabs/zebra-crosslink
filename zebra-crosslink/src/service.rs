//! Tower Service implementation for TFLService.
//!
//! This module integrates `TFLServiceHandle` with the `tower::Service` trait,
//! allowing it to handle asynchronous service requests.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::sync::{broadcast, Mutex};

use zebra_chain::block::Hash as BlockHash;
use zebra_state::{ReadRequest as ReadStateRequest, ReadResponse as ReadStateResponse};

use crate::{tfl_service_incoming_request, TFLBlockFinality, TFLServiceInternal};

impl tower::Service<TFLServiceRequest> for TFLServiceHandle {
    type Response = TFLServiceResponse;
    type Error = TFLServiceError;
    type Future = Pin<Box<dyn Future<Output = Result<TFLServiceResponse, TFLServiceError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: TFLServiceRequest) -> Self::Future {
        let duplicate_handle = self.clone();
        Box::pin(async move { tfl_service_incoming_request(duplicate_handle, request).await })
    }
}

/// Types of requests that can be made to the TFLService.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TFLServiceRequest {
    /// Increment the test value
    IncrementVal,
    /// Get the final block hash
    FinalBlockHash,
    /// Get a receiver for the final block hash
    FinalBlockRx,
    /// Get the finality status of a block
    BlockFinalityStatus(BlockHash),
}

/// Types of responses that can be returned by the TFLService.
#[derive(Debug)]
pub enum TFLServiceResponse {
    /// Incremented value
    ValIncremented(u64),
    /// Final block hash
    FinalBlockHash(Option<BlockHash>),
    /// Receiver for the final block hash
    FinalBlockRx(broadcast::Receiver<BlockHash>),
    /// Finality status of a block
    BlockFinalityStatus(Option<TFLBlockFinality>),
}

/// Errors that can occur when interacting with the TFLService.
#[derive(Debug)]
pub enum TFLServiceError {
    /// Not implemented error
    NotImplemented,
    /// Arbitrary error
    Misc(String),
}

/// A pinned-in-memory, heap-allocated, reference-counted, thread-safe, asynchronous function
/// pointer that takes a `ReadStateRequest` as input and returns a `ReadStateResponse` as output.
///
/// The error is boxed to allow for dynamic error types.
pub(crate) type ReadStateServiceProcedure = Arc<
    dyn Fn(
            ReadStateRequest,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            ReadStateResponse,
                            Box<dyn std::error::Error + Send + Sync>,
                        >,
                    > + Send,
            >,
        > + Send
        + Sync,
>;

/// `TFLServiceCalls` encapsulates the service calls that can be made to the TFLService.
#[derive(Clone)]
pub struct TFLServiceCalls {
    pub(crate) read_state: ReadStateServiceProcedure,
}
impl fmt::Debug for TFLServiceCalls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TFLServiceCalls")
    }
}

/// A wrapper around the `TFLServiceInternal` and `TFLServiceCalls` types, used to manage
/// the internal state of the TFLService and the service calls that can be made to it.
#[derive(Clone, Debug)]
pub struct TFLServiceHandle {
    pub(crate) internal: Arc<Mutex<TFLServiceInternal>>,
    pub(crate) call: TFLServiceCalls,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tower::Service;

    use zebra_state::ReadResponse as ReadStateResponse;

    use crate::BlockHeight;

    // Helper function to create a test TFLServiceHandle
    fn create_test_service() -> TFLServiceHandle {
        let internal = Arc::new(Mutex::new(TFLServiceInternal {
            val: 0,
            latest_final_hash: BlockHash([0_u8; 32]),
            latest_final_height: BlockHeight(0),
            is_tfl_activated: false,
            final_change_tx: broadcast::channel(16).0,
        }));

        let read_state_service: ReadStateServiceProcedure =
            Arc::new(|_req| Box::pin(async { Ok(ReadStateResponse::Tip(None)) }));

        TFLServiceHandle {
            internal,
            call: TFLServiceCalls {
                read_state: read_state_service,
            },
        }
    }

    #[tokio::test]
    async fn crosslink_returns_none_when_no_block_hash() {
        let mut service = create_test_service();
        let response = service
            .call(TFLServiceRequest::FinalBlockHash)
            .await
            .unwrap();
        assert!(matches!(response, TFLServiceResponse::FinalBlockHash(None)));
    }

    #[tokio::test]
    async fn crosslink_final_block_rx() {
        let mut service = create_test_service();

        // Subscribe to the final block hash updates
        let response = service.call(TFLServiceRequest::FinalBlockRx).await.unwrap();

        if let TFLServiceResponse::FinalBlockRx(mut receiver) = response {
            // Simulate a final block change
            let new_hash = BlockHash([1; 32]);
            {
                let internal = service.internal.lock().await;
                let _ = internal.final_change_tx.send(new_hash);
            }

            // The receiver should get the new block hash
            let received_hash = receiver.recv().await.unwrap();
            assert_eq!(received_hash, new_hash);
        } else {
            panic!("Unexpected response: {:?}", response);
        }
    }
}
