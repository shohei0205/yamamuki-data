//! 描画済みの地図の色に、陰影の倍率を適用する。
use eframe::{
    egui::{self, ColorImage, Pos2, Rect},
    egui_wgpu::{self, RenderState, wgpu},
};
use wgpu::util::DeviceExt;

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    darken: wgpu::RenderPipeline,
    brighten: wgpu::RenderPipeline,
    srgb_target: bool,
}

impl Renderer {
    pub fn new(state: &RenderState) -> Self {
        Self::for_target(&state.device, &state.queue, state.target_format)
    }

    fn for_target(device: &wgpu::Device, queue: &wgpu::Queue, target: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map hillshade"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map hillshade"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map hillshade"),
            source: wgpu::ShaderSource::Wgsl(include_str!("surface_shade.wgsl").into()),
        });
        let pipeline = |entry, src_factor, dst_factor| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 16,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                    }],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor,
                                dst_factor,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::Zero,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                multiview: None,
                cache: None,
            })
        };
        Self {
            device: device.clone(),
            queue: queue.clone(),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            // 暗部は dst * (1 - src)、明部は dst * (1 + src) とする。
            darken: pipeline(
                "darken",
                wgpu::BlendFactor::Zero,
                wgpu::BlendFactor::OneMinusSrc,
            ),
            brighten: pipeline("brighten", wgpu::BlendFactor::Dst, wgpu::BlendFactor::One),
            layout,
            srgb_target: target.is_srgb(),
        }
    }

    pub fn upload(&self, image: &ColorImage, neutral: u8, strength: f32) -> wgpu::BindGroup {
        let bytes: Vec<_> = image
            .pixels
            .iter()
            .flat_map(|color| {
                if color.a() == 0 {
                    return [0; 2];
                }
                let mut gain =
                    (1. + strength * (f32::from(color.r()) / f32::from(neutral) - 1.)).max(0.);
                if self.srgb_target {
                    // sRGBの描画先ではブレンドが線形色空間で行われる。
                    gain = gain.powf(2.2);
                }
                [(1. - gain).max(0.), (gain - 1.).max(0.)]
                    .map(|amount| (amount * 255.).round() as u8)
            })
            .collect();
        let size = wgpu::Extent3d {
            width: image.size[0] as u32,
            height: image.size[1] as u32,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hillshade multipliers"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width * 2),
                rows_per_image: None,
            },
            size,
        );
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hillshade tile"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    pub fn paint(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        tiles: Vec<([Pos2; 4], wgpu::BindGroup)>,
    ) {
        if tiles.is_empty() {
            return;
        }
        painter.add(egui_wgpu::Callback::new_paint_callback(
            rect,
            self.draw(rect, tiles),
        ));
    }

    fn draw(&self, rect: Rect, tiles: Vec<([Pos2; 4], wgpu::BindGroup)>) -> Draw {
        let mut vertices = Vec::with_capacity(tiles.len() * 96);
        let mut textures = Vec::with_capacity(tiles.len());
        for (corners, texture) in tiles {
            let uv = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
            for i in [0, 1, 2, 0, 2, 3] {
                let x = 2. * (corners[i].x - rect.left()) / rect.width() - 1.;
                let y = 1. - 2. * (corners[i].y - rect.top()) / rect.height();
                vertices.extend(
                    [x, y, uv[i][0], uv[i][1]]
                        .into_iter()
                        .flat_map(f32::to_le_bytes),
                );
            }
            textures.push(texture);
        }
        let vertices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("hillshade map quads"),
                contents: &vertices,
                usage: wgpu::BufferUsages::VERTEX,
            });
        Draw {
            vertices,
            textures,
            darken: self.darken.clone(),
            brighten: self.brighten.clone(),
        }
    }
}

struct Draw {
    vertices: wgpu::Buffer,
    textures: Vec<wgpu::BindGroup>,
    darken: wgpu::RenderPipeline,
    brighten: wgpu::RenderPipeline,
}

impl egui_wgpu::CallbackTrait for Draw {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        self.draw(pass);
    }
}

impl Draw {
    fn draw(&self, pass: &mut wgpu::RenderPass<'static>) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for pipeline in [&self.darken, &self.brighten] {
            pass.set_pipeline(pipeline);
            for (i, texture) in self.textures.iter().enumerate() {
                pass.set_bind_group(0, texture, &[]);
                let start = i as u32 * 6;
                pass.draw(start..start + 6, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    #[test]
    #[ignore = "GPUの描画先でブレンドを確認する"]
    fn online_map_keeps_its_color_under_hillshade() {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = Renderer::for_target(&device, &queue, format);
        let image = ColorImage {
            size: [4, 1],
            pixels: vec![
                Color32::from_gray(68),
                Color32::from_gray(136),
                Color32::from_gray(204),
                Color32::TRANSPARENT,
            ],
        };
        let shade = renderer.upload(&image, 136, 1.);
        let rect = Rect::from_min_max(Pos2::ZERO, egui::pos2(4., 1.));
        let draw = renderer.draw(
            rect,
            vec![(
                [
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                ],
                shade,
            )],
        );
        let size = wgpu::Extent3d {
            width: 4,
            height: 1,
            depth_or_array_layers: 1,
        };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let view = target.create_view(&Default::default());
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.2,
                                g: 0.4,
                                b: 0.6,
                                a: 1.,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            draw.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            size,
        );
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (send, recv) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
        device.poll(wgpu::Maintain::Wait);
        recv.recv().unwrap().unwrap();
        let bytes = slice.get_mapped_range();
        let expected: [[u8; 4]; 4] = [
            [26, 51, 77, 255],
            [51, 102, 153, 255],
            [77, 153, 230, 255],
            [51, 102, 153, 255],
        ];
        for (actual, expected) in bytes[..16].chunks_exact(4).zip(expected) {
            assert!(
                actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                "{actual:?} != {expected:?}"
            );
        }
    }
}
