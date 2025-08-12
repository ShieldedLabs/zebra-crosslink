#![allow(unexpected_cfgs, unused, missing_docs)]
use std::io::Read;
use std::sync::Arc;

use bytes::{Bytes, BytesMut};

use malachitebft_core_types::SigningScheme;
use malachitebft_core_types::{
    Context, Extension, NilOrVal, Round, SignedExtension, ValidatorSet as _, VoteType, VotingPower,
};

use serde::{Deserialize, Serialize};

use malachitebft_proto::{Error as ProtoError, Protobuf};
use zebra_chain::serialization::{ZcashDeserialize, ZcashDeserializeInto, ZcashSerialize};

use core::fmt;

use malachitebft_core_types::{
    CertificateError, CommitCertificate, CommitSignature, PolkaSignature, SignedProposal,
    SignedProposalPart, SignedVote, SigningProvider, Validator as _,
};

pub use malachitebft_app::types::Keypair as MalKeyPair;
pub use malachitebft_core_consensus::{
    LocallyProposedValue as MalLocallyProposedValue, ProposedValue as MalProposedValue,
    VoteExtensionError as MalVoteExtensionError,
};
pub use malachitebft_core_types::{
    CommitCertificate as MalCommitCertificate, CommitSignature as MalCommitSignature,
    Round as MalRound, Validity as MalValidity, VoteExtensions as MalVoteExtensions,
};

pub use ed25519_zebra::ed25519::SignatureBytes;
pub use ed25519_zebra::Signature;
pub use ed25519_zebra::SigningKey as MalPrivateKey;
pub use ed25519_zebra::VerificationKeyBytes as MalPublicKey;

use prost::Message;

use malachitebft_app::engine::util::streaming::{StreamContent, StreamId, StreamMessage};
pub use malachitebft_codec::Codec;
use malachitebft_core_consensus::SignedConsensusMsg;
use malachitebft_core_types::{PolkaCertificate, VoteSet};
use malachitebft_sync::PeerId;

pub use malachitebft_app::node::EngineHandle;
pub use malachitebft_app_channel::app::config as mconfig;
pub use malachitebft_app_channel::app::events::RxEvent;
pub use malachitebft_app_channel::app::node::NodeConfig;
pub use malachitebft_app_channel::app::types::sync::RawDecidedValue;
pub use malachitebft_app_channel::AppMsg as BFTAppMsg;
pub use malachitebft_app_channel::Channels;
pub use malachitebft_app_channel::ConsensusMsg;
pub use malachitebft_app_channel::NetworkMsg;

use super::{BftBlock, Blake3Hash};

#[derive(Serialize, Deserialize)]
#[serde(remote = "Round")]
enum RoundDef {
    Nil,
    Some(u32),
}

/// A part of a value for a height, round. Identified in this scope by the sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MalStreamedProposal {
    pub height: MalHeight,
    #[serde(with = "RoundDef")]
    pub round: Round,
    #[serde(with = "RoundDef")]
    pub pol_round: Round,
    pub proposer: MalPublicKey,
    pub data_bytes: Vec<u8>,
    pub signature: Signature,
}

impl MalStreamedProposal {
    pub fn new(
        height: MalHeight,
        round: Round,
        pol_round: Round,
        proposer: MalPublicKey,
        data_bytes: Vec<u8>,
        signature: Signature,
    ) -> Self {
        Self {
            height,
            round,
            pol_round,
            proposer,
            data_bytes,
            signature,
        }
    }

    pub fn to_sign_bytes(&self) -> Bytes {
        malachitebft_proto::Protobuf::to_bytes(self).unwrap()
    }
}

impl malachitebft_core_types::ProposalPart<MalContext> for MalStreamedProposal {
    fn is_first(&self) -> bool {
        true
    }

    fn is_last(&self) -> bool {
        true
    }
}

impl Protobuf for MalStreamedProposal {
    type Proto = malctx_schema_proto::StreamedProposal;

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn from_proto(proto: Self::Proto) -> Result<Self, ProtoError> {
        Ok(MalStreamedProposal {
            height: MalHeight::new(proto.height),
            round: Round::new(proto.round),
            pol_round: Round::from(proto.pol_round),
            proposer: From::<[u8; 32]>::from(
                proto
                    .proposer
                    .as_ref()
                    .try_into()
                    .or_else(|_| Err(ProtoError::missing_field::<Self::Proto>("proposer")))?,
            ),
            data_bytes: proto.data_bytes.to_vec(),
            signature: proto
                .signature
                .ok_or_else(|| ProtoError::missing_field::<Self::Proto>("signature"))
                .and_then(mal_decode_signature)?,
        })
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn to_proto(&self) -> Result<Self::Proto, ProtoError> {
        Ok(Self::Proto {
            height: self.height.as_u64(),
            round: self.round.as_u32().unwrap(),
            pol_round: self.pol_round.as_u32(),
            proposer: self.proposer.as_ref().to_vec().into(),
            data_bytes: self.data_bytes.clone().into(),
            signature: Some(mal_encode_signature(&self.signature)),
        })
    }
}

/// A blockchain height
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MalHeight(u64);

impl MalHeight {
    pub const fn new(height: u64) -> Self {
        Self(height)
    }

    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    pub fn increment(&self) -> Self {
        Self(self.0 + 1)
    }

