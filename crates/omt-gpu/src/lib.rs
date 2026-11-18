//! `omt-gpu` — GPU 3D terrain for the Map Viewer (wgpu, off-screen).
//!
//! The height field the CPU path ray-casts (`omt_render::voxel::Grid`) is
//! turned into a blocky mesh — one top quad per cell plus side quads down
//! to lower neighbours — and rasterized off-screen with a depth buffer,
//! sun lighting and distance fog, then read back as RGBA. Off-screen
//! rendering works whatever backend the UI itself uses, so views of
//! thousands of blocks stay interactive. The CPU renderer remains the
//! fallback when no GPU adapter is available.

pub mod blocks;
mod target;

use std::sync::{Arc, Mutex, OnceLock};

use glam::{Mat4, Vec3};
use omt_render::voxel::{Cam3, Grid};
use wgpu::util::DeviceExt;

const SHADER: &str = r#"
struct U { mvp: mat4x4<f32>, eye: vec4<f32>, sun: vec4<f32>, fog: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct VIn {
  @location(0) corner: vec3<f32>, @location(1) normal: vec3<f32>,
  @location(2) cell: vec2<u32>, @location(3) span: vec2<f32>, @location(4) color: vec4<f32>,
};
struct VOut { @builtin(position) p: vec4<f32>, @location(0) c: vec3<f32>, @location(1) w: vec3<f32>, @location(2) n: vec3<f32> };
@vertex fn vs(v: VIn) -> VOut {
  // Column = unit box scaled to the cell; y from bottom (span.x) to top (span.y).
  let h = mix(v.span.x, v.span.y, v.corner.y);
  let y = u.eye.y + (h - u.eye.y) * u.eye.w;
  let w = vec3<f32>((f32(v.cell.x) + v.corner.x) * u.sun.w, y, (f32(v.cell.y) + v.corner.z) * u.sun.w);
  var o: VOut;
  o.p = u.mvp * vec4<f32>(w, 1.0);
  let side = select(0.85, 1.0, v.normal.y > 0.5);
  o.c = v.color.rgb * side;
  o.w = w;
  o.n = v.normal;
  return o;
}
@fragment fn fs(i: VOut) -> @location(0) vec4<f32> {
  let n = normalize(i.n);
  let diffuse = max(dot(n, normalize(u.sun.xyz)), 0.0);
  let lit = i.c * (0.6 + 0.5 * diffuse);
  let d = distance(i.w, u.eye.xyz) / u.fog.w;
  let f = clamp(pow(d, 1.6) * 0.9, 0.0, 0.9);
  return vec4<f32>(mix(lit, u.fog.rgb, f), 1.0);
}
"#;

/// Unit-box vertex: corner f32×3 + normal f32×3.
const VSTRIDE: u64 = 24;
/// Per-column instance: cell u16×2, (bottom, top) f32×2, color unorm8×4.
const ISTRIDE: u64 = 16;

/// The shared GPU device + pipeline.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// Terrain uniforms (rewritten per frame, never reallocated).
    ubuf: wgpu::Buffer,
    ubind: wgpu::BindGroup,
    /// Textured block pipelines (created on first use).
    blocks: OnceLock<blocks::BlockPipe>,
    /// The reused render target; locking it serializes frames.
    target: Mutex<Option<target::Target>>,
    pub adapter_name: String,
}

/// A terrain mesh uploaded to the GPU.
pub struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    instances: wgpu::Buffer,
    count: u32,
    cell: f32,
    /// World position of mesh-space (0, 0, 0).
    origin: (f64, f64),
    /// Columns (instances).
    pub columns: u32,
}

static GPU: OnceLock<Option<Arc<Gpu>>> = OnceLock::new();

/// The process-wide GPU (initialized on first use; `None` = no adapter).
pub fn gpu() -> Option<Arc<Gpu>> {
    GPU.get_or_init(|| Gpu::new().ok().map(Arc::new)).clone()
}

pub(crate) const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub(crate) const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Multisample count (4× is guaranteed for these formats).
pub(crate) const SAMPLES: u32 = 4;
/// Terrain uniform bytes: mvp + eye + sun + fog.
const TERRAIN_U: u64 = (16 + 12) * 4;

