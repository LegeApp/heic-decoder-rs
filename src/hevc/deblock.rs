//! Deblocking filter (H.265 section 8.7.2)
//!
//! The deblocking filter smooths block edges caused by block-based coding
//! to improve visual quality. It operates on:
//! - Transform block (TU) boundaries
//! - Prediction block (PU) boundaries
//!
//! Process steps:
//! 1. Mark edges to filter (8.7.2.2, 8.7.2.3)
//! 2. Derive boundary strength bS (8.7.2.4)
//! 3. Apply filtering decisions and filters (8.7.2.5)

use super::params::{Pps, Sps};
use super::picture::DecodedFrame;
use super::slice::SliceHeader;
use alloc::vec;
use alloc::vec::Vec;

/// Beta table for deblocking threshold (H.265 Table 8-17)
const BETA_TABLE: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17,
    18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, 58, 60, 62,
    64,
];

/// TC table for deblocking threshold (H.265 Table 8-17)
const TC_TABLE: [u8; 54] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2,
    3, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 22, 24,
];

/// Edge type for deblocking
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdgeType {
    Vertical = 0,
    Horizontal = 1,
}

/// Deblocking context for a single CTU/CU
///
/// Tracks edge flags and boundary strength values during deblocking.
/// Edge flags mark which 4x4 grid boundaries need filtering.
/// Boundary strength (bS) values determine filter strength: 0=skip, 1=weak, 2=strong.
pub struct DeblockingContext {
    /// Edge flags for vertical edges (per 4x4 block)
    ver_edge_flags: Vec<u8>,
    /// Edge flags for horizontal edges (per 4x4 block)
    hor_edge_flags: Vec<u8>,
    /// Boundary strength for vertical edges (per 4x4 block)
    ver_bs: Vec<u8>,
    /// Boundary strength for horizontal edges (per 4x4 block)
    hor_bs: Vec<u8>,
    /// Stride for edge arrays (in 4x4 block units)
    stride: usize,
}

impl DeblockingContext {
    /// Create new deblocking context for image dimensions
    pub fn new(width: u32, height: u32) -> Self {
        // Edge flags and bS are stored per 4x4 block
        let width_4x4 = width.div_ceil(4) as usize;
        let height_4x4 = height.div_ceil(4) as usize;
        let size = width_4x4 * height_4x4;

        Self {
            ver_edge_flags: vec![0; size],
            hor_edge_flags: vec![0; size],
            ver_bs: vec![0; size],
            hor_bs: vec![0; size],
            stride: width_4x4,
        }
    }

    /// Get index for 4x4 block at (x, y) in pixel coordinates
    #[inline]
    fn idx(&self, x: u32, y: u32) -> usize {
        let x_4x4 = (x >> 2) as usize;
        let y_4x4 = (y >> 2) as usize;
        y_4x4 * self.stride + x_4x4
    }

    /// Set edge flag at pixel position (x, y)
    #[inline]
    fn set_edge_flag(&mut self, x: u32, y: u32, edge_type: EdgeType, value: u8) {
        let idx = self.idx(x, y);
        match edge_type {
            EdgeType::Vertical => self.ver_edge_flags[idx] = value,
            EdgeType::Horizontal => self.hor_edge_flags[idx] = value,
        }
    }

    /// Get edge flag at pixel position (x, y)
    #[inline]
    fn get_edge_flag(&self, x: u32, y: u32, edge_type: EdgeType) -> u8 {
        let idx = self.idx(x, y);
        match edge_type {
            EdgeType::Vertical => self.ver_edge_flags[idx],
            EdgeType::Horizontal => self.hor_edge_flags[idx],
        }
    }

    /// Set boundary strength at pixel position (x, y)
    #[inline]
    fn set_bs(&mut self, x: u32, y: u32, edge_type: EdgeType, value: u8) {
        let idx = self.idx(x, y);
        match edge_type {
            EdgeType::Vertical => self.ver_bs[idx] = value,
            EdgeType::Horizontal => self.hor_bs[idx] = value,
        }
    }

    /// Get boundary strength at pixel position (x, y)
    #[inline]
    fn get_bs(&self, x: u32, y: u32, edge_type: EdgeType) -> u8 {
        let idx = self.idx(x, y);
        match edge_type {
            EdgeType::Vertical => self.ver_bs[idx],
            EdgeType::Horizontal => self.hor_bs[idx],
        }
    }

    /// Clear all edge flags and boundary strength
    fn clear(&mut self) {
        self.ver_edge_flags.fill(0);
        self.hor_edge_flags.fill(0);
        self.ver_bs.fill(0);
        self.hor_bs.fill(0);
    }
}