    pub fn decrement(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl Default for MalHeight {
    fn default() -> Self {
        malachitebft_core_types::Height::ZERO
    }
}

impl fmt::Display for MalHeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for MalHeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MalHeight({})", self.0)
    }
}

impl malachitebft_core_types::Height for MalHeight {
    const ZERO: Self = Self(0);
    const INITIAL: Self = Self(1);

    fn increment_by(&self, n: u64) -> Self {
        Self(self.0 + n)
    }

    fn decrement_by(&self, n: u64) -> Option<Self> {
        Some(Self(self.0.saturating_sub(n)))
    }

    fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Protobuf for MalHeight {
    type Proto = u64;

    fn from_proto(proto: Self::Proto) -> Result<Self, ProtoError> {
        Ok(Self(proto))
    }

    fn to_proto(&self) -> Result<Self::Proto, ProtoError> {
        Ok(self.0)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MalPublicKey2(pub MalPublicKey);

impl fmt::Display for MalPublicKey2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.as_ref() {
            write!(f, "{:02X}", byte)?;
        }
        Ok(())
    }
}

impl fmt::Debug for MalPublicKey2 {
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

impl malachitebft_core_types::Address for MalPublicKey2 {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Copy, Serialize, Deserialize)]
pub struct MalValueId(pub Blake3Hash);

impl std::fmt::Display for MalValueId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Protobuf for MalValueId {
    type Proto = malctx_schema_proto::ValueId;

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn from_proto(proto: Self::Proto) -> Result<Self, ProtoError> {
        let bytes = proto
            .value
            .ok_or_else(|| ProtoError::missing_field::<Self::Proto>("value"))?;

        let len = bytes.len();
        let bytes = <[u8; 32]>::try_from(bytes.as_ref()).map_err(|_| {
            ProtoError::Other(format!(
                "Invalid value id bytes length, got {len} bytes expected {}",
                32
            ))
        })?;

        Ok(MalValueId(Blake3Hash(bytes)))
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn to_proto(&self) -> Result<Self::Proto, ProtoError> {
        Ok(malctx_schema_proto::ValueId {
            value: Some(self.0 .0.to_vec().into()),
        })
    }
}

/// The value to decide on
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MalValue {
    pub value_bytes: Vec<u8>, // BftBlock,
}

impl MalValue {
    pub fn new_block(value: BftBlock) -> Self {
        Self {
            value_bytes: value.zcash_serialize_to_vec().unwrap(),
        }
    }

    pub fn id(&self) -> MalValueId {
        MalValueId(Blake3Hash(blake3::hash(&self.value_bytes).into()))
    }
}

impl malachitebft_core_types::Value for MalValue {
    type Id = MalValueId;

    fn id(&self) -> MalValueId {
        self.id()
    }
}

impl Protobuf for MalValue {
    type Proto = malctx_schema_proto::Value;

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn from_proto(proto: Self::Proto) -> Result<Self, ProtoError> {
        let value_bytes = proto
            .value
            .ok_or_else(|| ProtoError::missing_field::<Self::Proto>("value"))?;

        Ok(MalValue {
            value_bytes: value_bytes.to_vec(),
        })
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn to_proto(&self) -> Result<Self::Proto, ProtoError> {
        use bytes::BufMut;

        Ok(malctx_schema_proto::Value {
            value: Some(self.value_bytes.clone().into()),
        })
    }
}

#[derive(Debug)]
pub struct MalEd25519Provider {
    private_key: MalPrivateKey,
}

impl MalEd25519Provider {
    pub fn new(private_key: MalPrivateKey) -> Self {
        Self { private_key }
    }

    pub fn sign(&self, data: &[u8]) -> Signature {
        self.private_key.sign(data)
    }

    pub fn verify(&self, data: &[u8], signature: &Signature, public_key: &MalPublicKey) -> bool {
        ed25519_zebra::VerificationKey::try_from(public_key.clone())
            .and_then(|vk| vk.verify(signature, data))
            .is_ok()
    }
}

impl SigningProvider<MalContext> for MalEd25519Provider {
    fn sign_vote(&self, vote: MalVote) -> SignedVote<MalContext> {
        let signature = self.sign(&vote.to_bytes()).to_bytes();
        SignedVote::new(vote, signature)
    }

    fn verify_signed_vote(
        &self,
        vote: &MalVote,
        signature: &SignatureBytes,
        public_key: &MalPublicKey,
    ) -> bool {
        self.verify(
            &vote.to_bytes(),
            &Signature::from_bytes(signature),
            public_key,
        )
    }

    fn sign_proposal(&self, proposal: MalProposal) -> SignedProposal<MalContext> {
        let signature = self.sign(&proposal.to_sign_bytes()).to_bytes();
        SignedProposal::new(proposal, signature)
    }

    fn verify_signed_proposal(
        &self,
        proposal: &MalProposal,
        signature: &SignatureBytes,
        public_key: &MalPublicKey,
    ) -> bool {
        self.verify(
            &proposal.to_sign_bytes(),
            &Signature::from_bytes(signature),
            public_key,
        )
    }

    fn sign_proposal_part(
        &self,
        proposal_part: MalStreamedProposal,
    ) -> SignedProposalPart<MalContext> {
        let signature = self.sign(&proposal_part.to_sign_bytes()).to_bytes();
        SignedProposalPart::new(proposal_part, signature)
    }

    fn verify_signed_proposal_part(
        &self,
        proposal_part: &MalStreamedProposal,
        signature: &SignatureBytes,
        public_key: &MalPublicKey,
    ) -> bool {
        self.verify(
            &proposal_part.to_sign_bytes(),
            &Signature::from_bytes(signature),
            public_key,
        )
    }

