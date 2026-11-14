use crate::map::{Tile, View};
use eframe::{
    egui::{self, Pos2, Rect},
    egui_wgpu::{self, RenderState, wgpu},
};
use wgpu::util::DeviceExt;

const SECONDS: f64 = 0.22;
pub const FULL_UV: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.));

pub struct State<T> {
    pub current: Option<T>,
    previous: Option<(T, f64)>,
}

impl<T> Default for State<T> {
    fn default() -> Self {
        Self {
            current: None,
            previous: None,
        }
    }
}

impl<T: PartialEq> State<T> {
    pub fn update(&mut self, target: T, ready: bool, fade: bool, now: f64) {
        if self
            .previous
            .as_ref()
            .is_some_and(|(_, start)| now >= start + SECONDS)
        {
            self.previous = None;
        }
        if self.previous.is_none() && ready && self.current.as_ref() != Some(&target) {
            let previous = self.current.replace(target);
            if fade {
                self.previous = previous.map(|previous| (previous, now));
            }
        }
    }

    pub fn blend(&self, ctx: &egui::Context, now: f64) -> Option<(&T, f32)> {
        let (previous, start) = self.previous.as_ref()?;
        ctx.request_repaint();
        let t = ((now - start) / SECONDS).clamp(0., 1.) as f32;
        Some((previous, t * t * (3. - 2. * t)))
    }
}

// 大きく引いたときは、表示範囲を旧レベルの細かさで全面展開しない。
pub fn can_blend(old: u32, new: u32, view_zoom: f64) -> bool {
    old.max(new) <= view_zoom.floor() as u32 + 2
}

pub fn sample(tile: Tile, z: u32, uv: Rect) -> (Tile, Rect) {
    let shift = tile.z - z;
    let source = Tile {
        z,
        x: tile.x >> shift,
        y: tile.y >> shift,
    };
    let scale = (1u32 << shift) as f32;
    let offset = egui::vec2(
        (tile.x - (source.x << shift)) as f32,
        (tile.y - (source.y << shift)) as f32,
    );
    let min = uv.min + offset * uv.size() / scale;
    (source, Rect::from_min_size(min, uv.size() / scale))
}

pub type Backend<'a> = Option<(&'a RenderState, &'a Renderer)>;

pub struct Quad {
    pub tile: Tile,
    pub uv: [Rect; 2],
    pub textures: [Option<wgpu::TextureView>; 2],
}

impl Quad {
    pub fn new(
        tile: Tile,
        mut uv: [Rect; 2],
        mut textures: [Option<wgpu::TextureView>; 2],
        ready: [bool; 2],
    ) -> Option<Self> {
        if textures.iter().all(Option::is_none) {
            return None;
        }
        if !ready[0] {
            uv[0] = uv[1];
            textures[0] = textures[1].clone();
        } else if !ready[1] {
            uv[1] = uv[0];
            textures[1] = textures[0].clone();
        }
        Some(Self { tile, uv, textures })
    }
}

#[derive(Default)]
struct TextureViews {
    frame: u64,
    entries: std::collections::HashMap<wgpu::Texture, (wgpu::TextureView, u64)>,
}
pub struct Renderer {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    sampler_group: wgpu::BindGroup,
    empty: wgpu::TextureView,
    color: wgpu::RenderPipeline,
    darken: wgpu::RenderPipeline,
    brighten: wgpu::RenderPipeline,
    gamma_shade: bool,
    views: std::sync::Mutex<TextureViews>,
}

impl Renderer {
    pub fn new(state: &RenderState) -> Self {
        Self::for_target(&state.device, state.target_format)
    }