/// Metadata tracker for deblocking filter decisions
///
/// Stores per-block information needed for boundary strength derivation:
/// - Transform block boundaries (split_transform_flag)
/// - Prediction modes (intra vs inter)
/// - Non-zero coefficient flags
pub struct DeblockMetadata {
    /// Split transform flags (per 4x4 block, stores whether TU was split)
    split_transform: Vec<bool>,
    /// Prediction modes (per 4x4 block: 0=inter, 1=intra)
    pred_mode: Vec<u8>,
    /// Non-zero coefficient flags (per 4x4 block: has any non-zero coeffs in TU)
    nonzero_coeff: Vec<bool>,
    /// Stride in 4x4 blocks
    stride: usize,
}

impl DeblockMetadata {
    pub fn new(width: u32, height: u32) -> Self {
        let width_4x4 = width.div_ceil(4) as usize;
        let height_4x4 = height.div_ceil(4) as usize;
        let size = width_4x4 * height_4x4;

        Self {
            split_transform: vec![false; size],
            pred_mode: vec![0; size],
            nonzero_coeff: vec![false; size],
            stride: width_4x4,
        }
    }

    #[inline]
    fn idx(&self, x: u32, y: u32) -> usize {
        let x_4x4 = (x >> 2) as usize;
        let y_4x4 = (y >> 2) as usize;
        y_4x4 * self.stride + x_4x4
    }

    /// Mark a transform block as split
    pub fn set_split_transform(&mut self, x: u32, y: u32, split: bool) {
        let idx = self.idx(x, y);
        self.split_transform[idx] = split;
    }

    pub fn get_split_transform(&self, x: u32, y: u32) -> bool {
        let idx = self.idx(x, y);
        self.split_transform[idx]
    }

    /// Set prediction mode (0=inter, 1=intra)
    pub fn set_pred_mode(&mut self, x: u32, y: u32, is_intra: bool) {
        let idx = self.idx(x, y);
        self.pred_mode[idx] = if is_intra { 1 } else { 0 };
    }

    pub fn get_pred_mode(&self, x: u32, y: u32) -> u8 {
        let idx = self.idx(x, y);
        self.pred_mode[idx]
    }

    /// Set non-zero coefficient flag for TU
    pub fn set_nonzero_coeff(&mut self, x: u32, y: u32, has_nonzero: bool) {
        let idx = self.idx(x, y);
        self.nonzero_coeff[idx] = has_nonzero;
    }

    pub fn get_nonzero_coeff(&self, x: u32, y: u32) -> bool {
        let idx = self.idx(x, y);
        self.nonzero_coeff[idx]
    }
}

/// Apply deblocking filter to decoded frame
///
/// Entry point for deblocking. Processes all edges in the image:
/// 1. Vertical edges first (left to right)
/// 2. Horizontal edges second (top to bottom, using filtered vertical edges)
///
/// For I-slices (HEIC), most edges will be intra-predicted with bS=2 (strong filter).
pub fn apply_deblocking_filter(
    frame: &mut DecodedFrame,
    sps: &Sps,
    pps: &Pps,
    header: &SliceHeader,
    metadata: &DeblockMetadata,
) {
    // Skip if deblocking disabled
    if header.slice_deblocking_filter_disabled_flag {
        return;
    }

    let width = frame.width;
    let height = frame.height;

    let mut ctx = DeblockingContext::new(width, height);

    // Process each CTB
    let log2_ctb_size = sps.log2_min_luma_coding_block_size_minus3 + 3 + sps.log2_diff_max_min_luma_coding_block_size;
    let ctb_size = 1u32 << log2_ctb_size;
    let pic_width_in_ctbs = width.div_ceil(ctb_size);
    let pic_height_in_ctbs = height.div_ceil(ctb_size);

    for ctb_y in 0..pic_height_in_ctbs {
        for ctb_x in 0..pic_width_in_ctbs {
            let x0 = ctb_x * ctb_size;
            let y0 = ctb_y * ctb_size;

            // For each CTB, process vertical then horizontal edges
            process_ctb_edges(
                frame,
                &mut ctx,
                metadata,
                sps,
                pps,
                header,
                x0,
                y0,
                ctb_size,
            );
        }
    }
}

