//! Textured block rendering on the GPU: the real blocks of a
//! [`BlockGrid`] become a face-culled mesh (a face is emitted only where a
//! block touches air or a see-through block), textured from a texture
//! *array* (one deduplicated 16×16 layer per distinct face texture, so
//! nothing bleeds between tiles) with a full mip chain: magnified texels
//! stay crisp (nearest), distant faces are filtered (no shimmer while the
//! camera moves). Cut-outs (flowers, leaves, glass) use alpha-to-coverage
//! with coverage-preserving mips so foliage stays solid far away; water is
//! a second, blended pass. Fog fades the edge of the loaded area into the
//! sky (a round render distance). 4× MSAA, same reused off-screen target
//! and camera as the terrain renderer.

use omt_render::blocks3d::BlockGrid;
use omt_render::voxel::Cam3;
use wgpu::util::DeviceExt;

use crate::{COLOR, DEPTH, Gpu, SAMPLES, view_proj};

const SHADER: &str = r#"
// sky: zenith rgb + the horizon row in pixels (the sky gradient's end).
struct U { mvp: mat4x4<f32>, eye: vec4<f32>, fog: vec4<f32>, sky: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var atlas: texture_2d_array<f32>;
@group(1) @binding(1) var samp: sampler;
struct VIn { @location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) shade: f32, @location(3) layer: u32 };
struct VOut {
  @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) shade: f32,
  @location(2) w: vec3<f32>, @location(3) @interpolate(flat) layer: u32,
};
@vertex fn vs(v: VIn) -> VOut {
  var o: VOut;
  o.p = u.mvp * vec4<f32>(v.pos, 1.0);
  o.uv = v.uv;
  o.shade = v.shade;
  o.w = v.pos;
  o.layer = min(v.layer, u32(u.eye.w));
  return o;
}
fn shaded(i: VOut, t: vec4<f32>) -> vec3<f32> {
  // Horizontal distance: the loaded area is a square column; fade its
  // edge fully into the sky so the render distance reads as a circle.
  // Fog toward the sky *behind this pixel* (same gradient the background
  // gets), so fully fogged far terrain is indistinguishable from the sky.
  let d = distance(i.w.xz, u.eye.xz) / u.fog.w;
  let behind = mix(u.sky.rgb, u.fog.rgb, clamp(i.p.y / u.sky.w, 0.0, 1.0));
  return mix(t.rgb * i.shade, behind, smoothstep(0.55, 1.0, d));
}
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
  let t = textureSample(atlas, samp, i.uv, i.layer);
  // Sharpened alpha → crisp alpha-to-coverage edges at every distance.
  let a = clamp((t.a - 0.5) / max(fwidth(t.a), 0.0001) + 0.5, 0.0, 1.0);
  if (a <= 0.0) { discard; }
  return vec4<f32>(shaded(i, t), a);
}
@fragment fn fs_water(i: VOut) -> @location(0) vec4<f32> {
  let t = textureSample(atlas, samp, i.uv, i.layer);
  return vec4<f32>(shaded(i, t), 0.72);
}
"#;

/// Vertex: position f32×3, uv f32×2, shade f32, layer u32.
const STRIDE: u64 = 28;
/// Floats (well, 4-byte words) per vertex.
const WORDS: usize = 7;
/// Mip levels of a 16×16 tile (16, 8, 4, 2, 1).
const MIPS: u32 = 5;
/// Block uniform bytes: mvp + eye + fog + sky.
const BLOCK_U: u64 = (16 + 12) * 4;
/// Faces cap (memory guard).
const MAX_FACES: usize = 8_000_000;

pub(crate) struct BlockPipe {
    opaque: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    tlayout: wgpu::BindGroupLayout,
    ubuf: wgpu::Buffer,
    ubind: wgpu::BindGroup,
}

/// A block mesh + its atlas on the GPU.
pub struct BlockMesh {
    opaque: (wgpu::Buffer, wgpu::Buffer, u32),
    water: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    atlas: wgpu::BindGroup,
    /// Highest valid texture layer.
    max_layer: u32,
    origin: (f64, f64),
    /// Faces in the mesh.
    pub faces: usize,
    /// Distinct face textures (array layers).
    pub layers: usize,
}

