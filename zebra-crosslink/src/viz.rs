//! Internal vizualization
#![allow(unexpected_cfgs, unused, missing_docs)]
// TODO:
// [ ] request range
// [ ] non-finalized side-chain
// [ ] uncross edges

use crate::{test_format::*, *};
use macroquad::{
    camera::*,
    color::{self, colors::*},
    input::*,
    math::{vec2, Circle, FloatExt, Rect, Vec2},
    shapes::{self, draw_triangle},
    telemetry::{self, end_zone as end_zone_unchecked, ZoneGuard},
    text::{self, TextDimensions, TextParams},
    texture::{self, Texture2D},
    time,
    ui::{self, hash, root_ui, widgets},
    window,
};
use static_assertions::*;
use std::{
    cmp::{max, min},
    collections::HashMap,
    sync::Arc,
    thread::JoinHandle,
};
use zebra_chain::{
    serialization::{ZcashDeserialize, ZcashSerialize},
    transaction::{LockTime, Transaction},
    work::difficulty::{CompactDifficulty, INVALID_COMPACT_DIFFICULTY},
};

const IS_DEV: bool = true;
fn dev(show_in_dev: bool) -> bool {
    IS_DEV && show_in_dev
}

fn if_dev<F>(show_in_dev: bool, f: F)
where
    F: FnOnce(),
{
    if IS_DEV && show_in_dev {
        f();
    }
}

// consistent zero-initializers
// TODO: create a derive macro
trait _0 {
    const _0: Self;
}
impl _0 for Vec2 {
    const _0: Vec2 = vec2(0., 0.);
}
impl _0 for Rect {
    const _0: Rect = Rect::new(0., 0., 0., 0.);
}
impl _0 for Circle {
    const _0: Circle = Circle::new(0., 0., 0.);
}

trait _1 {
    const _1: Self;
}
impl _1 for Vec2 {
    const _1: Vec2 = Vec2::ONE;
}

#[allow(unused)]
trait Unit {
    const UNIT: Self;
}
impl Unit for Vec2 {
    const UNIT: Vec2 = Vec2::_1;
}
impl Unit for Rect {
    const UNIT: Rect = Rect::new(0., 0., 1., 1.);
}
impl Unit for Circle {
    const UNIT: Circle = Circle::new(0., 0., 1.);
}

#[derive(Debug, Copy, Clone)]
struct BBox {
    min: Vec2,
    max: Vec2,
}

fn flt_min(a: f32, b: f32) -> f32 {
    assert!(!a.is_nan());
    assert!(!b.is_nan());

    if a <= b {
        a
    } else {
        b
    }
}

fn flt_max(a: f32, b: f32) -> f32 {
    assert!(!a.is_nan());
    assert!(!b.is_nan());

    if a <= b {
        b
    } else {
        a
    }
}