/// Process vertical and horizontal edges for a single CTB
fn process_ctb_edges(
    frame: &mut DecodedFrame,
    ctx: &mut DeblockingContext,
    metadata: &DeblockMetadata,
    sps: &Sps,
    pps: &Pps,
    header: &SliceHeader,
    x0: u32,
    y0: u32,
    ctb_size: u32,
) {
    let width = frame.width;
    let height = frame.height;

    // Clamp CTB to image bounds
    let ctb_width = ctb_size.min(width - x0);
    let ctb_height = ctb_size.min(height - y0);

    // Clear context for this CTB
    ctx.clear();

    // 1. Mark vertical edges and derive boundary strength
    let filter_left_edge = x0 > 0 && !is_slice_or_tile_boundary(sps, pps, header, x0 - 1, y0, x0, y0);
    mark_edges_for_ctb(ctx, metadata, x0, y0, ctb_width, ctb_height, EdgeType::Vertical, filter_left_edge);
    derive_boundary_strength_ctb(ctx, metadata, x0, y0, ctb_width, ctb_height, EdgeType::Vertical);

    // 2. Filter vertical edges (luma then chroma)
    filter_edges_luma(frame, ctx, sps, pps, x0, y0, ctb_width, ctb_height, EdgeType::Vertical);
    filter_edges_chroma(frame, ctx, sps, pps, x0, y0, ctb_width, ctb_height, EdgeType::Vertical);

    // 3. Mark horizontal edges and derive boundary strength
    let filter_top_edge = y0 > 0 && !is_slice_or_tile_boundary(sps, pps, header, x0, y0 - 1, x0, y0);
    mark_edges_for_ctb(ctx, metadata, x0, y0, ctb_width, ctb_height, EdgeType::Horizontal, filter_top_edge);
    derive_boundary_strength_ctb(ctx, metadata, x0, y0, ctb_width, ctb_height, EdgeType::Horizontal);

    // 4. Filter horizontal edges (luma then chroma, using filtered vertical edges)
    filter_edges_luma(frame, ctx, sps, pps, x0, y0, ctb_width, ctb_height, EdgeType::Horizontal);
    filter_edges_chroma(frame, ctx, sps, pps, x0, y0, ctb_width, ctb_height, EdgeType::Horizontal);
}

/// Check if edge crosses a slice or tile boundary where filtering is disabled
fn is_slice_or_tile_boundary(
    _sps: &Sps,
    _pps: &Pps,
    header: &SliceHeader,
    _x_p: u32,
    _y_p: u32,
    _x_q: u32,
    _y_q: u32,
) -> bool {
    // For single-slice HEIC images, no slice boundaries
    // Tile support not implemented yet
    !header.slice_loop_filter_across_slices_enabled_flag
}

/// Mark edges to filter for a CTB (H.265 8.7.2.2, 8.7.2.3)
///
/// Marks transform block and prediction block boundaries.
/// For I-slices, only TU boundaries matter (PU is always 2Nx2N).
fn mark_edges_for_ctb(
    ctx: &mut DeblockingContext,
    metadata: &DeblockMetadata,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
    edge_type: EdgeType,
    filter_outer_edge: bool,
) {
    // Mark edges on 4x4 grid (8.7.2.2)
    // For now, mark all 8x8 grid boundaries (minimum TU size)
    let step = 8u32;

    match edge_type {
        EdgeType::Vertical => {
            // Mark vertical edges
            for y in (0..height).step_by(4) {
                for x in (0..width).step_by(step as usize) {
                    let abs_x = x0 + x;
                    let abs_y = y0 + y;

                    let should_mark = if x == 0 {
                        filter_outer_edge
                    } else {
                        // Check if this is a TU boundary
                        x % 8 == 0 || metadata.get_split_transform(abs_x, abs_y)
                    };

                    if should_mark {
                        ctx.set_edge_flag(abs_x, abs_y, edge_type, 1);
                    }
                }
            }
        }
        EdgeType::Horizontal => {
            // Mark horizontal edges
            for y in (0..height).step_by(step as usize) {
                for x in (0..width).step_by(4) {
                    let abs_x = x0 + x;
                    let abs_y = y0 + y;

                    let should_mark = if y == 0 {
                        filter_outer_edge
                    } else {
                        // Check if this is a TU boundary
                        y % 8 == 0 || metadata.get_split_transform(abs_x, abs_y)
                    };

                    if should_mark {
                        ctx.set_edge_flag(abs_x, abs_y, edge_type, 1);
                    }
                }
            }
        }
    }
}

