//! Probe: hints capture vs write lookup on one real record.

use omt_nbt::{ByteOrder, Tag};

fn main() {
    let root = std::path::PathBuf::from("/home/cyrixjd115/Dev/Dev/Reverse_Engineering/UMT_Crack/test_worlds/bedrock-lostsurfur");
    let w = omt_bedrock::BedrockWorld::open_read_only(&root).unwrap();
    let raw = w.iter_raw().unwrap();
    let target: Vec<u8> = vec![0x09, 0, 0, 0, 0x05, 0, 0, 0, 0x31];
    let v = raw.iter().find(|(k, _)| **k == target).unwrap().1.clone();

    let mut offset = 0usize;
    let mut rebuilt: Vec<u8> = Vec::new();
    let mut entry = 0usize;
    while offset < v.len() {
        let (nbt, used, hints) = omt_nbt::read_prefix_with_hints(&v[offset..], ByteOrder::Little).unwrap();
        println!("entry {entry}: {} hints: {:?}", hints.len(), {
            let v: Vec<_> = hints.iter().take(6).collect();
            v
        });
        let out = omt_nbt::write_with_list_hints(&nbt, ByteOrder::Little, &hints).unwrap();
        let orig = &v[offset..offset + used];
        println!("  bytes {} → {} equal={}", orig.len(), out.len(), orig == out);
        rebuilt.extend(&out);
        offset += used;
        entry += 1;
    }
    println!("total {} → {}", v.len(), rebuilt.len());
    let _ = Tag::Int(0);
}