impl Gpu {
    pub fn new() -> Result<Self, String> {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: None, force_fallback_adapter: false })
                .await
                .map_err(|e| e.to_string())?;
            let adapter_name = adapter.get_info().name;
            // The adapter's own limits (texture-array layers for the block
            // atlas, large render targets).
            let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor { label: Some("omt-gpu"), required_limits: adapter.limits(), ..Default::default() }).await.map_err(|e| e.to_string())?;
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("terrain"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                }],
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&layout], push_constant_ranges: &[] });
            let vattrs = [
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
            ];
            let iattrs = [
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint16x2, offset: 0, shader_location: 2 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 4, shader_location: 3 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 12, shader_location: 4 },
            ];
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("terrain"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[
                        wgpu::VertexBufferLayout { array_stride: VSTRIDE, step_mode: wgpu::VertexStepMode::Vertex, attributes: &vattrs },
                        wgpu::VertexBufferLayout { array_stride: ISTRIDE, step_mode: wgpu::VertexStepMode::Instance, attributes: &iattrs },
                    ],
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::Less, stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: COLOR, blend: None, write_mask: wgpu::ColorWrites::COLOR })] }),
                multiview: None,
                cache: None,
            });
            let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("terrain-u"), size: TERRAIN_U, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let ubind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }] });
            Ok(Self { device, queue, pipeline, ubuf, ubind, blocks: OnceLock::new(), target: Mutex::new(None), adapter_name })
        })
    }

    /// Build + upload the blocky mesh of a height field.
    pub fn mesh(&self, g: &Grid) -> Mesh {
        let (verts, idx) = unit_box();
        let inst = build_instances(g);
        let columns = (inst.len() / ISTRIDE as usize) as u32;
        let vertices = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
        let indices = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX });
        let instances = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &inst, usage: wgpu::BufferUsages::VERTEX });
        Mesh { vertices, indices, instances, count: idx.len() as u32, cell: g.cell as f32, origin: (g.x0, g.z0), columns }
    }

    /// Render `w × h` RGBA (rows top to bottom), 4× MSAA.
    pub fn render(&self, mesh: &Mesh, cam: &Cam3, w: u32, h: u32, sky: ([u8; 3], [u8; 3])) -> Vec<u8> {
        let (w, h) = (w.max(8), h.max(8));
        let (mvp, eye) = view_proj(cam, mesh.origin, w, h);
        let horizon = [sky.1[0] as f32 / 255.0, sky.1[1] as f32 / 255.0, sky.1[2] as f32 / 255.0];
        let sun = Vec3::new(-0.45, 0.8, -0.35).normalize();
        let mut u: Vec<f32> = mvp.to_cols_array().to_vec();
        u.extend([eye.x, eye.y, eye.z, cam.scale, sun.x, sun.y, sun.z, mesh.cell, horizon[0], horizon[1], horizon[2], cam.far]);
        let mut slot = self.target.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.queue.write_buffer(&self.ubuf, 0, bytemuck::cast_slice(&u));
        self.offscreen(&mut slot, w, h, cam.pitch, sky, &mut |pass| {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.ubind, &[]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_vertex_buffer(1, mesh.instances.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, 0..mesh.columns);
        })
    }
}

/// View-projection for a [`Cam3`] (same framing as the CPU renderers)
/// relative to a mesh `origin` (x, z); returns (matrix, eye).
pub(crate) fn view_proj(cam: &Cam3, origin: (f64, f64), w: u32, h: u32) -> (Mat4, Vec3) {
    let eye = Vec3::new((cam.x - origin.0) as f32, cam.y, (cam.z - origin.1) as f32);
    let focal = (w as f32 / 2.0) / (cam.fov / 2.0).tan();
    // The CPU renderer's horizon shift → a look-up angle.
    let up = (cam.pitch * h as f32 / focal).atan();
    let fwd = Vec3::new(cam.yaw.sin() * up.cos(), up.sin(), -cam.yaw.cos() * up.cos());
    let aspect = w as f32 / h as f32;
    let fovy = 2.0 * ((cam.fov / 2.0).tan() / aspect).atan();
    (Mat4::perspective_rh(fovy, aspect, 0.2, cam.far * 1.6) * Mat4::look_to_rh(eye, fwd, Vec3::Y), eye)
}

