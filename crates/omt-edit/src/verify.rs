//! World integrity check: open the world fresh (read-only) and decode
//! everything an edit can touch — every chunk column of every dimension
//! (Bedrock: subchunks + data3D + block entities; Java: region chunks)
//! plus entity storage (Bedrock `digp` → actor records; Java `entities/`
//! regions). Any decode failure is reported with its location. Used after
//! edits (tests, `omt verify`), as the strongest check available without
//! launching the game.

use std::path::Path;

use omt_bedrock::keys::{self, tag};
use omt_world::WorldStore;
use rayon::prelude::*;

/// What a verification found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VerifyReport {
    pub chunks: usize,
    pub block_entities: usize,
    pub entities: usize,
    /// Human-readable failures (empty = the world decodes cleanly).
    pub errors: Vec<String>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Verify a world folder (Java or Bedrock), all dimensions.
pub fn verify(root: &Path) -> Result<VerifyReport, String> {
    match omt_world::detect(root).map_err(|e| e.to_string())? {
        omt_world::Detected::BedrockFolder { .. } => {
            let w = omt_bedrock::BedrockWorld::open_read_only(root).map_err(|e| e.to_string())?;
            Ok(verify_bedrock(&w))
        }
        omt_world::Detected::JavaFolder { .. } => {
            let w = omt_java::JavaWorld::open(root).map_err(|e| e.to_string())?;
            Ok(verify_java(&w))
        }
        omt_world::Detected::Archive(p) => Err(format!("{} is an archive — extract it first", p.display())),
    }
}

fn verify_bedrock(w: &omt_bedrock::BedrockWorld) -> VerifyReport {
    let mut r = VerifyReport::default();
    for dim in 0..3u8 {
        let Ok(chunks) = w.list_chunks(dim) else {
            r.errors.push(format!("dim {dim}: chunk listing failed"));
            continue;
        };
        let results: Vec<(usize, usize, Vec<String>)> = chunks
            .par_iter()
            .map(|&(x, z)| {
                let mut errs = Vec::new();
                if let Err(e) = w.read_chunk(dim, x, z) {
                    errs.push(format!("dim {dim} chunk {x},{z}: {e}"));
                }
                if let Ok(Some(raw)) = w.raw_get(&keys::data3d_key(x, z, dim))
                    && let Err(e) = omt_bedrock::Data3d::parse(&raw)
                {
                    errs.push(format!("dim {dim} chunk {x},{z} data3D: {e}"));
                }
                let mut bes = 0;
                if let Ok(Some(raw)) = w.raw_get(&keys::chunk_record(x, z, dim, tag::BLOCK_ENTITIES)) {
                    match crate::bedrock::parse_roots(&raw) {
                        Ok(v) => bes = v.len(),
                        Err(e) => errs.push(format!("dim {dim} chunk {x},{z} block entities: {e}")),
                    }
                }
                let mut ents = 0;
                if let Ok(Some(digp)) = w.raw_get(&keys::digp_key(x, z, dim)) {
                    if digp.len() % 8 != 0 {
                        errs.push(format!("dim {dim} chunk {x},{z}: digp length {} not a multiple of 8", digp.len()));
                    }
                    let ids: Vec<Vec<u8>> = digp.as_chunks::<8>().0.iter().map(|id| keys::actor_key(*id)).collect();
                    for v in w.get_exact(&ids).unwrap_or_default().into_iter().flatten() {
                        match omt_bedrock::store::parse_actor_nbt(&v) {
                            Ok(_) => ents += 1,
                            Err(e) => errs.push(format!("dim {dim} chunk {x},{z} actor: {e}")),
                        }
                    }
                }
                (bes, ents, errs)
            })
            .collect();
        r.chunks += chunks.len();
        for (b, e, errs) in results {
            r.block_entities += b;
            r.entities += e;
            r.errors.extend(errs);
        }
    }
    r
}

fn verify_java(w: &omt_java::JavaWorld) -> VerifyReport {
    let mut r = VerifyReport::default();
    for dim in 0..3u8 {
        let Ok(chunks) = w.list_chunks(dim) else { continue };
        let results: Vec<(usize, Vec<String>)> = chunks
            .par_iter()
            .map(|&(x, z)| {
                let mut errs = Vec::new();
                let mut bes = 0;
                match w.read_chunk_nbt(dim, x, z) {
                    Ok(Some(n)) => {
                        bes = n.tag.get("block_entities").and_then(omt_nbt::Tag::as_list).map_or(0, <[_]>::len);
                        if let Err(e) = w.read_chunk(dim, x, z) {
                            errs.push(format!("dim {dim} chunk {x},{z}: {e}"));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => errs.push(format!("dim {dim} chunk {x},{z}: {e}")),
                }
                (bes, errs)
            })
            .collect();
        r.chunks += chunks.len();
        for (b, errs) in results {
            r.block_entities += b;
            r.errors.extend(errs);
        }
        // Entity regions.
        let Some(dir) = w.entity_dimension_dir(dim) else { continue };
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("mca") {
                continue;
            }
            let Some((rx, rz)) = omt_java::RegionFile::coords_from_name(&p) else { continue };
            let Ok(region) = omt_java::RegionFile::open(&p) else {
                r.errors.push(format!("entity region {} unreadable", p.display()));
                continue;
            };
            for (x, z) in region.chunk_coords(rx, rz) {
                match region.read_chunk_from_file(x, z) {
                    Ok(n) => r.entities += n.tag.get("Entities").and_then(omt_nbt::Tag::as_list).map_or(0, <[_]>::len),
                    Err(e) => r.errors.push(format!("entities {x},{z}: {e}")),
                }
            }
        }
    }
    r
}
