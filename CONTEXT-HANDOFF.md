# CABAC Coefficient Decode Bug Investigation - Context Handoff

## Current State (2026-01-22)

**Status:** All 280 CTUs decode, but with 34 large coefficients (>500) indicating CABAC desync. SSIM2 = -965 (poor quality).

**Key Finding:** Individual CABAC primitives are CORRECT (hevc-compare tests pass). The bug is in context derivation or state tracking.

## Investigation Summary

### Problem
Large coefficients (>500) appear starting at byte 1316. These are caused by CABAC desync where bypass decodes produce many consecutive 1-bits, leading to huge Golomb-Rice values.

### Attempted Fixes (REVERTED - Made things worse)

1. **ctx_set = base (0 or 2)** instead of always 1:
   - Moved first large coeff from byte 1316 to byte 1411
   - Still had 32 large coefficients

2. **ctx_set = base + (prev_c1==0 ? 1 : 0)** with cross-subblock tracking:
   - Only decoded 225/280 CTUs (worse!)
   - First large coeff moved to byte 1112
   - This is correct per H.265 spec but exposes other bugs

### The "Local Optima" Problem
**CRITICAL:** With multiple interacting bugs, fixing one bug can make metrics worse because it exposes other compensating bugs. The "wrong" ctx_set=1 was masking other issues.

## Current ctx_set Derivation (WRONG but stable)

```rust
// residual.rs lines 359-363
let mut c1 = 1u8;
let ctx_set = c1; // Always 1!
```

**Correct per H.265 9.3.4.2.6:**
- ctx_set = 0 if (sb_idx==0 OR c_idx>0) else 2  // Base
- ctx_set += 1 if previous subblock had any g1=1  // Increment

## Where to Focus Next

1. **Don't optimize for "CTUs decoded" or "SSIM score"** - these lead to local optima
2. **Use differential testing at coefficient level:**
   - hevc-compare crate can compare individual CABAC operations
   - Need to find FIRST operation where our decoder diverges from libde265

3. **Trace the EXACT sequence of operations for a specific TU:**
   - The debug tracing infrastructure is in place
   - Set `debug_call = residual_call_num == N` in residual.rs line 189
   - Compare operation sequence with libde265

## Key Files

- `src/hevc/residual.rs` - Coefficient decode logic (the bug is here)
- `src/hevc/cabac.rs` - CABAC decoder (primitives are correct)
- `crates/hevc-compare/` - C++ comparison infrastructure
- `/home/lilith/work/heic/spec/sections/` - H.265 spec organized by component

## Spec References

- 9.3.4.2.5 - sig_coeff_flag context derivation
- 9.3.4.2.6 - coeff_abs_level_greater1_flag context (ctxSet derivation)
- 9.3.4.2.7 - coeff_abs_level_greater2_flag context
- 10.3.4 - CABAC decoding flow

## Debug Trace Example

For call#256 (before reverting ctx_set fix):
```
DEBUG call#256: START log2=2 c_idx=0 scan=Diagonal byte=1305 cabac=(500,252)
DEBUG call#256: SUBBLOCK sb_idx=0 (0,0) byte=1305 cabac=(256,197)
  sig_coeff_flags: 14 coeffs at positions [0,1,2,3,4,5,6,7,8,9,10,11,12,13]
  g1[n=11]: ctx_set=1 gt1_ctx=3 -> true
  ...
  remaining[n=2]: base=1 rice=3 byte=1312 cabac=(256,255)
    -> remaining=2002 new_rice=4 final=2003  ← LARGE VALUE!
```

The CABAC state (256,255) with offset very close to range causes many consecutive 1-bits in bypass decode.

## Commands

```bash
# Run comparison test
cargo test --test compare_reference test_ssim2 -- --nocapture

# Run hevc-compare tests (verify primitives)
cd crates/hevc-compare && cargo test -- --nocapture

# Enable debug tracing for specific call
# Edit src/hevc/residual.rs line 189:
let debug_call = residual_call_num == 256;  # Set to call number of interest
```

## Next Steps

1. Read spec sections for ctxSet derivation (already in CLAUDE.md)
2. Add test comparing TU decode operation-by-operation with libde265
3. Find first diverging operation, not just first large coefficient
4. Once found, trace WHY that operation differs

## Don't Forget

- Check git status before starting work
- The spec is at `/home/lilith/work/heic/spec/sections/README.md`
- Individual CABAC primitives match libde265 - the bug is in orchestration