/// CPU-side mesh data (testable without a GPU).
pub struct MeshData {
    /// Vertex words (f32 bits; the layer word is a u32) + indices.
    pub opaque: (Vec<u32>, Vec<u32>),
    pub water: (Vec<u32>, Vec<u32>),
    /// Distinct 16×16 RGBA face textures (array layers).
    pub tiles: Vec<Vec<[u8; 4]>>,
    pub faces: usize,
}

/// Faces: normal index → (corner offsets, shade). Order of corners maps to
/// uv (0,0) (1,0) (1,1) (0,1) with v = 0 at the top of side faces.
const FACES: [([i32; 3], [[f32; 3]; 4], f32); 6] = [
    ([0, 1, 0], [[0., 1., 0.], [1., 1., 0.], [1., 1., 1.], [0., 1., 1.]], 1.0),
    ([0, -1, 0], [[0., 0., 1.], [1., 0., 1.], [1., 0., 0.], [0., 0., 0.]], 0.5),
    ([1, 0, 0], [[1., 1., 1.], [1., 1., 0.], [1., 0., 0.], [1., 0., 1.]], 0.8),
    ([-1, 0, 0], [[0., 1., 0.], [0., 1., 1.], [0., 0., 1.], [0., 0., 0.]], 0.8),
    ([0, 0, 1], [[0., 1., 1.], [1., 1., 1.], [1., 0., 1.], [0., 0., 1.]], 0.9),
    ([0, 0, -1], [[1., 1., 0.], [0., 1., 0.], [0., 0., 0.], [1., 0., 0.]], 0.9),
];

/// A 16×16 tile from a kind texture (padded / truncated to 256 texels).
fn tile16(tex: &[[u8; 4]]) -> Vec<[u8; 4]> {
    let mut t: Vec<[u8; 4]> = tex.iter().take(256).copied().collect();
    t.resize(256, [0, 0, 0, 0]);
    t
}

/// Build the mesh + deduplicated face textures of a grid on the CPU.
pub fn mesh_data(g: &BlockGrid) -> MeshData {
    // Layer of each kind's (top, side) texture; identical textures share.
    let mut tiles: Vec<Vec<[u8; 4]>> = Vec::new();
    let mut seen: std::collections::HashMap<Vec<[u8; 4]>, u32> = std::collections::HashMap::new();
    let mut layer_of = |tex: &[[u8; 4]]| -> u32 {
        let t = tile16(tex);
        *seen.entry(t.clone()).or_insert_with(|| {
            tiles.push(t);
            (tiles.len() - 1) as u32
        })
    };
    let kind_layers: Vec<[u32; 2]> = g.kinds.iter().map(|k| [layer_of(&k.top), layer_of(&k.side)]).collect();
    let see_through: Vec<bool> = g.kinds.iter().map(|k| k.water || k.top.iter().chain(&k.side).any(|p| p[3] < 128)).collect();
    let [sx, sy, sz] = g.size;
    let at = |x: i64, y: i64, z: i64| -> u16 {
        if x < 0 || y < 0 || z < 0 || x >= sx as i64 || y >= sy as i64 || z >= sz as i64 {
            return 0;
        }
        g.cells[(y as usize * sz + z as usize) * sx + x as usize]
    };
    let (mut ov, mut oi, mut wv, mut wi) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut faces = 0usize;
    for y in 0..sy as i64 {
        for z in 0..sz as i64 {
            for x in 0..sx as i64 {
                let id = at(x, y, z);
                if id == 0 {
                    continue;
                }
                let water = g.kinds[id as usize].water;
                for (fi, (n, corners, shade)) in FACES.iter().enumerate() {
                    let nid = at(x + n[0] as i64, y + n[1] as i64, z + n[2] as i64);
                    let exposed = if water { nid == 0 } else { nid == 0 || (nid != id && see_through[nid as usize]) };
                    if !exposed || faces >= MAX_FACES {
                        continue;
                    }
                    faces += 1;
                    let layer = kind_layers[id as usize][if fi < 2 { 0 } else { 1 }];
                    const UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
                    let (v, i) = if water { (&mut wv, &mut wi) } else { (&mut ov, &mut oi) };
                    let base = (v.len() / WORDS) as u32;
                    for (c, uv) in corners.iter().zip(UVS) {
                        // Water surfaces sit a little lower, like in game.
                        let cy = if water && fi == 0 { c[1] - 0.12 } else { c[1] };
                        let f = [x as f32 + c[0], (g.origin[1] as i64 + y) as f32 + cy, z as f32 + c[2], uv[0], uv[1], *shade];
                        v.extend(f.iter().map(|w| w.to_bits()));
                        v.push(layer);
                    }
                    i.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                }
            }
        }
    }
    MeshData { opaque: (ov, oi), water: (wv, wi), tiles, faces }
}

