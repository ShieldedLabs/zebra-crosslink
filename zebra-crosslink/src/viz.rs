//! Internal vizualization

use crate::*;
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
use std::collections::HashMap;
use std::sync::Arc;
use std::thread::JoinHandle;
use zebra_chain::work::difficulty::Work;

const IS_DEV: bool = true;
fn dev(show_in_dev: bool) -> bool {
    IS_DEV && show_in_dev
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

struct VizState {
    // general chain info
    latest_final_block: Option<(BlockHeight, BlockHash)>,
    bc_tip: Option<(BlockHeight, BlockHash)>,

    // requested info
    lo_height: BlockHeight,
    hashes: Vec<BlockHash>,
    blocks: Vec<Option<Arc<Block>>>,

    internal_proposed_bft_string: Option<String>,
    bft_block_strings: Vec<String>,
}

/// self-debug info
struct VizDbg {
    nodes_forces: HashMap<usize, Vec<Vec2>>,
}

impl VizDbg {
    fn new_force(&mut self, node_i: usize, force: Vec2) {
        if dev(true) {
            let mut forces = self.nodes_forces.get_mut(&node_i);
            if let Some(forces) = forces.as_mut() {
                forces.push(force);
            } else {
                self.nodes_forces.insert(node_i, vec![force]);
            }
        }
    }
}

#[derive(Clone)]
struct VizGlobals {
    state: std::sync::Arc<VizState>,
    // wanted_height_rng: (u32, u32),
    consumed: bool, // adds one-way syncing so service_viz_requests doesn't run too quickly
    bc_req_h: (i32, i32), // negative implies relative to tip
    // TODO: bft_req_h: (i32, i32),
    proposed_bft_string: Option<String>,
}
static VIZ_G: std::sync::Mutex<Option<VizGlobals>> = std::sync::Mutex::new(None);

const VIZ_REQ_N: u32 = zebra_state::MAX_BLOCK_REORG_HEIGHT;

fn abs_block_height(height: i32, tip: Option<(BlockHeight, BlockHash)>) -> BlockHeight {
    if height >= 0 {
        BlockHeight(height.try_into().unwrap())
    } else if let Some(tip) = tip {
        tip.0.sat_sub(-height)
    } else {
        BlockHeight(0)
    }
}

fn abs_block_heights(heights: (i32, i32), tip: Option<(BlockHeight, BlockHash)>) -> (BlockHeight, BlockHeight) {
    (abs_block_height(heights.0, tip), abs_block_height(heights.1, tip))
}

/// Bridge between tokio & viz code
pub async fn service_viz_requests(tfl_handle: crate::TFLServiceHandle) {
    let call = tfl_handle.clone().call;

    *VIZ_G.lock().unwrap() = Some(VizGlobals {
        state: std::sync::Arc::new(VizState {
            latest_final_block: None,
            bc_tip: None,

            lo_height: BlockHeight(0),
            hashes: Vec::new(),
            blocks: Vec::new(),

            internal_proposed_bft_string: None,
            bft_block_strings: Vec::new(),
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
                std::cmp::min(tip_height_hash.0, abs_block_height(lo, Some(tip_height_hash))),
                std::cmp::min(tip_height_hash.0, abs_block_height(hi, Some(tip_height_hash))),
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

        let (bft_block_strings, internal_proposed_bft_string) = {
            let internal = tfl_handle.internal.lock().await;
            (
                internal.bft_block_strings.clone(),
                internal.proposed_bft_string.clone(),
            )
        };
        let new_state = VizState {
            latest_final_block: tfl_final_block_height_hash(tfl_handle.clone()).await,
            bc_tip,
            lo_height,
            hashes,
            blocks,
            internal_proposed_bft_string,
            bft_block_strings,
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

type NodeRef = Option<usize>; // TODO: generational handle
#[derive(Clone, Debug)]
struct Node {
    // structure
    parent: NodeRef, // TODO: do we need explicit edges?
    link: NodeRef,

    // data
    kind: NodeKind,
    text: String,
    hash: Option<[u8; 32]>,
    height: u32,
    work: Option<Work>,
    txs_n: u32, // N.B. includes coinbase

    is_real: bool,

    // presentation
    pt: Vec2,
    vel: Vec2,
    acc: Vec2,
    rad: f32,
}

enum NodeInit {
    Node(Node),

    Dyn {
        parent: NodeRef, // TODO: do we need explicit edges?
        link: NodeRef,

        // data
        kind: NodeKind,
        text: String,
        hash: Option<[u8; 32]>,
        height: u32,
        work: Option<Work>,
        txs_n: u32, // N.B. includes coinbase

        is_real: bool,
    },

    BC {
        parent: NodeRef, // TODO: do we need explicit edges?
        link: NodeRef,
        hash: Option<[u8; 32]>,
        height: Option<u32>, // if None, works out from parent
        work: Option<Work>,
        txs_n: u32, // N.B. includes coinbase
        is_real: bool,
    },

    BFT {
        parent: NodeRef, // TODO: do we need explicit edges?
        link: NodeRef,
        text: String,
        height: Option<u32>, // if None, works out from parent
        is_real: bool,
    },
}


impl Node {
    fn circle(&self) -> Circle {
        Circle::new(self.pt.x, self.pt.y, self.rad)
    }

    fn hash_string(&self) -> Option<String> {
        let _z = ZoneGuard::new("hash_string()");
        self.hash.map(|hash| BlockHash(hash).to_string())
    }
}

impl HasBlockHash for Node {
    fn get_hash(&self) -> Option<BlockHash> {
        if self.kind == NodeKind::BC {
            assert!(self.hash.is_some());
            self.hash.map(BlockHash)
        } else {
            None
        }
    }
}

trait ByHandle<T, H> {
    fn get_by_handle(&self, handle: H) -> Option<&T>
    where
        Self: std::marker::Sized;
}

impl ByHandle<Node, NodeRef> for [Node] {
    fn get_by_handle(&self, handle: NodeRef) -> Option<&Node> {
        if let Some(handle) = handle {
            // TODO: generations
            self.get(handle)
        } else {
            None
        }
    }
}

// TODO: extract impl Eq for Node?
fn find_bc_node_by_hash<'a>(nodes: &'a [Node], hash: &BlockHash) -> Option<&'a Node> {
    let _z = ZoneGuard::new("find_bc_node_by_hash");
    nodes
        .iter()
        .find(|node| node.kind == NodeKind::BC && node.hash.as_ref() == Some(&hash.0))
}

fn find_bc_node_i_by_height(nodes: &[Node], height: BlockHeight) -> NodeRef {
    let _z = ZoneGuard::new("find_bc_node_i_by_height");
    nodes
        .iter()
        .position(|node| node.kind == NodeKind::BC && node.height == height.0)
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
}

/// Common GUI state that may need to be passed around
#[derive(Debug)]
struct VizCtx {
    // h: BlockHeight,
    screen_o: Vec2,
    screen_vel: Vec2,
    fix_screen_o: Vec2,
    mouse_drag: MouseDrag,
    mouse_drag_d: Vec2,
    old_mouse_pt: Vec2,
    nodes: Vec<Node>,
}


fn push_node(nodes: &mut Vec<Node>, node: NodeInit) -> NodeRef {
    // TODO: dynamically update length & rad
    // TODO: could overlay internal circle/rings for shielded/transparent

    let (mut new_node, needs_fixup): (Node, bool) = match node {
        NodeInit::Node(node) => (node, false),

        NodeInit::Dyn{ parent, link, kind, text, hash, height, work, txs_n, is_real } => {
            (
                Node {
                    parent,
                    link,
                    kind,
                    text,
                    hash,
                    height, // TODO: this should be exclusively determined by parent for Dyn
                    work,
                    txs_n,
                    is_real,
                    rad: match kind {
                        NodeKind::BC => ((txs_n as f32).sqrt() * 5.).min(50.),
                        NodeKind::BFT => 10.,
                    },

                    pt: Vec2::_0,
                    vel: Vec2::_0,
                    acc: Vec2::_0,
                },
                true
            )
        },

        NodeInit::BC {parent, link, hash, height, work, txs_n, is_real} => {
            // sqrt(txs_n) for radius means that the *area* is proportional to txs_n
            let rad = ((txs_n as f32).sqrt() * 5.).min(50.);

            // DUP
            let height = if let Some(height) = height {
                assert!(parent.is_none() || nodes[parent.unwrap()].height + 1 == height);
                height
            } else {
                nodes[parent.expect("Need at least 1 of parent or height")].height + 1
            };

            (
                Node {
                    parent,
                    link,

                    kind: NodeKind::BC,
                    hash,
                    height,
                    work,
                    txs_n,
                    is_real,

                    text: "".to_string(),
                    rad,

                    pt: Vec2::_0,
                    vel: Vec2::_0,
                    acc: Vec2::_0,
                },
                true
            )
        }

        NodeInit::BFT {parent, link, height, text, is_real} => {
            let rad = 10.;
            // DUP
            let height = if let Some(height) = height {
                assert!(parent.is_none() || nodes[parent.unwrap()].height + 1 == height);
                height
            } else {
                nodes[parent.expect("Need at least 1 of parent or height")].height + 1
            };

            (
                Node {
                    parent,
                    link,

                    hash: None,
                    work: None,
                    txs_n: 0,

                    kind: NodeKind::BFT,
                    height,
                    text,

                    is_real,

                    pt: vec2(100., 0.),
                    vel: Vec2::_0,
                    acc: Vec2::_0,
                    rad,
                },
                true
            )
        }
    };

    if needs_fixup {
        if let Some(parent) = new_node.parent {
            new_node.pt = nodes[parent].pt - vec2(0., nodes[parent].rad + new_node.rad + 10.);
        }
    }

    nodes.push(new_node);

    Some(nodes.len()-1)
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

fn draw_arrow_between_circles(
    bgn_circle: Circle,
    end_circle: Circle,
    thick: f32,
    head_size: f32,
    col: color::Color,
) {
    let arrow_bgn_pt = pt_on_circle_edge(bgn_circle, end_circle.point());
    let arrow_end_pt = pt_on_circle_edge(end_circle, bgn_circle.point());
    draw_arrow(arrow_bgn_pt, arrow_end_pt, thick, head_size, col);
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

/// Viz implementation root
async fn viz_main(
    png: image::DynamicImage,
    tokio_root_thread_handle: JoinHandle<()>,
) -> Result<(), crate::service::TFLServiceError> {
    let mut ctx = VizCtx {
        fix_screen_o: Vec2::_0,
        screen_o: Vec2::_0,
        screen_vel: Vec2::_0,
        mouse_drag: MouseDrag::Nil,
        mouse_drag_d: Vec2::_0,
        old_mouse_pt: {
            let (x, y) = mouse_position();
            Vec2 { x, y }
        },
        nodes: Vec::with_capacity(512),
    };

    let nodes = &mut ctx.nodes;
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
        let dt = 1. / 60.;
        rot_deg += 360. / 1.25 * dt; // 1 rotation per 1.25 seconds

        if VIZ_G.lock().unwrap().is_some() {
            break;
        }
        window::next_frame().await;
    }

    let mut hover_circle_rad = 0.;
    let mut old_hover_node_i: NodeRef = None;
    // we track this as you have to mouse down *and* up on the same node to count as clicking on it
    let mut mouse_dn_node_i: NodeRef = None;
    let mut click_node_i: NodeRef = None;
    let mut bft_parent: NodeRef = None;
    let mut bc_hi: NodeRef = None;
    let mut bc_lo: NodeRef = None;
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
    let mut node_str = String::new();
    let mut target_bc_str = String::new();

    let mut edit_proposed_bft_string = String::new();
    let mut proposed_bft_string: Option<String> = None; // only for loop... TODO: rearrange

    let mut bc_work_max: u128 = 0;
    let mut rng = rand::rngs::StdRng::seed_from_u64(0);

    let mut dbg = VizDbg {
        nodes_forces: HashMap::new(),
    };

    loop {
        dbg.nodes_forces.clear();

        let ch_w = root_ui().calc_size("#").x; // only meaningful if monospace

        // TFL DATA ////////////////////////////////////////
        if tokio_root_thread_handle.is_finished() {
            break Ok(());
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
                let (h_lo, h_hi) = abs_block_heights(g.bc_req_h, g.state.bc_tip);

                // TODO: track cached bc lo/hi and reference off that
                let mut bids_c = 0;
                let mut bids = [0u32; 2];

                let req_o = std::cmp::min(5, h_hi.0 - h_lo.0);

                if let (Some(on_screen_lo), Some(bc_lo)) = (bc_h_lo_prev, bc_lo) {
                    if on_screen_lo <= nodes[bc_lo].height + req_o {
                        bids[bids_c] = nodes[bc_lo].height;
                        bids_c += 1;
                    }
                }
                if let (Some(on_screen_hi), Some(bc_hi)) = (bc_h_hi_prev, bc_hi) {
                    if on_screen_hi + req_o >= nodes[bc_hi].height {
                        bids[bids_c] = nodes[bc_hi].height + VIZ_REQ_N;
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
                info!(
                    "changing requested block range from {:?} to {:?}",
                    g.bc_req_h, bc_req_h
                );
            }

            lock.as_mut().unwrap().bc_req_h = bc_req_h;
            lock.as_mut().unwrap().proposed_bft_string = proposed_bft_string;
            lock.as_mut().unwrap().consumed = true;
            proposed_bft_string = None;
            g
        };

        // Cache nodes
        // TODO: handle non-contiguous chunks
        let z_cache_blocks = begin_zone("cache blocks");
        // TODO: safer access
        let new_bc_hi = bc_lo.map_or(false, |i| {
            let lo_height = g.state.lo_height.0;
            let lo_node = &nodes[i];
            lo_node.height <= lo_height
        });

        if new_bc_hi {
            // TODO: extract common code
            for (i, hash) in g.state.hashes.iter().enumerate() {
                let _z = ZoneGuard::new("cache block");

                if find_bc_node_by_hash(nodes, hash).is_none() {
                    let (work, txs_n) = g.state.blocks[i].as_ref().map_or((None, 0), |block| {
                        (
                            block.header.difficulty_threshold.to_work(),
                            block.transactions.len() as u32,
                        )
                    });

                    if let Some(work) = work {
                        bc_work_max = std::cmp::max(bc_work_max, work.as_u128());
                    }

                    bc_hi = push_node(nodes, NodeInit::BC {
                        parent: bc_hi, // TODO: can be implicit...
                        link: None,

                        hash: Some(hash.0),
                        height: Some(g.state.lo_height.0 + i as u32),
                        work,
                        txs_n,
                        is_real: true,
                    });

                    if bc_lo.is_none() {
                        bc_lo = bc_hi;
                    }
                }
            }
        } else {
            for (i, hash) in g.state.hashes.iter().enumerate().rev() {
                let _z = ZoneGuard::new("cache block");

                if find_bc_node_by_hash(nodes, hash).is_none() {
                    let (work, txs_n) = g.state.blocks[i].as_ref().map_or((None, 0), |block| {
                        (
                            block.header.difficulty_threshold.to_work(),
                            block.transactions.len() as u32,
                        )
                    });

                    if let Some(work) = work {
                        bc_work_max = std::cmp::max(bc_work_max, work.as_u128());
                    }

                    let height = g.state.lo_height.0 + i as u32;

                    let new_lo = push_node(nodes, NodeInit::BC {
                        parent: None, // new lowest
                        link: None,

                        hash: Some(hash.0),
                        height: Some(height), // TODO: -?
                        work,
                        txs_n,
                        is_real: true,
                    });

                    if let Some(child) = bc_lo {
                        nodes[new_lo.unwrap()].pt = nodes[child].pt + vec2(0., 100.);

                        assert!(nodes[child].parent.is_none());
                            assert!(
                                nodes[child].height == height + 1,
                                "child: {}, new: {}",
                                nodes[child].height,
                                height
                            );
                            nodes[child].parent = new_lo;
                        }
                        bc_lo = new_lo;

                        if bc_hi.is_none() {
                            bc_hi = bc_lo;
                        }
                    }
                }
            }

            let new_bft_height = bft_parent
                .and_then(|i| nodes.get(i))
                .map_or(0, |parent| parent.height + 1) as usize;
            let strings = &g.state.bft_block_strings;
            for i in new_bft_height..strings.len() {
                let s: Vec<&str> = strings[i].split(":").collect();
                bft_parent = push_node(nodes, NodeInit::BFT {
                    // TODO: distance should be proportional to difficulty of newer block
                    parent: bft_parent,
                    is_real: false,

                    text: s[0].to_owned(),
                    link: {
                        let bc: Option<u32> = s.get(1).unwrap_or(&"").trim().parse().ok();
                        if let Some(bc_i) = bc {
                            find_bc_node_i_by_height(nodes, BlockHeight(bc_i))
                        } else {
                            None
                        }
                    },
                    height: bft_parent.map_or(Some(0), |_| None),
                });
        }

        end_zone(z_cache_blocks);

        // INPUT ////////////////////////////////////////
        let mouse_pt = {
            let (x, y) = mouse_position();
            Vec2 { x, y }
        };
        let mouse_l_is_down = is_mouse_button_down(MouseButton::Left);
        let mouse_l_is_pressed = is_mouse_button_pressed(MouseButton::Left);
        let mouse_l_is_released = is_mouse_button_released(MouseButton::Left);
        let mouse_is_over_ui = root_ui().is_mouse_over(mouse_pt);
        let mouse_l_is_world_down = !mouse_is_over_ui && mouse_l_is_down;
        let mouse_l_is_world_pressed = !mouse_is_over_ui && mouse_l_is_pressed;
        let mouse_l_is_world_released = !mouse_is_over_ui && mouse_l_is_released;

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
                if let MouseDrag::World(_) = ctx.mouse_drag {
                    ctx.screen_vel = mouse_pt - ctx.old_mouse_pt; // ALT: average of last few frames?
                }
                ctx.mouse_drag = MouseDrag::Nil;
            }
            ctx.mouse_drag_d = Vec2::_0;
        }

        if let MouseDrag::World(press_pt) = ctx.mouse_drag {
            ctx.mouse_drag_d = mouse_pt - press_pt;
            ctx.screen_vel = mouse_pt - ctx.old_mouse_pt;
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

        ctx.screen_o = ctx.fix_screen_o - ctx.mouse_drag_d; // preview drag

        // WORLD SPACE DRAWING ////////////////////////////////////////
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
        set_camera(&world_camera); // NOTE: can use push/pop camera state if useful
        let world_mouse_pt = world_camera.screen_to_world(mouse_pt);
        let world_bbox = BBox {
            min: world_camera.screen_to_world(Vec2::_0),
            max: world_camera
                .screen_to_world(vec2(window::screen_width(), window::screen_height())),
        };

        const TEST_BBOX: bool = false; // TODO: add to a DEBUG menu
        let world_bbox = if TEST_BBOX {
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

        if TEST_BBOX {
            draw_rect_lines(world_rect, 2., MAGENTA);
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
                    bft_parent = push_node(nodes, NodeInit::BFT {
                        parent: bft_parent,
                        is_real: false,

                        text: node_str.clone(),
                        link: {
                            let bc: Option<u32> = target_bc_str.trim().parse().ok();
                            if let Some(bc_i) = bc {
                                find_bc_node_i_by_height(nodes, BlockHeight(bc_i))
                            } else {
                                None
                            }
                        },
                        height: bft_parent.map_or(Some(0), |_| None),
                    });

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


        // HANDLE NODE SELECTION ////////////////////////////////////////////////////////////
        let hover_node_i: NodeRef = if mouse_is_over_ui {
            None
        } else {
            // Selection ring (behind node circle)
            let mut hover_node_i: NodeRef = None;
            for (i, node) in nodes.iter().enumerate() {
                let circle = node.circle();
                if circle.contains(&world_mouse_pt) {
                    hover_node_i = Some(i);
                    break;
                }
            }

            hover_node_i
        };

        let rad_mul = if let Some(node_i) = hover_node_i {
            old_hover_node_i = hover_node_i;
            std::f32::consts::SQRT_2
        } else {
            0.9
        };

        if let Some(old_hover_node_i) = old_hover_node_i {
            let old_hover_node = &nodes[old_hover_node_i];
            let target_rad = old_hover_node.rad * rad_mul;
            hover_circle_rad = hover_circle_rad.lerp(target_rad, 0.1);
            if hover_circle_rad > old_hover_node.rad {
                let col = if mouse_l_is_world_down {
                    YELLOW
                } else {
                    SKYBLUE
                };
                draw_ring(make_circle(old_hover_node.pt, hover_circle_rad), 2., 1., col);
            }
        }

        // TODO: this is lower precedence than inbuilt macroquad UI to allow for overlap
        if mouse_l_is_world_pressed {
            mouse_dn_node_i = hover_node_i;
        } else if mouse_l_is_world_released && hover_node_i == mouse_dn_node_i {
            // node is clicked on
            if hover_node_i.is_some()
                && (is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl))
            {
                let hover_node = &nodes[hover_node_i.unwrap()];
                push_node(nodes, NodeInit::Dyn {
                    parent: hover_node_i,
                    link: None,

                    kind: hover_node.kind,
                    height: hover_node.height + 1,

                    text: "".to_string(),
                    hash: if hover_node.kind == NodeKind::BC  { Some([0;32])  } else { None },
                    is_real: false,
                    work: None,
                    txs_n: 0,
                });
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
        // - move perpendicularly/horizontally away from non-coincident edges

        // calculate forces
        let spring_stiffness = 160.;
        for node_i in 0..nodes.len() {
            // parent-child height - O(n) //////////////////////////////
            let a_pt =
                nodes[node_i].pt + vec2(rng.gen_range(0. ..=0.0001), rng.gen_range(0. ..=0.0001));
            if let Some(parent_i) = nodes[node_i].parent {
                if let Some(parent) = nodes.get(parent_i) {
                    let intended_height = nodes[node_i].work.map_or(100., |work| {
                        150. * work.as_u128() as f32 / bc_work_max as f32
                    });
                    let target_pt = vec2(parent.pt.x, parent.pt.y - intended_height);
                    let v = a_pt - target_pt;
                    let force = -vec2(0.1 * spring_stiffness * v.x, 0.5 * spring_stiffness * v.y);
                    nodes[node_i].acc += force;

                    dbg.new_force(node_i, force);
                }
            }

            // any-node distance - O(n^2) //////////////////////////////
            // TODO: spatial partitioning
            for node_i2 in 0..nodes.len() {
                let node_b = &nodes[node_i2];
                if node_i2 != node_i {
                    let b_to_a = a_pt - node_b.pt;
                    let dist_sq = b_to_a.length_squared();
                    let target_dist = 50.;
                    if dist_sq < (target_dist * target_dist) {
                        // fallback to push coincident nodes apart horizontally
                        let dir = b_to_a.normalize_or(vec2(1., 0.));
                        let target_pt = node_b.pt + dir * target_dist;
                        let v = a_pt - target_pt;
                        let force = -vec2(1.5 * spring_stiffness * v.x, 1. * spring_stiffness * v.y);
                        nodes[node_i].acc += force;

                        dbg.new_force(node_i, force);
                    }
                }
            }
        }

        let dt = 1. / 60.;
        if true {
            // apply forces
            for node in &mut *nodes {
                node.vel += node.acc * dt;
                node.pt += node.vel * dt;

                node.acc = -0.5 * spring_stiffness * node.vel; // TODO: or slight drag?
            }
        }

        // DRAW NODES & SELECTED-NODE UI ////////////////////////////////////////////////////////////
        let unique_chars_n = block_hash_unique_chars_n(nodes);

        if let Some(click_node_i) = click_node_i {
            let _z = ZoneGuard::new("click node UI");

            let click_node = &mut nodes[click_node_i];
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
                },
            );
        }


        // ALT: EoA
        let (mut bc_h_lo, mut bc_h_hi): (Option<u32>, Option<u32>) = (None, None);
        let z_draw_nodes = begin_zone("draw nodes");
        for (i, node) in nodes.iter().enumerate() {
            // draw nodes that are on-screen
            let _z = ZoneGuard::new("draw node");
            // NOTE: grows *upwards*
            let circle = node.circle();

            {
                // TODO: check line->screen intersections
                let _z = ZoneGuard::new("draw links");
                if let Some(parent_i) = node.parent {
                    draw_arrow_between_circles(circle, nodes[parent_i].circle(), 2., 9., GRAY);
                };
                if let Some(link) = node.link {
                    draw_arrow_between_circles(circle, nodes[link].circle(), 2., 9., PINK);
                }
            }

            if circle.overlaps_rect(&world_rect) {
                // node is on screen

                let is_final = if node.kind == NodeKind::BC {
                    bc_h_lo = Some(bc_h_lo.map_or(node.height, |h| std::cmp::min(h, node.height)));
                    bc_h_hi = Some(bc_h_hi.map_or(node.height, |h| std::cmp::max(h, node.height)));
                    if let Some((final_h, _)) = g.state.latest_final_block {
                        node.height <= final_h.0
                    } else {
                        false
                    }
                } else {
                    true
                };

                let col = if click_node_i.is_some() && i == click_node_i.unwrap() {
                    RED
                } else if hover_node_i.is_some() && i == hover_node_i.unwrap() {
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
                }
                end_zone(z_hash_string);

                let z_node_text = begin_zone("node text");
                if node.text.len() > 0 {
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

        if dev(true) {
            // draw forces
            // draw resultant force
            for node in &mut *nodes {
                if node.acc != Vec2::_0 {
                    draw_arrow_lines(node.pt, node.pt + node.acc * dt, 1., 9., PURPLE);
                }
            }

            // draw component forces
            for (node_i, forces) in dbg.nodes_forces.iter() {
                let node = &nodes[*node_i];
                for force in forces {
                    if *force != Vec2::_0 {
                        draw_arrow_lines(node.pt, node.pt + *force * dt, 1., 9., ORANGE);
                    };
                }
            }
        }

        // SCREEN SPACE UI ////////////////////////////////////////
        set_default_camera();

        draw_horizontal_line(mouse_pt.y, 1., DARKGRAY);
        draw_vertical_line(mouse_pt.x, 1., DARKGRAY);

        // VIZ DEBUG INFO ////////////////////
        if IS_DEV {
            let dbg_str = format!(
                "{:2.3} ms\n\
                target: {}, offset: {}, zoom: {}\n\
                screen offset: {:8.3}, drag: {:7.3}, vel: {:7.3}\n\
                {} hashes\n\
                Node height range: [{:?}, {:?}]\n\
                req: {:?} == {:?}\n\
                Proposed BFT: {:?}",
                time::get_frame_time() * 1000.,
                world_camera.target,
                world_camera.offset,
                world_camera.zoom,
                ctx.screen_o,
                ctx.mouse_drag_d,
                ctx.screen_vel,
                g.state.hashes.len(),
                bc_h_lo,
                bc_h_hi,
                g.bc_req_h,
                abs_block_heights(g.bc_req_h, g.state.bc_tip),
                g.state.internal_proposed_bft_string,
            );
            draw_multiline_text(
                &dbg_str,
                vec2(0.5 * font_size, font_size),
                font_size,
                None,
                WHITE,
            );

            // draw mouse point's world location
            let old_pt = world_camera.screen_to_world(ctx.old_mouse_pt);
            draw_multiline_text(
                &format!("{}\n{}\n{}", mouse_pt, world_mouse_pt, old_pt),
                mouse_pt + vec2(5., -5.),
                font_size,
                None,
                WHITE,
            );

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
pub fn main(tokio_root_thread_handle: JoinHandle<()>) {
    let png_bytes = include_bytes!("../../book/theme/favicon.png");
    let png = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png).unwrap();

    let config = window::Conf {
        window_title: "Zcash blocks".to_owned(),
        fullscreen: false,
        window_width: 1200,
        window_height: 800,
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
mod tests {
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
