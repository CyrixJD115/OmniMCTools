//! fuzz_subchunk_codec — arbitrary bytes as a Bedrock subchunk (v8/v9).
//!
//! Invariants:
//! * `Subchunk::parse` either errors (typed) or succeeds — never panics.
//! * On success, serialize → parse again reproduces the decoded value
//!   (decode⇄encode idempotence), including the bits==0 single-entry shape.
//! * `Data3d::parse` (v9 companion codec) is also accept-or-error.

#![no_main]

use libfuzzer_sys::fuzz_target;
use omt_bedrock::{Data3d, Subchunk};

fuzz_target!(|data: &[u8]| {
    if let Ok(sub) = Subchunk::parse(data, None) {
        let re = sub.serialize();
        let re_parsed = Subchunk::parse(&re, None)
            .expect("re-parse of our own serialization must succeed");
        assert_eq!(re_parsed, sub, "decode⇄encode must round-trip");
    }

    // data3D biome codec shares the packed-bits machinery.
    if let Ok(d3) = Data3d::parse(data) {
        let re = d3.serialize();
        if re.len() >= data.len() {
            let re_parsed = Data3d::parse(&re).expect("re-parse of our own serialization");
            assert_eq!(re_parsed, d3, "data3d decode⇄encode must round-trip");
        }
    }
});