/// Mip chain of a 16×16 tile (levels 16, 8, 4, 2, 1; RGBA bytes each).
/// Colors are alpha-weighted (no dark fringes); alpha is rescaled per
/// level so the share of texels passing the 0.5 cut-out threshold matches
/// level 0 — foliage keeps its density in the distance instead of
/// thinning out.
pub fn mip_chain(tile: &[[u8; 4]]) -> Vec<Vec<u8>> {
    let coverage = |t: &[[f32; 4]], s: f32| t.iter().filter(|p| p[3] * s >= 0.5).count() as f32 / t.len() as f32;
    let mut cur: Vec<[f32; 4]> = tile.iter().map(|p| p.map(|c| c as f32 / 255.0)).collect();
    let target = coverage(&cur, 1.0);
    let to_bytes = |t: &[[f32; 4]]| t.iter().flat_map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect::<Vec<u8>>();
    let mut out = vec![to_bytes(&cur)];
    let mut n = 16usize;
    while n > 1 {
        let m = n / 2;
        let mut next = vec![[0.0f32; 4]; m * m];
        for y in 0..m {
            for x in 0..m {
                let q = [cur[2 * y * n + 2 * x], cur[2 * y * n + 2 * x + 1], cur[(2 * y + 1) * n + 2 * x], cur[(2 * y + 1) * n + 2 * x + 1]];
                let a: f32 = q.iter().map(|p| p[3]).sum();
                let rgb = if a > 0.0 { [0, 1, 2].map(|k| q.iter().map(|p| p[k] * p[3]).sum::<f32>() / a) } else { [0, 1, 2].map(|k| q.iter().map(|p| p[k]).sum::<f32>() / 4.0) };
                next[y * m + x] = [rgb[0], rgb[1], rgb[2], a / 4.0];
            }
        }
        // Coverage-preserving alpha scale (only for cut-out textures).
        if target > 0.0 && target < 1.0 {
            let (mut lo, mut hi) = (0.25f32, 8.0f32);
            for _ in 0..16 {
                let mid = (lo + hi) / 2.0;
                if coverage(&next, mid) < target { lo = mid } else { hi = mid }
            }
            let s = (lo + hi) / 2.0;
            for p in &mut next {
                p[3] = (p[3] * s).min(1.0);
            }
        }
        out.push(to_bytes(&next));
        cur = next;
        n = m;
    }
    out
}

