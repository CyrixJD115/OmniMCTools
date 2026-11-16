//! Probe: relation between actorprefix key bytes and UniqueID.
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("world"));
    let w = omt_bedrock::BedrockWorld::open_read_only(&root).expect("open");
    let mut n = 0;
    for (k, _) in w.key_index().expect("keys") {
        let Some(id) = k.strip_prefix(omt_bedrock::keys::ACTOR_PREFIX) else { continue };
        let v = w.raw_get(&k).unwrap().unwrap();
        let nbt = omt_nbt::read(&v, omt_nbt::ByteOrder::Little).unwrap();
        let uid = nbt.tag.get("UniqueID").and_then(|t| t.as_i64()).unwrap();
        let be = u64::from_be_bytes(id.try_into().unwrap());
        let le = u64::from_le_bytes(id.try_into().unwrap());
        let pos = nbt.tag.get("Pos").map(omt_nbt::snbt::to_snbt).unwrap_or_default();
        let defs = nbt.tag.get("definitions").map(omt_nbt::snbt::to_snbt).unwrap_or_default();
        println!("key BE {be:#x} LE {le:#x} UniqueID {uid} ({:#x}) Pos {pos} defs {defs}", uid as u64);
        n += 1;
        if n >= 4 { break; }
    }
}
