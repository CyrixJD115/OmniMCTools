//! fuzz_region_reader — arbitrary bytes as an Anvil `.mca` region image.
//!
//! Invariant: `RegionFile::from_bytes` + `read_chunk` turn truncated files,
//! garbage offsets/sector counts, and failed decompression into typed errors —
//! never a panic or infinite loop.

#![no_main]

use libfuzzer_sys::fuzz_target;
use omt_java::RegionFile;
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    let path = Path::new("fuzz.mca");
    let Ok(region) = RegionFile::from_bytes(path, data) else {
        return; // header-level rejection — the expected path for most inputs
    };
    // Chunk coordinates from the input (all-zero when too short); the reader
    // must bounds-check every derived offset/sector count.
    let word = |i: usize| -> i32 {
        u32::from_le_bytes([
            data.get(i).copied().unwrap_or(0),
            data.get(i + 1).copied().unwrap_or(0),
            data.get(i + 2).copied().unwrap_or(0),
            data.get(i + 3).copied().unwrap_or(0),
        ]) as i32
    };
    let _ = region.read_chunk(data, word(0), word(4));
});
