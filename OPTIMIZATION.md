# HEIC Decoder Optimization Guide

## Testing Methodology

This project uses a **Rust reference-based testing** approach to ensure optimizations don't introduce bugs.

### Workflow

#### 1. Generate Reference (ONE TIME ONLY)
```bash
cargo test generate_rust_reference --test optimization_safety -- --ignored --nocapture
```

This creates pixel-perfect reference outputs from the current unoptimized code in `tests/references/`.

**⚠️ IMPORTANT:** Only run this ONCE before starting optimizations! This is your baseline.

#### 2. Make Optimizations
Edit the code, add SIMD, whatever you want!

#### 3. Verify After EVERY Change
```bash
cargo test verify_against_reference --test optimization_safety -- --nocapture
```

This ensures your optimized code produces **pixel-perfect** output matching the reference.

- ✅ **PASS** = Your optimization is safe!
- ❌ **FAIL** = Your optimization introduced a bug - roll back and fix!

#### 4. Optional: Visual Diff for Debugging
```bash
cargo test generate_visual_diff --test optimization_safety -- --ignored --nocapture
```

Creates amplified diff images in `tests/diffs/` for visual inspection.

### Sanity Check
```bash
cargo test sanity_check_decoder_works --test optimization_safety
```

Quick test to ensure the decoder works at all (no pixel comparison).

---

## Optimization Plan

### Priority 1: Transform (IDCT) - **HIGHEST IMPACT**
- [ ] IDCT 32x32 (biggest win: 8-12x speedup expected)
- [ ] IDCT 16x16 (6-10x speedup expected)
- [ ] IDCT 8x8 (4-6x speedup expected)
- [ ] IDCT/IDST 4x4 (3-5x speedup expected)

**File:** `src/hevc/transform.rs`

**Strategy:**
- Use AVX2 SIMD (`std::arch::x86_64`)
- Process 8 i16 values at once
- Horizontal vectorization of matrix multiply
- Use `_mm256_madd_epi16` for multiply-accumulate

### Priority 2: Intra Prediction
- [ ] Planar mode (lines 360-397)
- [ ] Angular mode (lines 458-619)
- [ ] DC mode (lines 399-456)

**File:** `src/hevc/intra.rs`

### Priority 3: YUV→RGB Conversion
- [ ] `to_rgb()` and `to_rgba()` functions

**File:** `src/hevc/picture.rs`

### Priority 4: Dequantization
- [ ] `dequantize()` function

**File:** `src/hevc/transform.rs` (lines 374-397)

### Priority 5: Deblocking Filter
- [ ] Luma filtering
- [ ] Chroma filtering

**File:** `src/hevc/deblock.rs`

---

## Test Files

- **example.heic** (702KB) - Main test file, 1280x854
- 20240601_170601.heic (15MB) - Large file, enable for final verification

---

## Verification Criteria

- **MAX_PIXEL_DIFF = 0**: Pixel-perfect match required
- **MAX_DIFF_PERCENTAGE = 0.0%**: All pixels must match exactly

This can be relaxed to allow ~1% margin if needed for floating-point errors, but start with pixel-perfect.

---

## Git Workflow

```bash
# Before optimizations
git add tests/references/
git commit -m "Add optimization baseline references"

# After each successful optimization
git add src/
cargo test verify_against_reference --test optimization_safety
git commit -m "Optimize: [description] - verified pixel-perfect"
```

---

## Current Status

✅ Reference baseline generated
✅ Verification test working
🔧 Ready to start optimizations!

### Next Steps
1. Start with IDCT 32x32 (biggest win)
2. Verify after implementation
3. Move to IDCT 16x16
4. Continue down the priority list
