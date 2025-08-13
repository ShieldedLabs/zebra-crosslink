use std::{ops::Deref, panic::AssertUnwindSafe, sync::Weak};

use color_eyre::owo_colors::OwoColorize;
use futures::FutureExt;
use malachitebft_core_types::{NilOrVal, VoteType};
use tracing_subscriber::fmt::format::PrettyVisitor;
use zerocopy::IntoBytes;

use crate::malctx::malctx_schema_proto::ProposedValue;

use super::*;
use zebra_chain::serialization::{
    ReadZcashExt, SerializationError, ZcashDeserialize, ZcashSerialize,
};

/*

TODO LIST
DONE 1. Make sure multiplayer works.
DONE 2. Remove streaming and massively simplify how payloads are communicated. And also refactor the MalValue to just be an array of bytes.
DONE 3. Sledgehammer in malachite to allow the vote extension scheme that we want.
DONE 4. Define the FAT pointer type that contains a blake3hash and signatures.
DONE 5. Make sure that syncing within malachite works.
DONE 6.a. Merge BFTPayload and BFTBlock into one single structure.
6.b remove fields zooko wants removed.
7. Lock in the formats and bake out some tests.
8. See if we can forget proposals for lower rounds than the current round. Then stop at_this_height_previously_seen_proposals from being an infinitely growing array.
9. Remove redundant integrity hash from the "streamed" proposal.
10. Hide noisy malachite logs
9. Sign the proposal data rather the hash

*/

pub struct RunningMalachite {
    pub should_terminate: bool,

    pub height: u64,
    pub round: u32,
    pub validator_set: Vec<MalValidator>,
}

pub async fn start_malachite(
    tfl_handle: TFLServiceHandle,
    at_height: u64,
    validators_at_height: Vec<MalValidator>,
    my_private_key: MalPrivateKey,
    bft_config: BFTConfig,
) -> Arc<TokioMutex<RunningMalachite>> {
    start_malachite_with_start_delay(
        tfl_handle,
        at_height,
        validators_at_height,
        my_private_key,
        bft_config,
        Duration::from_secs(0),
    )
    .await
}
pub async fn start_malachite_with_start_delay(
    tfl_handle: TFLServiceHandle,
    at_height: u64,
    validators_at_height: Vec<MalValidator>,
    my_private_key: MalPrivateKey,
    bft_config: BFTConfig,
    start_delay: Duration,
) -> Arc<TokioMutex<RunningMalachite>> {
    let ctx = MalContext {};
    let codec = MalProtobufCodec;
    let bft_node_handle = BFTNode {
        private_key: my_private_key.clone(),
    };

    {
        let mut td = TEMP_DIR_FOR_WAL.lock().unwrap();
        *td = Some(
            tempfile::Builder::new()
                .prefix(&format!(
                    "aah_very_annoying_that_the_wal_is_required_id_is_{}",
                    rand::rngs::OsRng.next_u64()
                ))
                .tempdir()
                .unwrap(),
        );
    }

    let arc_handle = Arc::new(TokioMutex::new(RunningMalachite {
        should_terminate: false,

        height: at_height,
        round: 0,
        validator_set: validators_at_height.clone(),
    }));

    let weak_self = Arc::downgrade(&arc_handle);
    tokio::spawn(async move {
        tokio::time::sleep(start_delay).await;
        let (channels, engine_handle) = malachitebft_app_channel::start_engine(
            ctx,
            codec,
            bft_node_handle,
            bft_config,
            Some(MalHeight::new(at_height)),
            MalValidatorSet {
                validators: validators_at_height,
            },
        )
        .await
        .unwrap();

        match AssertUnwindSafe(malachite_system_main_loop(
            tfl_handle,
            weak_self,
            channels,
            engine_handle,
            my_private_key,
        ))
        .catch_unwind()
        .await
        {
            Ok(()) => (),
            Err(error) => {
                eprintln!("panic occurred inside mal_system.rs: {:?}", error);
                std::process::abort();
            }
        }
    });
    arc_handle
}

