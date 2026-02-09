//! Optimization safety tests - ensures SIMD/optimizations don't change output
//!
//! WORKFLOW:
//! 1. Run `cargo test generate_rust_reference -- --ignored` ONCE to create reference
//! 2. Make optimizations
//! 3. Run `cargo test verify_against_reference` to ensure pixel-perfect output
//! 4. If test fails, optimization introduced a bug - roll back and fix

use heic_decoder::HeicDecoder;
use std::fs;
use std::path::Path;

/// Test files available in the repository
/// Start with smaller files for faster iteration
const TEST_FILES: &[&str] = &[
    "libheif/examples/example.heic",  // 702KB - good for testing
    // "20240601_170601.heic",  // 15MB - too large for quick iteration, enable for full verification
];

/// Directory for reference outputs
const REFERENCE_DIR: &str = "tests/references";

/// Maximum allowed pixel difference (for floating point error tolerance)
/// 0 = pixel perfect, 1 = allow ±1 per channel
const MAX_PIXEL_DIFF: u8 = 0;

/// Maximum percentage of pixels that can differ (for statistical tolerance)
/// 0.0 = all pixels must match exactly
/// 0.01 = allow 1% of pixels to differ by MAX_PIXEL_DIFF
const MAX_DIFF_PERCENTAGE: f64 = 0.0;

#[derive(Debug)]
struct PixelDiffStats {
    total_pixels: usize,
    identical_pixels: usize,
    differing_pixels: usize,
    max_diff_per_channel: u8,
    avg_diff: f64,
    diff_histogram: [usize; 256],
}

impl PixelDiffStats {
    fn new() -> Self {
        Self {
            total_pixels: 0,
            identical_pixels: 0,
            differing_pixels: 0,
            max_diff_per_channel: 0,
            avg_diff: 0.0,
            diff_histogram: [0; 256],
        }
    }

    fn passes_threshold(&self) -> bool {
        let diff_pct = (self.differing_pixels as f64 / self.total_pixels as f64) * 100.0;
        self.max_diff_per_channel <= MAX_PIXEL_DIFF && diff_pct <= MAX_DIFF_PERCENTAGE
    }

    fn print(&self) {
        println!("  Total pixels: {}", self.total_pixels);
        println!("  Identical pixels: {} ({:.2}%)",
            self.identical_pixels,
            (self.identical_pixels as f64 / self.total_pixels as f64) * 100.0
        );
        println!("  Differing pixels: {} ({:.2}%)",
            self.differing_pixels,
            (self.differing_pixels as f64 / self.total_pixels as f64) * 100.0
        );
        println!("  Max diff per channel: {}", self.max_diff_per_channel);
        println!("  Average diff: {:.4}", self.avg_diff);

        // Print histogram for non-zero diffs
        if self.differing_pixels > 0 {
            println!("  Difference histogram:");
            for (diff, &count) in self.diff_histogram.iter().enumerate() {
                if count > 0 && diff > 0 {
                    println!("    Diff {}: {} pixels", diff, count);
                }
            }
        }
    }
}

fn compare_rgb_data(reference: &[u8], current: &[u8]) -> PixelDiffStats {
    assert_eq!(reference.len(), current.len(), "RGB data length mismatch");

    let mut stats = PixelDiffStats::new();
    let num_pixels = reference.len() / 3;
    stats.total_pixels = num_pixels;

    let mut total_diff_sum: u64 = 0;

    for i in 0..num_pixels {
        let idx = i * 3;
        let r_ref = reference[idx];
        let g_ref = reference[idx + 1];
        let b_ref = reference[idx + 2];

        let r_cur = current[idx];
        let g_cur = current[idx + 1];
        let b_cur = current[idx + 2];

        let r_diff = (r_ref as i16 - r_cur as i16).unsigned_abs() as u8;
        let g_diff = (g_ref as i16 - g_cur as i16).unsigned_abs() as u8;
        let b_diff = (b_ref as i16 - b_cur as i16).unsigned_abs() as u8;

        let max_diff = r_diff.max(g_diff).max(b_diff);

        if max_diff == 0 {
            stats.identical_pixels += 1;
        } else {
            stats.differing_pixels += 1;
            stats.max_diff_per_channel = stats.max_diff_per_channel.max(max_diff);
            total_diff_sum += max_diff as u64;
            stats.diff_histogram[max_diff as usize] += 1;
        }
    }

    if stats.differing_pixels > 0 {
        stats.avg_diff = total_diff_sum as f64 / stats.differing_pixels as f64;
    }

    stats
}

/// Generate reference outputs from current (unoptimized) implementation
/// Run this ONCE before starting optimizations:
/// `cargo test generate_rust_reference -- --ignored --nocapture`
#[test]
#[ignore]
fn generate_rust_reference() {
    println!("\n=== GENERATING RUST REFERENCE OUTPUTS ===\n");

    // Create reference directory
    fs::create_dir_all(REFERENCE_DIR).expect("Failed to create reference directory");

    let decoder = HeicDecoder::new();

    for &test_file in TEST_FILES {
        if !Path::new(test_file).exists() {
            println!("Skipping {} (not found)", test_file);
            continue;
        }

        println!("Processing {}...", test_file);

        let data = fs::read(test_file).expect("Failed to read test file");
        let image = decoder.decode(&data).expect("Failed to decode");

        // Save RGB reference
        let ref_path = format!("{}/{}.rgb", REFERENCE_DIR, test_file.replace('/', "_"));
        fs::write(&ref_path, &image.data).expect("Failed to write reference");

        // Save metadata (dimensions)
        let meta_path = format!("{}/{}.meta", REFERENCE_DIR, test_file.replace('/', "_"));
        let meta = format!("{}x{}", image.width, image.height);
        fs::write(&meta_path, meta).expect("Failed to write metadata");

        println!("  ✓ Saved reference: {}x{} ({} bytes)",
            image.width, image.height, image.data.len());
    }

    println!("\n=== Reference generation complete! ===");
    println!("References saved to: {}", REFERENCE_DIR);
    println!("\nNow you can safely make optimizations and run:");
    println!("  cargo test verify_against_reference");
}

