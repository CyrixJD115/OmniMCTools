//! **Web map export** (uNmINeD "Web page"): a self-contained folder with a
//! tile pyramid (`tiles/<level>/<tx>_<tz>.png`, 512 px, styled like the
//! viewer) and an `index.html` with a small dependency-free viewer — pan,
//! wheel zoom, coordinates, grid, spawn and player markers. "Update changed
//! regions only" keeps a manifest of tile fingerprints and re-renders only
//! tiles whose chunks changed since the last export.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use omt_render::{MapSource, Palette, TILE_CHUNKS, TileKey};

use crate::viewer_export::Look;

/// What to export.
#[derive(Debug, Clone)]
pub struct WebJob {
    /// Chunk rectangle (inclusive).
    pub chunks: (i32, i32, i32, i32),
    /// Most detailed level (0 = 1 px per block) and least detailed.
    pub max_detail: u8,
    pub min_detail: u8,
    pub changed_only: bool,
    pub grid: bool,
    /// Markers: (label, x, z, kind) — kind "spawn" / "player".
    pub markers: Vec<(String, f64, f64, &'static str)>,
    pub title: String,
}

/// Coarsest useful level: the whole area in about one tile.
pub fn auto_min_detail(chunks: (i32, i32, i32, i32)) -> u8 {
    let span = (chunks.2 - chunks.0 + 1).max(chunks.3 - chunks.1 + 1).max(1);
    let mut l = 0u8;
    while (TILE_CHUNKS << l) < span && l < omt_render::MAX_LEVEL {
        l += 1;
    }
    l
}

/// Tile keys of one level covering the chunk rect.
fn keys(level: u8, c: (i32, i32, i32, i32)) -> Vec<TileKey> {
    let a = TileKey::containing(level, c.0, c.1);
    let b = TileKey::containing(level, c.2, c.3);
    (a.tz..=b.tz).flat_map(|tz| (a.tx..=b.tx).map(move |tx| TileKey::new(level, tx, tz))).collect()
}

/// Number of tiles the job renders (upper bound).
pub fn tile_count(job: &WebJob) -> u64 {
    (job.max_detail..=job.min_detail).map(|l| keys(l, job.chunks).len() as u64).sum()
}

fn write_png(path: &Path, px: u32, rgba: &[u8]) -> Result<(), String> {
    crate::viewer_export::write_png(path, px, px, rgba)
}

/// Run the export into `dir`.
pub fn export_web(src: &MapSource, look: Look, job: &WebJob, dir: &Path, cancel: &AtomicBool, done: &AtomicU64) -> Result<String, String> {
    let manifest_path = dir.join("tiles").join("manifest.json");
    let old: HashMap<String, u64> = if job.changed_only { std::fs::read_to_string(&manifest_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default() } else { HashMap::new() };
    let mut manifest: HashMap<String, u64> = HashMap::new();
    let (mut written, mut skipped) = (0u64, 0u64);
    let style_tag = {
        let mut h = omt_render::source::Fnv::new();
        h.bytes(format!("{:?}{:?}", look.opts, look.style).as_bytes());
        h.finish()
    };
    for level in job.max_detail..=job.min_detail {
        let ldir = dir.join("tiles").join(level.to_string());
        std::fs::create_dir_all(&ldir).map_err(|e| e.to_string())?;
        let ks = keys(level, job.chunks);
        let results: Vec<Result<(String, u64, bool), String>> = omt_render::workers::install(look.threads, || {
            use rayon::prelude::*;
            ks.par_iter()
                .map(|&key| {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("cancelled".to_string());
                    }
                    let name = format!("{}/{}_{}", level, key.tx, key.tz);
                    let (x0, z0, x1, z1) = key.chunk_rect();
                    let fp = src.fingerprint(x0, z0 - 1, x1, z1) ^ style_tag;
                    let path = ldir.join(format!("{}_{}.png", key.tx, key.tz));
                    done.fetch_add(1, Ordering::Relaxed);
                    if job.changed_only && old.get(&name) == Some(&fp) && path.is_file() {
                        return Ok((name, fp, false));
                    }
                    let cancelled = || cancel.load(Ordering::Relaxed);
                    let Some(t) = omt_render::build_tile(src, key, None, Palette::embedded(), &look.opts, &cancelled) else { return Err("cancelled".to_string()) };
                    if t.px == 0 {
                        let _ = std::fs::remove_file(&path);
                        return Ok((name, fp, false));
                    }
                    let span = key.span_blocks() as f32;
                    let view = omt_render::style::TileView { px: t.px as usize, bpp: span / t.px as f32, rgba: &t.rgba, heights: &t.heights, surface: &t.surface, biomes: &t.biomes, names: &t.names };
                    let rgba = omt_render::style::stylize(&view, &look.style);
                    write_png(&path, t.px, &rgba)?;
                    Ok((name, fp, true))
                })
                .collect()
        });
        for r in results {
            let (name, fp, wrote) = r?;
            manifest.insert(name, fp);
            if wrote {
                written += 1;
            } else {
                skipped += 1;
            }
        }
    }
    std::fs::write(&manifest_path, serde_json::to_string(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("index.html"), page(job)).map_err(|e| e.to_string())?;
    Ok(format!("web map: {written} tiles written, {skipped} unchanged/empty → {}", dir.join("index.html").display()))
}

fn page(job: &WebJob) -> String {
    let markers: Vec<serde_json::Value> = job.markers.iter().map(|(l, x, z, k)| serde_json::json!({ "label": l, "x": x, "z": z, "kind": k })).collect();
    let cfg = serde_json::json!({
        "title": job.title,
        "tile": TILE_CHUNKS * 16,
        "minLevel": job.max_detail,
        "maxLevel": job.min_detail,
        "bounds": [job.chunks.0 * 16, job.chunks.1 * 16, (job.chunks.2 + 1) * 16, (job.chunks.3 + 1) * 16],
        "grid": job.grid,
        "markers": markers,
    });
    let title = job.title.replace('<', "&lt;");
    WEB_PAGE.replace("{{TITLE}}", &title).replace("{{CONFIG}}", &cfg.to_string())
}

/// The viewer page (vanilla JS, no external resources).
const WEB_PAGE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{{TITLE}}</title>
<style>
html,body{margin:0;height:100%;background:#111317;color:#e8e8e8;font:13px system-ui,sans-serif;overflow:hidden}
canvas{display:block;width:100%;height:100%;cursor:grab;image-rendering:pixelated}
#bar{position:fixed;left:10px;top:10px;background:#1c1f26e6;border:1px solid #333;border-radius:6px;padding:6px 10px;display:flex;gap:10px;align-items:center}
#bar b{font-weight:600}#bar button{background:#2a2e38;color:#ddd;border:1px solid #444;border-radius:4px;padding:2px 8px;cursor:pointer}
#pos{position:fixed;left:10px;bottom:10px;background:#1c1f26e6;border:1px solid #333;border-radius:6px;padding:4px 8px}
#credit{position:fixed;right:10px;bottom:10px;opacity:.6}
</style></head><body>
<canvas id="c"></canvas>
<div id="bar"><b>{{TITLE}}</b><button id="zi">+</button><button id="zo">−</button><button id="fit">Fit</button><label><input type="checkbox" id="g"> Grid</label><label><input type="checkbox" id="m" checked> Markers</label></div>
<div id="pos">X — Z —</div><div id="credit">OmniMCTools web map</div>
<script>
const C={{CONFIG}};
const cv=document.getElementById('c'),cx=cv.getContext('2d');
let W=0,H=0,dpr=1,cam={x:(C.bounds[0]+C.bounds[2])/2,z:(C.bounds[1]+C.bounds[3])/2,zoom:1};
const cache=new Map();
document.getElementById('g').checked=C.grid;
function resize(){dpr=window.devicePixelRatio||1;W=cv.clientWidth;H=cv.clientHeight;cv.width=W*dpr;cv.height=H*dpr;draw();}
function fit(){const w=C.bounds[2]-C.bounds[0],h=C.bounds[3]-C.bounds[1];cam.x=C.bounds[0]+w/2;cam.z=C.bounds[1]+h/2;cam.zoom=Math.min(W/w,H/h)*0.95;draw();}
function level(){let l=Math.round(Math.log2(1/cam.zoom));return Math.max(C.minLevel,Math.min(C.maxLevel,l));}
function tile(l,tx,tz){const k=l+'/'+tx+'_'+tz;let t=cache.get(k);if(!t){t=new Image();t.onload=draw;t.onerror=()=>{t.bad=true};t.src='tiles/'+k+'.png';cache.set(k,t);}return t;}
function toScreen(x,z){return[(x-cam.x)*cam.zoom+W/2,(z-cam.z)*cam.zoom+H/2];}
function draw(){
 cx.setTransform(dpr,0,0,dpr,0,0);cx.fillStyle='#111317';cx.fillRect(0,0,W,H);cx.imageSmoothingEnabled=cam.zoom<1;
 const l=level(),span=C.tile<<l;
 const x0=cam.x-W/2/cam.zoom,z0=cam.z-H/2/cam.zoom,x1=cam.x+W/2/cam.zoom,z1=cam.z+H/2/cam.zoom;
 for(let tz=Math.floor(z0/span);tz<=Math.floor(z1/span);tz++)for(let tx=Math.floor(x0/span);tx<=Math.floor(x1/span);tx++){
  if((tx+1)*span<C.bounds[0]||tx*span>C.bounds[2]||(tz+1)*span<C.bounds[1]||tz*span>C.bounds[3])continue;
  const t=tile(l,tx,tz);if(!t.complete||t.bad||!t.naturalWidth)continue;
  const[sx,sz]=toScreen(tx*span,tz*span);const s=span*cam.zoom;cx.drawImage(t,sx,sz,s+0.5,s+0.5);}
 if(document.getElementById('g').checked){const step=cam.zoom*16>=8?16:512;cx.strokeStyle=step==16?'rgba(255,255,255,.08)':'rgba(64,220,240,.35)';cx.beginPath();
  for(let x=Math.floor(x0/step)*step;x<=x1;x+=step){const[s]=toScreen(x,0);cx.moveTo(s,0);cx.lineTo(s,H);}
  for(let z=Math.floor(z0/step)*step;z<=z1;z+=step){const[,s]=toScreen(0,z);cx.moveTo(0,s);cx.lineTo(W,s);}cx.stroke();}
 if(document.getElementById('m').checked)for(const m of C.markers){const[sx,sz]=toScreen(m.x,m.z);if(sx<-50||sz<-50||sx>W+50||sz>H+50)continue;
  cx.fillStyle=m.kind=='spawn'?'#fff':'#f2c14e';cx.beginPath();cx.arc(sx,sz,5,0,7);cx.fill();cx.strokeStyle='#000';cx.stroke();
  cx.font='12px system-ui';cx.fillStyle='#000a';const w=cx.measureText(m.label).width;cx.fillRect(sx-w/2-4,sz+8,w+8,16);cx.fillStyle='#fff';cx.fillText(m.label,sx-w/2,sz+20);}
}
let drag=null;
cv.onmousedown=e=>{drag={x:e.clientX,y:e.clientY};cv.style.cursor='grabbing'};
window.onmouseup=()=>{drag=null;cv.style.cursor='grab'};
window.onmousemove=e=>{const bx=cam.x+(e.clientX-W/2)/cam.zoom,bz=cam.z+(e.clientY-H/2)/cam.zoom;
 document.getElementById('pos').textContent='X '+Math.floor(bx)+'  Z '+Math.floor(bz);
 if(drag){cam.x-=(e.clientX-drag.x)/cam.zoom;cam.z-=(e.clientY-drag.y)/cam.zoom;drag={x:e.clientX,y:e.clientY};draw();}};
function zoomAt(f,px,py){const bx=cam.x+(px-W/2)/cam.zoom,bz=cam.z+(py-H/2)/cam.zoom;cam.zoom=Math.max(1/(2**(C.maxLevel+2)),Math.min(16,cam.zoom*f));cam.x=bx-(px-W/2)/cam.zoom;cam.z=bz-(py-H/2)/cam.zoom;draw();}
cv.onwheel=e=>{e.preventDefault();zoomAt(e.deltaY<0?1.25:0.8,e.clientX,e.clientY)};
document.getElementById('zi').onclick=()=>zoomAt(2,W/2,H/2);document.getElementById('zo').onclick=()=>zoomAt(.5,W/2,H/2);
document.getElementById('fit').onclick=fit;document.getElementById('g').onchange=draw;document.getElementById('m').onchange=draw;
window.onresize=resize;resize();fit();
</script></body></html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_page() {
        assert_eq!(auto_min_detail((0, 0, 31, 31)), 0);
        assert_eq!(auto_min_detail((-100, -100, 100, 100)), 3);
        let job = WebJob { chunks: (-40, -40, 39, 39), max_detail: 0, min_detail: 2, changed_only: false, grid: true, markers: vec![("Spawn".into(), 0.0, 0.0, "spawn")], title: "My <World>".into() };
        assert_eq!(tile_count(&job), 16 + 4 + 4);
        let html = page(&job);
        assert!(html.contains("My &lt;World>") && html.contains("\"maxLevel\":2") && !html.contains("{{"));
    }

    /// Fixture-gated: `OMT_EXPORT_FIXTURE=<java world>` writes a small web
    /// map into `OMT_WEB_OUT` (or a temp dir) and re-runs it incrementally.
    #[test]
    #[ignore = "needs OMT_EXPORT_FIXTURE=<java world folder>"]
    fn export_web_writes_pyramid() {
        let Ok(root) = std::env::var("OMT_EXPORT_FIXTURE") else { return };
        let w = std::sync::Arc::new(omt_java::JavaWorld::open(std::path::Path::new(&root)).unwrap());
        let src = MapSource::java(&w, 0).unwrap();
        let (a, b, c, d) = src.chunks().core_bounds(0.02).unwrap();
        let out = std::env::var("OMT_WEB_OUT").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join(format!("omt-web-{}", std::process::id())));
        let job = WebJob { chunks: (a, b, c, d), max_detail: 2, min_detail: auto_min_detail((a, b, c, d)).max(2), changed_only: true, grid: true, markers: vec![("Spawn".into(), 0.0, 0.0, "spawn")], title: "fixture".into() };
        let look = Look { opts: omt_render::RenderOptions { relief: false, ..Default::default() }, style: omt_render::style::StyleParams::default(), threads: 4 };
        let first = export_web(&src, look, &job, &out, &AtomicBool::new(false), &AtomicU64::new(0)).unwrap();
        assert!(out.join("index.html").is_file() && out.join("tiles/2").is_dir(), "{first}");
        let again = export_web(&src, look, &job, &out, &AtomicBool::new(false), &AtomicU64::new(0)).unwrap();
        assert!(again.contains(" 0 tiles written"), "second run must skip unchanged tiles: {again}");
    }
}