async fn malachite_system_terminate_engine(
    mut engine_handle: EngineHandle,
    mut post_pending_block_to_push_to_core_reply: Option<
        tokio::sync::oneshot::Sender<ConsensusMsg<MalContext>>,
    >,
    mut post_pending_block_to_push_to_core_reply_data: Option<ConsensusMsg<MalContext>>,
) {
    engine_handle.actor.stop(None);
    if let Some(reply) = post_pending_block_to_push_to_core_reply.take() {
        reply
            .send(
                post_pending_block_to_push_to_core_reply_data
                    .take()
                    .unwrap(),
            )
            .unwrap();
    }
}

async fn malachite_system_main_loop(
    tfl_handle: TFLServiceHandle,
    weak_self: Weak<TokioMutex<RunningMalachite>>,
    mut channels: Channels<MalContext>,
    mut engine_handle: EngineHandle,
    my_private_key: MalPrivateKey,
) {
    let codec = MalProtobufCodec;
    let my_public_key = (&my_private_key).into();
    let my_signing_provider = MalEd25519Provider::new(my_private_key.clone());

    let mut pending_block_to_push_to_core: Option<(BftBlock, FatPointerToBftBlock)> = None;
    let mut post_pending_block_to_push_to_core_reply: Option<
        tokio::sync::oneshot::Sender<ConsensusMsg<MalContext>>,
    > = None;
    let mut post_pending_block_to_push_to_core_reply_data: Option<ConsensusMsg<MalContext>> = None;

    let mut bft_msg_flags = 0;
    let mut at_this_height_previously_seen_proposals: Vec<
        Option<Box<MalProposedValue<MalContext>>>,
    > = Vec::new();

    let mut decided_certificates_by_height = HashMap::new();

    loop {
        let running_malachite = if let Some(arc) = weak_self.upgrade() {
            arc
        } else {
            malachite_system_terminate_engine(
                engine_handle,
                post_pending_block_to_push_to_core_reply,
                post_pending_block_to_push_to_core_reply_data,
            )
            .await;
            return;
        };

        if let Some((pending_block, fat_pointer)) = &pending_block_to_push_to_core {
            if !new_decided_bft_block_from_malachite(&tfl_handle, pending_block, fat_pointer).await
            {
                tokio::time::sleep(Duration::from_millis(800)).await;

                let mut lock = running_malachite.lock().await;
                if lock.should_terminate {
                    malachite_system_terminate_engine(
                        engine_handle,
                        post_pending_block_to_push_to_core_reply,
                        post_pending_block_to_push_to_core_reply_data,
                    )
                    .await;
                    return;
                }
                continue;
            }
            pending_block_to_push_to_core = None;
            post_pending_block_to_push_to_core_reply
                .take()
                .unwrap()
                .send(
                    post_pending_block_to_push_to_core_reply_data
                        .take()
                        .unwrap(),
                )
                .unwrap();
        }

        let mut something_to_do_maybe = None;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            ret = channels.consensus.recv() => { something_to_do_maybe = ret; }
        };

        let mut lock = running_malachite.lock().await;
        if lock.should_terminate {
            malachite_system_terminate_engine(
                engine_handle,
                post_pending_block_to_push_to_core_reply,
                post_pending_block_to_push_to_core_reply_data,
            )
            .await;
            return;
        }

        if something_to_do_maybe.is_none() {
            continue;
        }
        let app_msg = something_to_do_maybe.unwrap();

        #[allow(clippy::never_loop)]
        loop {
            match app_msg {
                BFTAppMsg::ConsensusReady { reply } => {
                    bft_msg_flags |= 1 << BFTMsgFlag::ConsensusReady as u64;
                    reply
                        .send((
                            MalHeight::new(lock.height),
                            MalValidatorSet {
                                validators: lock.validator_set.clone(),
                            },
                        ))
                        .unwrap();
                }
                BFTAppMsg::GetHistoryMinHeight { reply } => {
                    bft_msg_flags |= 1 << BFTMsgFlag::GetHistoryMinHeight as u64;
                    reply.send(MalHeight::new(1)).unwrap();
                }
                BFTAppMsg::StartedRound {
                    height,
                    round,
                    proposer,
                    reply_value,
                } => {
                    assert_eq!(height.as_u64(), lock.height);
                    let round = round.as_u32().unwrap() as u32;

                    info!(%height, %round, %proposer, "Started round");
                    bft_msg_flags |= 1 << BFTMsgFlag::StartedRound as u64;
                    lock.round = round;

                    // If we have already built or seen a value for this height and round,
                    // send it back to consensus. This may happen when we are restarting after a crash.
                    let maybe_prev_seen_block = at_this_height_previously_seen_proposals
                        .get(round as usize)
                        .cloned()
                        .flatten();
                    if let Some(proposal) = maybe_prev_seen_block {
                        info!(%height, %round, "Replaying already known proposed value: {}", proposal.value.id());
                        reply_value.send(Some(proposal.as_ref().clone())).unwrap();
                    } else {
                        reply_value.send(None).unwrap();
                    }
                }
                BFTAppMsg::GetValue {
                    height,
                    round,
                    timeout,
                    reply,
                } => {
                    bft_msg_flags |= 1 << BFTMsgFlag::GetValue as u64;
                    let round = round.as_u32().unwrap() as usize;
                    info!(%height, %round, "Consensus is requesting a value to propose. Timeout = {} ms.", timeout.as_millis());

                    if pending_block_to_push_to_core.is_some() {
                        warn!("Still waiting to push previous block.");
                        break;
                    }

                    // Here it is important that, if we have previously built a value for this height and round,
                    // we send back the very same value.
                    let maybe_prev_seen_block = at_this_height_previously_seen_proposals
                        .get(round)
                        .map(|x| x.as_ref().map(|x| x.as_ref().clone()))
                        .flatten();

                    let proposal = if let Some(proposal) = maybe_prev_seen_block {
                        info!(%height, %round, "Replaying already known proposed value (That I proposed?): {}", proposal.value.id());

                        assert_eq!(height, proposal.height);
                        assert_eq!(round, proposal.round.as_u32().unwrap() as usize);
                        let other_type = MalLocallyProposedValue {
                            height: proposal.height,
                            round: proposal.round,
                            value: proposal.value,
                        };
                        other_type
                    } else {
                        if let Some(block) =
                            propose_new_bft_block(&tfl_handle, &my_public_key).await
                        {
                            info!("Proposing block with hash: {}", block.blake3_hash());
                            if let Some(last_block) =
                                tfl_handle.internal.lock().await.bft_blocks.last()
                            {
                                debug_assert_eq!(
                                    block.previous_block_fat_ptr.points_at_block_hash(),
                                    last_block.blake3_hash()
                                );
                            }
                            let other_type: MalLocallyProposedValue<MalContext> =
                                MalLocallyProposedValue {
                                    height: height,
                                    round: MalRound::new(round as u32),
                                    value: MalValue::new_block(block),
                                };
                            {
                                // The POL round is always nil when we propose a newly built value.
                                // See L15/L18 of the Tendermint algorithm.
                                let pol_round = MalRound::Nil;

                                let other_other_type = MalProposedValue {
                                    height: other_type.height,
                                    round: other_type.round,
                                    valid_round: pol_round,
                                    proposer: MalPublicKey2(my_public_key),
                                    value: other_type.value.clone(),
                                    validity: MalValidity::Valid,
                                };
                                while at_this_height_previously_seen_proposals.len() < round + 1 {
                                    at_this_height_previously_seen_proposals.push(None);
                                }
                                at_this_height_previously_seen_proposals[round] =
                                    Some(Box::new(other_other_type));
                            }
                            other_type
                        } else {
                            warn!("Asked to propose but had nothing to offer!");
                            break;
                        }
                    };

                    reply.send(proposal.clone()).unwrap();

                    // The POL round is always nil when we propose a newly built value.
                    // See L15/L18 of the Tendermint algorithm.
                    let pol_round = MalRound::Nil;

                    // Include metadata about the proposal
                    let data_bytes = proposal.value.value_bytes;

                    let mut hasher = blake3::Hasher::new();
                    hasher.update(proposal.height.as_u64().to_le_bytes().as_slice());
                    hasher.update(proposal.round.as_i64().to_le_bytes().as_slice());
                    hasher.update(&data_bytes);
                    let hash: [u8; 32] = hasher.finalize().into();
                    let signature = my_signing_provider.sign(&hash);

                    let streamed_proposal = MalStreamedProposal {
                        height: proposal.height,
                        round: proposal.round,
                        pol_round,
                        proposer: my_public_key, // @Zooko: security orange flag: should be hash of key instead; this should only be used for lookup
                        data_bytes,
                        signature,
                    };

                    let stream_id = {
                        let mut bytes = Vec::with_capacity(size_of::<u64>() + size_of::<u32>());
                        bytes.extend_from_slice(&height.as_u64().to_be_bytes());
                        bytes.extend_from_slice(&(round as u32).to_be_bytes());
                        malachitebft_app_channel::app::types::streaming::StreamId::new(bytes.into())
                    };

                    let mut msgs = vec![
                        malachitebft_app_channel::app::types::streaming::StreamMessage::new(
                            stream_id.clone(),
                            0,
                            malachitebft_app_channel::app::streaming::StreamContent::Data(
                                streamed_proposal,
                            ),
                        ),
                        malachitebft_app_channel::app::types::streaming::StreamMessage::new(
                            stream_id,
                            1,
                            malachitebft_app_channel::app::streaming::StreamContent::Fin,
                        ),
                    ];

                    for stream_message in msgs {
                        //info!(%height, %round, "Streaming proposal part: {stream_message:?}");
                        channels
                            .network
                            .send(NetworkMsg::PublishProposalPart(stream_message))
                            .await
                            .unwrap();
                    }
                }
                BFTAppMsg::GetDecidedValue { height, reply } => {
                    bft_msg_flags |= 1 << BFTMsgFlag::GetDecidedValue as u64;

                    reply
                        .send(
                            get_historical_bft_block_at_height(&tfl_handle, height.as_u64())
                                .await
                                .map(|(block, fat_pointer)| {
                                    let vote_template = fat_pointer.get_vote_template();

                                    let certificate = MalCommitCertificate {
                                        height: vote_template.height,
                                        round: vote_template.round,
                                        value_id: MalValueId(fat_pointer.points_at_block_hash()),
                                        commit_signatures: fat_pointer
                                            .signatures
                                            .into_iter()
                                            .map(|sig| MalCommitSignature {
                                                address: MalPublicKey2(sig.public_key.into()),
                                                signature: sig.vote_signature,
                                            })
                                            .collect(),
                                    };

                                    RawDecidedValue {
                                        value_bytes: codec
                                            .encode(&MalValue {
                                                value_bytes: block
                                                    .zcash_serialize_to_vec()
                                                    .unwrap()
                                                    .into(),
                                            })
                                            .unwrap(),
                                        certificate,
                                    }
                                }),
                        )
                        .unwrap();
                }
                BFTAppMsg::ProcessSyncedValue {
                    height,
                    round,
                    proposer,
                    value_bytes,
                    reply,
                } => {
                    // ASSUMPTION: This will only generate a Decided event if the certificate is correct. Therefore the purpose here is only to decode the value.
                    // We should therefore be able to pospone validation until after decided. Should a catastrophe occur malachite can be restarted.

                    info!(%height, %round, "Processing synced value");
                    bft_msg_flags |= 1 << BFTMsgFlag::ProcessSyncedValue as u64;

                    let mal_value: MalValue = codec.decode(value_bytes).unwrap();
                    let mut value: MalProposedValue<MalContext> = MalProposedValue {
                        height,
                        round,
                        valid_round: MalRound::Nil,
                        proposer,
                        value: mal_value,
                        validity: MalValidity::Valid,
                    };
                    if value.height.as_u64() != lock.height {
                        // Omg, we have to reject blocks ahead of the current height.
                        value.validity = MalValidity::Invalid;
                    } else {
                        lock.round = value.round.as_u32().unwrap();
                        while at_this_height_previously_seen_proposals.len()
                            < lock.round as usize + 1
                        {
                            at_this_height_previously_seen_proposals.push(None);
                        }
                        at_this_height_previously_seen_proposals[lock.round as usize] =
                            Some(Box::new(value.clone()));
                    }
                    reply.send(value).unwrap();
                }
                BFTAppMsg::GetValidatorSet { height, reply } => {
                    bft_msg_flags |= 1 << BFTMsgFlag::GetValidatorSet as u64;
                    if height.as_u64() == lock.height {
                        reply
                            .send(MalValidatorSet {
                                validators: lock.validator_set.clone(),
                            })
                            .unwrap();
                    }
                }
                BFTAppMsg::Decided {
                    certificate,
                    extensions,
                    reply,
                } => {
                    info!(
                        height = %certificate.height,
                        round = %certificate.round,
                        block_hash = %certificate.value_id,
                        "Consensus has decided on value"
                    );
                    assert_eq!(lock.height, certificate.height.as_u64());
                    assert_eq!(lock.round as i64, certificate.round.as_i64());
                    bft_msg_flags |= 1 << BFTMsgFlag::Decided as u64;

                    let fat_pointer = FatPointerToBftBlock::from(&certificate);
                    info!("Fat pointer to tip is now: {}", fat_pointer);
                    assert!(fat_pointer.validate_signatures());
                    assert_eq!(certificate.value_id.0, fat_pointer.points_at_block_hash());

                    let decided_value = at_this_height_previously_seen_proposals
                        [lock.round as usize]
                        .take()
                        .unwrap();
                    assert_eq!(decided_value.value.id(), certificate.value_id);

                    assert!(decided_certificates_by_height
                        .insert(lock.height, certificate)
                        .is_none());

                    assert!(pending_block_to_push_to_core.is_none());
                    pending_block_to_push_to_core = Some((
                        BftBlock::zcash_deserialize(&*decided_value.value.value_bytes)
                            .expect("infallible"),
                        fat_pointer,
                    ));

                    lock.height += 1;
                    lock.round = 0;
                    at_this_height_previously_seen_proposals.clear();

                    post_pending_block_to_push_to_core_reply = Some(reply);
                    post_pending_block_to_push_to_core_reply_data =
                        Some(malachitebft_app_channel::ConsensusMsg::StartHeight(
                            MalHeight::new(lock.height),
                            MalValidatorSet {
                                validators: lock.validator_set.clone(),
                            },
                        ));
                }
                BFTAppMsg::ExtendVote {
                    height,
                    round,
                    value_id,
                    reply,
                } => {
                    bft_msg_flags |= 1 << BFTMsgFlag::ExtendVote as u64;
                    reply.send(Some(Bytes::new())).unwrap();
                }
                BFTAppMsg::VerifyVoteExtension {
                    height,
                    round,
                    value_id,
                    extension,
                    reply,
                } => {
                    bft_msg_flags |= 1 << BFTMsgFlag::VerifyVoteExtension as u64;
                    if extension.len() == 0 {
                        reply.send(Ok(())).unwrap();
                    } else {
                        reply
                            .send(Err(MalVoteExtensionError::InvalidVoteExtension))
                            .unwrap();
                    };
                }

                // On the receiving end of these proposal parts (ie. when we are not the proposer),
                // we need to process these parts and re-assemble the full value.
                // To this end, we store each part that we receive and assemble the full value once we
                // have all its constituent parts. Then we send that value back to consensus for it to
                // consider and vote for or against it (ie. vote `nil`), depending on its validity.
                BFTAppMsg::ReceivedProposalPart { from, part, reply } => {
                    bft_msg_flags |= 1 << BFTMsgFlag::ReceivedProposalPart as u64;
                    let sequence = part.sequence;

                    use malctx::MalStreamedProposal;
                    // Check if we have a full proposal
                    let peer_id = from;
                    let msg = part;

                    if msg.content.as_data().is_none() {
                        reply.send(None).unwrap();
                        break;
                    }
                    let parts = msg.content.as_data().unwrap();
                    let proposal_round = parts.round.as_i64();

                    info!(
                        height = %lock.height,
                        round = %lock.round,
                        part.height = %parts.height,
                        part.round = %parts.round,
                        part.sequence = %sequence,
                        "Received proposal from the network..."
                    );

                    // Check if the proposal is outdated or from the future...
                    if parts.height.as_u64() != lock.height
                        || proposal_round < lock.round as i64
                        || proposal_round > lock.round as i64 + 1
                    {
                        warn!("Outdated or future proposal, ignoring");
                        reply.send(None).unwrap();
                        break;
                    }

                    if lock
                        .validator_set
                        .iter()
                        .position(|v| v.public_key == parts.proposer)
                        .is_none()
                    {
                        warn!("Invalid proposer, ignoring");
                        reply.send(None).unwrap();
                        break;
                    }

                    // signature verification
                    let mut hasher = blake3::Hasher::new();
                    hasher.update(parts.height.as_u64().to_le_bytes().as_slice());
                    hasher.update(parts.round.as_i64().to_le_bytes().as_slice());
                    hasher.update(&parts.data_bytes);
                    let hash: [u8; 32] = hasher.finalize().into();

                    // Verify the signature
                    if my_signing_provider.verify(&hash, &parts.signature, &parts.proposer) == false
                    {
                        warn!("Invalid signature, ignoring");
                        reply.send(None).unwrap();
                        break;
                    }

                    // Re-assemble the proposal from its parts
                    let block = parts
                        .data_bytes
                        .zcash_deserialize_into::<BftBlock>()
                        .unwrap();
                    info!("Received block proposal with hash: {}", block.blake3_hash());

                    let validity = if validate_bft_block_from_malachite(&tfl_handle, &block).await {
                        MalValidity::Valid
                    } else {
                        MalValidity::Invalid
                    };

                    let value: MalProposedValue<MalContext> = MalProposedValue {
                        height: parts.height,
                        round: parts.round,
                        valid_round: parts.pol_round,
                        proposer: MalPublicKey2(parts.proposer),
                        value: MalValue::new_block(block),
                        validity,
                    };

                    info!(
                        "Storing undecided proposal {} {}",
                        value.height, value.round
                    );
                    assert_eq!(value.height.as_u64(), lock.height);
                    let proposal_round = value.round.as_u32().unwrap() as usize;
                    while at_this_height_previously_seen_proposals.len()
                        < proposal_round as usize + 1
                    {
                        at_this_height_previously_seen_proposals.push(None);
                    }
                    at_this_height_previously_seen_proposals[proposal_round as usize] =
                        Some(Box::new(value.clone()));

                    reply.send(Some(value)).unwrap();
                }
                _ => panic!("AppMsg variant not handled: {:?}", app_msg),
            }
            break;
        }

        push_new_bft_msg_flags(&tfl_handle, bft_msg_flags).await;
    }
}