    fn for_target(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LOD crossfade"),
            entries: &[texture(0), texture(1)],
        });
        // D3D12の共有サンプラー領域は2048個。タイルごとに複製せず全描画で一つ使う。
        let sampler_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("共通の画像サンプラー"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let sampler_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("全タイル共通の画像サンプラー"),
            layout: &sampler_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Sampler(&sampler),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout, &sampler_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("LOD crossfade"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lod_fade.wgsl").into()),
        });
        let pipeline = |entry, src_factor, dst_factor, shade| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 28,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32],
                    }],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent { src_factor, dst_factor, operation: wgpu::BlendOperation::Add },
                            alpha: wgpu::BlendComponent::OVER,
                        }),
                        write_mask: if shade { wgpu::ColorWrites::COLOR } else { wgpu::ColorWrites::ALL },
                    })],
                }),
                multiview: None,
                cache: None,
            })
        };
        let color = pipeline(
            if format.is_srgb() {
                "linear_color"
            } else {
                "gamma_color"
            },
            wgpu::BlendFactor::One,
            wgpu::BlendFactor::OneMinusSrcAlpha,
            false,
        );
        let darken = pipeline(
            "darken",
            wgpu::BlendFactor::Zero,
            wgpu::BlendFactor::OneMinusSrc,
            true,
        );
        let brighten = pipeline(
            "brighten",
            wgpu::BlendFactor::Dst,
            wgpu::BlendFactor::One,
            true,
        );
        let empty = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("透明色・中立陰影"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self {
            device: device.clone(),
            layout,
            sampler_group,
            empty,
            gamma_shade: !format.is_srgb(),
            views: Default::default(),
            color,
            darken,
            brighten,
        }
    }

    pub fn color_view(
        &self,
        renderer: &egui_wgpu::Renderer,
        id: egui::TextureId,
        frame: u64,
    ) -> Option<wgpu::TextureView> {
        let texture = renderer.texture(&id)?.texture.as_ref()?;
        let mut cache = self.views.lock().unwrap();
        if cache.frame != frame {
            cache
                .entries
                .retain(|_, (_, used)| *used >= frame.saturating_sub(2));
            cache.frame = frame;
        }
        let entry = cache
            .entries
            .entry(texture.clone())
            .or_insert_with(|| (texture.create_view(&Default::default()), frame));
        entry.1 = frame;
        Some(entry.0.clone())
    }
    pub fn paint(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        view: &View,
        color: Vec<Quad>,
        shades: Vec<Quad>,
        amount: f32,
        direct: bool,
    ) {
        if direct {
            for (quad, shade) in shades
                .into_iter()
                .map(|q| (q, true))
                .chain(color.into_iter().map(|q| (q, false)))
            {
                yamamuki_terrain_view::direct_material::capture(
                    painter,
                    yamamuki_terrain_view::direct_material::Capture {
                        rect: Rect::from_points(&view.tile_corners(rect, quad.tile)),
                        uv: quad.uv,
                        textures: quad.textures,
                        amount,
                        shade,
                        gamma_shade: shade && self.gamma_shade,
                    },
                );
            }
            return;
        }
        if let Some(draw) = self.draw(rect, view, color, shades, amount) {
            painter.add(egui_wgpu::Callback::new_paint_callback(rect, draw));
        }
    }

    fn draw(
        &self,
        rect: Rect,
        view: &View,
        color: Vec<Quad>,
        shades: Vec<Quad>,
        amount: f32,
    ) -> Option<Draw> {
        let shade_count = shades.len();
        let mut textures = Vec::new();
        let mut vertices = Vec::new();
        for quad in shades.into_iter().chain(color) {
            let corners = view.tile_corners(rect, quad.tile);
            let uv = quad.uv.map(|uv| {
                [
                    uv.left_top(),
                    uv.right_top(),
                    uv.right_bottom(),
                    uv.left_bottom(),
                ]
            });
            for i in [0, 1, 2, 0, 2, 3] {
                let x = 2. * (corners[i].x - rect.left()) / rect.width() - 1.;
                let y = 1. - 2. * (corners[i].y - rect.top()) / rect.height();
                vertices.extend(
                    [x, y, uv[0][i].x, uv[0][i].y, uv[1][i].x, uv[1][i].y, amount]
                        .into_iter()
                        .flat_map(f32::to_le_bytes),
                );
            }
            let [old, new] = quad.textures;
            textures.push(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            old.as_ref().unwrap_or(&self.empty),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            new.as_ref().unwrap_or(&self.empty),
                        ),
                    },
                ],
            }));
        }
        if textures.is_empty() {
            return None;
        }
        let vertices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &vertices,
                usage: wgpu::BufferUsages::VERTEX,
            });
        Some(Draw {
            sampler_group: self.sampler_group.clone(),
            vertices,
            textures,
            shade_count,
            color: self.color.clone(),
            darken: self.darken.clone(),
            brighten: self.brighten.clone(),
        })
    }
}

