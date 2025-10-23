//! The block header.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use thiserror::Error;

use crate::{
    fmt::HexDebug,
    parameters::Network,
    serialization::{TrustedPreallocate, MAX_PROTOCOL_MESSAGE_LEN},
    work::{difficulty::CompactDifficulty, equihash::Solution},
};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io;

use super::{merkle, Commitment, CommitmentError, Hash, Height};

#[cfg(any(test, feature = "proptest-impl"))]
use proptest_derive::Arbitrary;

#[serde_with::serde_as]
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
/// A (temporary) small fixed-size buffer for communicating crosslink dev/test commands
pub struct CommandBuf {
    #[serde_as(as = "serde_with::Bytes")]
    /// Data buffer to contain short command
    pub data: [u8; 128],
}
impl CommandBuf {
    /// size of the internal buffer
    pub const SIZE: usize = 128;

    /// Create an empty command buffer
    pub fn empty() -> Self {
        CommandBuf { data: [0; 128] }
    }

    /// get a rust string from the fixed-size buffer
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(str: &str) -> Self {
        let mut buf = Self::empty();
        let n = std::cmp::min(str.len(), Self::SIZE);
        buf.data[..n].copy_from_slice(&str.as_bytes()[..n]);
        buf
    }

    /// get a rust string from the fixed-size buffer
    pub fn to_str(&self) -> &str {
        let mut c = 0;
        while c < self.data.len() {
            if self.data[c] == 0 {
                break;
            }
            c += 1;
        }
        std::str::from_utf8(&self.data[..c]).expect("init with valid UTF-8")
    }
}

/// A block header, containing metadata about a block.
///
/// How are blocks chained together? They are chained together via the
/// backwards reference (previous header hash) present in the block
/// header. Each block points backwards to its parent, all the way
/// back to the genesis block (the first block in the blockchain).
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Header {
    /// The block's version field. This is supposed to be `4`:
    ///
    /// > The current and only defined block version number for Zcash is 4.
    ///
    /// but this was not enforced by the consensus rules, and defective mining
    /// software created blocks with other versions, so instead it's effectively
    /// a free field. The only constraint is that it must be at least `4` when
    /// interpreted as an `i32`.
    pub version: u32,

    /// The hash of the previous block, used to create a chain of blocks back to
    /// the genesis block.
    ///
    /// This ensures no previous block can be changed without also changing this
    /// block's header.
    pub previous_block_hash: Hash,

    /// The root of the Bitcoin-inherited transaction Merkle tree, binding the
    /// block header to the transactions in the block.
    ///
    /// Note that because of a flaw in Bitcoin's design, the `merkle_root` does
    /// not always precisely bind the contents of the block (CVE-2012-2459). It
    /// is sometimes possible for an attacker to create multiple distinct sets of
    /// transactions with the same Merkle root, although only one set will be
    /// valid.
    pub merkle_root: merkle::Root,

    /// Zcash blocks contain different kinds of commitments to their contents,
    /// depending on the network and height.
    ///
    /// The interpretation of this field has been changed multiple times,
    /// without incrementing the block [`version`](Self::version). Therefore,
    /// this field cannot be parsed without the network and height. Use
    /// [`Block::commitment`](super::Block::commitment) to get the parsed
    /// [`Commitment`].
    pub commitment_bytes: HexDebug<[u8; 32]>,

    /// The block timestamp is a Unix epoch time (UTC) when the miner
    /// started hashing the header (according to the miner).
    pub time: DateTime<Utc>,

    /// An encoded version of the target threshold this block's header
    /// hash must be less than or equal to, in the same nBits format
    /// used by Bitcoin.
    ///
    /// For a block at block height `height`, bits MUST be equal to
    /// `ThresholdBits(height)`.
    ///
    /// [Bitcoin-nBits](https://bitcoin.org/en/developer-reference#target-nbits)
    pub difficulty_threshold: CompactDifficulty,

    /// An arbitrary field that miners can change to modify the header
    /// hash in order to produce a hash less than or equal to the
    /// target threshold.
    pub nonce: HexDebug<[u8; 32]>,

    /// The Equihash solution.
    pub solution: Solution,

    /// Crosslink fat pointer to PoS block.
    pub fat_pointer_to_bft_block: FatPointerToBftBlock,

    /// A command string used during development.
    pub temp_command_buf: CommandBuf,
}

/// A bundle of signed votes for a block
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FatPointerToBftBlock {
    /// The fixed size portion of the the fat pointer.
    #[serde(with = "serde_big_array::BigArray")]
    pub vote_for_block_without_finalizer_public_key: [u8; 76 - 32],
    /// The array of signatures in the fat pointer.
    pub signatures: Vec<FatPointerSignature>,
}

/// A signature inside a fat pointer to a BFT Block.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FatPointerSignature {
    /// The public key associated with this signature.
    pub public_key: [u8; 32],
    #[serde(with = "serde_big_array::BigArray")]
    /// The actual ed25519 signature itself.
    pub vote_signature: [u8; 64],
}

impl FatPointerSignature {
    /// Convert the fat pointer signature to a fixed size byte array.
    pub fn to_bytes(&self) -> [u8; 32 + 64] {
        let mut buf = [0_u8; 32 + 64];
        buf[0..32].copy_from_slice(&self.public_key);
        buf[32..32 + 64].copy_from_slice(&self.vote_signature);
        buf
    }
    /// Convert a fixed size array of bytes into a fat pointer signature.
    pub fn from_bytes(bytes: &[u8; 32 + 64]) -> FatPointerSignature {
        Self {
            public_key: bytes[0..32].try_into().unwrap(),
            vote_signature: bytes[32..32 + 64].try_into().unwrap(),
        }
    }
}

