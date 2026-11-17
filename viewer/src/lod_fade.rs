use crate::map::{Tile, View};
use eframe::{
    egui::{self, Pos2, Rect},
    egui_wgpu::{self, RenderState, wgpu},
};
use wgpu::util::DeviceExt;
use yamamuki_terrain_view::direct_material::Interpolation;

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
    old.max(new) <= (view_zoom.floor() as u32).min(new).saturating_add(2)
}

#[cfg(test)]
mod transition_budget_tests {
    #[test]
    fn lowering_online_limit_does_not_expand_old_detail_over_coarse_material_regions() {
        // 上限を下げた広域範囲を旧レベルの細かさで全面列挙しない。
        assert!(!super::can_blend(18, 6, 18.));
        assert!(!super::can_blend(12, 6, 18.));
        assert!(super::can_blend(7, 6, 18.));
        assert!(super::can_blend(11, 12, 12.));
        assert!(!super::can_blend(18, 12, 6.));
    }
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
    pub edge_filter: Interpolation,
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
        Some(Self {
            tile,
            uv,
            textures,
            edge_filter: Interpolation::Bilinear,
        })
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
    sampler_layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    programs: std::sync::OnceLock<Programs>,
    gamma_shade: bool,
    views: std::sync::Mutex<TextureViews>,
}

