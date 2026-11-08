//! fuzz_nbt_reader — arbitrary bytes into the NBT reader and container sniffer.
//!
//! Invariant: `omt_nbt::read` (BE and LE) and `Container::decode` either
//! return a typed error or a successful parse — never panic, never hang
//! (the reader enforces MAX_DEPTH = 512 internally).

#![no_main]

use libfuzzer_sys::fuzz_target;
use omt_nbt::{ByteOrder, Container};

fuzz_target!(|data: &[u8]| {
    // Both byte orders must accept-or-reject cleanly.
    let _ = omt_nbt::read(data, ByteOrder::Big);
    let _ = omt_nbt::read(data, ByteOrder::Little);

    // Container sniffing: gzip/zlib/raw dispatch on the same bytes.
    if let Ok((raw, _container)) = Container::decode(data) {
        // A decoded payload is re-fed through the reader (cheap: it is
        // bounded by the input size).
        let _ = omt_nbt::read(&raw, ByteOrder::Big);
        let _ = omt_nbt::read(&raw, ByteOrder::Little);
    }
});