/// Derive boundary strength for marked edges (H.265 8.7.2.4)
///
/// Boundary strength values:
/// - bS = 0: No filtering (inter blocks with similar motion)
/// - bS = 1: Weak filtering (transform edge with non-zero coefficients)
/// - bS = 2: Strong filtering (at least one intra block)
fn derive_boundary_strength_ctb(
    ctx: &mut DeblockingContext,
    metadata: &DeblockMetadata,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
    edge_type: EdgeType,
) {
    let (dx, dy) = match edge_type {
        EdgeType::Vertical => (1, 0),   // Compare left (P) and right (Q) sides
        EdgeType::Horizontal => (0, 1), // Compare top (P) and bottom (Q) sides
    };

    for y in (0..height).step_by(4) {
        for x in (0..width).step_by(4) {
            let abs_x = x0 + x;
            let abs_y = y0 + y;

            // Skip if edge not marked
            if ctx.get_edge_flag(abs_x, abs_y, edge_type) == 0 {
                continue;
            }

            // Get P side (before edge) and Q side (after edge)
            let (x_p, y_p) = if dx == 1 {
                (abs_x.saturating_sub(1), abs_y)
            } else {
                (abs_x, abs_y.saturating_sub(1))
            };
            let (x_q, y_q) = (abs_x, abs_y);

            // Derive boundary strength (H.265 8.7.2.4)
            let bs = if metadata.get_pred_mode(x_p, y_p) == 1 || metadata.get_pred_mode(x_q, y_q) == 1 {
                // At least one side is intra -> strong filter
                2
            } else if metadata.get_nonzero_coeff(x_p, y_p) || metadata.get_nonzero_coeff(x_q, y_q) {
                // Transform edge with non-zero coefficients -> weak filter
                1
            } else {
                // No filtering needed (would check motion vectors for inter)
                0
            };

            ctx.set_bs(abs_x, abs_y, edge_type, bs);
        }
    }
}

/// Filter luma edges for a CTB (H.265 8.7.2.5)
fn filter_edges_luma(
    frame: &mut DecodedFrame,
    ctx: &DeblockingContext,
    _sps: &Sps,
    pps: &Pps,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
    edge_type: EdgeType,
) {
    let stride = frame.width as usize;

    // Base QP for beta/tc table lookup
    let qp_offset = pps.pps_beta_offset_div2 * 2;
    let base_qp = 0; // Would use slice QP + cu_qp_delta

    for y in (0..height).step_by(4) {
        for x in (0..width).step_by(4) {
            let abs_x = x0 + x;
            let abs_y = y0 + y;

            let bs = ctx.get_bs(abs_x, abs_y, edge_type);
            if bs == 0 {
                continue;
            }

            // Calculate QP for threshold lookup
            let qp_l = (base_qp + qp_offset).clamp(0, 51) as usize;
            let beta = BETA_TABLE[qp_l] as i32;
            let tc_offset = pps.pps_tc_offset_div2 * 2;
            let tc_val = TC_TABLE[(qp_l as i32 + tc_offset as i32 + 2).clamp(0, 53) as usize] as i32;

            filter_luma_edge(
                &mut frame.y_plane,
                stride,
                abs_x,
                abs_y,
                edge_type,
                bs,
                beta,
                tc_val,
            );
        }
    }
}

/// Filter chroma edges for a CTB (H.265 8.7.2.5)
fn filter_edges_chroma(
    frame: &mut DecodedFrame,
    ctx: &DeblockingContext,
    _sps: &Sps,
    pps: &Pps,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
    edge_type: EdgeType,
) {
    // Chroma is half resolution for 4:2:0
    let chroma_stride = (frame.width / 2) as usize;
    let qp_offset = pps.pps_beta_offset_div2 * 2;
    let base_qp = 0;

    for y in (0..height).step_by(8) {
        for x in (0..width).step_by(8) {
            let abs_x = x0 + x;
            let abs_y = y0 + y;

            let bs = ctx.get_bs(abs_x, abs_y, edge_type);
            if bs < 2 {
                // Chroma only filtered at strong boundaries (bS=2)
                continue;
            }

            let qp_c = (base_qp + qp_offset).clamp(0, 51) as usize;
            let tc_val = TC_TABLE[(qp_c as i32 + pps.pps_tc_offset_div2 as i32 * 2 + 2).clamp(0, 53) as usize] as i32;

            // Chroma coordinates (half resolution)
            let cx = abs_x / 2;
            let cy = abs_y / 2;

            filter_chroma_edge(&mut frame.cb_plane, chroma_stride, cx, cy, edge_type, tc_val);
            filter_chroma_edge(&mut frame.cr_plane, chroma_stride, cx, cy, edge_type, tc_val);
        }
    }
}