/// A bundle of signed votes for a block
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)] //, serde::Serialize, serde::Deserialize)]
pub struct FatPointerToBftBlock {
    pub vote_for_block_without_finalizer_public_key: [u8; 76 - 32],
    pub signatures: Vec<FatPointerSignature>,
}

impl std::fmt::Display for FatPointerToBftBlock {
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

impl From<&MalCommitCertificate<MalContext>> for FatPointerToBftBlock {
    fn from(certificate: &MalCommitCertificate<MalContext>) -> FatPointerToBftBlock {
        let vote_template = MalVote {
            validator_address: MalPublicKey2([0_u8; 32].into()),
            value: NilOrVal::Val(certificate.value_id), // previous_block_hash
            height: certificate.height,
            typ: VoteType::Precommit,
            round: certificate.round,
        };
        let vote_for_block_without_finalizer_public_key: [u8; 76 - 32] =
            vote_template.to_bytes()[32..].try_into().unwrap();

        FatPointerToBftBlock {
            vote_for_block_without_finalizer_public_key,
            signatures: certificate
                .commit_signatures
                .iter()
                .map(|commit_signature| {
                    let public_key: MalPublicKey2 = commit_signature.address;
                    let signature: [u8; 64] = commit_signature.signature;

                    FatPointerSignature {
                        public_key: public_key.0.into(),
                        vote_signature: signature,
                    }
                })
                .collect(),
        }
    }
}

impl FatPointerToBftBlock {
    pub fn null() -> FatPointerToBftBlock {
        FatPointerToBftBlock {
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
    pub fn try_from_bytes(bytes: &Vec<u8>) -> Option<FatPointerToBftBlock> {
        if bytes.len() < 76 - 32 + 2 {
            return None;
        }
        let vote_for_block_without_finalizer_public_key = bytes[0..76 - 32].try_into().unwrap();
        let len = u16::from_le_bytes(bytes[2..76 - 32].try_into().unwrap()) as usize;

        if 76 - 32 + 2 + len * (32 + 64) > bytes.len() {
            return None;
        }
        let rem = &bytes[76 - 32 + 2..];
        let signatures = rem
            .chunks_exact(32 + 64)
            .map(|chunk| FatPointerSignature::from_bytes(chunk.try_into().unwrap()))
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

impl ZcashSerialize for FatPointerToBftBlock {
    fn zcash_serialize<W: io::Write>(&self, mut writer: W) -> Result<(), io::Error> {
        writer.write_all(&self.vote_for_block_without_finalizer_public_key);
        writer.write_u16::<LittleEndian>(self.signatures.len() as u16);
        for signature in &self.signatures {
            writer.write_all(&signature.to_bytes());
        }
        Ok(())
    }
}

impl ZcashDeserialize for FatPointerToBftBlock {
    fn zcash_deserialize<R: io::Read>(mut reader: R) -> Result<Self, SerializationError> {
        let mut vote_for_block_without_finalizer_public_key = [0u8; 76 - 32];
        reader.read_exact(&mut vote_for_block_without_finalizer_public_key)?;

        let len = reader.read_u16::<LittleEndian>()?;
        let mut signatures: Vec<FatPointerSignature> = Vec::with_capacity(len.into());
        for _ in 0..len {
            let mut signature_bytes = [0u8; 32 + 64];
            reader.read_exact(&mut signature_bytes)?;
            signatures.push(FatPointerSignature::from_bytes(&signature_bytes));
        }

        Ok(FatPointerToBftBlock {
            vote_for_block_without_finalizer_public_key,
            signatures,
        })
    }
}

/// A vote signature for a block
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)] //, serde::Serialize, serde::Deserialize)]
pub struct FatPointerSignature {
    pub public_key: [u8; 32],
    pub vote_signature: [u8; 64],
}

impl FatPointerSignature {
    pub fn to_bytes(&self) -> [u8; 32 + 64] {
        let mut buf = [0_u8; 32 + 64];
        buf[0..32].copy_from_slice(&self.public_key);
        buf[32..32 + 64].copy_from_slice(&self.vote_signature);
        buf
    }
    pub fn from_bytes(bytes: &[u8; 32 + 64]) -> FatPointerSignature {
        Self {
            public_key: bytes[0..32].try_into().unwrap(),
            vote_signature: bytes[32..32 + 64].try_into().unwrap(),
        }
    }
}