/// Verify current implementation against Rust reference
/// Run this after EVERY optimization to ensure no regressions:
/// `cargo test verify_against_reference -- --nocapture`
#[test]
fn verify_against_reference() {
    println!("\n=== VERIFYING AGAINST RUST REFERENCE ===\n");

    let decoder = HeicDecoder::new();
    let mut all_passed = true;

    for &test_file in TEST_FILES {
        if !Path::new(test_file).exists() {
            println!("Skipping {} (test file not found)", test_file);
            continue;
        }

        let ref_path = format!("{}/{}.rgb", REFERENCE_DIR, test_file.replace('/', "_"));
        let meta_path = format!("{}/{}.meta", REFERENCE_DIR, test_file.replace('/', "_"));

        if !Path::new(&ref_path).exists() {
            println!("⚠️  No reference for {} - run generate_rust_reference first!", test_file);
            all_passed = false;
            continue;
        }

        println!("Testing {}...", test_file);

        // Load reference
        let ref_data = fs::read(&ref_path).expect("Failed to read reference");
        let meta = fs::read_to_string(&meta_path).expect("Failed to read metadata");
        let dims: Vec<&str> = meta.split('x').collect();
        let ref_width: u32 = dims[0].parse().unwrap();
        let ref_height: u32 = dims[1].parse().unwrap();

        // Decode current
        let test_data = fs::read(test_file).expect("Failed to read test file");
        let current = decoder.decode(&test_data).expect("Failed to decode");

        // Verify dimensions
        if current.width != ref_width || current.height != ref_height {
            println!("  ❌ DIMENSION MISMATCH!");
            println!("     Reference: {}x{}", ref_width, ref_height);
            println!("     Current:   {}x{}", current.width, current.height);
            all_passed = false;
            continue;
        }

        // Compare pixels
        let stats = compare_rgb_data(&ref_data, &current.data);
        stats.print();

        if stats.passes_threshold() {
            println!("  ✅ PASS - Output matches reference!");
        } else {
            println!("  ❌ FAIL - Output differs from reference!");
            println!("     Max allowed diff: {} per channel", MAX_PIXEL_DIFF);
            println!("     Max allowed diff percentage: {:.2}%", MAX_DIFF_PERCENTAGE);
            all_passed = false;
        }

        println!();
    }

    if !all_passed {
        panic!("\n❌ OPTIMIZATION VERIFICATION FAILED!\n\
                Some outputs don't match the reference.\n\
                Your optimization may have introduced a bug.\n\
                Please review the changes and fix any discrepancies.");
    } else {
        println!("✅ ALL TESTS PASSED - Optimizations are safe!");
    }
}

/// Generate visual diff for debugging (writes to tests/diffs/)
/// `cargo test generate_visual_diff -- --ignored --nocapture`
#[test]
#[ignore]
fn generate_visual_diff() {
    use std::io::Write;

    let diff_dir = "tests/diffs";
    fs::create_dir_all(diff_dir).expect("Failed to create diff directory");

    let decoder = HeicDecoder::new();

    for &test_file in TEST_FILES {
        if !Path::new(test_file).exists() {
            continue;
        }

        let ref_path = format!("{}/{}.rgb", REFERENCE_DIR, test_file.replace('/', "_"));
        if !Path::new(&ref_path).exists() {
            continue;
        }

        println!("Generating diff for {}...", test_file);

        let ref_data = fs::read(&ref_path).unwrap();
        let meta = fs::read_to_string(
            format!("{}/{}.meta", REFERENCE_DIR, test_file.replace('/', "_"))
        ).unwrap();
        let dims: Vec<&str> = meta.split('x').collect();
        let width: u32 = dims[0].parse().unwrap();
        let height: u32 = dims[1].parse().unwrap();

        let test_data = fs::read(test_file).unwrap();
        let current = decoder.decode(&test_data).unwrap();

        // Generate amplified difference image
        let mut diff_data = Vec::new();
        for i in 0..(width * height) as usize {
            let idx = i * 3;
            for c in 0..3 {
                let diff = (ref_data[idx + c] as i16 - current.data[idx + c] as i16).abs();
                // Amplify by 10x for visibility
                diff_data.push((diff * 10).min(255) as u8);
            }
        }

        // Write as PPM
        let filename = test_file.replace('/', "_").replace(".heic", "_diff.ppm");
        let diff_path = format!("{}/{}", diff_dir, filename);
        let mut file = fs::File::create(&diff_path).unwrap();
        write!(file, "P6\n{} {}\n255\n", width, height).unwrap();
        file.write_all(&diff_data).unwrap();

        println!("  Wrote diff to: {}", diff_path);
    }
}

/// Quick sanity check - just ensure decoder works
#[test]
fn sanity_check_decoder_works() {
    let decoder = HeicDecoder::new();

    for &test_file in TEST_FILES {
        if Path::new(test_file).exists() {
            let data = fs::read(test_file).unwrap();
            let result = decoder.decode(&data);
            assert!(result.is_ok(), "Failed to decode {}: {:?}", test_file, result.err());

            let image = result.unwrap();
            assert!(image.data.len() > 0, "Empty image data for {}", test_file);
            assert!(image.width > 0 && image.height > 0, "Invalid dimensions for {}", test_file);
        }
    }
}