impl FatPointerToBftBlock {
    /// Shorthand for an all null bytes and zero signature count fat pointer.
    pub fn null() -> FatPointerToBftBlock {
        FatPointerToBftBlock {
            vote_for_block_without_finalizer_public_key: [0_u8; 76 - 32],
            signatures: Vec::new(),
        }
    }

    /// Serialize this fat pointer into a dynamically sized byte array.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.vote_for_block_without_finalizer_public_key);
        buf.extend_from_slice(&(self.signatures.len() as u16).to_le_bytes());
        for s in &self.signatures {
            buf.extend_from_slice(&s.to_bytes());
        }
        buf
    }
    /// Try to deserialize a fat pointer from the provided dynamic array of bytes.
    #[allow(clippy::reversed_empty_ranges, clippy::unwrap_in_result)]
    pub fn try_from_bytes(bytes: &[u8]) -> Option<FatPointerToBftBlock> {
        if bytes.len() < 76 - 32 + 2 {
            return None;
        }
        let vote_for_block_without_finalizer_public_key = bytes[0..76 - 32].try_into().ok()?;
        let len = u16::from_le_bytes(bytes[76 - 32..2].try_into().ok()?) as usize;

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

    /// Get the blake3 hash bytes of the BFT block that this fat pointer points to.
    pub fn points_at_block_hash(&self) -> [u8; 32] {
        self.vote_for_block_without_finalizer_public_key[0..32]
            .try_into()
            .unwrap()
    }
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

impl crate::serialization::ZcashSerialize for FatPointerToBftBlock {
    fn zcash_serialize<W: io::Write>(&self, mut writer: W) -> Result<(), io::Error> {
        writer.write_all(&self.vote_for_block_without_finalizer_public_key)?;
        writer.write_u16::<LittleEndian>(self.signatures.len() as u16)?;
        for signature in &self.signatures {
            writer.write_all(&signature.to_bytes())?;
        }
        Ok(())
    }
}

impl crate::serialization::ZcashDeserialize for FatPointerToBftBlock {
    fn zcash_deserialize<R: io::Read>(
        mut reader: R,
    ) -> Result<Self, crate::serialization::SerializationError> {
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

/// TODO: Use this error as the source for zebra_consensus::error::BlockError::Time,
/// and make `BlockError::Time` add additional context.
/// See <https://github.com/ZcashFoundation/zebra/issues/1021> for more details.
#[allow(missing_docs)]
#[derive(Error, Debug)]
pub enum BlockTimeError {
    #[error("invalid time {0:?} in block header {1:?} {2:?}: block time is more than 2 hours in the future ({3:?}). Hint: check your machine's date, time, and time zone.")]
    InvalidBlockTime(
        DateTime<Utc>,
        crate::block::Height,
        crate::block::Hash,
        DateTime<Utc>,
    ),
}

impl Header {
    /// TODO: Inline this function into zebra_consensus::block::check::time_is_valid_at.
    /// See <https://github.com/ZcashFoundation/zebra/issues/1021> for more details.
    #[allow(clippy::unwrap_in_result)]
    pub fn time_is_valid_at(
        &self,
        now: DateTime<Utc>,
        height: &Height,
        hash: &Hash,
    ) -> Result<(), BlockTimeError> {
        let two_hours_in_the_future = now
            .checked_add_signed(Duration::hours(2))
            .expect("calculating 2 hours in the future does not overflow");
        if self.time <= two_hours_in_the_future {
            Ok(())
        } else {
            Err(BlockTimeError::InvalidBlockTime(
                self.time,
                *height,
                *hash,
                two_hours_in_the_future,
            ))?
        }
    }

    /// Get the parsed block [`Commitment`] for this header.
    /// Its interpretation depends on the given `network` and block `height`.
    pub fn commitment(
        &self,
        network: &Network,
        height: Height,
    ) -> Result<Commitment, CommitmentError> {
        Commitment::from_bytes(*self.commitment_bytes, network, height)
    }

    /// Compute the hash of this header.
    pub fn hash(&self) -> Hash {
        Hash::from(self)
    }
}

/// A header with a count of the number of transactions in its block.
/// This structure is used in the Bitcoin network protocol.
///
/// The transaction count field is always zero, so we don't store it in the struct.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(any(test, feature = "proptest-impl"), derive(Arbitrary))]
pub struct CountedHeader {
    /// The header for a block
    pub header: Arc<Header>,
}

/// The serialized size of a Zcash block header.
///
/// Includes the equihash input, 32-byte nonce, 3-byte equihash length field, and equihash solution.
const BLOCK_HEADER_LENGTH: usize = crate::work::equihash::Solution::INPUT_LENGTH
    + 32
    + 3
    + crate::work::equihash::SOLUTION_SIZE
    + (76 - 32 + 2)
    + 128;

/// The minimum size for a serialized CountedHeader.
///
/// A CountedHeader has BLOCK_HEADER_LENGTH bytes + 1 or more bytes for the transaction count
pub(crate) const MIN_COUNTED_HEADER_LEN: usize = BLOCK_HEADER_LENGTH + 1;

/// The Zcash accepted block version.
///
/// The consensus rules do not force the block version to be this value but just equal or greater than it.
/// However, it is suggested that submitted block versions to be of this exact value.
pub const ZCASH_BLOCK_VERSION: u32 = 4;

impl TrustedPreallocate for CountedHeader {
    fn max_allocation() -> u64 {
        // Every vector type requires a length field of at least one byte for de/serialization.
        // Therefore, we can never receive more than (MAX_PROTOCOL_MESSAGE_LEN - 1) / MIN_COUNTED_HEADER_LEN counted headers in a single message
        ((MAX_PROTOCOL_MESSAGE_LEN - 1) / MIN_COUNTED_HEADER_LEN) as u64
    }
}