fn flt_min_max(a: f32, b: f32) -> (f32, f32) {
    assert!(!a.is_nan());
    assert!(!b.is_nan());

    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

impl BBox {
    fn union(a: BBox, b: BBox) -> BBox {
        assert!(a.min.x <= a.max.x);
        assert!(a.min.y <= a.max.y);
        assert!(b.min.x <= b.max.x);
        assert!(b.min.y <= b.max.y);

        BBox {
            min: vec2(flt_min(a.min.x, b.min.x), flt_min(a.min.y, b.min.y)),
            max: vec2(flt_max(a.max.x, b.max.x), flt_max(a.max.y, b.max.y)),
        }
    }

    fn expand(a: BBox, v: Vec2) -> BBox {
        BBox {
            min: a.min - v,
            max: a.max + v,
        }
    }

    fn clamp(a: BBox, v: Vec2) -> Vec2 {
        v.clamp(a.min, a.max)
    }

    fn update_union(&mut self, b: BBox) {
        *self = Self::union(*self, b);
    }
}

impl _0 for BBox {
    const _0: BBox = BBox {
        min: Vec2::_0,
        max: Vec2::_0,
    };
}

impl From<Rect> for BBox {
    fn from(rect: Rect) -> Self {
        let min = rect.point();
        BBox {
            min,
            max: min + rect.size(),
        }
    }
}

impl From<Vec2> for BBox {
    fn from(v: Vec2) -> Self {
        BBox { min: v, max: v }
    }
}

impl From<BBox> for Rect {
    fn from(bbox: BBox) -> Self {
        Rect {
            x: bbox.min.x,
            y: bbox.min.y,
            w: bbox.max.x - bbox.min.x,
            h: bbox.max.y - bbox.min.y,
        }
    }
}

// NOTE: the telemetry profiler uses a mutable global like this; we're
//       continuing the pattern rather than adding a new approach.
static mut PROFILER_ZONE_DEPTH: u32 = 0;

#[allow(unsafe_code)]
fn begin_zone(name: &str) -> u32 {
    telemetry::begin_zone(name);
    unsafe {
        PROFILER_ZONE_DEPTH += 1;
        PROFILER_ZONE_DEPTH
    }
}

#[allow(unsafe_code)]
fn end_zone(active_depth: u32) {
    unsafe {
        let expected_depth = PROFILER_ZONE_DEPTH;
        // TODO: could maintain additional stack info to help diagnose mismatch
        // (which we'd get for free if macroquad hadn't made it private!)
        // Almost all uses of this could be on the call stack with location info...
        // Or we could maintain an array/vector.
        assert_eq!(
            expected_depth, active_depth,
            "mismatched telemetry zone begin/end pairs"
        );
        PROFILER_ZONE_DEPTH -= 1;
    }
    end_zone_unchecked();
}

// TODO
// /// Smaller block format: only transfer/cache what we need
// struct VizBlock {
//     difficulty: CompactDifficulty, // to be converted to_work()
//     txs_n: u32,
// }

/// TFL state communicated to the visualizer
#[derive(Debug)]
pub struct VizState {
    // general chain info
    /// Height & hash of the TFL finality point
    pub latest_final_block: Option<(BlockHeight, BlockHash)>,
    /// Height & hash of the best-chain tip
    pub bc_tip: Option<(BlockHeight, BlockHash)>,

    // requested info //
    /// A range of hashes from the PoW chain, as requested by the visualizer.
    /// Ascending in height from `lo_height`. Parallel to `blocks`.
    pub height_hashes: Vec<(BlockHeight, BlockHash)>,
    /// A range of blocks from the PoW chain, as requested by the visualizer.
    /// Ascending in height from `lo_height`. Parallel to `hashes`.
    pub blocks: Vec<Option<Arc<Block>>>,

    /// Value that this finalizer is currently intending to propose for the next BFT block.
    pub internal_proposed_bft_string: Option<String>,
    /// Flags of the BFT messages
    pub bft_msg_flags: u64,
    /// Vector of all decided BFT blocks, indexed by height-1.
    pub bft_blocks: Vec<BftBlock>,
    /// Fat pointer to the BFT tip (all other fat pointers are available at height+1)
    pub fat_pointer_to_bft_tip: FatPointerToBftBlock,
}

/// Functions & structures for serializing visualizer state to/from disk.
// pub mod serialization {
//     use super::*;
//     use chrono::Utc;
//     use serde::{Deserialize, Serialize};
//     use std::fs;
//     use zebra_chain::{
//         block::merkle::Root,
//         block::{Block, Hash as BlockHash, Header as BlockHeader, Height as BlockHeight},
//         fmt::HexDebug,
//         work::equihash::Solution,
//     };

//     #[derive(Serialize, Deserialize)]
//     struct MinimalBlockExport {
//         difficulty: u32,
//         txs_n: u32,
//         previous_block_hash: [u8; 32],
//     }

//     #[derive(Serialize, Deserialize)]
//     struct MinimalVizStateExport {
//         latest_final_block: Option<(u32, [u8; 32])>,
//         bc_tip: Option<(u32, [u8; 32])>,
//         height_hashes: Vec<(u32, [u8; 32])>,
//         blocks: Vec<Option<MinimalBlockExport>>,
//         bft_blocks: Vec<BftBlock>,
//     }

//     impl From<(&VizState, &VizCtx, &ZcashCrosslinkParameters)> for MinimalVizStateExport {
//         fn from(data: (&VizState, &VizCtx, &ZcashCrosslinkParameters)) -> Self {
//             let (state, ctx, params) = data;
//             let nodes = &ctx.nodes;

//             fn u32_from_compact_difficulty(difficulty: CompactDifficulty) -> u32 {
//                 u32::from_be_bytes(difficulty.bytes_in_display_order())
//             }

//             let mut height_hashes: Vec<(u32, [u8; 32])> = state
//                 .height_hashes
//                 .iter()
//                 .map(|h| (h.0 .0, h.1 .0))
//                 .collect();
//             let mut blocks: Vec<Option<MinimalBlockExport>> = state
//                 .blocks
//                 .iter()
//                 .map(|opt| {
//                     opt.as_ref().map(|b| MinimalBlockExport {
//                         difficulty: u32_from_compact_difficulty(b.header.difficulty_threshold),
//                         txs_n: b.transactions.len() as u32,
//                         previous_block_hash: b.header.previous_block_hash.0,
//                     })
//                 })
//                 .collect();

//             let mut bft_blocks = state.bft_blocks.clone();

//             for (i, node) in nodes.iter().enumerate() {
//                 match node.kind {
//                     NodeKind::BC => {
//                         height_hashes.push((
//                             node.height,
//                             node.hash().expect("PoW nodes should have hashes"),
//                         ));
//                         blocks.push(Some(MinimalBlockExport {
//                             difficulty: u32_from_compact_difficulty(
//                                 node.difficulty.map_or(INVALID_COMPACT_DIFFICULTY, |d| d),
//                             ),
//                             txs_n: node.txs_n,
//                             previous_block_hash: if let Some(parent) = ctx.get_node(node.parent) {
//                                 assert!(parent.height + 1 == node.height);
//                                 parent.hash().expect("PoW nodes should have hashes")
//                             } else {
//                                 let mut hash = [0u8; 32];
//                                 // for (k, v) in &ctx.missing_bc_parents {
//                                 //     if *v == Some(i) {
//                                 //         hash = *k;
//                                 //     }
//                                 // }
//                                 hash
//                             },
//                         }));
//                     }

//                     NodeKind::BFT => {
//                         let parent_id = if let Some(parent) = ctx.get_node(node.parent) {
//                             assert!(parent.height + 1 == node.height);
//                             parent.id().expect("BFT nodes should have ids")
//                         } else {
//                             0
//                         };

//                         bft_blocks.push(node.header.as_bft().unwrap().clone());
//                     }
//                 }
//             }

//             Self {
//                 latest_final_block: state.latest_final_block.map(|(h, hash)| (h.0, hash.0)),
//                 bc_tip: state.bc_tip.map(|(h, hash)| (h.0, hash.0)),
//                 height_hashes,
//                 blocks,
//                 bft_blocks,
//             }
//         }
//     }

//     impl From<MinimalVizStateExport> for VizGlobals {
//         fn from(export: MinimalVizStateExport) -> Self {
//             let state = Arc::new(VizState {
//                 latest_final_block: export
//                     .latest_final_block
//                     .map(|(h, hash)| (BlockHeight(h), BlockHash(hash))),
//                 bc_tip: export
//                     .bc_tip
//                     .map(|(h, hash)| (BlockHeight(h), BlockHash(hash))),
//                 height_hashes: export
//                     .height_hashes
//                     .into_iter()
//                     .map(|h| (BlockHeight(h.0), BlockHash(h.1)))
//                     .collect(),
//                 blocks: export
//                     .blocks
//                     .into_iter()
//                     .map(|opt| {
//                         opt.map(|b| {
//                             Arc::new(Block {
//                                 header: Arc::new(BlockHeader {
//                                     version: 0,
//                                     previous_block_hash: BlockHash(b.previous_block_hash),
//                                     merkle_root: Root::from_bytes_in_display_order(&[0u8; 32]),
//                                     commitment_bytes: HexDebug([0u8; 32]),
//                                     time: Utc::now(),
//                                     difficulty_threshold:
//                                         CompactDifficulty::from_bytes_in_display_order(
//                                             &b.difficulty.to_be_bytes(),
//                                         )
//                                         .expect("valid difficulty"),
//                                     nonce: HexDebug([0u8; 32]),
//                                     solution: Solution::for_proposal(),
//                                 }),
//                                 // dummy transactions, just so we have txs_n
//                                 transactions: {
//                                     let tx = Arc::new(Transaction::V1 {
//                                         lock_time: LockTime::Height(BlockHeight(0)),
//                                         inputs: Vec::new(),
//                                         outputs: Vec::new(),
//                                     });
//                                     vec![tx; b.txs_n as usize]
//                                 },
//                             })
//                         })
//                     })
//                     .collect(),
//                 internal_proposed_bft_string: Some("From Export".into()),
//                 bft_msg_flags: 0,
//                 bft_blocks: export.bft_blocks,
//             });

//             VizGlobals {
//                 params: &PROTOTYPE_PARAMETERS,
//                 state,
//                 bc_req_h: (0, 0),
//                 consumed: true,
//                 proposed_bft_string: None,
//             }
//         }
//     }

//     /// Read a global state struct from the data in a file
//     pub fn read_from_file(path: &str) -> VizGlobals {
//         let raw = fs::read_to_string(path).expect(&format!("{} exists", path));
//         let export: MinimalVizStateExport = serde_json::from_str(&raw).expect("valid export JSON");
//         let globals: VizGlobals = export.into();
//         globals
//     }

//     /// Read a global state struct from the data in a file and apply it to the current global state
//     pub fn init_from_file(path: &str) {
//         let globals = read_from_file(path);
//         *VIZ_G.lock().unwrap() = Some(globals);
//     }

//     /// Write the current visualizer state to a file
//     pub(crate) fn write_to_file_internal(
//         path: &str,
//         state: &VizState,
//         ctx: &VizCtx,
//         params: &ZcashCrosslinkParameters,
//     ) {
//         use serde_json::to_string_pretty;
//         use std::fs;

//         let viz_export = MinimalVizStateExport::from((&*state, ctx, params));
//         // eprintln!("Dumping state to file \"{}\", {:?}", path, state);
//         let json = to_string_pretty(&viz_export).expect("serialization success");
//         fs::write(path, json).expect("write success");
//     }

//     /// Write the current visualizer state to a file
//     pub fn write_to_file(path: &str, state: &VizState, params: &ZcashCrosslinkParameters) {
//         write_to_file_internal(path, state, &VizCtx::default(), params)
//     }
// }

/// Self-debug info
struct VizDbg {
    nodes_forces: HashMap<NodeRef, Vec<Vec2>>,
}

impl VizDbg {
    fn new_force(&mut self, node_ref: NodeRef, force: Vec2) {
        if dev(true) {
            let mut forces = self.nodes_forces.get_mut(&node_ref);
            if let Some(forces) = forces.as_mut() {
                forces.push(force);
            } else {
                self.nodes_forces.insert(node_ref, vec![force]);
            }
        }
    }
}

enum Sound {
    HoverNode,
    NewNode,
}
#[cfg(feature = "audio")]
const SOUNDS_N: usize = 2; // TODO: get automatically from enum?
#[cfg(feature = "audio")]
static G_SOUNDS: std::sync::Mutex<[Option<macroquad::audio::Sound>; SOUNDS_N]> =
    std::sync::Mutex::new([const { None }; SOUNDS_N]);

async fn init_audio(config: &VizConfig) {
    #[cfg(feature = "audio")]
    {
        let mut lock = G_SOUNDS.lock();
        let sounds = lock.as_mut().unwrap();
        sounds[Sound::HoverNode as usize] = macroquad::audio::load_sound_from_bytes(
            include_bytes!("../res/impactGlass_heavy_000.ogg"),
        )
        .await
        .ok();
        sounds[Sound::NewNode as usize] =
            macroquad::audio::load_sound_from_bytes(include_bytes!("../res/toggle_001.ogg"))
                .await
                .ok();
    }
}

fn play_sound_once(config: &VizConfig, sound: Sound) {
    #[cfg(feature = "audio")]
    if config.audio_on {
        if let Some(sound) = &G_SOUNDS.lock().unwrap()[sound as usize] {
            macroquad::audio::play_sound(
                sound,
                macroquad::audio::PlaySoundParams {
                    looped: false,
                    volume: config.audio_volume * config.audio_volume, // scale better than linear
                },
            );
        }
    }
}

/// Any global data stored for visualization.
/// In practice this stores the data for communicating between TFL (in tokio-land)
/// and the viz thread.
#[derive(Clone)]
pub struct VizGlobals {
    /// Crosslink parameters
    params: &'static ZcashCrosslinkParameters,
    /// TFL state communicated from TFL to Viz
    pub state: std::sync::Arc<VizState>,
    // wanted_height_rng: (u32, u32),
    /// Allows for one-way syncing so service_viz_requests doesn't run too quickly
    pub consumed: bool,
    /// The range of PoW blocks requested from Viz to TFL.
    /// Translates to `hashes`, `blocks` in `VizState`
    pub bc_req_h: (i32, i32), // negative implies relative to tip
    // TODO: bft_req_h: (i32, i32),
    /// Value for this finalizer node to propose for the next BFT block.
    pub proposed_bft_string: Option<String>,
}
static VIZ_G: std::sync::Mutex<Option<VizGlobals>> = std::sync::Mutex::new(None);

/// Blocks to be injected into zebra via getblocktemplate, submitblock etc
static G_FORCE_BLOCKS: std::sync::Mutex<Vec<Arc<Block>>> = std::sync::Mutex::new(Vec::new());

const VIZ_REQ_N: u32 = zebra_state::MAX_BLOCK_REORG_HEIGHT;

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

/// Bridge between tokio & viz code
pub async fn service_viz_requests(
    tfl_handle: crate::TFLServiceHandle,
    params: &'static ZcashCrosslinkParameters,
) {
    let call = tfl_handle.clone().call;

    *VIZ_G.lock().unwrap() = Some(VizGlobals {
        params,
        state: std::sync::Arc::new(VizState {
            latest_final_block: None,
            bc_tip: None,

            height_hashes: Vec::new(),
            blocks: Vec::new(),

            internal_proposed_bft_string: None,
            bft_msg_flags: 0,
            bft_blocks: Vec::new(),
            fat_pointer_to_bft_tip: FatPointerToBftBlock::null(),
        }),

        // NOTE: bitwise not of x (!x in rust) is the same as -1 - x
        // (in 2s complement, which is how Rust's signed ints are represented)
        // i.e. it's accessing in reverse order from the tip
        bc_req_h: (!VIZ_REQ_N as i32, !0),
        proposed_bft_string: None,
        consumed: true,
    });

    loop {
        let old_g = VIZ_G.lock().unwrap().as_ref().unwrap().clone();

        if old_g.proposed_bft_string.is_some() {
            tfl_handle.internal.lock().await.proposed_bft_string =
                old_g.proposed_bft_string.clone();
        }

        if !old_g.consumed {
            std::thread::sleep(std::time::Duration::from_micros(500));
            continue;
        }
        let mut new_g = old_g.clone();
        new_g.consumed = false;

        {
            let mut lock = G_FORCE_BLOCKS.lock().unwrap();
            let mut force_feed_blocks: &mut Vec<Arc<Block>> = lock.as_mut();
            for block in force_feed_blocks.drain(..) {
                (call.force_feed_pow)(block);
            }
        }

        #[allow(clippy::never_loop)]
        let (lo_height, bc_tip, hashes, blocks) = loop {
            let (lo, hi) = (new_g.bc_req_h.0, new_g.bc_req_h.1);
            assert!(
                lo <= hi || (lo >= 0 && hi < 0),
                "lo ({}) should be below hi ({})",
                lo,
                hi
            );

            let tip_height_hash: (BlockHeight, BlockHash) = {
                if let Ok(ReadStateResponse::Tip(Some(tip_height_hash))) =
                    (call.read_state)(ReadStateRequest::Tip).await
                {
                    tip_height_hash
                } else {
                    error!("Failed to read tip");
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
                } else if let Ok(ReadStateResponse::BlockHeader { hash, .. }) =
                    (call.read_state)(ReadStateRequest::BlockHeader(h.into())).await
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

            let (hashes, blocks) =
                tfl_block_sequence(&call, lo_height_hash.1, Some(hi_height_hash), true, true).await;
            break (lo_height_hash.0, Some(tip_height_hash), hashes, blocks);
        };

        let (bft_msg_flags, mut bft_blocks, fat_pointer_to_bft_tip, internal_proposed_bft_string) = {
            let mut internal = tfl_handle.internal.lock().await;
            let bft_msg_flags = internal.bft_msg_flags;
            internal.bft_msg_flags = 0;

            // TODO: is this still necessary?
            // TODO: O(<n)
            // for i in 0..bft_blocks.len() {
            //     let block = &mut bft_blocks[i];
            //     if block.finalization_candidate_height == 0 && !block.headers.is_empty()
            //     {
            //         // TODO: block.finalized()
            //         if let Some(BlockHeight(h)) =
            //             block_height_from_hash(&call, block.headers[0].hash()).await
            //         {
            //             block.finalization_candidate_height = h;
            //             //
            //             // let mut internal = tfl_handle.internal.lock().await;
            //             // internal.bft_blocks[i]
            //             //     .finalization_candidate_height = h;
            //         }
            //     }
            // }

            (
                bft_msg_flags,
                internal.bft_blocks.clone(),
                internal.fat_pointer_to_tip.clone(),
                internal.proposed_bft_string.clone(),
            )
        };

        let mut height_hashes = Vec::with_capacity(hashes.len());
        for i in 0..hashes.len() {
            height_hashes.push((BlockHeight(lo_height.0 + i as u32), hashes[i]));
        }

        let new_state = VizState {
            latest_final_block: tfl_final_block_height_hash(&tfl_handle).await,
            bc_tip,
            height_hashes,
            blocks,
            internal_proposed_bft_string,
            bft_msg_flags,
            bft_blocks,
            fat_pointer_to_bft_tip,
        };

        new_g.state = Arc::new(new_state);
        *VIZ_G.lock().unwrap() = Some(new_g);
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum NodeKind {
    BC,
    BFT,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum NodeId {
    None,
    Hash([u8; 32]),
    Index(usize),
}

// type NodeRef = Option<(usize)>; // TODO: generational handle

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
struct NodeHdl {
    idx: u32,
    // generation: u32,
}

type NodeRef = Option<NodeHdl>;

#[derive(Clone, Debug)]
enum VizHeader {
    None,
    BlockHeader(BlockHeader),
    BftBlock(BftBlockAndFatPointerToIt), // ALT: just keep an array of *hashes*
}

impl VizHeader {
    fn as_bft(&self) -> Option<&BftBlockAndFatPointerToIt> {
        match self {
            VizHeader::BftBlock(val) => Some(val),
            _ => None,
        }
    }

    fn as_pow(&self) -> Option<&BlockHeader> {
        match self {
            VizHeader::BlockHeader(val) => Some(val),
            _ => None,
        }
    }
}

impl From<Option<BftBlockAndFatPointerToIt>> for VizHeader {
    fn from(v: Option<BftBlockAndFatPointerToIt>) -> VizHeader {
        match v {
            Some(val) => VizHeader::BftBlock(val),
            None => VizHeader::None,
        }
    }
}

impl From<Option<BlockHeader>> for VizHeader {
    fn from(v: Option<BlockHeader>) -> VizHeader {
        match v {
            Some(val) => VizHeader::BlockHeader(val),
            None => VizHeader::None,
        }
    }
}

/// A node on the graph/network diagram.
/// Contains a bundle of attributes that can represent PoW or BFT blocks.
#[derive(Clone, Debug)]
struct Node {
    // structure
    parent: NodeRef, // TODO: do we need explicit edges?
    // generation: u32,

    // data
    kind: NodeKind,
    text: String,
    id: NodeId,
    height: u32,
    difficulty: Option<CompactDifficulty>,
    txs_n: u32, // N.B. includes coinbase
    header: VizHeader,
    bc_block: Option<Arc<Block>>,

    is_real: bool,

    // presentation
    pt: Vec2,
    vel: Vec2,
    acc: Vec2,
    rad: f32,
}

fn tfl_nominee_from_node(ctx: &VizCtx, node: &Node) -> NodeRef {
    match &node.header {
        VizHeader::BftBlock(bft_block) => {
            if let Some(pow_block) = bft_block.block.headers.last() {
                ctx.find_bc_node_by_hash(&pow_block.hash())
            } else {
                None
            }
        }

        _ => None,
    }
}

fn tfl_finalized_from_node(ctx: &VizCtx, node: &Node) -> NodeRef {
    match &node.header {
        VizHeader::BftBlock(bft_block) => {
            if let Some(pow_block) = bft_block.block.headers.first() {
                ctx.find_bc_node_by_hash(&pow_block.hash())
            } else {
                None
            }
        }

        _ => None,
    }
}

enum NodeInit {
    Node {
        node: Node,
        needs_fixup: bool,
    },

    Dyn {
        parent: NodeRef, // TODO: do we need explicit edges?

        // data
        kind: NodeKind,
        text: String,
        id: NodeId,
        height: u32,
        header: VizHeader,
        difficulty: Option<CompactDifficulty>,
        txs_n: u32, // N.B. includes coinbase
        bc_block: Option<Arc<Block>>,

        is_real: bool,
    },

    BC {
        parent: NodeRef, // TODO: do we need explicit edges?
        hash: [u8; 32],
        height: u32,
        difficulty: Option<CompactDifficulty>,
        header: BlockHeader,
        bc_block: Option<Arc<Block>>,
        txs_n: u32, // N.B. includes coinbase
        is_real: bool,
    },

    BFT {
        parent: NodeRef, // TODO: do we need explicit edges?
        hash: [u8; 32],
        text: String,
        bft_block: BftBlockAndFatPointerToIt,
        height: Option<u32>, // if None, works out from parent
        is_real: bool,
    },
}

impl Node {
    fn circle(&self) -> Circle {
        Circle::new(self.pt.x, self.pt.y, self.rad)
    }

    fn id(&self) -> Option<usize> {
        match self.id {
            NodeId::Index(hash) => Some(hash),
            _ => None,
        }
    }

    fn hash(&self) -> Option<[u8; 32]> {
        match self.id {
            NodeId::Hash(hash) => Some(hash),
            _ => None,
        }
    }

    fn hash_string(&self) -> Option<String> {
        let _z = ZoneGuard::new("hash_string()");
        self.hash().map(|hash| BlockHash(hash).to_string())
    }
}

impl HasBlockHash for Node {
    fn get_hash(&self) -> Option<BlockHash> {
        if self.kind == NodeKind::BC {
            assert!(self.hash().is_some());
            self.hash().map(BlockHash)
        } else {
            None
        }
    }
}

// trait ByHandle<T, H> {
//     fn get_by_handle(&self, handle: H) -> Option<&T>
//     where
//         Self: std::marker::Sized;
// }

// impl ByHandle<Node, NodeRef> for [Node] {
//     fn get_by_handle(&self, handle: NodeRef) -> Option<&Node> {
//         if let Some(handle) = handle {
//             // TODO: generations
//             self.get(handle)
//         } else {
//             None
//         }
//     }
// }

fn find_bc_node_i_by_height(nodes: &[Node], height: BlockHeight) -> NodeRef {
    let _z = ZoneGuard::new("find_bc_node_i_by_height");
    nodes
        .iter()
        .position(|node| node.kind == NodeKind::BC && node.height == height.0)
        .map(|i| NodeHdl { idx: i as u32 })
}

fn find_bft_node_by_height(nodes: &[Node], height: u32) -> Option<&Node> {
    let _z = ZoneGuard::new("find_bft_node_by_height");
    nodes
        .iter()
        .find(|node| node.kind == NodeKind::BFT && node.height == height)
}

fn str_partition_at(string: &str, at: usize) -> (&str, &str) {
    (&string[..at], &string[at..])
}
#[derive(Debug)]
enum MouseDrag {
    Nil,
    Ui,
    World(Vec2), // start point (may need a different name?)
    Node {
        start_pt: Vec2,
        node: NodeRef,
        mouse_to_node: Vec2,
    },
}

#[derive(Debug, Clone)]
struct AccelEntry {
    nodes: Vec<NodeRef>,
}

/// (Spatial) Acceleration
#[derive(Debug, Clone)]
struct Accel {
    y_to_nodes: HashMap<i64, AccelEntry>,
}

const ACCEL_GRP_SIZE: f32 = 1620.;
// const ACCEL_GRP_SIZE : f32 = 220.;

struct VizConfig {
    test_bbox: bool,
    show_accel_areas: bool,
    show_bft_ui: bool,
    show_top_info: bool,
    show_mouse_info: bool,
    show_profiler: bool,
    show_bft_msgs: bool,
    pause_incoming_blocks: bool,
    node_load_kind: usize,
    new_node_ratio: f32,
    audio_on: bool,
    audio_volume: f32,
    draw_resultant_forces: bool,
    draw_component_forces: bool,
    do_force_links: bool,
    do_force_any: bool,
    do_force_edge: bool,
}

/// Common GUI state that may need to be passed around
#[derive(Debug)]
pub(crate) struct VizCtx {
    // h: BlockHeight,
    screen_o: Vec2,
    screen_vel: Vec2,
    fix_screen_o: Vec2,
    mouse_drag: MouseDrag,
    mouse_drag_d: Vec2,
    old_mouse_pt: Vec2,
    nodes: Vec<Node>,
    nodes_bbox: BBox,
    accel: Accel,

    bc_lo: NodeRef,
    bc_hi: NodeRef,
    bc_work_max: u128,
    missing_bc_parents: HashMap<[u8; 32], NodeRef>,
    bc_by_hash: HashMap<[u8; 32], NodeRef>, // ALT: unify BC/BFT maps, assuming hashes don't conflict

    bft_by_hash: HashMap<[u8; 32], NodeRef>, // ALT: unify BC/BFT maps, assuming hashes don't conflict
    bft_block_hi_i: usize,
    bft_last_added: NodeRef,
    bft_fake_id: usize,
}

impl VizCtx {
    fn get_node(&self, node_ref: NodeRef) -> Option<&Node> {
        if let Some(node_hdl) = node_ref {
            let i = node_hdl.idx as usize;
            if i < self.nodes.len() {
                if true {
                    // TODO: generation
                    return Some(&self.nodes[i]);
                }
            }
        }

        None
    }

    fn node(&mut self, node_ref: NodeRef) -> Option<&mut Node> {
        if let Some(node_hdl) = node_ref {
            let i = node_hdl.idx as usize;
            if i < self.nodes.len() {
                if true {
                    // TODO: generation
                    return Some(&mut self.nodes[i]);
                }
            }
        }

        None
    }

    fn node_ref(&self, idx: usize) -> NodeRef {
        if idx < self.nodes.len() {
            Some(NodeHdl { idx: idx as u32 })
        } else {
            None
        }
    }

    fn push_node(
        &mut self,
        config: &VizConfig,
        node: NodeInit,
        parent_hash: Option<[u8; 32]>,
    ) -> NodeRef {
        play_sound_once(config, Sound::NewNode);

        // TODO: could overlay internal circle/rings for shielded/transparent
        let (mut new_node, needs_fixup): (Node, bool) = match node {
            NodeInit::Node { node, needs_fixup } => (node, needs_fixup),

            NodeInit::Dyn {
                parent,
                kind,
                text,
                id,
                header,
                bc_block,
                height,
                difficulty,
                txs_n,
                is_real,
            } => {
                (
                    Node {
                        parent,
                        kind,
                        text,
                        id,
                        height, // TODO: this should be exclusively determined by parent for Dyn
                        header,
                        difficulty,
                        txs_n,
                        is_real,
                        bc_block,
                        rad: match kind {
                            NodeKind::BC => ((txs_n as f32).sqrt() * 5.).min(50.),
                            NodeKind::BFT => 10.,
                        },

                        pt: Vec2::_0,
                        vel: Vec2::_0,
                        acc: Vec2::_0,
                    },
                    true,
                )
            }

            NodeInit::BC {
                parent,
                hash,
                height,
                header,
                difficulty,
                bc_block,
                txs_n,
                is_real,
            } => {
                // sqrt(txs_n) for radius means that the *area* is proportional to txs_n
                let rad = ((txs_n as f32).sqrt() * 5.).min(50.);

                // DUP
                assert!(self
                    .node(parent)
                    .map_or(true, |parent| parent.height + 1 == height));

                (
                    Node {
                        parent,

                        kind: NodeKind::BC,
                        id: NodeId::Hash(hash),
                        height,
                        header: VizHeader::BlockHeader(header),
                        difficulty,
                        txs_n,
                        is_real,
                        bc_block,

                        text: "".to_string(),
                        rad,

                        pt: Vec2::_0,
                        vel: Vec2::_0,
                        acc: Vec2::_0,
                    },
                    true,
                )
            }

            NodeInit::BFT {
                parent,
                height,
                bft_block,
                hash,
                text,
                is_real,
            } => {
                let rad = 10.;
                // DUP
                let height = if let Some(height) = height {
                    assert!(self
                        .node(parent)
                        .map_or(true, |parent| parent.height + 1 == height));
                    height
                } else {
                    self.node(parent)
                        .expect("Need at least 1 of parent or height")
                        .height
                        + 1
                };

                (
                    Node {
                        parent,

                        id: NodeId::Hash(hash),
                        difficulty: None,
                        txs_n: 0,

                        kind: NodeKind::BFT,
                        height,
                        header: VizHeader::BftBlock(bft_block),
                        text,
                        bc_block: None,

                        is_real,

                        pt: vec2(100., 0.),
                        vel: Vec2::_0,
                        acc: Vec2::_0,
                        rad,
                    },
                    true,
                )
            }
        };

        let i = self.nodes.len();
        // NOTE: currently referencing OOB until this is added
        let node_ref = Some(NodeHdl { idx: i as u32 });

        if let Some(node_hash) = new_node.hash() {
            match new_node.kind {
                NodeKind::BC => self.bc_by_hash.insert(node_hash, node_ref),
                NodeKind::BFT => self.bft_by_hash.insert(node_hash, node_ref),
            };
        }

        let mut child_ref: NodeRef = None;
        if new_node.kind == NodeKind::BC {
            if let Some(hi_node) = self.get_node(self.bc_hi) {
                new_node.pt = hi_node.pt;
            }

            // find & link possible parent
            if let Some(parent_hash) = parent_hash {
                let parent_ref = self.find_bc_node_by_hash(&BlockHash(parent_hash));
                if let Some(parent) = self.node(parent_ref) {
                    assert!(new_node.parent.is_none() || new_node.parent == parent_ref);
                    assert!(
                        // NOTE(Sam): Spurius crash sometimes
                        parent.height + 1 == new_node.height,
                        "parent height: {}, new height: {}",
                        parent.height,
                        new_node.height
                    );

                    new_node.parent = parent_ref;
                } else if parent_hash != [0; 32] {
                    self.missing_bc_parents.insert(parent_hash, node_ref);
                }
            }

            // find & link possible child
            if let Some(node_hash) = new_node.hash() {
                if let Some(&child_ref) = self.missing_bc_parents.get(&node_hash) {
                    if let Some(child) = self.node(child_ref) {
                        new_node.pt = child.pt + vec2(0., child.rad + new_node.rad + 30.); // TODO: handle positioning when both parent & child are set

                        assert!(child.parent.is_none());
                        assert!(
                            child.height == new_node.height + 1,
                            "child height: {}, new height: {}",
                            child.height,
                            new_node.height
                        );
                        child.parent = node_ref;
                    }

                    self.missing_bc_parents.remove(&node_hash);
                }
            }
        }

        if needs_fixup {
            if let Some(parent) = self.get_node(new_node.parent) {
                new_node.pt = parent.pt;
                // new_node.vel = self.nodes[parent].vel;
                // new_node.vel = self.nodes[parent].vel;
            }

            let (rel_pt, dy) = match new_node.kind {
                NodeKind::BC => {
                    if let Some(parent) = self.get_node(new_node.parent) {
                        (
                            Some(parent.pt),
                            node_dy_from_work(&new_node, self.bc_work_max),
                        )
                    } else if let Some(child) = self.get_node(child_ref) {
                        (Some(child.pt), -node_dy_from_work(child, self.bc_work_max))
                    } else {
                        (None, 0.)
                    }
                }

                NodeKind::BFT => (
                    self.get_node(tfl_nominee_from_node(self, &new_node))
                        .map(|node| node.pt),
                    0.,
                ),
            };

            let target_y = if let Some(rel_pt) = rel_pt {
                rel_pt.y - dy
            } else {
                new_node.pt.y - 100.
            };

            new_node.pt.y = if new_node.parent.is_none() {
                target_y // init the first BFT node in the right place
            } else {
                new_node.pt.y.lerp(target_y, config.new_node_ratio)
            };

            if let Some(parent) = self.get_node(new_node.parent) {
                if new_node.pt.y > parent.pt.y {
                    // warn!("points have been spawned inverted");
                }
            }
        }

        match new_node.kind {
            NodeKind::BC => {
                if let Some(work) = new_node
                    .difficulty
                    .and_then(|difficulty| difficulty.to_work())
                {
                    self.bc_work_max = std::cmp::max(self.bc_work_max, work.as_u128());
                }

                if self
                    .get_node(self.bc_lo)
                    .map_or(true, |node| new_node.height < node.height)
                {
                    self.bc_lo = node_ref;
                }

                if self
                    .get_node(self.bc_hi)
                    .map_or(true, |node| new_node.height < node.height)
                {
                    self.bc_hi = node_ref;
                }
            }

            NodeKind::BFT => {}
        }

        let pos = new_node.pt;
        self.nodes.push(new_node);

        self.move_node_to(node_ref, pos);
        node_ref
    }

    fn push_bc_block(
        &mut self,
        config: &VizConfig,
        block: &Arc<Block>,
        height_hash: &(BlockHeight, BlockHash),
    ) {
        let _z = ZoneGuard::new("push BC block");

        if self.find_bc_node_by_hash(&height_hash.1).is_none() {
            // NOTE: ignore if we don't have block (header) data; we can add it later if we
            // have all the data then.
            self.push_node(
                &config,
                NodeInit::BC {
                    parent: None,

                    hash: height_hash.1 .0,
                    height: height_hash.0 .0,
                    header: *block.header,
                    difficulty: Some(block.header.difficulty_threshold),
                    txs_n: block.transactions.len() as u32,
                    is_real: true,
                    bc_block: Some(block.clone()),
                },
                Some(block.header.previous_block_hash.0),
            );
        }
    }

    fn clear_nodes(&mut self) {
        self.nodes.clear();
        (self.bc_lo, self.bc_hi) = (None, None);
        self.bc_work_max = 0;
        self.missing_bc_parents.clear();
        self.bft_block_hi_i = 0;
        self.bc_by_hash.clear();
        self.nodes_bbox = BBox::_0;
        self.accel.y_to_nodes.clear();
    }

    fn find_bc_node_by_hash(&self, hash: &BlockHash) -> NodeRef {
        let _z = ZoneGuard::new("find_bc_node_by_hash");
        *self.bc_by_hash.get(&hash.0).unwrap_or(&None)
    }

    fn find_bft_node_by_hash(&self, hash: &Blake3Hash) -> NodeRef {
        let _z = ZoneGuard::new("find_bft_node_by_hash");
        *self.bft_by_hash.get(&hash.0).unwrap_or(&None)
    }

    fn move_node_to(&mut self, node_ref: NodeRef, new_pos: Vec2) {
        let node = if let Some(node) = self.node(node_ref) {
            node
        } else {
            return;
        };
        let old_pos = node.pt;
        node.pt = new_pos;

        // find group
        let old_y = (old_pos.y * (1. / ACCEL_GRP_SIZE)).ceil() as i64;
        let new_y = (new_pos.y * (1. / ACCEL_GRP_SIZE)).ceil() as i64;

        // remove from old location
        if old_y != new_y {
            if let Some(accel) = self.accel.y_to_nodes.get_mut(&old_y) {
                let accel_i = accel.nodes.iter().position(|node| *node == node_ref);
                if let Some(accel_i) = accel_i {
                    accel.nodes.swap_remove(accel_i);
                }

                if accel.nodes.is_empty() {
                    self.accel.y_to_nodes.remove(&old_y);
                }
            }
        }

        // add to new location
        self.accel
            .y_to_nodes
            .entry(new_y)
            .and_modify(|accel| {
                if let None = accel.nodes.iter().position(|node| *node == node_ref) {
                    accel.nodes.push(node_ref);
                }
            })
            .or_insert(AccelEntry {
                nodes: vec![node_ref],
            });
    }
}

impl Default for VizCtx {
    fn default() -> Self {
        VizCtx {
            fix_screen_o: Vec2::new(0.0, -300.0),
            screen_o: Vec2::_0,
            screen_vel: Vec2::_0,
            mouse_drag: MouseDrag::Nil,
            mouse_drag_d: Vec2::_0,
            old_mouse_pt: Vec2::_0,
            nodes: Vec::with_capacity(512),
            nodes_bbox: BBox::_0,
            accel: Accel {
                y_to_nodes: HashMap::new(),
            },

            bc_lo: None,
            bc_hi: None,
            bc_work_max: 0,
            missing_bc_parents: HashMap::new(),
            bc_by_hash: HashMap::new(),
            bft_by_hash: HashMap::new(),

            bft_block_hi_i: 0,
            bft_last_added: None,
            bft_fake_id: !0,
        }
    }
}

fn sat_sub_2_sided(val: i32, d: u32) -> i32 {
    let diff: i32 = d.try_into().unwrap();
    if val >= diff || val < 0 {
        val - diff
    } else {
        0
    }
}

fn sat_add_2_sided(val: i32, d: u32) -> i32 {
    let diff: i32 = d.try_into().unwrap();
    if val <= -1 - diff || val > 0 {
        val + diff
    } else {
        -1
    }
}

fn draw_texture(tex: &Texture2D, pt: Vec2, col: color::Color) {
    texture::draw_texture(tex, pt.x, pt.y, col)
}
fn draw_line(pt0: Vec2, pt1: Vec2, stroke_thickness: f32, col: color::Color) {
    shapes::draw_line(pt0.x, pt0.y, pt1.x, pt1.y, stroke_thickness, col)
}
fn draw_horizontal_line(y: f32, stroke_thickness: f32, col: color::Color) {
    shapes::draw_line(0., y, window::screen_width(), y, stroke_thickness, col);
}
fn draw_vertical_line(x: f32, stroke_thickness: f32, col: color::Color) {
    shapes::draw_line(x, 0., x, window::screen_height(), stroke_thickness, col);
}
fn draw_rect(rect: Rect, col: color::Color) {
    shapes::draw_rectangle(rect.x, rect.y, rect.w, rect.h, col)
}
fn draw_rect_lines(rect: Rect, thick: f32, col: color::Color) {
    shapes::draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, thick, col)
}
fn draw_circle(circle: Circle, col: color::Color) {
    let sides_n = 30; // TODO: base on radius
    shapes::draw_poly(circle.x, circle.y, sides_n, circle.r, 0., col)
}
fn draw_circle_lines(circle: Circle, thick: f32, col: color::Color) {
    let sides_n = 30; // TODO: base on radius
    shapes::draw_arc(circle.x, circle.y, sides_n, circle.r, 0., thick, 360.0, col)
}
fn draw_ring(circle: Circle, thick: f32, thick_ratio: f32, col: color::Color) {
    let sides_n = 30; // TODO: base on radius
    let r = circle.r - thick_ratio * thick;
    shapes::draw_arc(circle.x, circle.y, sides_n, r, 0., thick, 360.0, col)
}

fn draw_arc(
    pt: Vec2,
    rad: f32,
    rot_deg: f32,
    arc_deg: f32,
    thick: f32,
    thick_ratio: f32,
    col: color::Color,
) {
    let sides_n = 30; // TODO: base on radius
    let r = rad - thick_ratio * thick;
    shapes::draw_arc(pt.x, pt.y, sides_n, r, rot_deg, thick, arc_deg, col)
}

fn draw_arrow(bgn_pt: Vec2, end_pt: Vec2, thick: f32, head_size: f32, col: color::Color) {
    let dir = (end_pt - bgn_pt).normalize_or_zero();
    let line_end_pt = end_pt - dir * head_size;
    let perp = dir.perp() * 0.5 * head_size;
    draw_line(bgn_pt, line_end_pt, thick, col);
    draw_triangle(end_pt, line_end_pt + perp, line_end_pt - perp, col);
}

fn draw_arrow_lines(bgn_pt: Vec2, end_pt: Vec2, thick: f32, head_size: f32, col: color::Color) {
    let dir = (end_pt - bgn_pt).normalize_or_zero();
    let line_end_pt = end_pt - dir * head_size;
    let perp = dir.perp() * 0.5 * head_size;
    draw_line(bgn_pt, end_pt, thick, col);
    draw_line(end_pt, line_end_pt + perp, thick, col);
    draw_line(end_pt, line_end_pt - perp, thick, col);
}

fn draw_x(pt: Vec2, rad: f32, thick: f32, col: color::Color) {
    let v = 0.5 * std::f32::consts::SQRT_2 * rad;
    draw_line(pt + vec2(-v, v), pt + vec2(v, -v), thick, col);
    draw_line(pt + vec2(v, v), pt + vec2(-v, -v), thick, col);
}

fn draw_crosshair(pt: Vec2, rad: f32, thick: f32, col: color::Color) {
    draw_line(pt - vec2(0., rad), pt + vec2(0., rad), thick, col);
    draw_line(pt - vec2(rad, 0.), pt + vec2(rad, 0.), thick, col);
}

fn draw_text(text: &str, pt: Vec2, font_size: f32, col: color::Color) -> TextDimensions {
    text::draw_text(text, pt.x, pt.y, font_size, col)
}
fn draw_multiline_text(
    text: &str,
    pt: Vec2,
    font_size: f32,
    line_distance_factor: Option<f32>,
    col: color::Color,
) {
    text::draw_multiline_text(text, pt.x, pt.y, font_size, line_distance_factor, col)
}

fn circles_closest_pts(a: Circle, b: Circle) -> (Vec2, Vec2) {
    (
        pt_on_circle_edge(a, b.point()),
        pt_on_circle_edge(b, a.point()),
    )
}

fn draw_arrow_between_circles(
    bgn_circle: Circle,
    end_circle: Circle,
    thick: f32,
    head_size: f32,
    col: color::Color,
) -> (Vec2, Vec2) {
    let arrow_pts = circles_closest_pts(bgn_circle, end_circle);
    draw_arrow(arrow_pts.0, arrow_pts.1, thick, head_size, col);
    arrow_pts
}

/// `align` {0..=1, 0..=1} determines which point on the text's bounding box will be placed at `pt`
/// - Bottom-left (normal text) is {0,0}
/// - Bottom-right (right-aligned) is {1,0}
/// - Centred in both dimensions is: {0.5, 0.5}
///
/// N.B. this is the based on the dimensions provided by the font, which don't always match up
/// with a minimal bounding box nor a visual proportional weighting.
fn get_text_align_pt_ex(text: &str, pt: Vec2, params: &TextParams, align: Vec2) -> Vec2 {
    let align = align.clamp(Vec2::_0, Vec2::_1);
    let dims = text::measure_text(text, params.font, params.font_size, params.font_scale);
    pt + vec2(-align.x * dims.width, align.y * dims.height)
}

fn get_text_align_pt(text: &str, pt: Vec2, font_size: f32, align: Vec2) -> Vec2 {
    get_text_align_pt_ex(
        text,
        pt,
        &TextParams {
            font_size: font_size as u16,
            font_scale: 1.,
            ..Default::default()
        },
        align,
    )
}

fn draw_text_align_ex(text: &str, pt: Vec2, params: TextParams, align: Vec2) -> TextDimensions {
    let new_pt = get_text_align_pt_ex(text, pt, &params, align);
    // bounding boxes:
    // shapes::draw_rectangle_lines(pt.x,     pt.y-dims.height,     dims.width, dims.height, 2., BLACK);
    // shapes::draw_rectangle_lines(new_pt.x, new_pt.y-dims.height, dims.width, dims.height, 2., YELLOW);
    text::draw_text_ex(text, new_pt.x, new_pt.y, params)
}

/// see [`draw_text_align_ex`]
fn draw_text_align(
    text: &str,
    pt: Vec2,
    font_size: f32,
    col: color::Color,
    align: Vec2,
) -> TextDimensions {
    draw_text_align_ex(
        text,
        pt,
        TextParams {
            font_size: font_size as u16,
            font_scale: 1.0,
            color: col,
            ..Default::default()
        },
        align,
    )
}

fn draw_text_right_align(
    text: &str,
    pt: Vec2,
    font_size: f32,
    col: color::Color,
    ch_w: f32,
) -> TextDimensions {
    draw_text(
        text,
        pt - vec2(ch_w * text.len() as f32, 0.),
        font_size,
        col,
    )
}

/// assumes point is outside circle
fn pt_on_circle_edge(c: Circle, pt: Vec2) -> Vec2 {
    c.point().move_towards(pt, c.r)
}

fn make_circle(pt: Vec2, rad: f32) -> Circle {
    Circle {
        x: pt.x,
        y: pt.y,
        r: rad,
    }
}

// This exists because the existing [`Circle::scale`] mutates in place
fn circle_scale(circle: Circle, scale: f32) -> Circle {
    Circle {
        x: circle.x,
        y: circle.y,
        r: circle.r * scale,
    }
}

const DT: f32 = 1. / 60.;
const INV_DT: f32 = 1. / DT;
const INV_DT_SQ: f32 = 1. / (DT * DT);

/// TODO: can we avoid the sqrt round trip by doing computations on vectors
///       directly instead of on distances?
/// Both stiff_coeff & damp_coeff should be in the range [0.,1.]
/// The safe upper limit for stability depends on the number of springs connected to a node:
/// C_max ~= 1/(springs_n + 1)
///
/// If you have 2 free-floating nodes applying force to each other, plug the `reduced_mass` in for
/// mass
fn spring_force(d_pos: f32, vel: f32, mass: f32, stiff_coeff: f32, damp_coeff: f32) -> f32 {
    let stiff_force = INV_DT_SQ * d_pos * stiff_coeff;
    let damp_force = INV_DT * vel * damp_coeff;
    let mut result = -(stiff_force + damp_force) * mass;
    result += 0.;
    result
}

/// Effective mass when 2 free floating particles are both applying force to each other
/// inv_mass = 1/mass; inv_mass == 0 implies one point is completely fixed (i.e. not free-floating)
fn reduced_mass(inv_mass_a: f32, inv_mass_b: f32) -> f32 {
    1. / (inv_mass_a + inv_mass_b)
}

fn ui_camera_window<F: FnOnce(&mut ui::Ui)>(
    id: ui::Id,
    camera: &Camera2D,
    world_pt: Vec2,
    size: Vec2,
    f: F,
) -> bool {
    root_ui().move_window(id, camera.world_to_screen(world_pt));
    root_ui().window(id, vec2(0., 0.), size, f)
}

fn ui_dynamic_window<F: FnOnce(&mut ui::Ui)>(
    id: ui::Id,
    screen_pt: Vec2,
    size: Vec2,
    f: F,
) -> bool {
    root_ui().move_window(id, screen_pt);
    root_ui().window(id, vec2(0., 0.), size, f)
}

fn hash_from_u64(v: u64) -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash[..8].copy_from_slice(&v.to_le_bytes());
    hash
}

/// technically a line *segment*
fn closest_pt_on_line(line: (Vec2, Vec2), pt: Vec2) -> (Vec2, Vec2) {
    let bgn_to_end = line.1 - line.0;
    let bgn_to_pt = pt - line.0;
    let len = bgn_to_end.length();

    let t = bgn_to_pt.dot(bgn_to_end) / (len * len);
    let t_clamped = t.max(0.).min(1.);
    let norm = bgn_to_end.normalize_or(Vec2::_0);
    (line.0 + t_clamped * len * norm, norm)
}

fn checkbox(ui: &mut ui::Ui, id: ui::Id, label: &str, data: &mut bool) {
    let ch_w = ui.calc_size("#").x;
    widgets::Checkbox::new(hash!())
        .label(label)
        .pos(vec2(3. * ch_w, 0.))
        .ratio(0.)
        .ui(ui, data);
}

fn node_dy_from_work(node: &Node, bc_work_max: u128) -> f32 {
    node.difficulty
        .and_then(|difficulty| difficulty.to_work())
        .map_or(100., |work| {
            150. * work.as_u128() as f32 / bc_work_max as f32
        })
}

// naive lerp in existing color-space
fn color_lerp(a: color::Color, b: color::Color, t: f32) -> color::Color {
    color::Color {
        r: a.r.lerp(b.r, t),
        g: a.g.lerp(b.g, t),
        b: a.b.lerp(b.b, t),
        a: a.a.lerp(b.a, t),
    }
}

/// Viz implementation root
pub async fn viz_main(
    png: image::DynamicImage,
    tokio_root_thread_handle: Option<JoinHandle<()>>,
) -> Result<(), crate::service::TFLServiceError> {
    let mut ctx = VizCtx {
        old_mouse_pt: {
            let (x, y) = mouse_position();
            Vec2 { x, y }
        },
        ..VizCtx::default()
    };

    let bg_col = DARKBLUE;

    // wait for servicing thread to init
    let mut rot_deg = 0.;
    loop {
        // draw loading/splash screen
        let pt = vec2(0.5 * window::screen_width(), 0.5 * window::screen_height());

        window::clear_background(bg_col);
        let tex = Texture2D::from_rgba8(
            png.width().try_into().unwrap(),
            png.height().try_into().unwrap(),
            png.as_bytes(),
        );
        let tex_size = tex.size();
        draw_texture(&tex, pt - vec2(0.5 * tex_size.x, 1.2 * tex_size.y), WHITE);

        draw_text_align(
            "Waiting for zebra to start up...",
            pt,
            40.,
            WHITE,
            vec2(0.5, 0.5),
        );

        draw_arc(pt + vec2(0., 90.), 30., rot_deg, 60., 3., 0., WHITE);
        rot_deg += 360. / 1.25 * DT; // 1 rotation per 1.25 seconds

        if VIZ_G.lock().unwrap().is_some() {
            break;
        }
        window::next_frame().await;
    }

    let mut hover_circle_rad = 0.;
    let mut recent_hover_node_i: NodeRef = None;
    let mut old_hover_node_i: NodeRef = None;
    // we track this as you have to mouse down *and* up on the same node to count as clicking on it
    let mut mouse_dn_node_i: NodeRef = None;
    let mut click_node_i: NodeRef = None;
    let font_size = 30.;

    let base_style = root_ui()
        .style_builder()
        .font_size(font_size as u16)
        .build();
    let skin = ui::Skin {
        editbox_style: base_style.clone(),
        label_style: base_style.clone(),
        button_style: base_style.clone(),
        combobox_style: base_style.clone(),
        ..root_ui().default_skin()
    };
    root_ui().push_skin(&skin);

    let (mut bc_h_lo_prev, mut bc_h_hi_prev) = (None, None);
    let mut goto_str = String::new();
    let mut node_str = String::new();
    let mut target_bc_str = String::new();

    let mut edit_proposed_bft_string = String::new();
    let mut proposed_bft_string: Option<String> = None; // only for loop... TODO: rearrange

    let mut track_node_h: Option<i32> = None;
    let mut track_continuously: bool = true;
    let mut track_bft: bool = false;
    let mut rng = rand::rngs::StdRng::seed_from_u64(0);
    let mut new_h_rng: Option<(i32, i32)> = None;

    let mut tray_is_open = false;
    let mut tray_x = 0.;

    let mut dbg = VizDbg {
        nodes_forces: HashMap::new(),
    };

    let mut config = VizConfig {
        test_bbox: false,
        show_accel_areas: false,
        show_bft_ui: false,
        show_top_info: false,
        show_mouse_info: false,
        show_profiler: true,
        show_bft_msgs: true,
        pause_incoming_blocks: false,
        new_node_ratio: 0.9,
        node_load_kind: 0,
        audio_on: false,
        audio_volume: 0.6,
        draw_resultant_forces: false,
        draw_component_forces: false,
        do_force_links: true,
        do_force_any: true,
        do_force_edge: true,
    };

    let mut bft_msg_flags = 0;
    let mut bft_msg_vals = [0_u8; BFTMsgFlag::COUNT];

    let mut edit_tf = (Vec::<u8>::new(), Vec::<TFInstr>::new());

    init_audio(&config).await;

    loop {
        dbg.nodes_forces.clear();

        let ch_w = root_ui().calc_size("#").x; // only meaningful if monospace

        // TFL DATA ////////////////////////////////////////
        if let Some(ref thread_handle) = tokio_root_thread_handle {
            if thread_handle.is_finished() {
                break Ok(());
            }
        }

        // TODO: should we move/copy this to the end so that we can overlap frame rendering with
        // gathering data?
        let g = {
            let mut lock = VIZ_G.lock().unwrap();
            let g = lock.as_ref().unwrap().clone();

            // TODO: do outside mutex
            let bc_req_h: (i32, i32) = {
                // Hard requirements:
                // - when starting on mainnet, we want to be at the tip (!0/-1)
                // - if we're moving smoothly, we should add directly above or below the current
                //   lo/hi range to maintain a continuous chain
                // - handle being at the start of the chain
                // - handle being at the end of the chain
                // - handle the entire chain being smaller than the request range
                // Soft requirements:
                // - there should be no visible wait for loading new items on screen when scrolling
                // - minimize the number of repeated requests
                // - the request range shouldn't ping-pong between 2 values
                let h_rng = if let Some(new_h_rng) = new_h_rng {
                    new_h_rng
                } else {
                    g.bc_req_h
                };
                let (h_lo, h_hi) = abs_block_heights(h_rng, g.state.bc_tip);

                // TODO: track cached bc lo/hi and reference off that
                let mut bids_c = 0;
                let mut bids = [0u32; 2];

                let req_o = min(5, h_hi.0 - h_lo.0);

                if let (Some(on_screen_lo), Some(bc_lo)) = (bc_h_lo_prev, ctx.node(ctx.bc_lo)) {
                    if on_screen_lo <= bc_lo.height + req_o {
                        bids[bids_c] = bc_lo.height;
                        bids_c += 1;
                    }
                }
                if let (Some(on_screen_hi), Some(bc_hi)) = (bc_h_hi_prev, ctx.node(ctx.bc_hi)) {
                    if on_screen_hi + req_o >= bc_hi.height {
                        bids[bids_c] = bc_hi.height + VIZ_REQ_N;
                        bids_c += 1;
                    }
                }

                let mut new_hi = match bids_c {
                    1 => bids[0],
                    2 => (bids[0] + bids[1]) / 2,
                    _ => h_hi.0,
                } as i32;

                if let Some(tip) = g.state.bc_tip {
                    if new_hi + req_o as i32 >= tip.0 .0 as i32 {
                        new_hi = !0; // tip may have moved; track it
                    }
                };

                (sat_sub_2_sided(new_hi, VIZ_REQ_N), new_hi)
            };

            if bc_req_h != g.bc_req_h {
                // info!(
                //     "changing requested block range from {:?} to {:?}",
                //     g.bc_req_h, bc_req_h
                // );
            }

            lock.as_mut().unwrap().bc_req_h = bc_req_h;
            lock.as_mut().unwrap().proposed_bft_string = proposed_bft_string;
            lock.as_mut().unwrap().consumed = true;
            proposed_bft_string = None;
            new_h_rng = None;
            g
        };

        // Cache nodes
        // TODO: handle non-contiguous chunks
        if !config.pause_incoming_blocks {
            let _z = ZoneGuard::new("cache blocks");

            // TODO: safer access
            let is_new_bc_hi = ctx.node(ctx.bc_lo).map_or(false, |lo_node| {
                if let Some((lo_height, _)) = g.state.height_hashes.get(0) {
                    lo_node.height <= lo_height.0
                } else {
                    false
                }
            });

            if is_new_bc_hi {
                for (i, height_hash) in g.state.height_hashes.iter().enumerate() {
                    if let Some(block) = g.state.blocks[i].as_ref() {
                        ctx.push_bc_block(&config, &block, height_hash);
                    } else {
                        warn!(
                            "No data currently associated with PoW block {:?}",
                            height_hash.1
                        );
                    }
                }
            } else {
                for (i, height_hash) in g.state.height_hashes.iter().enumerate().rev() {
                    if let Some(block) = g.state.blocks[i].as_ref() {
                        ctx.push_bc_block(&config, &block, height_hash);
                    } else {
                        warn!(
                            "No data currently associated with PoW block {:?}",
                            height_hash.1
                        );
                    }
                }
            }

            bft_msg_flags |= g.state.bft_msg_flags;

            let blocks = &g.state.bft_blocks;
            for i in ctx.bft_block_hi_i..blocks.len() {
                let hash = blocks[i].blake3_hash();
                if ctx.find_bft_node_by_hash(&hash).is_none() {
                    let bft_block = blocks[i].clone();
                    if let Some(fat_ptr) = fat_pointer_to_block_at_height(
                        blocks,
                        &g.state.fat_pointer_to_bft_tip,
                        (i + 1) as u64,
                    ) {
                        let bft_parent =
                            if bft_block.previous_block_fat_ptr == FatPointerToBftBlock::null() {
                                if i != 0 {
                                    error!(
                                        "block at height {} does not have a previous_block_hash",
                                        i + 1
                                    );
                                    assert!(false);
                                }
                                None
                            } else if let Some(bft_parent) = ctx.find_bft_node_by_hash(
                                &bft_block.previous_block_fat_ptr.points_at_block_hash(),
                            ) {
                                Some(bft_parent)
                            } else {
                                assert!(
                                    false,
                                    "couldn't find parent at hash {}",
                                    &bft_block.previous_block_fat_ptr.points_at_block_hash()
                                );
                                None
                            };

                        ctx.bft_last_added = ctx.push_node(
                            &config,
                            NodeInit::BFT {
                                // TODO: distance should be proportional to difficulty of newer block
                                parent: bft_parent,
                                is_real: true,

                                hash: hash.0,

                                bft_block: BftBlockAndFatPointerToIt {
                                    block: bft_block,
                                    fat_ptr,
                                },
                                text: "".to_string(),
                                height: bft_parent.map_or(Some(1), |_| None),
                            },
                            None,
                        );
                    } else {
                        error!(
                            "no matching fat pointer for BFT block {}",
                            bft_block.blake3_hash()
                        );
                    }
                }
            }
            ctx.bft_block_hi_i = blocks.len();
        }

        let world_camera = Camera2D {
            target: ctx.screen_o,
            // this makes it so that when resizing the window, the centre of the screen stays there.
            offset: vec2(1. / window::screen_width(), 1. / window::screen_height()),
            zoom: vec2(
                1. / window::screen_width() * 2.,
                1. / window::screen_height() * 2.,
            ),
            ..Default::default()
        };

        let world_screen_size = world_camera
            .screen_to_world(vec2(window::screen_width(), window::screen_height()))
            - world_camera.screen_to_world(Vec2::_0);

        // INPUT ////////////////////////////////////////
        let mouse_pt = {
            let (x, y) = mouse_position();
            Vec2 { x, y }
        };
        let world_mouse_pt = world_camera.screen_to_world(mouse_pt);
        let old_world_mouse_pt = world_camera.screen_to_world(ctx.old_mouse_pt);
        let mouse_l_is_down = is_mouse_button_down(MouseButton::Left);
        let mouse_l_is_pressed = is_mouse_button_pressed(MouseButton::Left);
        let mouse_l_is_released = is_mouse_button_released(MouseButton::Left);
        let mouse_is_over_ui = root_ui().is_mouse_over(mouse_pt);
        let mouse_l_is_world_down = !mouse_is_over_ui && mouse_l_is_down;
        let mouse_l_is_world_pressed = !mouse_is_over_ui && mouse_l_is_pressed;
        let mouse_l_is_world_released = !mouse_is_over_ui && mouse_l_is_released;

        let mut mouse_was_node_drag = false; // @jank: shouldn't need extra state for this

        // NOTE: currently if the mouse is over UI we don't let it drag the world around.
        // This means that if the user starts clicking on a button and then changes their mind,
        // they can release it off the button to cancel the action. Otherwise the button gets
        // "stuck" to their mouse.
        // ALT: allow world dragging when mouse is over ui window chrome, but not interactive
        // elements.
        if mouse_l_is_pressed {
            ctx.mouse_drag = if mouse_is_over_ui {
                MouseDrag::Ui
            } else {
                root_ui().clear_input_focus();
                MouseDrag::World(mouse_pt)
            };
        } else {
            if mouse_l_is_released {
                ctx.fix_screen_o -= ctx.mouse_drag_d; // follow drag preview

                // used for momentum after letting go
                match ctx.mouse_drag {
                    MouseDrag::World(_) => ctx.screen_vel = mouse_pt - ctx.old_mouse_pt, // ALT: average of last few frames?
                    MouseDrag::Node { .. } => mouse_was_node_drag = true,
                    _ => {}
                }
                ctx.mouse_drag = MouseDrag::Nil;
            }
            ctx.mouse_drag_d = Vec2::_0;
        }

        if let MouseDrag::World(press_pt) = ctx.mouse_drag {
            ctx.mouse_drag_d = mouse_pt - press_pt;

            let old_hover_node = ctx.get_node(old_hover_node_i);
            if let Some(old_hover_node) = old_hover_node {
                if ctx.mouse_drag_d.length_squared() > 2. * 2. {
                    let start_pt = world_camera.screen_to_world(press_pt);
                    ctx.mouse_drag = MouseDrag::Node {
                        start_pt,
                        node: old_hover_node_i,
                        mouse_to_node: old_hover_node.pt - start_pt,
                    };
                }
            } else {
                ctx.screen_vel = mouse_pt - ctx.old_mouse_pt;
            }
            // window::clear_background(BLUE); // TODO: we may want a more subtle version of this
        } else {
            let (scroll_x, scroll_y) = mouse_wheel();
            // Potential per platform conditional compilation needed.
            ctx.screen_vel += vec2(8.0 * scroll_x, 8.0 * scroll_y);

            ctx.fix_screen_o -= ctx.screen_vel; // apply "momentum"
            ctx.screen_vel = ctx.screen_vel.lerp(Vec2::_0, 0.12); // apply friction
        }
        window::clear_background(bg_col);

        if is_key_down(KeyCode::Escape) {
            ctx.mouse_drag_d = Vec2::_0;
            ctx.mouse_drag = MouseDrag::Nil;
        }

        if is_key_down(KeyCode::LeftControl) && is_key_pressed(KeyCode::E) {
            // serialization::write_to_file_internal(
            //     "./zebra-crosslink/viz_state.json",
            //     &g.state,
            //     &ctx,
            //     g.params,
            // );
        }

        ctx.fix_screen_o = BBox::clamp(
            BBox::expand(ctx.nodes_bbox, 0.5 * world_screen_size),
            ctx.fix_screen_o,
        );
        ctx.screen_o = ctx.fix_screen_o - ctx.mouse_drag_d; // preview drag

        // WORLD SPACE DRAWING ////////////////////////////////////////
        set_camera(&world_camera); // NOTE: can use push/pop camera state if useful
        let world_bbox = BBox {
            min: world_camera.screen_to_world(Vec2::_0),
            max: world_camera
                .screen_to_world(vec2(window::screen_width(), window::screen_height())),
        };

        let world_bbox = if config.test_bbox {
            // test bounding box functionality by shrinking it on screen
            let t = 0.25;
            BBox {
                min: Vec2 {
                    x: world_bbox.min.x.lerp(world_bbox.max.x, t),
                    y: world_bbox.min.y.lerp(world_bbox.max.y, t),
                },
                max: Vec2 {
                    x: world_bbox.max.x.lerp(world_bbox.min.x, t),
                    y: world_bbox.max.y.lerp(world_bbox.min.y, t),
                },
            }
        } else {
            world_bbox
        };
        assert!(world_bbox.min.x < world_bbox.max.x);
        assert!(world_bbox.min.y < world_bbox.max.y);
        let world_rect = Rect::from(world_bbox);

        if config.test_bbox {
            draw_rect_lines(world_rect, 2., MAGENTA);
        }

        if config.show_accel_areas {
            let col = LIGHTGRAY;
            for (y_grp, accel) in &ctx.accel.y_to_nodes {
                let y = *y_grp as f32 * ACCEL_GRP_SIZE; // draw at top of section
                let ol = (
                    flt_max(world_bbox.min.y, y - ACCEL_GRP_SIZE),
                    flt_min(world_bbox.max.y, y),
                );
                if ol.0 < ol.1 {
                    let mut l = vec2(world_bbox.min.x, y);
                    let mut r = vec2(world_bbox.max.x, y);
                    draw_line(l, r, 1., col);
                    l.y -= ACCEL_GRP_SIZE;
                    r.y -= ACCEL_GRP_SIZE;
                    draw_line(l, r, 1., col);

                    let mut str = format!("y: {}, {} [ ", y, accel.nodes.len());
                    for node_ref in &accel.nodes {
                        let new_str = if let (Some(node_hdl), Some(node)) =
                            (node_ref, ctx.get_node(*node_ref))
                        {
                            &format!("{} (h: {:?}), ", node_hdl.idx, node.height)
                        } else {
                            "None, "
                        };
                        str.push_str(new_str);
                    }
                    str.push_str("]");

                    draw_text(&str, l + vec2(ch_w, ch_w), 14., col);
                }
            }
        }

        if dev(false) {
            ui_camera_window(
                hash!(),
                &world_camera,
                vec2(200., 20.),
                vec2(300., 200.),
                |ui| {
                    ui.label(
                        None,
                        if ui.is_mouse_over(mouse_pt) {
                            "over"
                        } else {
                            "not over"
                        },
                    );
                    ui.label(
                        None,
                        if ui.is_mouse_captured() {
                            "captured"
                        } else {
                            "not captured"
                        },
                    );
                    if ui.button(None, "Click") {
                        // root_ui().button(None, "Clicked");
                    }
                },
            );

            ui_camera_window(
                hash!(),
                &world_camera,
                vec2(350., 300.),
                vec2(300., 200.),
                |ui| {
                    ui.label(None, &format!("over: {}", ui.is_mouse_over(mouse_pt)));
                    if ui.button(None, "Click") {
                        // root_ui().button(None, "Clicked");
                    }
                },
            );
        }

        // DEBUG NODE/BFT INPUTS ////////////////////////////////////////////////////////////
        if config.show_bft_ui {
            let text_size = vec2(32. * ch_w, 1.2 * font_size);
            let bc_i_size = vec2(15. * ch_w, text_size.y);
            // TODO: is there a nicer way of sizing windows to multiple items?
            let text_wnd_size =
                text_size + vec2(bc_i_size.x + 1.5 * font_size, 6.) + vec2(0., 1.2 * font_size);
            ui_dynamic_window(
                hash!(),
                vec2(
                    0.5 * font_size,
                    window::screen_height() - (text_wnd_size.y + 0.5 * font_size),
                ),
                text_wnd_size,
                |ui| {
                    let mut enter_pressed = false;
                    enter_pressed |= widgets::Editbox::new(hash!(), text_size)
                        .multiline(false)
                        .ui(ui, &mut node_str)
                        && (is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter));

                    ui.same_line(text_size.x + font_size);

                    enter_pressed |= widgets::Editbox::new(hash!(), bc_i_size)
                        .multiline(false)
                        .filter(&|ch| char::is_ascii_digit(&ch))
                        .ui(ui, &mut target_bc_str)
                        && (is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter));

                    if enter_pressed {
                        let id = {
                            ctx.bft_fake_id -= 1;
                            ctx.bft_fake_id + 1
                        };

                        let bft_block = BftBlock {
                            version: 0,
                            height: 0,
                            previous_block_fat_ptr: FatPointerToBftBlock::null(),
                            finalization_candidate_height: 0,
                            headers: loop {
                                let bc: Option<u32> = target_bc_str.trim().parse().ok();
                                if bc.is_none() {
                                    break Vec::new();
                                }

                                let node = if let Some(node) = ctx.node(find_bc_node_i_by_height(
                                    &ctx.nodes,
                                    BlockHeight(bc.unwrap()),
                                )) {
                                    node
                                } else {
                                    break Vec::new();
                                };

                                break match node.header {
                                    VizHeader::BlockHeader(hdr) => vec![hdr],
                                    _ => Vec::new(),
                                };
                            },
                        };

                        ctx.bft_last_added = ctx.push_node(
                            &config,
                            NodeInit::BFT {
                                parent: ctx.bft_last_added,
                                is_real: false,

                                hash: bft_block.blake3_hash().0,

                                bft_block: BftBlockAndFatPointerToIt {
                                    block: bft_block,
                                    fat_ptr: FatPointerToBftBlock::null(),
                                },
                                text: node_str.clone(),
                                height: ctx.bft_last_added.map_or(Some(0), |_| None),
                            },
                            None,
                        );

                        node_str = "".to_string();
                        target_bc_str = "".to_string();
                    }

                    if widgets::Editbox::new(hash!(), vec2(32. * ch_w, font_size))
                        .multiline(false)
                        .ui(ui, &mut edit_proposed_bft_string)
                        && (is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter))
                    {
                        proposed_bft_string = Some(edit_proposed_bft_string.clone())
                    }
                },
            );
        }

        // UI CONTROLS ////////////////////////////////////////////////////////////
        let track_button_txt = "Track height";
        let goto_button_txt = "Goto height";
        let controls_txt_size = vec2(12. * ch_w, font_size);
        let controls_wnd_size =
            controls_txt_size + vec2((track_button_txt.len() + 2) as f32 * ch_w, 2.2 * font_size);
        ui_dynamic_window(
            hash!(),
            vec2(
                window::screen_width() - (controls_wnd_size.x + 0.5 * font_size),
                window::screen_height() - (controls_wnd_size.y + 0.5 * font_size),
            ),
            controls_wnd_size,
            |ui| {
                let enter_pressed = widgets::Editbox::new(hash!(), controls_txt_size)
                    .multiline(false)
                    .filter(&|ch| char::is_ascii_digit(&ch) || ch == '-')
                    .ui(ui, &mut goto_str)
                    && (is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter));

                ui.same_line(controls_txt_size.x + ch_w);

                if ui.button(
                    None,
                    if track_continuously {
                        track_button_txt
                    } else {
                        goto_button_txt
                    },
                ) || enter_pressed
                {
                    track_node_h = goto_str.trim().parse::<i32>().ok();
                }

                widgets::Checkbox::new(hash!())
                    .label("Track Continuously")
                    .ratio(0.12)
                    .ui(ui, &mut track_continuously);
                widgets::Checkbox::new(hash!())
                    .label("Target BFT Block")
                    .ratio(0.12)
                    .ui(ui, &mut track_bft);
            },
        );

        if let Some(h) = track_node_h {
            if track_bft {
                let h = h as isize;
                let abs_h: isize = if h < 0 {
                    g.state.bft_blocks.len() as isize + 1 + h
                } else {
                    h
                };

                let mut clear_bft_bc_h = None;
                if let Some(node) = find_bft_node_by_height(&ctx.nodes, abs_h as u32) {
                    let mut is_done = false;
                    if let Some(bft_block) = node.header.as_bft() {
                        if let Some(bc_hdr) = bft_block.block.headers.last() {
                            if ctx.find_bc_node_by_hash(&bc_hdr.hash()).is_none() {
                                clear_bft_bc_h = Some(
                                    bft_block.block.finalization_candidate_height
                                        + bft_block.block.headers.len() as u32
                                        - 1,
                                );
                                is_done = true;
                            }
                        }
                    }

                    if !is_done {
                        let d_y: f32 = node.pt.y - ctx.fix_screen_o.y;
                        ctx.fix_screen_o.y += 0.4 * d_y;
                        if !track_continuously && d_y.abs() < 1. {
                            track_node_h = None;
                        }
                    }
                } else {
                    println!("couldn't find node at {}", h);
                    track_node_h = None; // ALT: track indefinitely until it appears
                }

                if let Some(bft_bc_h) = clear_bft_bc_h {
                    ctx.clear_nodes();
                    let hi = i32::try_from(bft_bc_h + VIZ_REQ_N / 2).unwrap();
                    new_h_rng = Some((sat_sub_2_sided(hi, VIZ_REQ_N), hi));
                    // NOTE: leaves track_node_h as Some to be fine-tuned once the
                    // BC nodes are present
                }
            } else {
                let abs_h = abs_block_height(h, g.state.bc_tip);
                if let Some(node) = ctx.node(find_bc_node_i_by_height(&ctx.nodes, abs_h)) {
                    let d_y: f32 = node.pt.y - ctx.fix_screen_o.y;
                    ctx.fix_screen_o.y += 0.4 * d_y;
                    if !track_continuously && d_y.abs() < 1. {
                        track_node_h = None;
                    }
                } else if g.state.bc_tip.is_some() && abs_h.0 <= g.state.bc_tip.unwrap().0 .0 {
                    ctx.clear_nodes();
                    let hi = i32::try_from(abs_h.0 + VIZ_REQ_N / 2).unwrap();
                    new_h_rng = Some((sat_sub_2_sided(hi, VIZ_REQ_N), hi));
                } else {
                    println!("couldn't find node at {}", h);
                    track_node_h = None; // ALT: track indefinitely until it appears
                }
            }
        }

        // HANDLE NODE SELECTION ////////////////////////////////////////////////////////////
        let drag_node_ref: NodeRef = if let MouseDrag::Node {
            node: node_ref,
            start_pt,
            mouse_to_node,
        } = ctx.mouse_drag
        {
            if let Some(drag_node) = ctx.node(node_ref) {
                drag_node.vel = world_mouse_pt - old_world_mouse_pt;
                ctx.move_node_to(node_ref, world_mouse_pt + mouse_to_node);
                node_ref
            } else {
                None
            }
        } else {
            None
        };

        let hover_node_i: NodeRef = if let Some(drag_node_i) = drag_node_ref {
            drag_node_ref
        } else if mouse_is_over_ui {
            None
        } else {
            // Selection ring (behind node circle)
            let mut hover_node_i: NodeRef = None;
            for (i, node) in ctx.nodes.iter().enumerate() {
                let circle = node.circle();
                if circle.contains(&world_mouse_pt) {
                    hover_node_i = ctx.node_ref(i);
                    break;
                }
            }

            hover_node_i
        };

        let rad_mul = if let Some(node_i) = hover_node_i {
            recent_hover_node_i = hover_node_i;
            std::f32::consts::SQRT_2
        } else {
            0.9
        };

        if let Some(old_hover_node) = ctx.node(recent_hover_node_i) {
            let target_rad = old_hover_node.rad * rad_mul;
            hover_circle_rad = hover_circle_rad.lerp(target_rad, 0.1);
            if hover_circle_rad > old_hover_node.rad {
                let col = if mouse_l_is_world_down {
                    YELLOW
                } else {
                    SKYBLUE
                };
                draw_ring(
                    make_circle(old_hover_node.pt, hover_circle_rad),
                    2.,
                    1.,
                    col,
                );
            }
        }

        if old_hover_node_i != hover_node_i && hover_node_i.is_some() {
            play_sound_once(&config, Sound::HoverNode);
        }
        old_hover_node_i = hover_node_i;

        // TODO: this is lower precedence than inbuilt macroquad UI to allow for overlap
        // TODO: we're sort of duplicating handling for mouse clicks & drags; dedup
        if mouse_l_is_world_pressed {
            mouse_dn_node_i = hover_node_i;
        } else if mouse_l_is_world_released
            && hover_node_i == mouse_dn_node_i
            && !mouse_was_node_drag
        {
            let hover_node = ctx.get_node(hover_node_i);
            // node is clicked on
            if hover_node.is_some()
                && (is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl))
            {
                // create new node
                let hover_node = hover_node.unwrap();
                let kind = hover_node.kind;
                let height = hover_node.height + 1;
                let is_bft = kind == NodeKind::BFT;
                let (header, id, bc_block) = match hover_node.kind {
                    NodeKind::BC => {
                        let header = BlockHeader {
                            version: 4,
                            previous_block_hash: BlockHash(
                                hover_node.hash().expect("should have a hash"),
                            ),
                            merkle_root: zebra_chain::block::merkle::Root([0; 32]),
                            commitment_bytes: zebra_chain::fmt::HexDebug([0; 32]),
                            time: chrono::Utc::now(),
                            difficulty_threshold: INVALID_COMPACT_DIFFICULTY,
                            nonce: zebra_chain::fmt::HexDebug([0; 32]),
                            solution: zebra_chain::work::equihash::Solution::for_proposal(),
                        };
                        let id = NodeId::Hash(header.hash().0);
                        (VizHeader::BlockHeader(header), id, None)
                    }

                    NodeKind::BFT => {
                        let bft_block = BftBlock {
                            version: 0,
                            height: 0,
                            previous_block_fat_ptr: FatPointerToBftBlock::null(),
                            finalization_candidate_height: 0,
                            headers: Vec::new(),
                        };

                        let id = NodeId::Hash(bft_block.blake3_hash().0);
                        let bft_block_and_ptr = BftBlockAndFatPointerToIt {
                            block: bft_block,
                            fat_ptr: FatPointerToBftBlock::null(),
                        };
                        (VizHeader::BftBlock(bft_block_and_ptr), id, None)
                    }
                };

                let node_ref = ctx.push_node(
                    &config,
                    NodeInit::Dyn {
                        parent: hover_node_i,

                        kind,
                        height,
                        bc_block,

                        text: "".to_string(),
                        id,
                        is_real: false,
                        difficulty: None,
                        header,
                        txs_n: (kind == NodeKind::BC) as u32,
                    },
                    None,
                );

                if is_bft {
                    ctx.bft_last_added = node_ref;
                }
            } else {
                click_node_i = hover_node_i;
            }
        }

        // APPLY SPRING FORCES ////////////////////////////////////////////////////////////
        // TODO:
        // - parent-child nodes should be a certain *height* apart (x-distance unimportant)
        //   - TODO: is this a symmetrical/asymmetrical/one-way force? (e.g. both parent & child
        //     move or just child)
        // - cross-chain links aim for horizontal(?)
        // - all other nodes try to enforce a certain distance
        // - move perpendicularly/horizontally away from non-coincident edges (strength
        // inversely-proportional to edge length? - As if equivalent mass stretched out across line...)

        #[derive(PartialEq)]
        enum SpringMethod {
            Old,
            Coeff,
        }
        let spring_method = SpringMethod::Coeff;

        let min_grp = (world_bbox.min.y * (1. / ACCEL_GRP_SIZE)).ceil() as i64 - 1;
        let max_grp = (world_bbox.max.y * (1. / ACCEL_GRP_SIZE)).ceil() as i64 + 1;
        let mut on_screen_node_refs: Vec<NodeRef> = Vec::with_capacity(ctx.nodes.len());
        for grp in min_grp..=max_grp {
            if let Some(accel) = ctx.accel.y_to_nodes.get(&grp) {
                for node_ref in &accel.nodes {
                    if node_ref.is_some() {
                        on_screen_node_refs.push(*node_ref);
                    }
                }
            }
        }

        // calculate forces
        let spring_stiffness = 160.;
        for node_ref in &on_screen_node_refs {
            let node_ref = *node_ref;
            if node_ref == drag_node_ref {
                continue;
            }

            let mut offscreen_new_pt = None;
            let (a_pt, a_vel, a_vel_mag) = if let Some(node) = ctx.get_node(node_ref) {
                if node.kind == NodeKind::BFT {
                    if let Some(bft_block) = node.header.as_bft() {
                        if !bft_block.block.headers.is_empty() {
                            let hdr_lo = ctx.find_bc_node_by_hash(
                                &bft_block.block.headers.first().unwrap().hash(),
                            );
                            let hdr_hi = ctx.find_bc_node_by_hash(
                                &bft_block.block.headers.last().unwrap().hash(),
                            );
                            if hdr_lo.is_none() && hdr_hi.is_none() {
                                if let Some(bc_lo) = ctx.get_node(ctx.bc_lo) {
                                    let max_hdr_h = bft_block.block.finalization_candidate_height
                                        + bft_block.block.headers.len() as u32
                                        - 1;
                                    if max_hdr_h < bc_lo.height {
                                        offscreen_new_pt = Some(vec2(
                                            node.pt.x,
                                            node.pt.y.max(world_bbox.max.y + world_rect.h),
                                        ));
                                        // offscreen_new_pt = Some(vec2(node.pt.x, node.pt.y.max(bc_lo.pt.y + world_rect.h)));
                                    }
                                }

                                if let Some(bc_hi) = ctx.get_node(ctx.bc_hi) {
                                    let min_hdr_h = bft_block.block.finalization_candidate_height;
                                    if min_hdr_h > bc_hi.height {
                                        offscreen_new_pt = Some(vec2(
                                            node.pt.x,
                                            node.pt.y.min(world_bbox.min.y - world_rect.h),
                                        ));
                                        // offscreen_new_pt = Some(vec2(node.pt.x, node.pt.y.min(bc_hi.pt.y - world_rect.h)));
                                    }
                                }
                            }
                        }
                    }
                }
                (node.pt, node.vel, node.vel.length())
            } else {
                continue;
            };

            if let Some(new_pt) = offscreen_new_pt {
                // info!("move point offscreen: lo: {:?}, hi: {:?}: new: {}", ctx.get_node(ctx.bc_lo).and_then(|node| Some(node.pt.y)), ctx.get_node(ctx.bc_hi).and_then(|node| Some(node.pt.y)), new_pt);
                ctx.move_node_to(node_ref, new_pt);
                continue;
            }

            // apply friction
            {
                let node = ctx.node(node_ref).unwrap();
                let friction = -1. * a_vel;
                node.acc += friction;
                dbg.new_force(node_ref, friction);
            }

            // parent-child height/link height - O(n) //////////////////////////////

            let mut target_pt = a_pt; // default to applying no force
            let mut y_is_set = false;
            let mut y_counterpart = None;

            // match heights across links for BFT
            let a_link_ref = tfl_nominee_from_node(&ctx, ctx.get_node(node_ref).unwrap());
            if a_link_ref != drag_node_ref {
                if let Some(link) = ctx.node(a_link_ref) {
                    target_pt.y = link.pt.y - font_size * 3.0 / 3.0;
                    y_counterpart = a_link_ref;
                    y_is_set = true;
                }
            }

            // align x, set parent distance for PoW work/as BFT fallback
            let a_parent = ctx.get_node(node_ref).unwrap().parent;
            if a_parent != drag_node_ref {
                if let Some(parent) = ctx.get_node(a_parent) {
                    target_pt.x = parent.pt.x;

                    if a_pt.y > parent.pt.y {
                        // warn!("nodes have become inverted");
                    }

                    if !y_is_set {
                        let intended_dy =
                            node_dy_from_work(ctx.get_node(node_ref).unwrap(), ctx.bc_work_max);
                        target_pt.y = parent.pt.y - intended_dy;
                        y_counterpart = a_parent;
                        y_is_set = true;
                    }
                }
            }

            if config.do_force_links {
                // add link/parent based forces
                // TODO: if the alignment force is ~proportional to the total amount of work
                // above the parent, this could make sure the best chain is the most linear
                let force = match spring_method {
                    SpringMethod::Old => {
                        let v = a_pt - target_pt;
                        -vec2(0.1 * spring_stiffness * v.x, 0.5 * spring_stiffness * v.y)
                    }

                    SpringMethod::Coeff => Vec2 {
                        x: spring_force(a_pt.x - target_pt.x, a_vel.x, 0.5, 0.0003, 0.1),
                        y: spring_force(a_pt.y - target_pt.y, a_vel.y, 0.5, 0.01, 0.2),
                    },
                };

                let node = ctx.node(node_ref).unwrap();
                node.acc += force;
                dbg.new_force(node_ref, force);
                if let Some(node) = ctx.node(y_counterpart) {
                    node.acc -= force;
                    dbg.new_force(y_counterpart, -force);
                }
            }

            // any-node/any-edge distance - O(n^2) //////////////////////////////
            // TODO: spatial partitioning
            for node_i2 in &on_screen_node_refs {
                let node_i2 = *node_i2;
                if node_i2 == drag_node_ref || node_i2 == node_ref {
                    continue;
                }

                let (b_pt, b_to_a, dist_sq, b_parent, b_circle) =
                    if let Some(node_2) = ctx.get_node(node_i2) {
                        let b_pt = node_2.pt;
                        let b_to_a = a_pt - b_pt;
                        let dist_sq = b_to_a.length_squared();
                        (b_pt, b_to_a, dist_sq, node_2.parent, node_2.circle())
                    } else {
                        continue;
                    };

                let target_dist = 75.;
                if config.do_force_any && dist_sq < (target_dist * target_dist) {
                    // fallback to push coincident nodes apart horizontally
                    let mut dir = b_to_a.normalize_or(vec2(1., 0.));
                    let node = ctx.get_node(node_ref).unwrap();
                    let node_2 = ctx.get_node(node_i2).unwrap();
                    if node.kind != node_2.kind {
                        let mul = if node.kind == NodeKind::BC { -1. } else { 1. };

                        dir.x = mul * dir.x.abs();
                    }
                    let target_pt = b_pt + dir * target_dist;
                    let v = a_pt - target_pt;
                    let force = match spring_method {
                        SpringMethod::Old => {
                            -vec2(1.5 * spring_stiffness * v.x, 1. * spring_stiffness * v.y)
                        }
                        // NOTE: 0.5 is the reduced mass for 2 nodes of mass 1
                        SpringMethod::Coeff => {
                            v.normalize_or(vec2(0., 0.))
                                * spring_force(v.length(), a_vel_mag, 0.5, 0.02, 0.3)
                        }
                    };

                    let node = ctx.node(node_ref).unwrap();
                    node.acc += force;

                    dbg.new_force(node_ref, force);
                }

                // apply forces perpendicular to edges
                if config.do_force_edge {
                    if b_parent == drag_node_ref || b_parent == node_ref {
                        continue;
                    }

                    let parent = if let Some(parent) = ctx.get_node(b_parent) {
                        parent
                    } else {
                        continue;
                    };

                    // the maths here can be simplified significantly if this is a perf hit
                    let edge = circles_closest_pts(b_circle, parent.circle());
                    let (pt, norm_line) = closest_pt_on_line(edge, a_pt);
                    let line_to_node = a_pt - pt;
                    let target_dist = 15.;

                    if pt != edge.0
                        && pt != edge.1
                        && line_to_node.length_squared() < (target_dist * target_dist)
                    {
                        let perp_line = norm_line.perp(); // NOTE: N/A to general capsule
                        let target_pt = if perp_line.dot(line_to_node) > 0. {
                            pt + target_dist * perp_line
                        } else {
                            pt - target_dist * perp_line
                        };

                        let m = reduced_mass(1. / 1., 1. / 2.);

                        let v = a_pt - target_pt;
                        let force = match spring_method {
                            SpringMethod::Old => {
                                -vec2(1.5 * spring_stiffness * v.x, m * spring_stiffness * v.y)
                            }
                            SpringMethod::Coeff => {
                                perp_line * spring_force(v.length(), a_vel_mag, m, 0.01, 0.001)
                            }
                        };

                        {
                            let node = ctx.node(node_ref).unwrap();
                            node.acc += force;
                            dbg.new_force(node_ref, force);
                        }
                        {
                            let node_2 = ctx.node(node_i2).unwrap();
                            node_2.acc -= force;
                            dbg.new_force(node_i2, force);
                        }
                        {
                            let parent = ctx.node(b_parent).unwrap();
                            parent.acc -= force;
                            dbg.new_force(b_parent, force);
                        }
                    }
                }
            }
        }

        if true {
            // apply forces
            for node_ref in &on_screen_node_refs {
                let new_pt = {
                    let node = ctx.node(*node_ref).unwrap();
                    node.vel = node.vel + node.acc * DT;
                    node.pt + node.vel * DT
                };
                ctx.move_node_to(*node_ref, new_pt);

                let node = ctx.node(*node_ref).unwrap();
                // NOTE: after moving; before resetting acc to 0
                if config.draw_resultant_forces {
                    if node.acc != Vec2::_0 {
                        draw_arrow_lines(node.pt, node.pt + node.acc, 1., 9., PURPLE);
                    }
                }

                match spring_method {
                    SpringMethod::Old => node.acc = -0.5 * spring_stiffness * node.vel, // TODO: or slight drag?
                    _ => node.acc = Vec2::_0,
                }
            }
        }

        // DRAW NODES & SELECTED-NODE UI ////////////////////////////////////////////////////////////
        let unique_chars_n = block_hash_unique_chars_n(&ctx.nodes);

        if let Some(click_node) = ctx.node(click_node_i) {
            let _z = ZoneGuard::new("click node UI");

            ui_camera_window(
                hash!(),
                &world_camera,
                vec2(click_node.pt.x - 350., click_node.pt.y),
                vec2(72. * ch_w, 200.),
                |ui| {
                    if let Some(hash_str) = click_node.hash_string() {
                        // draw emphasized/deemphasized hash string (unpleasant API!)
                        // TODO: different hash presentations?
                        let (remain_hash_str, unique_hash_str) =
                            str_partition_at(&hash_str, hash_str.len() - unique_chars_n);
                        ui.label(None, "Hash: ");
                        ui.push_skin(&ui::Skin {
                            label_style: ui
                                .style_builder()
                                .font_size(font_size as u16)
                                .text_color(GRAY)
                                .build(),
                            ..skin.clone()
                        });
                        ui.same_line(0.);
                        ui.label(None, remain_hash_str);
                        ui.pop_skin();

                        // NOTE: unfortunately this sometimes offsets the text!
                        ui.same_line(0.);
                        ui.label(None, unique_hash_str);
                    }

                    if click_node.kind == NodeKind::BFT || click_node.text.len() > 0 {
                        ui.input_text(hash!(), "", &mut click_node.text);
                    }

                    if click_node.txs_n > 0 {
                        ui.label(None, &format!("Transactions: {}", click_node.txs_n));
                    }

                    match &click_node.header {
                        VizHeader::None => {}
                        VizHeader::BlockHeader(hdr) => {}
                        VizHeader::BftBlock(bft_block) => {
                            ui.label(None, "PoW headers:");
                            for i in 0..bft_block.block.headers.len() {
                                let pow_hdr = &bft_block.block.headers[i];
                                ui.label(
                                    None,
                                    &format!(
                                        "  {} - {}",
                                        bft_block.block.finalization_candidate_height + i as u32,
                                        pow_hdr.hash()
                                    ),
                                );
                            }
                        }
                    }

                    if let Some(block) = &click_node.bc_block {
                        if ui.button(None, "Dump serialization") {
                            let file = std::fs::File::create("block.hex");
                            block.zcash_serialize(file.unwrap());
                            println!("dumped block.hex");
                        }
                    }
                },
            );
        }

        // ALT: EoA
        let (mut bc_h_lo, mut bc_h_hi): (Option<u32>, Option<u32>) = (None, None);
        let z_draw_nodes = begin_zone("draw nodes");
        for (i, node) in ctx.nodes.iter().enumerate() {
            // draw nodes that are on-screen
            let _z = ZoneGuard::new("draw node");

            assert!(!node.pt.x.is_nan());
            assert!(!node.pt.y.is_nan());
            assert!(!node.vel.x.is_nan());
            assert!(!node.vel.y.is_nan());
            assert!(!node.acc.x.is_nan());
            assert!(!node.acc.y.is_nan());

            ctx.nodes_bbox.update_union(BBox::from(node.pt));

            // NOTE: grows *upwards*
            let circle = node.circle();

            {
                // TODO: check line->screen intersections
                let _z = ZoneGuard::new("draw links");
                if let Some(parent) = ctx.get_node(node.parent) {
                    let line = draw_arrow_between_circles(circle, parent.circle(), 2., 9., GRAY);
                    let (pt, _) = closest_pt_on_line(line, world_mouse_pt);
                    if_dev(false, || draw_x(pt, 5., 2., MAGENTA));
                };
                if let Some(link) = if false {
                    ctx.get_node(tfl_nominee_from_node(&ctx, node))
                } else {
                    ctx.get_node(tfl_finalized_from_node(&ctx, node))
                } {
                    draw_arrow_between_circles(circle, link.circle(), 2., 9., PINK);
                }
            }

            if circle.overlaps_rect(&world_rect) {
                // node is on screen

                let is_final = if node.kind == NodeKind::BC {
                    bc_h_lo = Some(bc_h_lo.map_or(node.height, |h| min(h, node.height)));
                    bc_h_hi = Some(bc_h_hi.map_or(node.height, |h| max(h, node.height)));
                    if let Some((final_h, _)) = g.state.latest_final_block {
                        node.height <= final_h.0
                    } else {
                        false
                    }
                } else {
                    true
                };

                let col = if click_node_i.is_some() && i == click_node_i.unwrap().idx as usize {
                    RED
                } else if hover_node_i.is_some() && i == hover_node_i.unwrap().idx as usize {
                    if mouse_l_is_world_down {
                        YELLOW
                    } else {
                        SKYBLUE
                    }
                } else {
                    WHITE
                }; // TODO: depend on finality

                if is_final {
                    draw_circle(circle, col);
                } else {
                    draw_ring(circle, 3., 1., col);
                }

                let circle_text_o = circle.r + 10.;

                let z_hash_string = begin_zone("hash string");
                if let Some(hash_str) = node.hash_string() {
                    let (remain_hash_str, unique_hash_str) =
                        str_partition_at(&hash_str, hash_str.len() - unique_chars_n);

                    let pt = vec2(circle.x - circle_text_o, circle.y + 0.3 * font_size); // TODO: DPI?

                    let z_get_text_align_1 = begin_zone("get_text_align_1");
                    let text_dims = draw_text_right_align(
                        &format!("{} - {}", unique_hash_str, node.height),
                        pt,
                        font_size,
                        WHITE,
                        ch_w,
                    );
                    end_zone(z_get_text_align_1);
                    let z_get_text_align_2 = begin_zone("get_text_align_2");
                    draw_text_right_align(
                        remain_hash_str,
                        pt - vec2(text_dims.width, 0.),
                        font_size,
                        LIGHTGRAY,
                        ch_w,
                    );
                    end_zone(z_get_text_align_2);
                } else {
                    let pt = vec2(circle.x + circle_text_o, circle.y + 0.3 * font_size); // TODO: DPI?

                    let z_get_text_align_1 = begin_zone("get_text_align_1");
                    let text_dims = draw_text(&format!("{}", node.height), pt, font_size, WHITE);
                    end_zone(z_get_text_align_1);
                }
                end_zone(z_hash_string);

                let z_node_text = begin_zone("node text");
                if !node.text.is_empty() {
                    draw_text_align(
                        &format!("{} - {}", node.height, node.text),
                        vec2(circle.x + circle_text_o, circle.y),
                        font_size,
                        WHITE,
                        vec2(0., 0.5),
                    );
                }
                end_zone(z_node_text);
            }
        }
        end_zone(z_draw_nodes);

        if let Some(node) = ctx.get_node(hover_node_i) {
            if let Some(bft_block) = node.header.as_bft() {
                for i in 0..bft_block.block.headers.len() {
                    let link = if let Some(link) =
                        ctx.get_node(ctx.find_bc_node_by_hash(&bft_block.block.headers[i].hash()))
                    {
                        link
                    } else {
                        break;
                    };

                    if i == 0 {
                        draw_circle(link.circle(), PINK);
                    } else {
                        draw_ring(link.circle(), 3., 1., PINK);
                    }
                }
            }
        }

        if config.draw_component_forces {
            for (node_ref, forces) in dbg.nodes_forces.iter() {
                if let Some(node) = ctx.node(*node_ref) {
                    for force in forces {
                        if *force != Vec2::_0 {
                            draw_arrow_lines(node.pt, node.pt + *force * DT, 1., 9., ORANGE);
                        };
                    }
                }
            }
        }

        // SCREEN SPACE UI ////////////////////////////////////////
        set_default_camera();

        draw_horizontal_line(mouse_pt.y, 1., DARKGRAY);
        draw_vertical_line(mouse_pt.x, 1., DARKGRAY);

        // CONTROL TRAY
        {
            let tray_w = 32. * ch_w; // NOTE: below ~26*ch_w this starts clipping the checkbox
            let target_tray_x = if tray_is_open { tray_w } else { 0. };
            tray_x = tray_x.lerp(target_tray_x, 0.1);

            let tray_pt = vec2(tray_x, 0.5 * font_size);
            let tray_button_size = 1.2 * font_size;
            let tray_button_rect = Rect {
                x: tray_pt.x,
                y: tray_pt.y,
                w: tray_button_size,
                h: tray_button_size,
            };

            let is_hover = tray_button_rect.contains(mouse_pt);
            tray_is_open ^= is_hover && mouse_l_is_pressed;
            draw_rect(tray_button_rect, if is_hover { GRAY } else { LIGHTGRAY });
            draw_text(
                if tray_is_open { "<" } else { ">" },
                tray_pt + vec2(0.33 * font_size, 0.8 * font_size),
                font_size,
                WHITE,
            );

            ui_dynamic_window(
                hash!(),
                vec2(tray_x - tray_w, 0.),
                vec2(tray_w, window::screen_height()),
                |ui| {
                    checkbox(
                        ui,
                        hash!(),
                        "Pause incoming blocks",
                        &mut config.pause_incoming_blocks,
                    );
                    checkbox(ui, hash!(), "Test window bounds", &mut config.test_bbox);
                    checkbox(
                        ui,
                        hash!(),
                        "Show spatial acceleration",
                        &mut config.show_accel_areas,
                    );
                    checkbox(ui, hash!(), "Show BFT creation UI", &mut config.show_bft_ui);
                    checkbox(ui, hash!(), "Show top info", &mut config.show_top_info);
                    checkbox(ui, hash!(), "Show mouse info", &mut config.show_mouse_info);
                    checkbox(ui, hash!(), "Show profiler", &mut config.show_profiler);
                    checkbox(ui, hash!(), "Show BFT messages", &mut config.show_bft_msgs);
                    if !config.show_bft_msgs {
                        bft_msg_flags = 0; // prevent buildup
                    }

                    checkbox(
                        ui,
                        hash!(),
                        "Enable linked-node forces",
                        &mut config.do_force_links,
                    );
                    checkbox(
                        ui,
                        hash!(),
                        "Enable any-node forces",
                        &mut config.do_force_any,
                    );
                    checkbox(
                        ui,
                        hash!(),
                        "Enable edge-node forces",
                        &mut config.do_force_edge,
                    );
                    checkbox(
                        ui,
                        hash!(),
                        "Draw node forces",
                        &mut config.draw_resultant_forces,
                    );
                    checkbox(
                        ui,
                        hash!(),
                        "Draw component node forces",
                        &mut config.draw_component_forces,
                    );

                    ui.label(None, "Spawn spring/stable ratio");
                    ui.slider(hash!(), "", 0. ..1., &mut config.new_node_ratio);

                    #[cfg(feature = "audio")]
                    {
                        checkbox(ui, hash!(), "Audio enabled", &mut config.audio_on);
                        ui.label(None, "Audio volume");
                        ui.slider(hash!(), "", 0. ..1., &mut config.audio_volume);
                    }

                    if ui.button(None, "Clear all") {
                        ctx.clear_nodes();
                    }

                    let path: std::path::PathBuf = "blocks.zeccltf".into();
                    const NODE_LOAD_INSTRS: usize = 0;
                    const NODE_LOAD_VIZ: usize = 1;
                    const NODE_LOAD_ZEBRA: usize = 2;
                    const NODE_LOAD_STRS: [&str; 3] = {
                        let mut strs = [""; 3];
                        strs[NODE_LOAD_INSTRS] = "edit";
                        strs[NODE_LOAD_VIZ] = "visualizer";
                        strs[NODE_LOAD_ZEBRA] = "zebra";
                        strs
                    };
                    widgets::ComboBox::new(hash!(), &NODE_LOAD_STRS)
                        .label("Load to")
                        .ui(ui, &mut config.node_load_kind);

                    if ui.button(None, "Load from serialization") {
                        if let Ok((bytes, tf)) = TF::read_from_file(&path) {
                            // TODO: this needs an API pass
                            for instr_i in 0..tf.instrs.len() {
                                let instr = &tf.instrs[instr_i];
                                info!(
                                    "Loading instruction {} ({})",
                                    TFInstr::str_from_kind(instr.kind),
                                    instr.kind
                                );

                                match tf_read_instr(&bytes, instr) {
                                    Some(TestInstr::LoadPoW(block)) => {
                                        let block: Arc<Block> = Arc::new(block);

                                        // NOTE (perf): block.hash() immediately reserializes the block to
                                        // hash the canonical form...

                                        let height_hash = (
                                            block
                                                .coinbase_height()
                                                .expect("Block should have a valid height"),
                                            block.hash(),
                                        );

                                        info!(
                                            "Successfully loaded block at height {:?}, hash {}",
                                            height_hash.0, height_hash.1
                                        );

                                        match config.node_load_kind {
                                            NODE_LOAD_VIZ => {
                                                ctx.push_bc_block(&config, &block, &height_hash)
                                            }
                                            NODE_LOAD_ZEBRA => {
                                                let mut lock = G_FORCE_BLOCKS.lock().unwrap();
                                                let mut force_feed_blocks: &mut Vec<Arc<Block>> =
                                                    lock.as_mut();
                                                force_feed_blocks.push(block);
                                            }
                                            _ | NODE_LOAD_INSTRS => {}
                                        };
                                    }

                                    Some(TestInstr::LoadPoS(_)) => {
                                        todo!("LOAD_POS");
                                    }

                                    Some(TestInstr::SetParams(params)) => {
                                        debug_assert!(
                                            instr_i == 0,
                                            "should only be set at the beginning"
                                        );
                                        todo!("Actually set params");
                                    }

                                    Some(TestInstr::ExpectPoWChainLength(_)) => {
                                        todo!();
                                    }

                                    Some(TestInstr::ExpectPoSChainLength(_)) => {
                                        todo!();
                                    }

                                    None => {}
                                }
                            }

                            edit_tf = (bytes, tf.instrs);
                        }
                    }

                    if ui.button(None, "Serialize all") {
                        let mut tf = TF::new(g.params);

                        // NOTE: skipping PoW genesis
                        for node_i in 1..ctx.nodes.len() {
                            let node = &ctx.nodes[node_i];
                            if node.kind == NodeKind::BC && node.bc_block.is_some() {
                                tf.push_instr_serialize(
                                    TFInstr::LOAD_POW,
                                    node.bc_block.as_ref().unwrap(),
                                );
                            } else if node.kind == NodeKind::BFT && node.header.as_bft().is_some() {
                                tf.push_instr_serialize(
                                    TFInstr::LOAD_POS,
                                    *node.header.as_bft().as_ref().unwrap(),
                                );
                            }
                        }

                        // let file = std::fs::File::create("blocks.zeccltf");
                        // block.zcash_serialize(file.unwrap());
                        tf.write_to_file(&path);
                    }

                    widgets::Group::new(hash!(), vec2(tray_w - 5., tray_w)).ui(ui, |ui| {
                        for instr_i in 0..edit_tf.1.len() {
                            let instr = &edit_tf.1[instr_i];

                            match tf_read_instr(&edit_tf.0, instr) {
                                Some(TestInstr::LoadPoW(block)) => {
                                    ui.label(None, &format!("PoW block: {}", block.hash()));
                                }
                                _ => ui.label(None, TFInstr::str_from_kind(instr.kind)),
                            }
                        }
                    });
                },
            );
        }

        if config.show_bft_msgs {
            for variant in BFTMsgFlag::iter() {
                // animates up, stays lit for a bit, then animates down
                let i = variant as usize;
                if !config.pause_incoming_blocks {
                    let target_val = ((bft_msg_flags >> i) & 1) * 255; // number of frames each direction

                    if target_val > bft_msg_vals[i] as u64 {
                        bft_msg_vals[i] += 1;
                    } else if target_val < bft_msg_vals[i] as u64 {
                        bft_msg_vals[i] -= 1;
                    } else {
                        bft_msg_flags &= !(1 << i);
                    }
                }

                let t = min(bft_msg_vals[i], 30) as f32 / 30.;
                let col = color_lerp(DARKGREEN, GREEN, t);
                let w = 0.6 * font_size;
                let y_pad = 2.;
                let min_y = 150.;
                let y = min_y + i as f32 * (w + y_pad);
                let mut rect = Rect {
                    x: window::screen_width() - w,
                    y,
                    w,
                    h: w,
                };
                draw_rect(rect, col);

                rect.h += y_pad; // prevent gaps in hover target
                if rect.contains(mouse_pt) {
                    draw_text_right_align(
                        &format!("{:?}", variant),
                        vec2(rect.x - ch_w, rect.y + 0.9 * w),
                        font_size,
                        WHITE,
                        ch_w,
                    );
                }
            }
        }

        // VIZ DEBUG INFO ////////////////////
        if dev(config.show_top_info) {
            let dbg_str = format!(
                "{:2.3} ms\n\
                target: {}, offset: {}, zoom: {}\n\
                screen offset: {:8.3}, drag: {:7.3}, vel: {:7.3}\n\
                {} PoW blocks\n\
                Node height range: [{:?}, {:?}]\n\
                req: {:?} == {:?}\n\
                Proposed BFT: {:?}\n\
                Tracked node: {:?}\n\
                Final: {:?}\n\
                Tip: {:?}",
                time::get_frame_time() * 1000.,
                world_camera.target,
                world_camera.offset,
                world_camera.zoom,
                ctx.screen_o,
                ctx.mouse_drag_d,
                ctx.screen_vel,
                g.state.height_hashes.len(),
                bc_h_lo,
                bc_h_hi,
                g.bc_req_h,
                abs_block_heights(g.bc_req_h, g.state.bc_tip),
                g.state.internal_proposed_bft_string,
                track_node_h,
                g.state.latest_final_block,
                g.state.bc_tip,
            );
            draw_multiline_text(
                &dbg_str,
                vec2(0.5 * font_size, font_size),
                font_size,
                None,
                WHITE,
            );
        }

        if dev(config.show_mouse_info) {
            // draw mouse point's world location
            draw_multiline_text(
                &format!("{}\n{}\n{}", mouse_pt, world_mouse_pt, old_world_mouse_pt),
                mouse_pt + vec2(5., -5.),
                font_size,
                None,
                WHITE,
            );
        }

        if dev(config.show_profiler) {
            macroquad_profiler::profiler(macroquad_profiler::ProfilerParams {
                fps_counter_pos: vec2(window::screen_width() - 120., 10.),
            });
        }

        // TODO: if is_quit_requested() { send message for tokio to quit, then join }
        bc_h_lo_prev = bc_h_lo;
        bc_h_hi_prev = bc_h_hi;
        ctx.old_mouse_pt = mouse_pt;
        window::next_frame().await
    }
}