impl Gpu {
    fn block_pipe(&self) -> &BlockPipe {
        self.blocks.get_or_init(|| {
            let d = &self.device;
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("blocks"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
            let ulayout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
            });
            let tlayout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[
                    wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
                    wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                ],
            });
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&ulayout, &tlayout], push_constant_ranges: &[] });
            let attrs = [
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 12, shader_location: 1 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 20, shader_location: 2 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 24, shader_location: 3 },
            ];
            let make = |fs: &str, blend: Option<wgpu::BlendState>, depth_write: bool, a2c: bool| {
                d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("blocks"),
                    layout: Some(&pl),
                    vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[wgpu::VertexBufferLayout { array_stride: STRIDE, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs }] },
                    primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                    depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: depth_write, depth_compare: wgpu::CompareFunction::Less, stencil: Default::default(), bias: Default::default() }),
                    multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: a2c },
                    fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: COLOR, blend, write_mask: wgpu::ColorWrites::COLOR })] }),
                    multiview: None,
                    cache: None,
                })
            };
            let ubuf = d.create_buffer(&wgpu::BufferDescriptor { label: Some("blocks-u"), size: BLOCK_U, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let ubind = d.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &ulayout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }] });
            BlockPipe { opaque: make("fs", None, true, true), water: make("fs_water", Some(wgpu::BlendState::ALPHA_BLENDING), false, false), tlayout, ubuf, ubind }
        })
    }

    /// Build + upload the textured block mesh of a grid.
    pub fn block_mesh(&self, g: &BlockGrid) -> BlockMesh {
        let data = mesh_data(g);
        let pipe = self.block_pipe();
        let d = &self.device;
        let upload = |(v, i): &(Vec<u32>, Vec<u32>)| {
            let vb = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(v), usage: wgpu::BufferUsages::VERTEX });
            let ib = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(i), usage: wgpu::BufferUsages::INDEX });
            (vb, ib, i.len() as u32)
        };
        // Texture array: one layer per distinct face texture, full mips
        // (layer-major data: layer 0 mips 0..4, layer 1 …). Layers past the
        // device limit (practically never) clamp to the last one.
        let limit = d.limits().max_texture_array_layers.max(1) as usize;
        let layers = data.tiles.len().clamp(1, limit);
        let mut bytes = Vec::with_capacity(layers * 1400);
        for i in 0..layers {
            let blank = vec![[0u8; 4]; 256];
            for lvl in mip_chain(data.tiles.get(i).unwrap_or(&blank)) {
                bytes.extend_from_slice(&lvl);
            }
        }
        let size = wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: layers as u32 };
        let tex = d.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor { label: Some("block-textures"), size, mip_level_count: MIPS, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] },
            wgpu::util::TextureDataOrder::LayerMajor,
            &bytes,
        );
        // Crisp up close (nearest), smooth with distance (trilinear mips).
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let atlas = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipe.tlayout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }],
        });
        BlockMesh {
            opaque: upload(&data.opaque),
            water: (!data.water.1.is_empty()).then(|| upload(&data.water)),
            atlas,
            max_layer: (layers - 1) as u32,
            origin: (g.origin[0] as f64, g.origin[2] as f64),
            faces: data.faces,
            layers,
        }
    }

    /// Render a block mesh (`w × h` RGBA, 4× MSAA). `cam.far` is the
    /// render distance: the fog reaches the sky color there.
    pub fn render_blocks(&self, mesh: &BlockMesh, cam: &Cam3, w: u32, h: u32, sky: ([u8; 3], [u8; 3])) -> Vec<u8> {
        let (w, h) = (w.max(8), h.max(8));
        let pipe = self.block_pipe();
        let (mvp, eye) = view_proj(cam, mesh.origin, w, h);
        let far = (cam.far).max(32.0);
        let mut u: Vec<f32> = mvp.to_cols_array().to_vec();
        u.extend([eye.x, eye.y, eye.z, mesh.max_layer as f32, sky.1[0] as f32 / 255.0, sky.1[1] as f32 / 255.0, sky.1[2] as f32 / 255.0, far]);
        u.extend([sky.0[0] as f32 / 255.0, sky.0[1] as f32 / 255.0, sky.0[2] as f32 / 255.0, crate::target::horizon_row(h, cam.pitch)]);
        let mut slot = self.target.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.queue.write_buffer(&pipe.ubuf, 0, bytemuck::cast_slice(&u));
        self.offscreen(&mut slot, w, h, cam.pitch, sky, &mut |pass| {
            pass.set_bind_group(0, &pipe.ubind, &[]);
            pass.set_bind_group(1, &mesh.atlas, &[]);
            pass.set_pipeline(&pipe.opaque);
            pass.set_vertex_buffer(0, mesh.opaque.0.slice(..));
            pass.set_index_buffer(mesh.opaque.1.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.opaque.2, 0, 0..1);
            if let Some((vb, ib, n)) = &mesh.water {
                pass.set_pipeline(&pipe.water);
                pass.set_vertex_buffer(0, vb.slice(..));
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..*n, 0, 0..1);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omt_render::blocks3d::Kind;

    fn scene() -> BlockGrid {
        let mut g = BlockGrid::empty([-32, 0, -32], [64, 40, 64]);
        g.kinds.push(Kind::flat([60, 170, 60, 255])); // 1 ground
        g.kinds.push(Kind::flat([210, 40, 40, 255])); // 2 tower
        for x in -32..32 {
            for z in -32..32 {
                g.set(x, 9, z, 1);
                g.set(x, 10, z, 1);
            }
        }
        for y in 11..35 {
            for x in -2..2 {
                for z in -22..-18 {
                    g.set(x, y, z, 2);
                }
            }
        }
        g
    }

    #[test]
    fn faces_are_culled() {
        let d = mesh_data(&scene());
        // Ground: 64×64 tops + bottoms + the outer rim (2 layers deep);
        // buried faces between the two ground layers are culled.
        let ground = 64 * 64 * 2 + 4 * 64 * 2;
        // Tower: 4×4 top + 4 sides × 4 × 24 (its bottom touches ground).
        let tower = 16 + 4 * 4 * 24;
        assert_eq!(d.faces, ground + tower - 16, "tower covers 16 ground tops");
        assert_eq!(d.opaque.1.len(), d.faces * 6);
        assert_eq!(d.opaque.0.len(), d.faces * 4 * WORDS);
        assert!(d.water.1.is_empty());
        // Air + two flat kinds, each sharing top/side: 3 distinct layers.
        assert_eq!(d.tiles.len(), 3);
    }

    #[test]
    fn mips_keep_cutout_coverage() {
        // A checkerboard of opaque / clear 2×2 blocks: 50% coverage.
        let tile: Vec<[u8; 4]> = (0..256).map(|i| if ((i % 16) / 2 + (i / 16) / 2) % 2 == 0 { [200, 50, 50, 255] } else { [0, 0, 0, 0] }).collect();
        let mips = mip_chain(&tile);
        assert_eq!(mips.iter().map(Vec::len).collect::<Vec<_>>(), vec![1024, 256, 64, 16, 4]);
        // Colors stay red (alpha-weighted, no black fringe) at every level.
        for m in &mips {
            assert!(m.chunks(4).filter(|p| p[3] > 0).all(|p| p[0] > 150 && p[1] < 80), "{m:?}");
        }
        // 8×8: each texel is one 2×2 block → exactly half pass the cut.
        assert_eq!(mips[1].chunks(4).filter(|p| p[3] >= 128).count(), 32);
        // 1×1: the whole tile in one texel at alpha ≈ 0.5, which
        // alpha-to-coverage turns into half the samples (50% coverage).
        assert!((118..=138).contains(&mips[4][3]), "{:?}", mips[4]);
        let opaque: Vec<[u8; 4]> = vec![[10, 200, 10, 255]; 256];
        assert!(mip_chain(&opaque).iter().all(|m| m.chunks(4).all(|p| p == [10, 200, 10, 255])));
    }

    #[test]
    fn renders_textured_blocks_on_the_gpu() {
        let Some(gpu) = crate::gpu() else { return };
        let mesh = gpu.block_mesh(&scene());
        let cam = Cam3 { x: 0.0, z: 0.0, y: 20.0, pitch: 0.0, ..Default::default() };
        let (w, h) = (64u32, 48u32);
        let img = gpu.render_blocks(&mesh, &cam, w, h, ([10, 20, 60], [150, 180, 220]));
        let px = |x: u32, y: u32| [img[((y * w + x) * 4) as usize], img[((y * w + x) * 4 + 1) as usize], img[((y * w + x) * 4 + 2) as usize]];
        assert!((0..h / 2).any(|y| { let p = px(w / 2, y); p[0] > 60 && p[0] as u32 > 2 * p[1] as u32 }), "tower");
        let g = px(1, h - 1);
        assert!(g[1] > g[0] && g[1] > g[2], "ground {g:?}");
        assert_eq!(px(0, 0), [10, 20, 60]);
    }
}