/// A unit box without its bottom face: 5 faces × 4 corners
/// (corner xyz in 0..1, normal), plus indices.
pub fn unit_box() -> (Vec<f32>, Vec<u32>) {
    let faces: [([f32; 3], [[f32; 3]; 4]); 5] = [
        ([0.0, 1.0, 0.0], [[0., 1., 0.], [1., 1., 0.], [1., 1., 1.], [0., 1., 1.]]),
        ([1.0, 0.0, 0.0], [[1., 1., 0.], [1., 1., 1.], [1., 0., 1.], [1., 0., 0.]]),
        ([-1.0, 0.0, 0.0], [[0., 1., 1.], [0., 1., 0.], [0., 0., 0.], [0., 0., 1.]]),
        ([0.0, 0.0, 1.0], [[1., 1., 1.], [0., 1., 1.], [0., 0., 1.], [1., 0., 1.]]),
        ([0.0, 0.0, -1.0], [[0., 1., 0.], [1., 1., 0.], [1., 0., 0.], [0., 0., 0.]]),
    ];
    let mut v = Vec::new();
    let mut idx = Vec::new();
    for (i, (n, corners)) in faces.iter().enumerate() {
        for c in corners {
            v.extend_from_slice(c);
            v.extend_from_slice(n);
        }
        let b = (i * 4) as u32;
        idx.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    (v, idx)
}

/// One instance per filled cell: the column spans from its lowest
/// neighbour (sides only where exposed) to its top.
pub fn build_instances(g: &Grid) -> Vec<u8> {
    let n = g.n;
    let mut out = Vec::with_capacity(n * n * ISTRIDE as usize);
    let h_at = |q: i64, r: i64| -> Option<f32> {
        if q < 0 || r < 0 || q >= n as i64 || r >= n as i64 {
            return None;
        }
        let h = g.height[r as usize * n + q as usize];
        (!h.is_nan()).then_some(h)
    };
    for r in 0..n as i64 {
        for q in 0..n as i64 {
            let Some(h) = h_at(q, r) else { continue };
            let top = h + 1.0;
            let bottom = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().map(|&(dq, dr)| h_at(q + dq, r + dr).map_or(h - 6.0, |nh| nh + 1.0)).fold(top, f32::min);
            let c = g.color[r as usize * n + q as usize];
            out.extend_from_slice(&(q as u16).to_le_bytes());
            out.extend_from_slice(&(r as u16).to_le_bytes());
            out.extend_from_slice(&bottom.to_le_bytes());
            out.extend_from_slice(&top.to_le_bytes());
            out.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pillar_grid() -> Grid {
        let f = |x: f64, z: f64| -> Option<(f32, [u8; 3])> {
            if x.abs() < 6.0 && (z + 50.0).abs() < 6.0 { Some((150.0, [200, 30, 30])) } else { Some((60.0, [40, 160, 40])) }
        };
        Grid::build(0.0, 0.0, 128.0, 128, &f)
    }

    #[test]
    fn instances_span_to_the_lowest_neighbour() {
        let g = pillar_grid();
        let inst = build_instances(&g);
        assert_eq!(inst.len(), 128 * 128 * ISTRIDE as usize, "one per cell");
        let f = |i: usize, o: usize| f32::from_le_bytes(inst[i * 16 + o..i * 16 + o + 4].try_into().unwrap());
        // A flat interior cell: bottom == top (no exposed sides).
        let flat = 64 * 128 + 10;
        assert_eq!(f(flat, 4), f(flat, 8));
        // The pillar's edge cell reaches down to the ground.
        let edge = (64 - 25 - 3) * 128 + 64 + 2;
        assert!(f(edge, 8) > 150.0 && f(edge, 4) < 70.0, "{} {}", f(edge, 4), f(edge, 8));
        let (v, idx) = unit_box();
        assert_eq!((v.len(), idx.len()), (5 * 4 * 6, 30));
    }

    /// Needs a GPU adapter; skipped (passes) when none is available.
    #[test]
    fn renders_the_pillar_on_the_gpu() {
        let Some(gpu) = gpu() else { return };
        let g = pillar_grid();
        let mesh = gpu.mesh(&g);
        let cam = Cam3 { y: 90.0, pitch: 0.0, ..Default::default() };
        let (w, h) = (96u32, 64u32);
        let img = gpu.render(&mesh, &cam, w, h, ([10, 20, 60], [150, 180, 220]));
        assert_eq!(img.len(), (w * h * 4) as usize);
        let px = |x: u32, y: u32| [img[((y * w + x) * 4) as usize], img[((y * w + x) * 4 + 1) as usize], img[((y * w + x) * 4 + 2) as usize]];
        assert!((0..h / 2).any(|y| { let p = px(w / 2, y); p[0] > 60 && p[0] as u32 > 2 * p[1] as u32 }), "pillar ahead ({})", gpu.adapter_name);
        let ground = px(2, h - 2);
        assert!(ground[1] > ground[0], "ground {ground:?}");
        assert_eq!(px(0, 0), [10, 20, 60], "sky");
    }
}
