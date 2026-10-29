// SPDX-License-Identifier: GPL-3.0-or-later
//! wgpu (Vulkan) device and the egui renderer that draws into any texture view.

use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

pub fn instance() -> wgpu::Instance {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    wgpu::Instance::new(desc)
}

impl Gpu {
    pub fn new(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Self, String> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: surface,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("no Vulkan adapter: {e}"))?;
        let limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rynk"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            experimental_features: Default::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: Default::default(),
        }))
        .map_err(|e| format!("no Vulkan device: {e}"))?;
        Ok(Self { instance, adapter, device, queue })
    }
}

/// An egui context plus its GPU renderer for one output format
pub struct Canvas {
    pub ctx: egui::Context,
    renderer: Renderer,
}

impl Canvas {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let renderer = Renderer::new(&gpu.device, format, RendererOptions { dithering: true, ..Default::default() });
        Self { ctx: egui::Context::default(), renderer }
    }

    /// Runs one egui frame. `size` in px, `ppp` = pixels per point.
    pub fn run(&mut self, size: [u32; 2], ppp: f32, events: Vec<egui::Event>, time: f64, ui: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp))),
            events,
            time: Some(time),
            focused: true,
            max_texture_side: Some(4096),
            ..Default::default()
        };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
        self.ctx.run_ui(input, ui)
    }

    /// Tessellates and draws a frame into `view`
    pub fn draw(&mut self, gpu: &Gpu, view: &wgpu::TextureView, size: [u32; 2], out: egui::FullOutput, clear: egui::Color32) {
        let ppp = out.pixels_per_point;
        let prims = self.ctx.tessellate(out.shapes, ppp);
        let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
        for (id, deltas) in &out.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
        let extra = self.renderer.update_buffers(&gpu.device, &gpu.queue, &mut enc, &prims, &screen);
        {
            let [r, g, b, a] = clear.to_normalized_gamma_f32();
            let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: r as f64, g: g as f64, b: b as f64, a: a as f64 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            self.renderer.render(&mut pass, &prims, &screen);
        }
        gpu.queue.submit(extra.into_iter().chain(std::iter::once(enc.finish())));
        for id in &out.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}

/// Prefers a non-sRGB format: egui blends in gamma space
pub fn pick_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    formats
        .iter()
        .copied()
        .find(|f| matches!(f, wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm))
        .or_else(|| formats.iter().copied().find(|f| !f.is_srgb()))
        .unwrap_or(formats[0])
}