    fn sign_vote_extension(&self, extension: Bytes) -> SignedExtension<MalContext> {
        let signature = self.sign(extension.as_ref()).to_bytes();
        malachitebft_core_types::SignedMessage::new(extension, signature)
    }

    fn verify_signed_vote_extension(
        &self,
        extension: &Bytes,
        signature: &SignatureBytes,
        public_key: &MalPublicKey,
    ) -> bool {
        self.verify(&extension, &Signature::from_bytes(signature), public_key)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MalEd25519SigningScheme;

impl SigningScheme for MalEd25519SigningScheme {
    type DecodingError = &'static str;
    type Signature = ed25519_zebra::ed25519::SignatureBytes;
    type PublicKey = MalPublicKey;
    type PrivateKey = MalPrivateKey;

    /// Decode a signature from a byte array.
    fn decode_signature(bytes: &[u8]) -> Result<Self::Signature, Self::DecodingError> {
        bytes.try_into().map_err(|_| "The size was not 64 bytes.")
    }

    /// Encode a signature to a byte array.
    fn encode_signature(signature: &Self::Signature) -> Vec<u8> {
        signature.to_vec()
    }
}

/// A validator is a public key and voting power
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MalValidator {
    pub address: MalPublicKey2,
    pub public_key: MalPublicKey,
    pub voting_power: VotingPower,
}

impl MalValidator {
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn new(public_key: MalPublicKey, voting_power: VotingPower) -> Self {
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

impl malachitebft_core_types::Validator<MalContext> for MalValidator {
    fn address(&self) -> &MalPublicKey2 {
        &self.address
    }

    fn public_key(&self) -> &MalPublicKey {
        &self.public_key
    }

    fn voting_power(&self) -> VotingPower {
        self.voting_power
    }
}

/// A validator set contains a list of validators sorted by address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MalValidatorSet {
    pub validators: Vec<MalValidator>,
}

impl MalValidatorSet {
    pub fn new(validators: impl IntoIterator<Item = MalValidator>) -> Self {
        let mut validators: Vec<_> = validators.into_iter().collect();
        MalValidatorSet::sort_validators(&mut validators);

        assert!(!validators.is_empty());

        Self { validators }
    }

    /// The total voting power of the validator set
    pub fn total_voting_power(&self) -> VotingPower {
        self.validators.iter().map(|v| v.voting_power).sum()
    }

    /// Get a validator by its address
    pub fn get_by_address(&self, address: &MalPublicKey) -> Option<&MalValidator> {
        self.validators.iter().find(|v| &v.address.0 == address)
    }

    pub fn get_by_public_key(&self, public_key: &MalPublicKey) -> Option<&MalValidator> {
        self.validators.iter().find(|v| &v.public_key == public_key)
    }

    /// In place sort and deduplication of a list of validators
    fn sort_validators(vals: &mut Vec<MalValidator>) {
        // Sort the validators according to the current Tendermint requirements
        //
        // use core::cmp::Reverse;
        //
        // (v. 0.34 -> first by validator power, descending, then by address, ascending)
        // vals.sort_unstable_by(|v1, v2| {
        //     let a = (Reverse(v1.voting_power), &v1.address);
        //     let b = (Reverse(v2.voting_power), &v2.address);
        //     a.cmp(&b)
        // });

        vals.dedup();
    }

    pub fn get_keys(&self) -> Vec<MalPublicKey> {
        self.validators.iter().map(|v| v.public_key).collect()
    }
}

impl malachitebft_core_types::ValidatorSet<MalContext> for MalValidatorSet {
    fn count(&self) -> usize {
        self.validators.len()
    }

    fn total_voting_power(&self) -> VotingPower {
        self.total_voting_power()
    }

    fn get_by_address(&self, address: &MalPublicKey2) -> Option<&MalValidator> {
        self.get_by_address(&address.0)
    }

    fn get_by_index(&self, index: usize) -> Option<&MalValidator> {
        self.validators.get(index)
    }
}

/// A proposal for a value in a round
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MalProposal {
    pub height: MalHeight,
    pub round: Round,
    pub value: MalValue,
    pub pol_round: Round,
    pub validator_address: MalPublicKey2,
}

impl MalProposal {
    pub fn to_bytes(&self) -> Bytes {
        Protobuf::to_bytes(self).unwrap()
    }

    pub fn to_sign_bytes(&self) -> Bytes {
        Protobuf::to_bytes(self).unwrap()
    }
}

impl malachitebft_core_types::Proposal<MalContext> for MalProposal {
    fn height(&self) -> MalHeight {
        self.height
    }

    fn round(&self) -> Round {
        self.round
    }

    fn value(&self) -> &MalValue {
        &self.value
    }

    fn take_value(self) -> MalValue {
        self.value
    }

    fn pol_round(&self) -> Round {
        self.pol_round
    }

    fn validator_address(&self) -> &MalPublicKey2 {
        &self.validator_address
    }
}

impl Protobuf for MalProposal {
    type Proto = malctx_schema_proto::Proposal;

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn to_proto(&self) -> Result<Self::Proto, ProtoError> {
        Ok(Self::Proto {
            height: self.height.to_proto()?,
            round: self.round.as_u32().expect("round should not be nil"),
            value: Some(self.value.to_proto()?),
            pol_round: self.pol_round.as_u32(),
            validator_address: self.validator_address.0.as_ref().to_vec().into(),
        })
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn from_proto(proto: Self::Proto) -> Result<Self, ProtoError> {
        Ok(Self {
            height: MalHeight::from_proto(proto.height)?,
            round: Round::new(proto.round),
            value: MalValue::from_proto(
                proto
                    .value
                    .ok_or_else(|| ProtoError::missing_field::<Self::Proto>("value"))?,
            )?,
            pol_round: Round::from(proto.pol_round),
            validator_address: MalPublicKey2(From::<[u8; 32]>::from(
                proto.validator_address.as_ref().try_into().or_else(|_| {
                    Err(ProtoError::missing_field::<Self::Proto>(
                        "validator_address",
                    ))
                })?,
            )),
        })
    }
}

pub mod malctx_schema_proto {
    #![allow(missing_docs)]

    include!(concat!(env!("OUT_DIR"), "/mal_schema.rs"));
}

#[derive(Copy, Clone, Debug)]
pub struct MalProtobufCodec;

impl Codec<MalValue> for MalProtobufCodec {
    type Error = ProtoError;

    fn decode(&self, bytes: Bytes) -> Result<MalValue, Self::Error> {
        Protobuf::from_bytes(&bytes)
    }

    fn encode(&self, msg: &MalValue) -> Result<Bytes, Self::Error> {
        Protobuf::to_bytes(msg)
    }
}

impl Codec<MalStreamedProposal> for MalProtobufCodec {
    type Error = ProtoError;

    fn decode(&self, bytes: Bytes) -> Result<MalStreamedProposal, Self::Error> {
        Protobuf::from_bytes(&bytes)
    }

    fn encode(&self, msg: &MalStreamedProposal) -> Result<Bytes, Self::Error> {
        Protobuf::to_bytes(msg)
    }
}

impl Codec<Signature> for MalProtobufCodec {
    type Error = ProtoError;

    fn decode(&self, bytes: Bytes) -> Result<Signature, Self::Error> {
        let proto = malctx_schema_proto::Signature::decode(bytes.as_ref())?;
        mal_decode_signature(proto)
    }

    fn encode(&self, msg: &Signature) -> Result<Bytes, Self::Error> {
        Ok(Bytes::from(
            malctx_schema_proto::Signature {
                bytes: Bytes::copy_from_slice(msg.to_bytes().as_ref()),
            }
            .encode_to_vec(),
        ))
    }
}

impl Codec<SignedConsensusMsg<MalContext>> for MalProtobufCodec {
    type Error = ProtoError;

    fn decode(&self, bytes: Bytes) -> Result<SignedConsensusMsg<MalContext>, Self::Error> {
        let proto = malctx_schema_proto::SignedMessage::decode(bytes.as_ref())?;

        let signature = proto
            .signature
            .ok_or_else(|| {
                ProtoError::missing_field::<malctx_schema_proto::SignedMessage>("signature")
            })
            .and_then(mal_decode_signature)?
            .to_bytes();

        let proto_message = proto.message.ok_or_else(|| {
            ProtoError::missing_field::<malctx_schema_proto::SignedMessage>("message")
        })?;

        match proto_message {
            malctx_schema_proto::signed_message::Message::Proposal(proto) => {
                let proposal = MalProposal::from_proto(proto)?;
                Ok(SignedConsensusMsg::Proposal(SignedProposal::new(
                    proposal, signature,
                )))
            }
            malctx_schema_proto::signed_message::Message::Vote(vote) => {
                let vote = MalVote::from_bytes(&vote.as_ref().try_into().or_else(|_| {
                    Err(ProtoError::missing_field::<
                        malctx_schema_proto::SignedMessage,
                    >("vote"))
                })?);
                Ok(SignedConsensusMsg::Vote(SignedVote::new(vote, signature)))
            }
        }
    }

    fn encode(&self, msg: &SignedConsensusMsg<MalContext>) -> Result<Bytes, Self::Error> {
        match msg {
            SignedConsensusMsg::Vote(vote) => {
                let proto = malctx_schema_proto::SignedMessage {
                    message: Some(malctx_schema_proto::signed_message::Message::Vote(
                        vote.message.to_bytes().to_vec().into(),
                    )),
                    signature: Some(mal_encode_signature(&Signature::from_bytes(
                        &vote.signature,
                    ))),
                };
                Ok(Bytes::from(proto.encode_to_vec()))
            }
            SignedConsensusMsg::Proposal(proposal) => {
                let proto = malctx_schema_proto::SignedMessage {
                    message: Some(malctx_schema_proto::signed_message::Message::Proposal(
                        proposal.message.to_proto()?,
                    )),
                    signature: Some(mal_encode_signature(&Signature::from_bytes(
                        &proposal.signature,
                    ))),
                };
                Ok(Bytes::from(proto.encode_to_vec()))
            }
        }
    }
}

impl Codec<StreamMessage<MalStreamedProposal>> for MalProtobufCodec {
    type Error = ProtoError;

    fn decode(&self, bytes: Bytes) -> Result<StreamMessage<MalStreamedProposal>, Self::Error> {
        let proto = malctx_schema_proto::StreamMessage::decode(bytes.as_ref())?;

        let proto_content = proto.content.ok_or_else(|| {
            ProtoError::missing_field::<malctx_schema_proto::StreamMessage>("content")
        })?;

        let content = match proto_content {
            malctx_schema_proto::stream_message::Content::Data(data) => {
                StreamContent::Data(MalStreamedProposal::from_bytes(&data)?)
            }
            malctx_schema_proto::stream_message::Content::Fin(_) => StreamContent::Fin,
        };

        Ok(StreamMessage {
            stream_id: StreamId::new(proto.stream_id),
            sequence: proto.sequence,
            content,
        })
    }

    fn encode(&self, msg: &StreamMessage<MalStreamedProposal>) -> Result<Bytes, Self::Error> {
        let proto = malctx_schema_proto::StreamMessage {
            stream_id: msg.stream_id.to_bytes(),
            sequence: msg.sequence,
            content: match &msg.content {
                StreamContent::Data(data) => Some(
                    malctx_schema_proto::stream_message::Content::Data(data.to_bytes()?),
                ),
                StreamContent::Fin => Some(malctx_schema_proto::stream_message::Content::Fin(true)),
            },
        };

        Ok(Bytes::from(proto.encode_to_vec()))
    }
}

impl Codec<MalProposedValue<MalContext>> for MalProtobufCodec {
    type Error = ProtoError;

    fn decode(&self, bytes: Bytes) -> Result<MalProposedValue<MalContext>, Self::Error> {
        let proto = malctx_schema_proto::ProposedValue::decode(bytes.as_ref())?;

        let value = proto.value.ok_or_else(|| {
            ProtoError::missing_field::<malctx_schema_proto::ProposedValue>("value")
        })?;

        Ok(MalProposedValue {
            height: MalHeight::new(proto.height),
            round: Round::new(proto.round),
            valid_round: proto.valid_round.map(Round::new).unwrap_or(Round::Nil),
            proposer: MalPublicKey2(From::<[u8; 32]>::from(
                proto.proposer.as_ref().to_vec().try_into().or_else(|_| {
                    Err(ProtoError::missing_field::<
                        malctx_schema_proto::ProposedValue,
                    >("proposer"))
                })?,
            )),
            value: MalValue::from_proto(value)?,
            validity: MalValidity::from_bool(proto.validity),
        })
    }

    fn encode(&self, msg: &MalProposedValue<MalContext>) -> Result<Bytes, Self::Error> {
        let proto = malctx_schema_proto::ProposedValue {
            height: msg.height.as_u64(),
            round: msg.round.as_u32().unwrap(),
            valid_round: msg.valid_round.as_u32(),
            proposer: msg.proposer.0.as_ref().to_vec().into(),
            value: Some(msg.value.to_proto()?),
            validity: msg.validity.to_bool(),
        };

        Ok(Bytes::from(proto.encode_to_vec()))
    }
}

impl Codec<malachitebft_sync::Status<MalContext>> for MalProtobufCodec {
    type Error = ProtoError;

    fn decode(&self, bytes: Bytes) -> Result<malachitebft_sync::Status<MalContext>, Self::Error> {
        let proto = malctx_schema_proto::Status::decode(bytes.as_ref())?;

        let proto_peer_id = proto
            .peer_id
            .ok_or_else(|| ProtoError::missing_field::<malctx_schema_proto::Status>("peer_id"))?;

        Ok(malachitebft_sync::Status {
            peer_id: PeerId::from_bytes(proto_peer_id.id.as_ref()).unwrap(),
            tip_height: MalHeight::new(proto.height),
            history_min_height: MalHeight::new(proto.earliest_height),
        })
    }

    fn encode(&self, msg: &malachitebft_sync::Status<MalContext>) -> Result<Bytes, Self::Error> {
        let proto = malctx_schema_proto::Status {
            peer_id: Some(malctx_schema_proto::PeerId {
                id: Bytes::from(msg.peer_id.to_bytes()),
            }),
            height: msg.tip_height.as_u64(),
            earliest_height: msg.history_min_height.as_u64(),
        };

        Ok(Bytes::from(proto.encode_to_vec()))
    }
}

impl Codec<malachitebft_sync::Request<MalContext>> for MalProtobufCodec {
    type Error = ProtoError;

    fn decode(&self, bytes: Bytes) -> Result<malachitebft_sync::Request<MalContext>, Self::Error> {
        let proto = malctx_schema_proto::SyncRequest::decode(bytes.as_ref())?;
        let request = proto.request.ok_or_else(|| {
            ProtoError::missing_field::<malctx_schema_proto::SyncRequest>("request")
        })?;

        match request {
            malctx_schema_proto::sync_request::Request::ValueRequest(req) => {
                Ok(malachitebft_sync::Request::ValueRequest(
                    malachitebft_sync::ValueRequest::new(MalHeight::new(req.height)),
                ))
            }
            malctx_schema_proto::sync_request::Request::VoteSetRequest(req) => Ok(
                malachitebft_sync::Request::VoteSetRequest(malachitebft_sync::VoteSetRequest::new(
                    MalHeight::new(req.height),
                    Round::new(req.round),
                )),
            ),
        }
    }

    fn encode(&self, msg: &malachitebft_sync::Request<MalContext>) -> Result<Bytes, Self::Error> {
        let proto = match msg {
            malachitebft_sync::Request::ValueRequest(req) => malctx_schema_proto::SyncRequest {
                request: Some(malctx_schema_proto::sync_request::Request::ValueRequest(
                    malctx_schema_proto::ValueRequest {
                        height: req.height.as_u64(),
                    },
                )),
            },
            malachitebft_sync::Request::VoteSetRequest(req) => malctx_schema_proto::SyncRequest {
                request: Some(malctx_schema_proto::sync_request::Request::VoteSetRequest(
                    malctx_schema_proto::VoteSetRequest {
                        height: req.height.as_u64(),
                        round: req.round.as_u32().unwrap(),
                    },
                )),
            },
        };

        Ok(Bytes::from(proto.encode_to_vec()))
    }
}

impl Codec<malachitebft_sync::Response<MalContext>> for MalProtobufCodec {
    type Error = ProtoError;

    fn decode(&self, bytes: Bytes) -> Result<malachitebft_sync::Response<MalContext>, Self::Error> {
        mal_decode_sync_response(malctx_schema_proto::SyncResponse::decode(bytes)?)
    }

    fn encode(
        &self,
        response: &malachitebft_sync::Response<MalContext>,
    ) -> Result<Bytes, Self::Error> {
        mal_encode_sync_response(response).map(|proto| proto.encode_to_vec().into())
    }
}

pub fn mal_decode_sync_response(
    proto_response: malctx_schema_proto::SyncResponse,
) -> Result<malachitebft_sync::Response<MalContext>, ProtoError> {
    let response = proto_response.response.ok_or_else(|| {
        ProtoError::missing_field::<malctx_schema_proto::SyncResponse>("messages")
    })?;

    let response = match response {
        malctx_schema_proto::sync_response::Response::ValueResponse(value_response) => {
            malachitebft_sync::Response::ValueResponse(malachitebft_sync::ValueResponse::new(
                MalHeight::new(value_response.height),
                value_response
                    .value
                    .map(mal_decode_synced_value)
                    .transpose()?,
            ))
        }
        malctx_schema_proto::sync_response::Response::VoteSetResponse(vote_set_response) => {
            let height = MalHeight::new(vote_set_response.height);
            let round = Round::new(vote_set_response.round);
            let vote_set = vote_set_response.vote_set.ok_or_else(|| {
                ProtoError::missing_field::<malctx_schema_proto::VoteSet>("vote_set")
            })?;

            malachitebft_sync::Response::VoteSetResponse(malachitebft_sync::VoteSetResponse::new(
                height,
                round,
                mal_decode_vote_set(vote_set)?,
                vote_set_response
                    .polka_certificates
                    .into_iter()
                    .map(decode_polka_certificate)
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
    };
    Ok(response)
}

pub fn mal_encode_sync_response(
    response: &malachitebft_sync::Response<MalContext>,
) -> Result<malctx_schema_proto::SyncResponse, ProtoError> {
    let proto = match response {
        malachitebft_sync::Response::ValueResponse(value_response) => {
            malctx_schema_proto::SyncResponse {
                response: Some(malctx_schema_proto::sync_response::Response::ValueResponse(
                    malctx_schema_proto::ValueResponse {
                        height: value_response.height.as_u64(),
                        value: value_response
                            .value
                            .as_ref()
                            .map(mal_encode_synced_value)
                            .transpose()?,
                    },
                )),
            }
        }
        malachitebft_sync::Response::VoteSetResponse(vote_set_response) => {
            malctx_schema_proto::SyncResponse {
                response: Some(
                    malctx_schema_proto::sync_response::Response::VoteSetResponse(
                        malctx_schema_proto::VoteSetResponse {
                            height: vote_set_response.height.as_u64(),
                            round: vote_set_response
                                .round
                                .as_u32()
                                .expect("round should not be nil"),
                            vote_set: Some(mal_encode_vote_set(&vote_set_response.vote_set)?),
                            polka_certificates: vote_set_response
                                .polka_certificates
                                .iter()
                                .map(encode_polka_certificate)
                                .collect::<Result<Vec<_>, _>>()?,
                        },
                    ),
                ),
            }
        }
    };

    Ok(proto)
}

pub fn mal_encode_synced_value(
    synced_value: &malachitebft_sync::RawDecidedValue<MalContext>,
) -> Result<malctx_schema_proto::SyncedValue, ProtoError> {
    Ok(malctx_schema_proto::SyncedValue {
        value_bytes: synced_value.value_bytes.clone(),
        certificate: Some(mal_encode_commit_certificate(&synced_value.certificate)?),
    })
}

pub fn mal_decode_synced_value(
    proto: malctx_schema_proto::SyncedValue,
) -> Result<malachitebft_sync::RawDecidedValue<MalContext>, ProtoError> {
    let certificate = proto.certificate.ok_or_else(|| {
        ProtoError::missing_field::<malctx_schema_proto::SyncedValue>("certificate")
    })?;

    Ok(malachitebft_sync::RawDecidedValue {
        value_bytes: proto.value_bytes,
        certificate: mal_decode_commit_certificate(certificate)?,
    })
}

pub(crate) fn encode_polka_certificate(
    polka_certificate: &PolkaCertificate<MalContext>,
) -> Result<malctx_schema_proto::PolkaCertificate, ProtoError> {
    Ok(malctx_schema_proto::PolkaCertificate {
        height: polka_certificate.height.as_u64(),
        round: polka_certificate
            .round
            .as_u32()
            .expect("round should not be nil"),
        value_id: Some(polka_certificate.value_id.to_proto()?),
        signatures: polka_certificate
            .polka_signatures
            .iter()
            .map(
                |sig| -> Result<malctx_schema_proto::PolkaSignature, ProtoError> {
                    let signature = mal_encode_signature(&Signature::from_bytes(&sig.signature));
                    Ok(malctx_schema_proto::PolkaSignature {
                        validator_address: sig.address.0.as_ref().to_vec().into(),
                        signature: Some(signature),
                    })
                },
            )
            .collect::<Result<Vec<_>, _>>()?,
    })
}

pub(crate) fn decode_polka_certificate(
    certificate: malctx_schema_proto::PolkaCertificate,
) -> Result<PolkaCertificate<MalContext>, ProtoError> {
    let value_id = certificate
        .value_id
        .ok_or_else(|| {
            ProtoError::missing_field::<malctx_schema_proto::PolkaCertificate>("value_id")
        })
        .and_then(MalValueId::from_proto)?;

    Ok(PolkaCertificate {
        height: MalHeight::new(certificate.height),
        round: Round::new(certificate.round),
        value_id,
        polka_signatures: certificate
            .signatures
            .into_iter()
            .map(|sig| -> Result<PolkaSignature<MalContext>, ProtoError> {
                let signature = sig.signature.ok_or_else(|| {
                    ProtoError::missing_field::<malctx_schema_proto::PolkaCertificate>("signature")
                })?;
                let signature = mal_decode_signature(signature)?.to_bytes();
                let address = MalPublicKey2(From::<[u8; 32]>::from(
                    sig.validator_address
                        .as_ref()
                        .to_vec()
                        .try_into()
                        .or_else(|_| {
                            Err(ProtoError::missing_field::<
                                malctx_schema_proto::PolkaCertificate,
                            >("validator_address"))
                        })?,
                ));
                Ok(PolkaSignature::new(address, signature))
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

pub fn mal_decode_commit_certificate(
    certificate: malctx_schema_proto::CommitCertificate,
) -> Result<CommitCertificate<MalContext>, ProtoError> {
    let value_id = certificate
        .value_id
        .ok_or_else(|| {
            ProtoError::missing_field::<malctx_schema_proto::CommitCertificate>("value_id")
        })
        .and_then(MalValueId::from_proto)?;

    let commit_signatures = certificate
        .signatures
        .into_iter()
        .map(|sig| -> Result<CommitSignature<MalContext>, ProtoError> {
            let signature = sig.signature.ok_or_else(|| {
                ProtoError::missing_field::<malctx_schema_proto::CommitCertificate>("signature")
            })?;
            let signature = mal_decode_signature(signature)?.to_bytes();
            let address = MalPublicKey2(From::<[u8; 32]>::from(
                sig.validator_address
                    .as_ref()
                    .to_vec()
                    .try_into()
                    .or_else(|_| {
                        Err(ProtoError::missing_field::<
                            malctx_schema_proto::PolkaCertificate,
                        >("validator_address"))
                    })?,
            ));
            Ok(CommitSignature::new(address, signature))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let certificate = CommitCertificate {
        height: MalHeight::new(certificate.height),
        round: Round::new(certificate.round),
        value_id,
        commit_signatures,
    };

    Ok(certificate)
}

pub fn mal_encode_commit_certificate(
    certificate: &CommitCertificate<MalContext>,
) -> Result<malctx_schema_proto::CommitCertificate, ProtoError> {
    Ok(malctx_schema_proto::CommitCertificate {
        height: certificate.height.as_u64(),
        round: certificate.round.as_u32().expect("round should not be nil"),
        value_id: Some(certificate.value_id.to_proto()?),
        signatures: certificate
            .commit_signatures
            .iter()
            .map(
                |sig| -> Result<malctx_schema_proto::CommitSignature, ProtoError> {
                    let signature = mal_encode_signature(&Signature::from_bytes(&sig.signature));
                    Ok(malctx_schema_proto::CommitSignature {
                        validator_address: sig.address.0.as_ref().to_vec().into(),
                        signature: Some(signature),
                    })
                },
            )
            .collect::<Result<Vec<_>, _>>()?,
    })
}

pub fn mal_encode_vote_set(
    vote_set: &VoteSet<MalContext>,
) -> Result<malctx_schema_proto::VoteSet, ProtoError> {
    Ok(malctx_schema_proto::VoteSet {
        signed_votes: vote_set
            .votes
            .iter()
            .map(mal_encode_vote)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

pub fn mal_encode_vote(
    vote: &SignedVote<MalContext>,
) -> Result<malctx_schema_proto::SignedMessage, ProtoError> {
    Ok(malctx_schema_proto::SignedMessage {
        message: Some(malctx_schema_proto::signed_message::Message::Vote(
            vote.message.to_bytes().to_vec().into(),
        )),
        signature: Some(mal_encode_signature(&Signature::from_bytes(
            &vote.signature,
        ))),
    })
}

pub fn mal_decode_vote_set(
    vote_set: malctx_schema_proto::VoteSet,
) -> Result<VoteSet<MalContext>, ProtoError> {
    Ok(VoteSet {
        votes: vote_set
            .signed_votes
            .into_iter()
            .map(mal_decode_vote)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

pub fn mal_decode_vote(
    msg: malctx_schema_proto::SignedMessage,
) -> Result<SignedVote<MalContext>, ProtoError> {
    let signature = msg.signature.ok_or_else(|| {
        ProtoError::missing_field::<malctx_schema_proto::SignedMessage>("signature")
    })?;

    let vote = match msg.message {
        Some(malctx_schema_proto::signed_message::Message::Vote(v)) => Ok(v),
        _ => Err(ProtoError::Other(
            "Invalid message type: not a vote".to_string(),
        )),
    }?;

    let signature = mal_decode_signature(signature)?.to_bytes();
    let vote = MalVote::from_bytes(&vote.as_ref().to_vec().try_into().or_else(|_| {
        Err(ProtoError::missing_field::<
            malctx_schema_proto::SignedMessage,
        >("vote"))
    })?);
    Ok(SignedVote::new(vote, signature))
}

pub fn mal_encode_signature(signature: &Signature) -> malctx_schema_proto::Signature {
    malctx_schema_proto::Signature {
        bytes: Bytes::copy_from_slice(signature.to_bytes().as_ref()),
    }
}

pub fn mal_decode_signature(
    signature: malctx_schema_proto::Signature,
) -> Result<Signature, ProtoError> {
    let bytes = <[u8; 64]>::try_from(signature.bytes.as_ref())
        .map_err(|_| ProtoError::Other("Invalid signature length".to_string()))?;
    Ok(Signature::from_bytes(&bytes))
}

/// A vote for a value in a round
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MalVote {
    pub validator_address: MalPublicKey2,
    pub value: NilOrVal<MalValueId>,
    pub height: MalHeight,
    pub typ: VoteType,
    pub round: Round,
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
        if let NilOrVal::Val(value) = self.value {
            buf[32..64].copy_from_slice(&value.0 .0);
        }
        buf[64..72].copy_from_slice(&self.height.0.to_le_bytes());

        let mut merged_round_val: u32 = self.round.as_u32().unwrap() & 0x7fff_ffff;
        if self.typ == VoteType::Precommit {
            merged_round_val |= 0x8000_0000;
        }
        buf[72..76].copy_from_slice(&merged_round_val.to_le_bytes());
        buf
    }
    pub fn from_bytes(bytes: &[u8; 76]) -> MalVote {
        let validator_address =
            MalPublicKey2(From::<[u8; 32]>::from(bytes[0..32].try_into().unwrap()));
        let value_hash_bytes = bytes[32..64].try_into().unwrap();
        let value = if value_hash_bytes == [0_u8; 32] {
            NilOrVal::Nil
        } else {
            NilOrVal::Val(MalValueId(Blake3Hash(value_hash_bytes)))
        };
        let height = MalHeight(u64::from_le_bytes(bytes[64..72].try_into().unwrap()));

        let merged_round_val = u32::from_le_bytes(bytes[72..76].try_into().unwrap());

        let typ = if merged_round_val & 0x8000_0000 != 0 {
            VoteType::Precommit
        } else {
            VoteType::Prevote
        };
        let round = Round::Some(merged_round_val & 0x7fff_ffff);

        MalVote {
            validator_address,
            value,
            height,
            typ,
            round,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MalContext;

impl malachitebft_core_types::Vote<MalContext> for MalVote {
    fn height(&self) -> MalHeight {
        self.height
    }

    fn round(&self) -> Round {
        self.round
    }

    fn value(&self) -> &NilOrVal<MalValueId> {
        &self.value
    }

    fn take_value(self) -> NilOrVal<MalValueId> {
        self.value
    }

    fn vote_type(&self) -> VoteType {
        self.typ
    }

    fn validator_address(&self) -> &MalPublicKey2 {
        &self.validator_address
    }

    fn extension(&self) -> Option<&SignedExtension<MalContext>> {
        None
    }

    fn take_extension(&mut self) -> Option<SignedExtension<MalContext>> {
        None
    }

    fn extend(self, extension: SignedExtension<MalContext>) -> Self {
        use tracing::error;
        error!("SILENTLY DROPPING VOTE EXTENSION");
        self.clone()
    }
}

impl Default for MalContext {
    fn default() -> Self {
        Self
    }
}

impl Context for MalContext {
    type Address = MalPublicKey2;
    type ProposalPart = MalStreamedProposal;
    type Height = MalHeight;
    type Proposal = MalProposal;
    type ValidatorSet = MalValidatorSet;
    type Validator = MalValidator;
    type Value = MalValue;
    type Vote = MalVote;
    type Extension = Bytes;
    type SigningScheme = MalEd25519SigningScheme;

    fn select_proposer<'a>(
        &self,
        validator_set: &'a Self::ValidatorSet,
        height: Self::Height,
        round: Round,
    ) -> &'a Self::Validator {
        assert!(validator_set.count() > 0);
        assert!(round != Round::Nil && round.as_i64() >= 0);

        let proposer_index = {
            let height = height.as_u64() as usize;
            let round = round.as_i64() as usize;

            (height - 1 + round) % validator_set.count()
        };

        validator_set
            .get_by_index(proposer_index)
            .expect("proposer_index is valid")
    }

    fn new_proposal(
        &self,
        height: MalHeight,
        round: Round,
        value: MalValue,
        pol_round: Round,
        address: MalPublicKey2,
    ) -> MalProposal {
        MalProposal {
            height,
            round,
            value,
            pol_round,
            validator_address: address,
        }
    }

    fn new_prevote(
        &self,
        height: MalHeight,
        round: Round,
        value_id: NilOrVal<MalValueId>,
        address: MalPublicKey2,
    ) -> MalVote {
        MalVote {
            typ: VoteType::Prevote,
            height,
            round,
            value: value_id,
            validator_address: address,
        }
    }

    fn new_precommit(
        &self,
        height: MalHeight,
        round: Round,
        value_id: NilOrVal<MalValueId>,
        address: MalPublicKey2,
    ) -> MalVote {
        MalVote {
            typ: VoteType::Precommit,
            height,
            round,
            value: value_id,
            validator_address: address,
        }
    }
}