/// Apply luma edge filter at specific edge
fn filter_luma_edge(
    samples: &mut [u16],
    stride: usize,
    x: u32,
    y: u32,
    edge_type: EdgeType,
    bs: u8,
    beta: i32,
    tc: i32,
) {
    let x = x as usize;
    let y = y as usize;

    // Get sample indices for P and Q sides (4 samples each)
    let (p_idx, q_idx): (Vec<usize>, Vec<usize>) = match edge_type {
        EdgeType::Vertical => {
            // P side: 4 samples to left of edge, Q side: 4 samples at/right of edge
            let p = (0..4).map(|i| (y + i) * stride + x.saturating_sub(1)).collect();
            let q = (0..4).map(|i| (y + i) * stride + x).collect();
            (p, q)
        }
        EdgeType::Horizontal => {
            // P side: 4 samples above edge, Q side: 4 samples at/below edge
            let p = (0..4).map(|i| (y.saturating_sub(1)) * stride + x + i).collect();
            let q = (0..4).map(|i| y * stride + x + i).collect();
            (p, q)
        }
    };

    // Check all indices are valid
    for &idx in p_idx.iter().chain(q_idx.iter()) {
        if idx >= samples.len() {
            return;
        }
    }

    // Apply weak or strong filter based on bS
    if bs == 2 {
        // Strong filter for intra edges
        apply_strong_luma_filter(samples, &p_idx, &q_idx, beta, tc);
    } else {
        // Weak filter
        apply_weak_luma_filter(samples, &p_idx, &q_idx, beta, tc);
    }
}

/// Apply strong luma filter (H.265 8.7.2.5.7)
fn apply_strong_luma_filter(
    samples: &mut [u16],
    p_idx: &[usize],
    q_idx: &[usize],
    _beta: i32,
    tc: i32,
) {
    // Simplified strong filter
    for i in 0..4.min(p_idx.len()).min(q_idx.len()) {
        let p0 = samples[p_idx[i]] as i32;
        let q0 = samples[q_idx[i]] as i32;

        let delta = (q0 - p0).clamp(-tc, tc);
        samples[p_idx[i]] = (p0 + delta / 2).clamp(0, 255) as u16;
        samples[q_idx[i]] = (q0 - delta / 2).clamp(0, 255) as u16;
    }
}

/// Apply weak luma filter (H.265 8.7.2.5.8)
fn apply_weak_luma_filter(
    samples: &mut [u16],
    p_idx: &[usize],
    q_idx: &[usize],
    _beta: i32,
    tc: i32,
) {
    // Simplified weak filter
    for i in 0..4.min(p_idx.len()).min(q_idx.len()) {
        let p0 = samples[p_idx[i]] as i32;
        let q0 = samples[q_idx[i]] as i32;

        let delta = ((q0 - p0) * 9 / 16).clamp(-tc, tc);
        samples[p_idx[i]] = (p0 + delta).clamp(0, 255) as u16;
        samples[q_idx[i]] = (q0 - delta).clamp(0, 255) as u16;
    }
}

/// Apply chroma edge filter (H.265 8.7.2.5.9)
fn filter_chroma_edge(
    samples: &mut [u16],
    stride: usize,
    x: u32,
    y: u32,
    edge_type: EdgeType,
    tc: i32,
) {
    let x = x as usize;
    let y = y as usize;

    // Chroma filter is simpler - only filters 2 samples per side
    let (p_idx, q_idx): (Vec<usize>, Vec<usize>) = match edge_type {
        EdgeType::Vertical => {
            let p = (0..2).map(|i| (y + i) * stride + x.saturating_sub(1)).collect();
            let q = (0..2).map(|i| (y + i) * stride + x).collect();
            (p, q)
        }
        EdgeType::Horizontal => {
            let p = (0..2).map(|i| (y.saturating_sub(1)) * stride + x + i).collect();
            let q = (0..2).map(|i| y * stride + x + i).collect();
            (p, q)
        }
    };

    for &idx in p_idx.iter().chain(q_idx.iter()) {
        if idx >= samples.len() {
            return;
        }
    }

    for i in 0..2.min(p_idx.len()).min(q_idx.len()) {
        let p0 = samples[p_idx[i]] as i32;
        let q0 = samples[q_idx[i]] as i32;

        let delta = ((q0 - p0) / 2).clamp(-tc, tc);
        samples[p_idx[i]] = (p0 + delta).clamp(0, 255) as u16;
        samples[q_idx[i]] = (q0 - delta).clamp(0, 255) as u16;
    }
}