struct Draw {
    sampler_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    textures: Vec<wgpu::BindGroup>,
    shade_count: usize,
    color: wgpu::RenderPipeline,
    darken: wgpu::RenderPipeline,
    brighten: wgpu::RenderPipeline,
}

impl egui_wgpu::CallbackTrait for Draw {
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _: &egui_wgpu::CallbackResources,
    ) {
        self.render(pass);
    }
}

impl Draw {
    fn render(&self, pass: &mut wgpu::RenderPass<'static>) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_bind_group(1, &self.sampler_group, &[]);
        for (pipeline, range) in [
            (&self.darken, 0..self.shade_count),
            (&self.brighten, 0..self.shade_count),
            (&self.color, self.shade_count..self.textures.len()),
        ] {
            if range.is_empty() {
                continue;
            }
            pass.set_pipeline(pipeline);
            for i in range {
                pass.set_bind_group(0, &self.textures[i], &[]);
                let start = i as u32 * 6;
                pass.draw(start..start + 6, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_previous_until_target_is_ready_and_fades_once() {
        let mut state = State::default();
        state.update(9, true, true, 0.);
        state.update(10, false, true, 1.);
        assert_eq!(state.current, Some(9));
        assert!(state.previous.is_none());
        state.update(10, true, true, 2.);
        state.update(11, true, true, 2.1);
        assert_eq!(state.current, Some(10));
        assert_eq!(state.previous, Some((9, 2.)));
        state.update(11, true, true, 3.);
        assert_eq!(state.current, Some(11));
        assert_eq!(state.previous, Some((10, 3.)));
    }

    #[test]
    fn aligns_the_finer_tile_to_the_same_area_in_its_parent() {
        let tile = Tile { z: 4, x: 5, y: 6 };
        let (parent, uv) = sample(tile, 3, FULL_UV);
        assert_eq!(parent, Tile { z: 3, x: 2, y: 3 });
        assert_eq!(
            uv,
            Rect::from_min_max(egui::pos2(0.5, 0.), egui::pos2(1., 0.5))
        );
    }

    #[test]
    #[ignore = "GPUの実描画で色と陰影の補間を確認する"]
    fn gpu_crossfade_keeps_opacity_and_neutral_shade() {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let renderer = Renderer::for_target(&device, wgpu::TextureFormat::Rgba8Unorm);
        let size = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = |bytes: [u8; 4], format| {
            device
                .create_texture_with_data(
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: None,
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &bytes,
                )
                .create_view(&Default::default())
        };
        let quad = |colors: [[u8; 4]; 2], format| Quad {
            tile: Tile { z: 0, x: 0, y: 0 },
            uv: [FULL_UV; 2],
            textures: colors.map(|bytes| Some(texture(bytes, format))),
        };
        let color = quad(
            [[128, 0, 0, 128], [0, 128, 0, 128]],
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let shade = quad(
            [[128, 0, 0, 0], [0, 128, 0, 0]],
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let view = View {
            center: [0.5; 2],
            zoom: 0.,
            orientation: Default::default(),
        };
        let draw = renderer
            .draw(FULL_UV, &view, vec![color], vec![shade], 0.5)
            .unwrap();
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
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
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.create_view(&Default::default()),
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.,
                                g: 0.,
                                b: 0.5,
                                a: 1.,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            draw.render(&mut pass);
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
        let actual = &bytes[..4];
        assert!(
            actual
                .iter()
                .zip([64, 64, 64, 255])
                .all(|(a, b)| a.abs_diff(b) <= 1),
            "{actual:?}"
        );
    }
}
