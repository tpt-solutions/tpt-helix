//! Task: Build display-list -> `wgpu` GPU command buffer -> present pipeline.
//!
//! Rasterizes a [`crate::display_list::DisplayItem`] list with `wgpu`: each
//! rect becomes two triangles in a single vertex buffer, drawn in one render
//! pass. There's no on-screen surface here (this crate has no window yet —
//! that's [`crate::html`]'s neighbor, the AppFront crate's job); "present"
//! means resolving the render target to a readable RGBA8 buffer, the same
//! shape of output a windowed presenter would hand to `wgpu::Surface`.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::display_list::DisplayItem;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

const SHADER: &str = r#"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) color: vec4<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

/// An offscreen `wgpu` context able to rasterize a display list to RGBA8.
///
/// Headless (no `wgpu::Surface`): the render target is a plain texture that
/// gets copied out to a CPU buffer, since this crate doesn't own a window.
pub struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
}

impl GpuContext {
    /// Requests a `wgpu` adapter/device (any backend the host supports) and
    /// builds the single render pipeline this rasterizer uses.
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .ok()?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("display-list shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("display-list pipeline layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let target_format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("display-list pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Some(GpuContext {
            device,
            queue,
            pipeline,
        })
    }

    fn vertices_for(items: &[DisplayItem], width: f32, height: f32) -> Vec<Vertex> {
        let to_ndc =
            |x: f32, y: f32| -> [f32; 2] { [(x / width) * 2.0 - 1.0, 1.0 - (y / height) * 2.0] };
        let mut vertices = Vec::with_capacity(items.len() * 6);
        for item in items {
            let color = [item.color.r, item.color.g, item.color.b, item.color.a];
            let top_left = to_ndc(item.x, item.y);
            let top_right = to_ndc(item.x + item.width, item.y);
            let bottom_left = to_ndc(item.x, item.y + item.height);
            let bottom_right = to_ndc(item.x + item.width, item.y + item.height);
            let quad = [
                top_left,
                top_right,
                bottom_left,
                bottom_left,
                top_right,
                bottom_right,
            ];
            vertices.extend(quad.into_iter().map(|position| Vertex { position, color }));
        }
        vertices
    }

    /// Renders `items` (viewport-space pixels, per [`crate::display_list`])
    /// against a `width`x`height` transparent-black target and reads the
    /// result back as tightly-packed RGBA8 rows — the "present" step for a
    /// headless (no `wgpu::Surface`) pipeline.
    pub fn render_to_rgba8(&self, items: &[DisplayItem], width: u32, height: u32) -> Vec<u8> {
        let vertices = Self::vertices_for(items, width as f32, height as f32);

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("display-list target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("display-list encoder"),
            });

        if !vertices.is_empty() {
            let vertex_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("display-list vertices"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("display-list pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.draw(0..vertices.len() as u32, 0..1);
        } else {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("display-list pass (empty)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        // Row byte alignment `wgpu` requires for buffer<->texture copies.
        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("display-list readback"),
            size: (padded_bytes_per_row * height) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));

        let slice = output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().expect("failed to map readback buffer");

        let padded = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
        for row in padded.chunks(padded_bytes_per_row as usize) {
            pixels.extend_from_slice(&row[..unpadded_bytes_per_row as usize]);
        }
        drop(padded);
        output_buffer.unmap();
        pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_list::Color;

    #[test]
    fn renders_a_solid_rect() {
        let Some(gpu) = GpuContext::new() else {
            eprintln!("skipping: no wgpu adapter available in this environment");
            return;
        };
        let items = vec![DisplayItem {
            x: 2.0,
            y: 2.0,
            width: 4.0,
            height: 4.0,
            color: Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        }];
        let pixels = gpu.render_to_rgba8(&items, 8, 8);
        assert_eq!(pixels.len(), 8 * 8 * 4);

        let idx = ((4 * 8 + 4) * 4) as usize;
        assert_eq!(&pixels[idx..idx + 4], &[255, 0, 0, 255]);

        let corner_idx = 0usize;
        assert_eq!(&pixels[corner_idx..corner_idx + 4], &[0, 0, 0, 0]);
    }
}
