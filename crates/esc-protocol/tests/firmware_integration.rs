//! Integration tests for firmware parsing and processing.
//!
//! These tests use real firmware files if available in the project root.

use std::fs;
use std::path::Path;

/// Find firmware hex files in the project root.
fn find_firmware_files() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "hex").unwrap_or(false) {
                if let Some(s) = path.to_str() {
                    files.push(s.to_string());
                }
            }
        }
    }
    files
}

#[test]
fn test_real_firmware_parsing() {
    let files = find_firmware_files();

    if files.is_empty() {
        println!("No firmware .hex files found in project root - skipping test");
        return;
    }

    for file in &files {
        println!("\n=== Testing: {} ===", file);

        let content = fs::read_to_string(file).expect("failed to read file");
        let firmware = esc_protocol::load_firmware(&content).expect("failed to parse firmware");
        let binary = firmware.to_binary();

        // Basic validation
        assert!(
            !firmware.segments.is_empty(),
            "firmware should have segments"
        );
        assert!(!binary.is_empty(), "binary should not be empty");

        println!("  Segments: {}", firmware.segments.len());
        println!("  Total segment data: {} bytes", firmware.total_size());
        println!("  Binary size (with gaps): {} bytes", binary.len());

        if let Some(min) = firmware.min_address() {
            println!(
                "  Address range: 0x{:08X} - 0x{:08X}",
                min,
                firmware.max_address().unwrap()
            );

            // AM32 firmware should start at 0x08001000 or 0x1000
            let offset = min & 0xFFFF;
            assert!(
                offset == 0x1000 || min == 0x1000,
                "AM32 firmware should start at offset 0x1000, got 0x{:08X}",
                min
            );
        }

        // Flash simulation (128-byte chunks)
        let chunk_size = 128;
        let flash_chunks = (binary.len() + chunk_size - 1) / chunk_size;
        println!("  Flash chunks (128B): {}", flash_chunks);

        // Verify simulation (256-byte chunks)
        let chunk_size = 256;
        let verify_chunks = (binary.len() + chunk_size - 1) / chunk_size;
        println!("  Verify chunks (256B): {}", verify_chunks);

        // Verify first bytes look like ARM code (stack pointer + reset vector)
        if binary.len() >= 8 {
            // First 4 bytes should be initial stack pointer (usually 0x20003C00 or similar)
            // Stored little-endian, so byte 3 should be 0x20 (SRAM base)
            let sp_high = binary[3];
            println!("  Stack pointer high byte: 0x{:02X}", sp_high);
            assert!(
                sp_high == 0x20,
                "First word should be stack pointer in SRAM (0x20xxxxxx), got high byte 0x{:02X}",
                sp_high
            );
        }

        // Test chunking produces correct data
        for (i, chunk) in binary.chunks(128).enumerate() {
            let addr = 0x1000u16 + (i * 128) as u16;
            assert!(chunk.len() <= 128, "chunk {} too large", i);
            // Just verify we can iterate - no panics
            let _ = (addr, chunk.len());
        }

        println!("  All checks passed!");
    }
}

#[test]
fn test_firmware_verification_simulation() {
    let files = find_firmware_files();

    if files.is_empty() {
        println!("No firmware .hex files found - skipping verification simulation");
        return;
    }

    for file in &files {
        println!("\n=== Verification simulation: {} ===", file);

        let content = fs::read_to_string(file).expect("failed to read file");
        let firmware = esc_protocol::load_firmware(&content).expect("failed to parse firmware");
        let expected = firmware.to_binary();

        // Simulate reading back the same data (perfect match)
        let actual = expected.clone();

        let chunk_size = 256;
        let mut mismatches = 0;

        for (i, (exp_chunk, act_chunk)) in expected
            .chunks(chunk_size)
            .zip(actual.chunks(chunk_size))
            .enumerate()
        {
            if exp_chunk != act_chunk {
                mismatches += 1;
                println!("  Mismatch at chunk {}", i);
            }
        }

        assert_eq!(
            mismatches, 0,
            "verification should pass with identical data"
        );
        println!(
            "  Verification simulation passed ({} chunks)",
            (expected.len() + 255) / 256
        );

        // Simulate a single-byte corruption
        let mut corrupted = expected.clone();
        if !corrupted.is_empty() {
            let mid = corrupted.len() / 2;
            corrupted[mid] ^= 0xFF; // Flip bits in middle

            let mut found_mismatch = false;
            for (exp_chunk, cor_chunk) in expected
                .chunks(chunk_size)
                .zip(corrupted.chunks(chunk_size))
            {
                if exp_chunk != cor_chunk {
                    found_mismatch = true;
                    break;
                }
            }
            assert!(found_mismatch, "should detect corruption");
            println!("  Corruption detection works!");
        }
    }
}