/// Sync vizualization entry point wrapper (has to take place on main thread as an OS requirement)
pub fn main(tokio_root_thread_handle: Option<JoinHandle<()>>) {
    let png_bytes = include_bytes!("../../book/theme/favicon.png");
    let png = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png).unwrap();

    let test_name: &'static str = *TEST_NAME.lock().unwrap();
    let window_title = if test_name == "‰‰TEST_NAME_NOT_SET‰‰" {
        "Zebra Blocks".to_owned()
    } else {
        format!("TEST: {}", test_name)
    };

    let config = window::Conf {
        window_title,
        fullscreen: false,
        window_width: 1200,
        window_height: 800,
        sample_count: 8,
        icon: Some({
            let icon16 = png.thumbnail_exact(16, 16);
            let icon32 = png.thumbnail_exact(32, 32);
            let icon64 = png.thumbnail_exact(64, 64);
            miniquad::conf::Icon {
                small: icon16.as_bytes().try_into().expect("correct size"),
                medium: icon32.as_bytes().try_into().expect("correct size"),
                big: icon64.as_bytes().try_into().expect("correct size"),
            }
        }),
        ..Default::default()
    };
    macroquad::Window::from_config(config, async {
        if let Err(err) = viz_main(png, tokio_root_thread_handle).await {
            macroquad::logging::error!("Error: {:?}", err);
        }
    });

    // tokio_root_thread_handle.join();
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_sat_sub_2_sided_normal() {
        assert_eq!(sat_sub_2_sided(12, 6), 6);
    }
    #[test]
    fn test_sat_sub_2_sided_saturate_on_0() {
        assert_eq!(sat_sub_2_sided(3, 6), 0);
    }
    #[test]
    fn test_sat_sub_2_sided_negative() {
        assert_eq!(sat_sub_2_sided(-6, 3), -9);
    }

    #[test]
    fn test_sat_add_2_sided_normal() {
        assert_eq!(sat_add_2_sided(12, 6), 18);
    }
    #[test]
    fn test_sat_add_2_sided_saturate_on_neg1_exact() {
        assert_eq!(sat_add_2_sided(-3, 3), -1);
    }
    #[test]
    fn test_sat_add_2_sided_saturate_on_neg1() {
        assert_eq!(sat_add_2_sided(-3, 6), -1);
    }
    #[test]
    fn test_sat_add_2_sided_negative() {
        assert_eq!(sat_add_2_sided(-6, 3), -3);
    }
}
