//! The persistent off-screen render target: a 4× multisampled color +
//! depth attachment, the single-sample resolve texture and the readback
//! buffer. Allocated once per size and reused for every frame — creating
//! them per frame made GPU memory climb while the camera moved (wgpu frees
//! dropped resources lazily).

use crate::{COLOR, DEPTH, Gpu, SAMPLES};

pub(crate) struct Target {
    w: u32,
    h: u32,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    read: wgpu::Buffer,
    row: u32,
}

impl Target {
    fn new(d: &wgpu::Device, w: u32, h: u32) -> Self {
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let tex = |format, usage, samples| d.create_texture(&wgpu::TextureDescriptor { label: Some("omt-target"), size, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
        let msaa = tex(COLOR, wgpu::TextureUsages::RENDER_ATTACHMENT, SAMPLES).create_view(&Default::default());
        let depth = tex(DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT, SAMPLES).create_view(&Default::default());
        let resolve = tex(COLOR, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, 1);
        let resolve_view = resolve.create_view(&Default::default());
        let row = (w * 4).div_ceil(256) * 256;
        let read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("omt-readback"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        Self { w, h, msaa, depth, resolve, resolve_view, read, row }
    }
}

impl Gpu {
    /// Render into the (reused) target cleared to the horizon color, read
    /// it back as opaque RGBA (rows top to bottom), and paint the sky
    /// gradient behind the scene. The caller holds the render lock
    /// (`slot`), which also serializes uniform-buffer writes.
    pub(crate) fn offscreen(&self, slot: &mut Option<Target>, w: u32, h: u32, pitch: f32, sky: ([u8; 3], [u8; 3]), draw: &mut dyn FnMut(&mut wgpu::RenderPass<'_>)) -> Vec<u8> {
        if slot.as_ref().is_none_or(|t| (t.w, t.h) != (w, h)) {
            *slot = Some(Target::new(&self.device, w, h));
        }
        let t = slot.as_ref().expect("target");
        let horizon = [sky.1[0] as f64 / 255.0, sky.1[1] as f64 / 255.0, sky.1[2] as f64 / 255.0];
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let clear = wgpu::Color { r: horizon[0], g: horizon[1], b: horizon[2], a: 1.0 };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.msaa,
                    resolve_target: Some(&t.resolve_view),
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            draw(&mut pass);
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &t.resolve, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &t.read, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(t.row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = t.read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        {
            let data = slice.get_mapped_range();
            for y in 0..h as usize {
                px.extend_from_slice(&data[y * t.row as usize..y * t.row as usize + w as usize * 4]);
            }
        }
        t.read.unmap();
        // Sky gradient behind the scene (pixels still at the clear color);
        // every pixel opaque.
        let horizon_row = horizon_row(h, pitch);
        for (y, line) in px.chunks_exact_mut(w as usize * 4).enumerate() {
            let tt = (y as f32 / horizon_row).clamp(0.0, 1.0);
            let c = [0, 1, 2].map(|k| (sky.0[k] as f32 + (sky.1[k] as f32 - sky.0[k] as f32) * tt) as u8);
            for p in line.as_chunks_mut::<4>().0 {
                if p[..3] == sky.1 {
                    p[..3].copy_from_slice(&c);
                }
                p[3] = 255;
            }
        }
        px
    }
}

/// Pixel row where the sky gradient reaches the horizon color.
pub(crate) fn horizon_row(h: u32, pitch: f32) -> f32 {
    (h as f32 * (0.5 + pitch)).max(1.0)
}