struct Programs {
    color: wgpu::RenderPipeline,
    darken: wgpu::RenderPipeline,
    brighten: wgpu::RenderPipeline,
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
            sampler_layout,
            format,
            programs: Default::default(),
        }
    }

    // 元タイルを地形へ渡すだけなら、この2Dプログラムは不要。
    fn programs(&self) -> &Programs {
        self.programs.get_or_init(|| {
            let device=&self.device;
            let layout=&self.layout;
            let sampler_layout=&self.sampler_layout;
            let format=self.format;
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[layout, sampler_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("LOD crossfade"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("lod_fade.wgsl"),
                    include_str!("../terrain/src/edge_interpolation.wgsl")
                )
                .into(),
            ),
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
                        array_stride: 32,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32, 4 => Float32],
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
        let (color, darken, brighten) = std::thread::scope(|scope| {
            let color = scope.spawn(|| {
                pipeline(
                    if format.is_srgb() {
                        "linear_color"
                    } else {
                        "gamma_color"
                    },
                    wgpu::BlendFactor::One,
                    wgpu::BlendFactor::OneMinusSrcAlpha,
                    false,
                )
            });
            let darken = scope.spawn(|| {
                pipeline(
                    "darken",
                    wgpu::BlendFactor::Zero,
                    wgpu::BlendFactor::OneMinusSrc,
                    true,
                )
            });
            let brighten = scope.spawn(|| {
                pipeline(
                    "brighten",
                    wgpu::BlendFactor::Dst,
                    wgpu::BlendFactor::One,
                    true,
                )
            });
            (
                color.join().unwrap(),
                darken.join().unwrap(),
                brighten.join().unwrap(),
            )
        });

            Programs {color,darken,brighten}
        })
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
                        edge_filter: quad.edge_filter,
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
                    [
                        x,
                        y,
                        uv[0][i].x,
                        uv[0][i].y,
                        uv[1][i].x,
                        uv[1][i].y,
                        amount,
                        quad.edge_filter as u32 as f32,
                    ]
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
            color: self.programs().color.clone(),
            darken: self.programs().darken.clone(),
            brighten: self.programs().brighten.clone(),
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
    fn recording_native_tiles_preserves_commands_without_compiling_2d_programs() {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU");
        let (device, _) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let renderer = Renderer::for_target(&device, wgpu::TextureFormat::Rgba8Unorm);
        assert!(renderer.programs.get().is_none());
        let texture = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("受渡しだけを行う元タイル"),
                size: wgpu::Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let ctx = egui::Context::default();
        let output = ctx.run(Default::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            renderer.paint(
                &painter,
                Rect::from_min_size(Pos2::ZERO, egui::vec2(256., 256.)),
                &View {
                    center: [0.5, 0.5],
                    zoom: 0.,
                    orientation: Default::default(),
                },
                vec![Quad {
                    tile: Tile { z: 0, x: 0, y: 0 },
                    uv: [FULL_UV; 2],
                    textures: [Some(texture.clone()), Some(texture.clone())],
                    edge_filter: Interpolation::Edge,
                }],
                Vec::new(),
                1.,
                true,
            );
        });
        let commands = output
            .shapes
            .iter()
            .filter_map(|s| {
                let egui::Shape::Callback(c) = &s.shape else {
                    return None;
                };
                c.callback
                    .downcast_ref::<yamamuki_terrain_view::direct_material::Capture>()
            })
            .collect::<Vec<_>>();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].edge_filter, Interpolation::Edge);
        assert_eq!(commands[0].textures, [Some(texture.clone()), Some(texture)]);
        assert!(
            renderer.programs.get().is_none(),
            "元タイルの受渡しで未使用の2Dプログラムを作成している"
        );
    }
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
            edge_filter: Interpolation::Bilinear,
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
    #[test]
    #[ignore = "輪郭補間を実GPUで描き、斜線・縮小・分割境界を確認する"]
    fn gpu_edge_interpolation_keeps_diagonals_and_split_tile_seams() {
        check_gpu_interpolation(false);
    }
    #[test]
    #[ignore = "実GPUで連続階調の拡大時に画素境界の段差が生じないことを確認する"]
    fn gpu_continuous_tones_do_not_acquire_pixel_grid_steps() {
        check_gpu_interpolation(true);
    }
    #[test]
    #[ignore = "傾きが変わる単調グラデーションを実GPUで拡大して逆転を確認する"]
    fn gpu_curved_gradient_keeps_tone_order() {
        check_gpu_interpolation_profile(true, true);
    }
    #[test]
    #[ignore = "異なる色の面に接する境界線をGPUで拡大し、階段状の明暗変動を検査する"]
    fn gpu_colored_coastline_keeps_the_edge_connected() {
        check_gpu_interpolation_image(false, false, 1);
    }
    #[test]
    #[ignore = "輝度が等しい色同士の境界でも方向を復元できることをGPUで検査する"]
    fn gpu_equal_luminance_boundary_uses_color_direction() {
        check_gpu_interpolation_image(false, false, 2);
    }
    fn check_gpu_interpolation(gradient: bool) {
        check_gpu_interpolation_profile(gradient, false);
    }
    fn check_gpu_interpolation_profile(gradient: bool, curved: bool) {
        check_gpu_interpolation_image(gradient, curved, 0);
    }
    fn check_gpu_interpolation_image(gradient: bool, curved: bool, boundary: u8) {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU");
        let bench_frames = std::env::var("YAMAMUKI_FILTER_BENCH_FRAMES")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
            .clamp(1, 64);
        let timestamp =
            bench_frames > 1 && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                required_features: if timestamp {
                    wgpu::Features::TIMESTAMP_QUERY
                } else {
                    wgpu::Features::empty()
                },
                ..Default::default()
            },
            None,
        ))
        .unwrap();
        let renderer = Renderer::for_target(&device, wgpu::TextureFormat::Rgba8Unorm);
        let preview = std::env::var_os("YAMAMUKI_FILTER_PREVIEW").map(std::path::PathBuf::from);
        let input = preview.as_ref().map(|p| image::open(p).unwrap().to_rgba8());
        // 色の面境界の検査では、描画する8画素範囲の外側にも同じ解析形状を生成する。
        // 実タイルと同じ隣接3画素を持たせ、テクスチャ端のクランプを輪郭と混同しない。
        let analytic_gutter: i32 = if boundary == 0 { 0 } else { 3 };
        let source_side = (8 + 2 * analytic_gutter) as u32;
        let source_size = wgpu::Extent3d {
            width: input.as_ref().map_or(source_side, |i| i.width()),
            height: input.as_ref().map_or(source_side, |i| i.height()),
            depth_or_array_layers: 1,
        };
        let values = (-analytic_gutter..8 + analytic_gutter)
            .flat_map(|y| {
                (-analytic_gutter..8 + analytic_gutter).flat_map(move |x| {
                    let value = if gradient {
                        if curved {
                            [20, 21, 22, 45, 120, 195, 215, 220][x as usize] + y * 3
                        } else {
                            20 + x * 23 + y * 5
                        }
                    } else if x == y {
                        0
                    } else {
                        255
                    };
                    if boundary == 1 {
                        if x == y {
                            [96, 160, 240, 255]
                        } else if y > x {
                            [190, 215, 240, 255]
                        } else {
                            [255, 255, 255, 255]
                        }
                    } else if boundary == 2 {
                        if x > y {
                            [255, 128, 0, 255]
                        } else {
                            [0, 128, 255, 255]
                        }
                    } else {
                        [value as u8, value as u8, value as u8, 255]
                    }
                })
            })
            .collect::<Vec<u8>>();
        let values = input.as_ref().map_or(values, |i| i.as_raw().clone());
        let source = device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("斜め1px線"),
                    size: source_size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &values,
            )
            .create_view(&Default::default());
        let render_region = |edge_filter: Interpolation,
                             split,
                             pixels: u32,
                             sample_uv: Option<Rect>| {
            let height = if preview.is_some() {
                std::env::var("YAMAMUKI_FILTER_PREVIEW_HEIGHT")
                    .ok()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(pixels)
                    .clamp(1, 4096)
            } else {
                pixels
            };
            let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(pixels as f32, height as f32));
            let view = View {
                center: [0.5, 0.5],
                zoom: (pixels as f64 / 256.).log2(),
                orientation: serde_json::from_value(serde_json::json!({"rotation":{"s":0.,"c":1.},"vertical_scale":height as f32 / pixels as f32})).unwrap(),
            };
            let full = sample_uv.unwrap_or_else(|| {
                if preview.is_some() {
                    std::env::var("YAMAMUKI_FILTER_PREVIEW_UV")
                        .ok()
                        .and_then(|v| serde_json::from_str::<[f32; 4]>(&v).ok())
                        .map(|v| Rect::from_min_max(egui::pos2(v[0], v[1]), egui::pos2(v[2], v[3])))
                        .unwrap_or(FULL_UV)
                } else {
                    if boundary == 0 {
                        Rect::from_min_max(egui::pos2(0.125, 0.125), egui::pos2(0.875, 0.875))
                    } else {
                        Rect::from_min_max(
                            egui::pos2(4. / 14., 4. / 14.),
                            egui::pos2(10. / 14., 10. / 14.),
                        )
                    }
                }
            });
            let tiles = if split {
                (0..2)
                    .flat_map(|y| (0..2).map(move |x| Tile { z: 1, x, y }))
                    .collect()
            } else {
                vec![Tile { z: 0, x: 0, y: 0 }]
            };
            let quads = tiles
                .into_iter()
                .map(|tile| {
                    let side = (1u32 << tile.z) as f32;
                    let uv = Rect::from_min_size(
                        full.min + egui::vec2(tile.x as f32, tile.y as f32) * full.size() / side,
                        full.size() / side,
                    );
                    Quad {
                        tile,
                        uv: [uv; 2],
                        textures: [Some(source.clone()), Some(source.clone())],
                        edge_filter,
                    }
                })
                .collect();
            let draw = renderer.draw(rect, &view, quads, Vec::new(), 1.).unwrap();
            let size = wgpu::Extent3d {
                width: pixels,
                height,
                depth_or_array_layers: 1,
            };
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
            let pitch = (pixels * 4).div_ceil(256) * 256;
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(pitch * height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let queries = timestamp.then(|| {
                device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("拡大補間のGPU時間"),
                    ty: wgpu::QueryType::Timestamp,
                    count: bench_frames * 2,
                })
            });
            let query_result = timestamp.then(|| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(bench_frames) * 16,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                })
            });
            let query_read = timestamp.then(|| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: u64::from(bench_frames) * 16,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            for frame_index in 0..bench_frames {
                let mut pass = encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target.create_view(&Default::default()),
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        timestamp_writes: queries.as_ref().map(|q| {
                            wgpu::RenderPassTimestampWrites {
                                query_set: q,
                                beginning_of_pass_write_index: Some(frame_index * 2),
                                end_of_pass_write_index: Some(frame_index * 2 + 1),
                            }
                        }),
                        ..Default::default()
                    })
                    .forget_lifetime();
                draw.render(&mut pass);
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(pitch),
                        rows_per_image: None,
                    },
                },
                size,
            );
            if let (Some(q), Some(result), Some(read)) = (&queries, &query_result, &query_read) {
                encoder.resolve_query_set(q, 0..bench_frames * 2, result, 0);
                encoder.copy_buffer_to_buffer(result, 0, read, 0, u64::from(bench_frames) * 16);
            }
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            read.slice(..)
                .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
            device.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().unwrap();
            if let Some(read) = &query_read {
                let (tx, rx) = std::sync::mpsc::channel();
                read.slice(..)
                    .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
                device.poll(wgpu::Maintain::Wait);
                rx.recv().unwrap().unwrap();
                let bytes = read.slice(..).get_mapped_range();
                let times = bytes
                    .chunks_exact(16)
                    .map(|p| {
                        let a = u64::from_le_bytes(p[..8].try_into().unwrap());
                        let b = u64::from_le_bytes(p[8..].try_into().unwrap());
                        (b - a) as f64 * f64::from(queue.get_timestamp_period()) / 1e6
                    })
                    .collect::<Vec<_>>();
                let skip = usize::from(times.len() > 4) * 2;
                let ms = times[skip..].iter().sum::<f64>() / (times.len() - skip) as f64;
                eprintln!("拡大GPU: mode={edge_filter:?} {pixels}x{height} {ms:.3}ms");
                if let Some(input) = &preview {
                    std::fs::write(input.with_file_name(if edge_filter == Interpolation::Edge {"timing-edge.json"}else{"timing-bilinear.json"}),
                        serde_json::to_vec_pretty(&serde_json::json!({"adapter":adapter.get_info().name,"width":pixels,"height":height,"edge":edge_filter,"gpu_ms":ms,"frames":times})).unwrap()).unwrap();
                }
            }
            let bytes = read.slice(..).get_mapped_range();
            bytes
                .chunks_exact(pitch as usize)
                .flat_map(|row| row[..pixels as usize * 4].to_vec())
                .collect::<Vec<_>>()
        };
        let render = |mode, split, pixels| render_region(mode, split, pixels, None);
        if let Some(input) = preview.as_ref() {
            let pixels = std::env::var("YAMAMUKI_FILTER_PREVIEW_PIXELS")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(1024)
                .clamp(128, 4096);
            for (name, enabled) in [("bilinear", false), ("edge", true)] {
                let bytes = render(enabled.into(), false, pixels);
                let height = std::env::var("YAMAMUKI_FILTER_PREVIEW_HEIGHT")
                    .ok()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(pixels);
                image::RgbaImage::from_raw(pixels, height, bytes)
                    .unwrap()
                    .save(input.with_file_name(format!("preview-{name}.png")))
                    .unwrap();
            }
            return;
        }
        if boundary != 0 {
            let ordinary = render(Interpolation::Bilinear, false, 128);
            let edge = render(Interpolation::Edge, false, 128);
            let split = render(Interpolation::Edge, true, 128);
            assert!(
                edge.iter().zip(&split).all(|(a, b)| a.abs_diff(*b) <= 1),
                "色の境界がタイル分割で変わる"
            );
            assert!(edge.chunks_exact(4).all(|p| p[3] == 255));
            let shift = if boundary == 2 { 11 } else { 0 };
            let range = |pixels: &[u8]| {
                let values = (24usize..92)
                    .map(|y| {
                        let at = (y * 128 + y + shift) * 4;
                        i16::from(pixels[at + 2]) - i16::from(pixels[at])
                    })
                    .collect::<Vec<_>>();
                values.iter().max().unwrap() - values.iter().min().unwrap()
            };
            let old = range(&ordinary);
            let new = range(&edge);
            eprintln!("色の面境界の明暗振幅: 種類={boundary}, 輪郭={new}, バイリニア={old}");
            assert!(
                new * 4 < old * 3,
                "色の境界の階段状の変動が十分減っていない: {new}/{old}"
            );
            if boundary == 1 {
                let mean = |pixels: &[u8]| {
                    (24usize..92)
                        .map(|y| {
                            let at = (y * 128 + y) * 4;
                            f32::from(pixels[at + 2]) - f32::from(pixels[at])
                        })
                        .sum::<f32>()
                        / 68.
                };
                assert!(
                    mean(&edge) > mean(&ordinary) * 0.6,
                    "境界線を消して階段感を隠してはならない"
                );
            }
            return;
        }
        if gradient {
            let pixels = if curved { 512 } else { 128 };
            let enlarged = render(Interpolation::Edge, false, pixels);
            let split = render(Interpolation::Edge, true, pixels);
            assert!(
                enlarged
                    .iter()
                    .zip(&split)
                    .all(|(a, b)| a.abs_diff(*b) <= 1)
            );
            for row in enlarged.chunks_exact(pixels as usize * 4) {
                for pair in row.chunks_exact(4).collect::<Vec<_>>().windows(2) {
                    assert!(
                        pair[1][0] >= pair[0][0],
                        "濃淡の順序が逆転している: {:?} -> {:?}",
                        pair[0],
                        pair[1]
                    );
                    assert!(
                        pair[1][0].abs_diff(pair[0][0]) <= 3,
                        "拡大画像に画素境界の段差が生じた: {:?} -> {:?}",
                        pair[0],
                        pair[1]
                    );
                }
            }
            assert!(enlarged.chunks_exact(4).all(|p| p[3] == 255));
            if curved {
                let bilinear = render(Interpolation::Bilinear, false, pixels);
                let difference = enlarged
                    .iter()
                    .zip(&bilinear)
                    .map(|(a, b)| u64::from(a.abs_diff(*b)))
                    .sum::<u64>();
                assert!(
                    difference > u64::from(pixels) * u64::from(pixels),
                    "単調領域がバイリニアへ退避している"
                );
            }
            return;
        }
        // 窓の参加画素が替わる位置と、元画素の区画境界を横切る極細の領域を描く。
        // ほぼ同一座標の左右で色が跳べば、拡大時には四角いブロックとして見える。
        for boundary in [0.5, 0.5625] {
            let uv = Rect::from_min_max(
                egui::pos2(boundary - 1e-5, 0.125),
                egui::pos2(boundary + 1e-5, 0.875),
            );
            let seam = render_region(Interpolation::Edge, false, 128, Some(uv));
            for row in seam.chunks_exact(128 * 4) {
                assert!(
                    row[..4]
                        .iter()
                        .zip(&row[127 * 4..])
                        .all(|(a, b)| a.abs_diff(*b) <= 1),
                    "元画素の境界で色が跳んでいる"
                );
            }
        }
        let nearest = render(Interpolation::Nearest, false, 128);
        assert_eq!(
            nearest,
            render(Interpolation::Nearest, true, 128),
            "最近傍がタイル境界で変わる"
        );
        assert!(
            nearest
                .chunks_exact(4)
                .all(|p| (p[0] == 0 || p[0] == 255) && p[0] == p[1] && p[1] == p[2] && p[3] == 255),
            "最近傍で元画素以外の中間色が作られた"
        );
        let ordinary = render(Interpolation::Bilinear, false, 128);
        let edge = render(Interpolation::Edge, false, 128);
        let split = render(Interpolation::Edge, true, 128);
        assert!(
            edge.iter().zip(&split).all(|(a, b)| a.abs_diff(*b) <= 1),
            "分割境界で補間が変わる"
        );
        assert!(edge.chunks_exact(4).all(|p| p[3] == 255));
        // 1pxの斜線は元画素中心の間でも色が大きく変わらず、一本につながる必要がある。
        let center_range = |image: &[u8]| {
            let values = (24usize..104)
                .map(|i| image[(i * 128 + i) * 4])
                .collect::<Vec<_>>();
            u16::from(*values.iter().max().unwrap()) - u16::from(*values.iter().min().unwrap())
        };
        let edge_range = center_range(&edge);
        let bilinear_range = center_range(&ordinary);
        eprintln!("斜線中心の明暗振幅: 輪郭={edge_range}, バイリニア={bilinear_range}");
        assert!(
            edge_range < bilinear_range,
            "斜線中心の明暗の揺れが改善していない"
        );
        let dark = |image: &[u8]| image.chunks_exact(4).filter(|p| p[0] < 64).count();
        assert!(
            dark(&edge) > dark(&ordinary),
            "斜め線の連結が改善していない"
        );
        assert_eq!(
            render(Interpolation::Bilinear, false, 4),
            render(Interpolation::Edge, false, 4),
            "縮小画像を変えてはならない"
        );
    }
}
