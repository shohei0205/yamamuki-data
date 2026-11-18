//! 地形の画面上の地理座標から元タイルを直接参照する。合成用の縮小画像は作らない。
use crate::{selection::Camera, shared_data::WORLD};
use eframe::{egui, wgpu};
use std::sync::Arc;

/// 拡大時の補間方式。GPUのモード値と対応する。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Interpolation {
    #[default]
    Bilinear = 0,
    Edge = 1,
    Nearest = 2,
    Bicubic = 4,
    BSpline = 5,
}
impl Interpolation {
    pub const ALL: [Self; 5] = [
        Self::Nearest,
        Self::Bilinear,
        Self::Bicubic,
        Self::BSpline,
        Self::Edge,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Bilinear => "バイリニア",
            Self::Bicubic => "Bicubic",
            Self::BSpline => "Bスプライン",
            Self::Edge => "輪郭優先",
            Self::Nearest => "最近傍",
        }
    }
}
impl serde::Serialize for Interpolation {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Self::Bilinear => "bilinear",
            Self::Bicubic => "bicubic",
            Self::BSpline => "bspline",
            Self::Edge => "edge",
            Self::Nearest => "nearest",
        })
    }
}
impl From<bool> for Interpolation {
    fn from(value: bool) -> Self {
        if value { Self::Edge } else { Self::Bilinear }
    }
}
// 保存済みのboolと廃止した補間名は、現在の輪郭優先へ引き継ぐ。
impl<'de> serde::Deserialize<'de> for Interpolation {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Legacy(bool),
            Name(String),
        }
        match Saved::deserialize(d)? {
            Saved::Legacy(v) => Ok(v.into()),
            Saved::Name(v) => match v.as_str() {
                "bilinear" => Ok(Self::Bilinear),
                "edge-bilinear" => Ok(Self::Edge),
                "edge-single" => Ok(Self::Edge),
                "bicubic" => Ok(Self::Bicubic),
                "bspline" => Ok(Self::BSpline),
                "edge-geometric" => Ok(Self::Edge),
                // 廃止した比較用モードも通常の輪郭優先へ引き継ぐ。
                "edge" | "edge7" | "edge-multi" | "edge-curve" => Ok(Self::Edge),
                "nearest" => Ok(Self::Nearest),
                "edge-spline" => Ok(Self::Edge),
                _ => Err(serde::de::Error::custom("未対応の補間方式")),
            },
        }
    }
}

/// 合成済みの地表色に対応する、オンライン背景への乗算係数。
#[derive(Clone)]
pub struct SurfaceBlend {
    pub textures: [Option<wgpu::TextureView>; 2],
    pub uv: [egui::Rect; 2],
    pub group: usize,
}
fn texture_key(c: &Quad) -> [Option<wgpu::TextureView>; 4] {
    let gains = c
        .surface
        .as_ref()
        .map_or([None, None], |s| s.textures.clone());
    [
        c.textures[0].clone(),
        c.textures[1].clone(),
        gains[0].clone(),
        gains[1].clone(),
    ]
}

/// 2Dの描画命令から引き継ぐ元テクスチャ。座標は正規化Web Mercator。
#[derive(Clone)]
pub struct Quad {
    pub bounds: [f64; 4],
    pub uv: [egui::Rect; 2],
    pub textures: [Option<wgpu::TextureView>; 2],
    pub amount: f32,
    pub tint: [f32; 4],
    pub shade: bool,
    pub gamma_shade: bool,
    pub opaque: bool,
    pub surface: Option<SurfaceBlend>,
    pub edge_filter: Interpolation,
    pub border: bool,
}
/// 3D用の描画収集時だけ使うマーカー。通常の2D描画へは渡さない。
pub struct Capture {
    pub rect: egui::Rect,
    pub uv: [egui::Rect; 2],
    pub textures: [Option<wgpu::TextureView>; 2],
    pub amount: f32,
    pub shade: bool,
    pub gamma_shade: bool,
    pub opaque: bool,
    pub surface: Option<SurfaceBlend>,
    pub edge_filter: Interpolation,
}
pub fn capture(painter: &egui::Painter, command: Capture) {
    painter.add(egui::Shape::Callback(egui::PaintCallback {
        rect: command.rect,
        callback: Arc::new(command),
    }));
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    area: [f32; 4],
    uv0: [f32; 4],
    uv1: [f32; 4],
    tint: [f32; 4],
    settings: [f32; 4],
    earth: [f32; 4],
    eye: [f32; 4],
    gain_uv0: [f32; 4],
    gain_uv1: [f32; 4],
    visibility: [f32; 4],
}
#[derive(Default)]
struct Cache {
    uniform: Option<wgpu::Buffer>,
    capacity: usize,
    positions: Option<wgpu::TextureView>,
    lighting: Option<(wgpu::Texture, wgpu::TextureView)>,
    groups: std::collections::HashMap<[Option<wgpu::TextureView>; 4], wgpu::BindGroup>,
    groups_created: u64,
    buffers_created: u64,
    screen_bounds: Option<ScreenBounds>,
    coverage: Option<(wgpu::Texture, wgpu::TextureView)>,
    globe_bounds_pose: [u64; 4],
    globe_bounds: std::collections::HashMap<[u64; 4], Option<(glam::DVec3, glam::DVec3)>>,
}
struct ScreenBounds {
    positions: wgpu::TextureView,
    size: [u32; 2],
    capacity: usize,
    meta: wgpu::Buffer,
    compute: wgpu::BindGroup,
    vertex: wgpu::BindGroup,
}
pub(crate) struct Renderer {
    layout: wgpu::BindGroupLayout,
    color: wgpu::RenderPipeline,
    shade: wgpu::RenderPipeline,
    filtered: Option<Arc<std::sync::OnceLock<Result<FilteredPrograms, String>>>>,
    lighting: wgpu::RenderPipeline,
    occlusion: wgpu::RenderPipeline,
    sampler_group: wgpu::BindGroup,
    empty: wgpu::TextureView,
    bounds_compute_layout: wgpu::BindGroupLayout,
    bounds_vertex_layout: wgpu::BindGroupLayout,
    bounds_collect: wgpu::ComputePipeline,
    bounds_reduce: wgpu::ComputePipeline,
    cull: bool,
    cache: std::sync::Mutex<Cache>,
}
struct FilteredPrograms {
    color: wgpu::RenderPipeline,
    shade: wgpu::RenderPipeline,
}
impl Renderer {
    fn pipeline(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        pipeline_layout: &wgpu::PipelineLayout,
        entry: &str,
        color: wgpu::BlendComponent,
        mask: wgpu::ColorWrites,
    ) -> wgpu::RenderPipeline {
        crate::gpu_profile::timed(entry, || {
            let targets = [Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                blend: Some(wgpu::BlendState {
                    color,
                    alpha: wgpu::BlendComponent::OVER,
                }),
                write_mask: mask,
            })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(if entry == "lighting" { "vs" } else { "tile_vs" }),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: Default::default(),
                depth_stencil: if entry == "lighting" {
                    None
                } else {
                    Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: entry == "occlusion",
                        depth_compare: wgpu::CompareFunction::GreaterEqual,
                        stencil: Default::default(),
                        bias: Default::default(),
                    })
                },
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: if entry == "occlusion" { &[] } else { &targets },
                }),
                multiview: None,
                cache: None,
            })
        })
    }
    #[cfg(test)]
    pub fn new(device: &wgpu::Device) -> Self {
        Self::new_impl(device, None)
    }
    pub fn new_background(device: &wgpu::Device, ctx: &egui::Context) -> Self {
        Self::new_impl(device, Some(ctx.clone()))
    }
    pub fn interpolation_pending(&self) -> bool {
        self.filtered.as_ref().is_some_and(|p| p.get().is_none())
    }
    pub fn interpolation_error(&self) -> Option<String> {
        self.filtered.as_ref()?.get()?.as_ref().err().cloned()
    }
    fn new_impl(device: &wgpu::Device, background: Option<egui::Context>) -> Self {
        let texture = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: if binding == 0 {
                wgpu::ShaderStages::VERTEX_FRAGMENT
            } else {
                wgpu::ShaderStages::FRAGMENT
            },
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("地形へ元タイルを直接貼付"),
            entries: &[
                texture(0, false),
                texture(1, true),
                texture(2, true),
                texture(5, true),
                texture(6, false),
                texture(7, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Params>() as u64
                        ),
                    },
                    count: None,
                },
            ],
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
        let shader = crate::gpu_profile::timed("元タイル直接参照", || {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("元タイル直接参照"),
                source: wgpu::ShaderSource::Wgsl(
                    format!(
                        "{}\n{}\n{}\n{}",
                        include_str!("direct_material.wgsl"),
                        include_str!("globe.wgsl"),
                        include_str!("atmosphere.wgsl"),
                        if background.is_some() {
                            include_str!("basic_interpolation.wgsl")
                        } else {
                            include_str!("edge_interpolation.wgsl")
                        }
                    )
                    .into(),
                ),
            })
        });
        let storage = |binding, read_only, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bounds_compute_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("地図表示範囲の計算"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    storage(1, false, wgpu::ShaderStages::COMPUTE),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(32),
                        },
                        count: None,
                    },
                    storage(3, true, wgpu::ShaderStages::COMPUTE),
                    storage(4, false, wgpu::ShaderStages::COMPUTE),
                ],
            });
        let bounds_vertex_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("地図表示範囲の参照"),
                entries: &[storage(0, true, wgpu::ShaderStages::VERTEX)],
            });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("地形画素から地図の範囲を決める"),
            source: wgpu::ShaderSource::Wgsl(include_str!("material_bounds.wgsl").into()),
        });
        let compute_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bounds_compute_layout],
            push_constant_ranges: &[],
        });
        let compute = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&compute_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let bounds_collect = crate::gpu_profile::timed("地図範囲集約", || compute("collect"));
        let bounds_reduce = crate::gpu_profile::timed("地図矩形計算", || compute("bound"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout, &sampler_layout, &bounds_vertex_layout],
            push_constant_ranges: &[],
        });
        let pipeline = |entry, color, mask| {
            Self::pipeline(device, &shader, &pipeline_layout, entry, color, mask)
        };
        let blend = |src_factor, dst_factor| wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
        // 独立した描画プログラムは同時に作成し、起動時の直列待ちをなくす。
        let (color, shade, lighting) = std::thread::scope(|scope| {
            let color = scope.spawn(|| {
                pipeline(
                    "color",
                    blend(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
                    wgpu::ColorWrites::ALL,
                )
            });
            let shade = scope.spawn(|| {
                pipeline(
                    "shade",
                    blend(wgpu::BlendFactor::Dst, wgpu::BlendFactor::Src),
                    wgpu::ColorWrites::COLOR,
                )
            });
            let lighting = pipeline(
                "lighting",
                blend(wgpu::BlendFactor::One, wgpu::BlendFactor::Zero),
                wgpu::ColorWrites::COLOR,
            );
            (color.join().unwrap(), shade.join().unwrap(), lighting)
        });
        let occlusion = pipeline(
            "occlusion",
            blend(wgpu::BlendFactor::One, wgpu::BlendFactor::Zero),
            wgpu::ColorWrites::empty(),
        );
        let filtered = background.map(|ctx| {
            let result = Arc::new(std::sync::OnceLock::new());
            let ready = Arc::clone(&result);
            let device = device.clone();
            let layout = pipeline_layout.clone();
            std::thread::spawn(move || {
                let programs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("高品質画像補間"),
                        source: wgpu::ShaderSource::Wgsl(
                            format!(
                                "{}\n{}\n{}\n{}",
                                include_str!("direct_material.wgsl"),
                                include_str!("globe.wgsl"),
                                include_str!("atmosphere.wgsl"),
                                include_str!("edge_interpolation.wgsl")
                            )
                            .into(),
                        ),
                    });
                    std::thread::scope(|scope| {
                        let color = scope.spawn(|| {
                            Self::pipeline(
                                &device,
                                &shader,
                                &layout,
                                "color",
                                blend(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
                                wgpu::ColorWrites::ALL,
                            )
                        });
                        let shade = Self::pipeline(
                            &device,
                            &shader,
                            &layout,
                            "shade",
                            blend(wgpu::BlendFactor::Dst, wgpu::BlendFactor::Src),
                            wgpu::ColorWrites::COLOR,
                        );
                        FilteredPrograms {
                            color: color.join().expect("画像補間の準備に失敗"),
                            shade,
                        }
                    })
                }))
                .map_err(|panic| {
                    let text = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|v| v.to_string()))
                        .unwrap_or_else(|| "高品質補間の準備に失敗しました".into());
                    eprintln!("高品質補間の準備に失敗: {text}");
                    text
                });
                let _ = ready.set(programs);
                ctx.request_repaint();
            });
            result
        });
        Self {
            bounds_compute_layout,
            bounds_vertex_layout,
            bounds_collect,
            bounds_reduce,
            cull: std::env::var_os("YAMAMUKI_DISABLE_MATERIAL_CULLING").is_none(),
            cache: Default::default(),
            color,
            shade,
            filtered,
            lighting,
            occlusion,
            layout,
            sampler_group,
            empty: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("直接参照の透明タイル"),
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
                .create_view(&Default::default()),
        }
    }
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_texture: &wgpu::Texture,
        lighting_source: &wgpu::Texture,
        positions: &wgpu::TextureView,
        camera: &Camera,
        size: [u32; 2],
        commands: &[Quad],
    ) {
        self.render_region(
            device,
            queue,
            encoder,
            target_texture,
            lighting_source,
            positions,
            camera,
            size,
            commands,
            true,
            [-500., 4500.],
        );
    }
    // 地形OFFでは0mの球面だけを囲み、存在しない山のために全画面を塗らない。
    pub fn render_flat(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_texture: &wgpu::Texture,
        lighting_source: &wgpu::Texture,
        positions: &wgpu::TextureView,
        camera: &Camera,
        size: [u32; 2],
        commands: &[Quad],
    ) {
        self.render_region(
            device,
            queue,
            encoder,
            target_texture,
            lighting_source,
            positions,
            camera,
            size,
            commands,
            true,
            [0., 0.],
        );
    }
    fn render_region(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_texture: &wgpu::Texture,
        lighting_source: &wgpu::Texture,
        positions: &wgpu::TextureView,
        camera: &Camera,
        size: [u32; 2],
        commands: &[Quad],
        restrict_pixels: bool,
        heights: [f64; 2],
    ) {
        let _ = device.poll(wgpu::Maintain::Poll);
        let mut cache = self.cache.lock().unwrap();
        let bounds_pose = [
            camera.center[0].to_bits(),
            camera.center[1].to_bits(),
            heights[0].to_bits(),
            heights[1].to_bits(),
        ];
        let legacy_bounds = std::env::var_os("YAMAMUKI_DISABLE_MATERIAL_BOUND_REUSE").is_some();
        let anchor = [
            f64::from_bits(cache.globe_bounds_pose[0]),
            f64::from_bits(cache.globe_bounds_pose[1]),
        ];
        if (legacy_bounds && cache.globe_bounds_pose != bounds_pose)
            || cache.globe_bounds_pose[2..] != bounds_pose[2..]
            || (anchor[0] - camera.center[0]).hypot(anchor[1] - camera.center[1]) > 100000.
            || cache.globe_bounds.len() > 32768
        {
            cache.globe_bounds.clear();
            cache.globe_bounds_pose = bounds_pose;
        }

        let mut anchor_camera = camera.clone();
        anchor_camera.center = [
            f64::from_bits(cache.globe_bounds_pose[0]),
            f64::from_bits(cache.globe_bounds_pose[1]),
        ];
        let frame = GlobeFrame::new(anchor_camera.center, camera.center);
        let planes = visibility_planes(camera);
        // 地形へ貼れない画面外・遠方の画像は、GPU設定を作る前に除く。
        let commands = commands
            .iter()
            .filter(|c| {
                if camera.globe && !legacy_bounds {
                    let physical = *cache
                        .globe_bounds
                        .entry(c.bounds.map(f64::to_bits))
                        .or_insert_with(|| {
                            globe_physical_bounds(c.bounds, &anchor_camera, heights)
                        });
                    physical.is_none_or(|(lower, upper)| {
                        let (lower, upper) = frame.bounds(lower, upper);
                        box_visible(lower, upper, &planes)
                    })
                } else {
                    camera.may_be_visible(c.bounds.map(|v| v * WORLD))
                }
            })
            .collect::<Vec<_>>();
        let target = target_texture.create_view(&Default::default());
        let stride = (std::mem::size_of::<Params>() as u64).div_ceil(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        )) * u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let count = commands.len() + 1;
        let use_occlusion = heights != [0., 0.]
            && std::env::var_os("YAMAMUKI_DISABLE_MATERIAL_OCCLUSION").is_none()
            && opaque_overlap(&commands);
        let use_bounds = self.cull && restrict_pixels && heights != [0., 0.] && commands.len() >= 8;

        if cache.positions.as_ref() != Some(positions)
            || cache
                .lighting
                .as_ref()
                .is_none_or(|(texture, _)| texture != lighting_source)
        {
            cache.groups.clear();
            cache.positions = Some(positions.clone());
            cache.lighting = Some((
                lighting_source.clone(),
                lighting_source.create_view(&Default::default()),
            ));
        }
        if cache.capacity < count {
            cache.capacity = count.next_power_of_two();
            cache.uniform = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("再利用する直接貼付パラメーター"),
                size: cache.capacity as u64 * stride,
                usage: wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            }));
            cache.buffers_created += 1;
            cache.groups.clear();
        }
        let uniform = cache.uniform.as_ref().unwrap().clone();
        let lighting_view = cache.lighting.as_ref().unwrap().1.clone();
        let mut bytes = vec![0u8; count * stride as usize];
        let mut draws = Vec::with_capacity(count);
        let active: std::collections::HashSet<_> = commands
            .iter()
            .map(|c| texture_key(c))
            .chain(std::iter::once([None, None, None, None]))
            .collect();
        cache.groups.retain(|key, _| active.contains(key));
        let mut needed = std::collections::HashSet::new();
        for (i, command) in commands
            .iter()
            .map(Some)
            .chain(std::iter::once(None))
            .enumerate()
        {
            let mut params = command.map_or(Params::zeroed(), |c| Params {
                earth: [0.; 4],
                eye: [0.; 4],
                area: [
                    (c.bounds[0] * WORLD - camera.center[0]) as f32,
                    (c.bounds[1] * WORLD - camera.center[1]) as f32,
                    ((c.bounds[2] - c.bounds[0]) * WORLD) as f32,
                    ((c.bounds[3] - c.bounds[1]) * WORLD) as f32,
                ],
                uv0: [
                    c.uv[0].min.x,
                    c.uv[0].min.y,
                    c.uv[0].width(),
                    c.uv[0].height(),
                ],
                uv1: [
                    c.uv[1].min.x,
                    c.uv[1].min.y,
                    c.uv[1].width(),
                    c.uv[1].height(),
                ],
                gain_uv0: c.surface.as_ref().map_or([0.; 4], |s| {
                    [
                        s.uv[0].min.x,
                        s.uv[0].min.y,
                        s.uv[0].width(),
                        s.uv[0].height(),
                    ]
                }),
                gain_uv1: c.surface.as_ref().map_or([0.; 4], |s| {
                    [
                        s.uv[1].min.x,
                        s.uv[1].min.y,
                        s.uv[1].width(),
                        s.uv[1].height(),
                    ]
                }),
                visibility: [(i + 1) as f32 / count as f32, 0., 0., 0.],
                tint: c.tint,
                settings: [
                    c.amount,
                    f32::from(c.textures[0].is_some()),
                    f32::from(c.textures[1].is_some()),
                    f32::from(c.surface.is_none() && c.textures.iter().all(Option::is_none))
                        + 2. * f32::from(c.gamma_shade)
                        + 8. * (c.edge_filter as u32 & 3) as f32
                        + 64. * (c.edge_filter as u32 >> 2) as f32
                        + 4. * f32::from(c.border)
                        + 32. * f32::from(use_bounds)
                        + 256. * f32::from(c.surface.is_some())
                        + 512.
                            * f32::from(
                                c.surface.as_ref().is_some_and(|s| s.textures[0].is_some()),
                            )
                        + 1024.
                            * f32::from(
                                c.surface.as_ref().is_some_and(|s| s.textures[1].is_some()),
                            ),
                ],
            });
            if command.is_none() && camera.globe {
                let latitude = crate::globe::latitude(camera.center[1]);
                let back = camera.back();
                params.earth = [
                    latitude.sin() as f32,
                    latitude.cos() as f32,
                    crate::globe::RADIUS as f32,
                    1.,
                ];
                params.eye = [
                    (back.x * camera.distance) as f32,
                    (camera.height + back.y * camera.distance) as f32,
                    (back.z * camera.distance) as f32,
                    camera.scale() as f32,
                ];
            }
            let at = i * stride as usize;
            bytes[at..at + std::mem::size_of::<Params>()]
                .copy_from_slice(bytemuck::bytes_of(&params));
            let key = command.map_or([None, None, None, None], |c| texture_key(c));
            needed.insert(key.clone());
            if !cache.groups.contains_key(&key) {
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("再利用する元タイル直接貼付"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(positions),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(
                                key[0].as_ref().unwrap_or(&self.empty),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(
                                key[1].as_ref().unwrap_or(&self.empty),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::TextureView(
                                key[2].as_ref().unwrap_or(&self.empty),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::TextureView(
                                key[3].as_ref().unwrap_or(&self.empty),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(&lighting_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &uniform,
                                offset: 0,
                                size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
                            }),
                        },
                    ],
                });
                cache.groups.insert(key.clone(), group);
                cache.groups_created += 1;
            }
            let clip = command.map_or([0, 0, size[0], size[1]], |c| {
                if restrict_pixels {
                    if camera.globe {
                        let physical = *cache
                            .globe_bounds
                            .entry(c.bounds.map(f64::to_bits))
                            .or_insert_with(|| {
                                globe_physical_bounds(c.bounds, &anchor_camera, heights)
                            });
                        physical.map_or([0, 0, size[0], size[1]], |(lower, upper)| {
                            let (lower, upper) = frame.bounds(lower, upper);
                            globe_project_bounds(lower, upper, camera, size)
                        })
                    } else {
                        scissor_at_heights(c.bounds, camera, size, heights)
                    }
                } else {
                    [0, 0, size[0], size[1]]
                }
            });
            draws.push((
                cache.groups[&key].clone(),
                command.map(|c| c.shade),
                clip,
                (i as u64 * stride) as u32,
            ));
        }
        cache.groups.retain(|key, _| needed.contains(key));
        queue.write_buffer(&uniform, 0, &bytes);
        if cache
            .screen_bounds
            .as_ref()
            .is_none_or(|b| b.size != size || b.capacity < count || b.positions != *positions)
        {
            let capacity = count.next_power_of_two();
            let blocks = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("画面区画の地理範囲"),
                size: u64::from(size[0].div_ceil(32) * size[1].div_ceil(32)) * 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let rectangles = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("地図ごとの表示矩形"),
                size: capacity as u64 * 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let meta = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("地図範囲計算の設定"),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let compute = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("地図表示範囲の計算"),
                layout: &self.bounds_compute_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(positions),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: blocks.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: meta.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: rectangles.as_entire_binding(),
                    },
                ],
            });
            let vertex = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("地図の画面範囲"),
                layout: &self.bounds_vertex_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: rectangles.as_entire_binding(),
                }],
            });
            cache.screen_bounds = Some(ScreenBounds {
                positions: positions.clone(),
                size,
                capacity,
                meta,
                compute,
                vertex,
            });
        }
        let bounds = cache.screen_bounds.as_ref().unwrap();
        if use_bounds {
            queue.write_buffer(
                &bounds.meta,
                0,
                bytemuck::cast_slice(&[
                    size[0],
                    size[1],
                    size[0].div_ceil(32),
                    size[1].div_ceil(32),
                    commands.len() as u32,
                    (stride / 16) as u32,
                    0,
                    0,
                ]),
            );
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("実地形から地図の描画範囲を計算"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &bounds.compute, &[]);
            pass.set_pipeline(&self.bounds_collect);
            pass.dispatch_workgroups(size[0].div_ceil(32), size[1].div_ceil(32), 1);
            pass.set_pipeline(&self.bounds_reduce);
            pass.dispatch_workgroups(commands.len() as u32, 1, 1);
        }
        let bounds_vertex = bounds.vertex.clone();
        if cache
            .coverage
            .as_ref()
            .is_none_or(|(t, _)| t.width() != size[0] || t.height() != size[1])
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("不透明地図の最終描画順"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            cache.coverage = Some((texture, view));
        }
        let coverage = cache.coverage.as_ref().unwrap().1.clone();
        drop(cache);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("不透明地図に隠れる補間を除外"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &coverage,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if use_occlusion {
                pass.set_pipeline(&self.occlusion);
                pass.set_bind_group(1, &self.sampler_group, &[]);
                pass.set_bind_group(2, &bounds_vertex, &[]);
                for (i, c) in commands.iter().enumerate() {
                    if !opaque_command(c) {
                        continue;
                    }
                    let (group, _, clip, offset) = &draws[i];
                    if clip[2] == 0 || clip[3] == 0 {
                        continue;
                    }
                    pass.set_scissor_rect(clip[0], clip[1], clip[2], clip[3]);
                    pass.set_bind_group(0, group, &[*offset]);
                    if use_bounds {
                        pass.draw(0..6, i as u32..i as u32 + 1);
                    } else {
                        pass.draw(0..3, 0..1);
                    }
                }
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("元タイルを地形ピクセルへ合成"),
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &coverage,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_bind_group(1, &self.sampler_group, &[]);
        pass.set_bind_group(2, &bounds_vertex, &[]);
        let filtered = self
            .filtered
            .as_ref()
            .and_then(|p| p.get())
            .and_then(|p| p.as_ref().ok());
        let color_pipeline = filtered.map_or(&self.color, |p| &p.color);
        let shade_pipeline = filtered.map_or(&self.shade, |p| &p.shade);
        let mut last_surface = None;
        for (draw_index, (group, shade, clip, offset)) in draws.iter().enumerate() {
            let surface = commands
                .get(draw_index)
                .and_then(|c| c.surface.as_ref())
                .map(|s| s.group);
            if surface.is_some() && surface != last_surface {
                drop(pass);
                encoder.copy_texture_to_texture(
                    target_texture.as_image_copy(),
                    lighting_source.as_image_copy(),
                    wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                );
                pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("元タイルを地形ピクセルへ合成"),
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &coverage,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_bind_group(1, &self.sampler_group, &[]);
                pass.set_bind_group(2, &bounds_vertex, &[]);
            }
            last_surface = surface;
            if shade.is_none() || clip[2] == 0 || clip[3] == 0 {
                continue;
            }
            pass.set_scissor_rect(clip[0], clip[1], clip[2], clip[3]);
            pass.set_bind_group(0, group, &[*offset]);
            if *shade == Some(true) {
                for pipeline in [shade_pipeline] {
                    pass.set_pipeline(pipeline);
                    if use_bounds {
                        let i = *offset / (stride as u32);
                        pass.draw(0..6, i..i + 1);
                    } else {
                        pass.draw(0..3, 0..1);
                    }
                }
            } else {
                pass.set_pipeline(color_pipeline);
                if use_bounds {
                    let i = *offset / (stride as u32);
                    pass.draw(0..6, i..i + 1);
                } else {
                    pass.draw(0..3, 0..1);
                }
            }
        }
        drop(pass);
        // 同じ画面解像度のまま、従来のsRGB空間で地形の陰影を一度だけ適用する。
        encoder.copy_texture_to_texture(
            target_texture.as_image_copy(),
            lighting_source.as_image_copy(),
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("従来の色空間で地形陰影を適用"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.lighting);
        pass.set_bind_group(1, &self.sampler_group, &[]);
        pass.set_bind_group(2, &bounds_vertex, &[]);
        let (group, _, _, offset) = draws.last().unwrap();
        pass.set_bind_group(0, group, &[*offset]);
        pass.draw(0..3, 0..1);
    }
}
// 保存した接平面から現在の接平面へ箱を変換する。景色モードで注視点が
// 移動しても、高価なタイル単位の球面評価を繰り返さない。
struct GlobeFrame {
    rotation: glam::DMat3,
    origin: glam::DVec3,
}
impl GlobeFrame {
    fn new(anchor: [f64; 2], center: [f64; 2]) -> Self {
        fn basis(center: [f64; 2]) -> glam::DMat3 {
            let (sin_lat, cos_lat) = crate::globe::latitude(center[1]).sin_cos();
            let (sin_lon, cos_lon) = (center[0] / crate::globe::RADIUS).sin_cos();
            glam::DMat3::from_cols(
                glam::DVec3::new(cos_lon, 0., -sin_lon),
                glam::DVec3::new(cos_lat * sin_lon, sin_lat, cos_lat * cos_lon),
                glam::DVec3::new(sin_lat * sin_lon, -cos_lat, sin_lat * cos_lon),
            )
        }
        Self {
            rotation: basis(center).transpose() * basis(anchor),
            origin: crate::globe::local(center, anchor, 0.),
        }
    }
    fn bounds(&self, lower: glam::DVec3, upper: glam::DVec3) -> (glam::DVec3, glam::DVec3) {
        let center = self.rotation * ((lower + upper) * 0.5) + self.origin;
        let absolute = glam::DMat3::from_cols(
            self.rotation.x_axis.abs(),
            self.rotation.y_axis.abs(),
            self.rotation.z_axis.abs(),
        );
        let extent = absolute * ((upper - lower) * 0.5) + glam::DVec3::splat(1e-6);
        (center - extent, center + extent)
    }
}
// 方角だけが変わるフレームは保持済みの球面箱を使い、元タイルの緯度経度を再評価しない。
fn visibility_planes(c: &Camera) -> [(glam::DVec3, f64); 6] {
    use glam::DVec3;
    let back = c.back();
    let forward = -back;
    let right = DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
    let up = back.cross(right);
    let eye = back * c.distance + DVec3::Y * c.height;
    let vertical = c.tan_half_fov() * 1.15;
    let horizontal = vertical * c.viewport[0] / c.viewport[1];
    [
        (forward, -(c.distance / 10000.).max(0.2)),
        (-forward, c.far_depth()),
        (right + forward * horizontal, 0.),
        (-right + forward * horizontal, 0.),
        (up + forward * vertical, 0.),
        (-up + forward * vertical, 0.),
    ]
    .map(|(normal, distance)| (normal, distance - normal.dot(eye)))
}
fn box_visible(lower: glam::DVec3, upper: glam::DVec3, planes: &[(glam::DVec3, f64); 6]) -> bool {
    planes.iter().all(|(normal, distance)| {
        let support = glam::DVec3::new(
            if normal.x >= 0. { upper.x } else { lower.x },
            if normal.y >= 0. { upper.y } else { lower.y },
            if normal.z >= 0. { upper.z } else { lower.z },
        );
        normal.dot(support) + distance >= 0.
    })
}
// 画素から不透明と分かる画像だけを遮蔽元にし、背景参照する地表は除く。
fn opaque_command(c: &Quad) -> bool {
    c.opaque
        && !c.shade
        && !c.border
        && c.surface.is_none()
        && c.tint[3] >= 1.
        && c.textures.iter().all(Option::is_some)
}
fn opaque_overlap(commands: &[&Quad]) -> bool {
    let mut bounds = commands
        .iter()
        .filter(|c| opaque_command(c))
        .map(|c| c.bounds)
        .collect::<Vec<_>>();
    bounds.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
    for (i, a) in bounds.iter().enumerate() {
        for b in &bounds[i + 1..] {
            if b[0] >= a[2] {
                break;
            }
            if a[1] < b[3] && b[1] < a[3] {
                return true;
            }
        }
    }
    false
}
use bytemuck::Zeroable;
/// 可視標高範囲の投影を囲み、各タイルで画面全体を塗らない。
#[cfg(test)]
fn scissor(bounds: [f64; 4], c: &Camera, size: [u32; 2]) -> [u32; 4] {
    scissor_at_heights(bounds, c, size, [-500., 4500.])
}
fn scissor_at_heights(bounds: [f64; 4], c: &Camera, size: [u32; 2], heights: [f64; 2]) -> [u32; 4] {
    if c.globe {
        return globe_scissor_at_heights(bounds, c, size, heights);
    }
    let back = c.back();
    let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
    let up = back.cross(right);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for x in [bounds[0], bounds[2]] {
        for y in [bounds[1], bounds[3]] {
            for height in heights {
                let d = glam::DVec3::new(
                    (x * WORLD - c.center[0]) * c.scale(),
                    height - c.height,
                    (y * WORLD - c.center[1]) * c.scale(),
                );
                let depth = c.distance - d.dot(back);
                if depth <= 0. {
                    return [0, 0, size[0], size[1]];
                }
                let scale = size[1] as f64 / (2. * c.tan_half_fov() * depth);
                let p = [
                    size[0] as f64 / 2. + d.dot(right) * scale,
                    size[1] as f64 / 2. - d.dot(up) * scale,
                ];
                for i in 0..2 {
                    min[i] = min[i].min(p[i]);
                    max[i] = max[i].max(p[i]);
                }
            }
        }
    }
    let l = (min[0].floor() - 1.).clamp(0., size[0] as f64) as u32;
    let t = (min[1].floor() - 1.).clamp(0., size[1] as f64) as u32;
    let r = (max[0].ceil() + 1.).clamp(l as f64, size[0] as f64) as u32;
    let b = (max[1].ceil() + 1.).clamp(t as f64, size[1] as f64) as u32;
    [l, t, r - l, b - t]
}

// 球面を包む箱を投影し、極側や日付変更線でも曲がった辺を切らない。
fn globe_scissor_at_heights(
    bounds: [f64; 4],
    c: &Camera,
    size: [u32; 2],
    heights: [f64; 2],
) -> [u32; 4] {
    globe_physical_bounds(bounds, c, heights).map_or([0, 0, size[0], size[1]], |(lower, upper)| {
        globe_project_bounds(lower, upper, c, size)
    })
}
// 球面の箱は方角・画角に依存しない。左右回転では三角関数による再計算を避ける。
fn globe_physical_bounds(
    bounds: [f64; 4],
    c: &Camera,
    heights: [f64; 2],
) -> Option<(glam::DVec3, glam::DVec3)> {
    use glam::DVec3;
    let area = bounds.map(|v| v * WORLD);
    let span = (area[2] - area[0]).abs().max((area[3] - area[1]).abs());
    if span > crate::globe::RADIUS * 0.5 {
        return None;
    }
    let mut lower = DVec3::splat(f64::INFINITY);
    let mut upper = DVec3::splat(f64::NEG_INFINITY);
    const STEPS: u32 = 4;
    let latitude = crate::globe::latitude(c.center[1]);
    let (sin_lat, cos_lat) = latitude.sin_cos();
    // 5×5点の緯度・経度を行と列で共用し、高さごとに三角関数を計算し直さない。
    let rows: [(f64, f64, f64); 5] = std::array::from_fn(|y| {
        let world_y = area[1] + (area[3] - area[1]) * y as f64 / STEPS as f64;
        let b = crate::globe::latitude(world_y);
        let db = b - latitude;
        (b.cos(), db.sin(), (db / 2.).sin().powi(2))
    });
    let columns: [(f64, f64); 5] = std::array::from_fn(|x| {
        let world_x = area[0] + (area[2] - area[0]) * x as f64 / STEPS as f64;
        let dl = (world_x - c.center[0]) / crate::globe::RADIUS;
        (dl.sin(), (dl / 2.).sin().powi(2))
    });
    for (cos_b, sin_db, half_db) in rows {
        for (sin_dl, half_dl) in columns {
            for h in heights {
                let r = crate::globe::RADIUS + h;
                let p = DVec3::new(
                    r * cos_b * sin_dl,
                    h - r * (2. * half_db + 2. * cos_lat * cos_b * half_dl),
                    -r * (sin_db + 2. * sin_lat * cos_b * half_dl),
                );
                lower = lower.min(p);
                upper = upper.max(p);
            }
        }
    }
    // 格子点の間の球面の膨らみと、Mercator緯度の非線形な変化も含める。
    let dx = (area[2] - area[0]) / STEPS as f64;
    let dy = (area[3] - area[1]) / STEPS as f64;
    let padding = (dx * dx + dy * dy) / crate::globe::RADIUS + 0.1;
    lower -= DVec3::splat(padding);
    upper += DVec3::splat(padding);
    Some((lower, upper))
}
fn globe_project_bounds(
    lower: glam::DVec3,
    upper: glam::DVec3,
    c: &Camera,
    size: [u32; 2],
) -> [u32; 4] {
    use glam::DVec3;
    let back = c.back();
    let right = DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
    let up = back.cross(right);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for x in [lower.x, upper.x] {
        for y in [lower.y, upper.y] {
            for z in [lower.z, upper.z] {
                let d = DVec3::new(x, y - c.height, z);
                let depth = c.distance - d.dot(back);
                if depth <= 0. {
                    return [0, 0, size[0], size[1]];
                }
                let scale = size[1] as f64 / (2. * c.tan_half_fov() * depth);
                let p = [
                    size[0] as f64 / 2. + d.dot(right) * scale,
                    size[1] as f64 / 2. - d.dot(up) * scale,
                ];
                for i in 0..2 {
                    min[i] = min[i].min(p[i]);
                    max[i] = max[i].max(p[i]);
                }
            }
        }
    }
    let l = (min[0].floor() - 2.).clamp(0., size[0] as f64) as u32;
    let t = (min[1].floor() - 2.).clamp(0., size[1] as f64) as u32;
    let r = (max[0].ceil() + 2.).clamp(l as f64, size[0] as f64) as u32;
    let b = (max[1].ceil() + 2.).clamp(t as f64, size[1] as f64) as u32;
    [l, t, r - l, b - t]
}

#[cfg(test)]
mod tests {
    #[test]
    fn retained_globe_boxes_cover_terrain_when_observer_target_moves() {
        for latitude in [0.2, 0.4, 0.85] {
            let mut camera = Camera::default();
            camera.globe = true;
            camera.center = [WORLD * 0.88, WORLD * latitude];
            let bounds = [0.8799, latitude - 0.0001, 0.8801, latitude + 0.0001];
            let (lower, upper) = globe_physical_bounds(bounds, &camera, [-500., 4500.]).unwrap();
            for delta in [[100., -100.], [-500., 300.], [10000., 5000.]] {
                let mut moved = camera.clone();
                moved.center = [camera.center[0] + delta[0], camera.center[1] + delta[1]];
                let frame = GlobeFrame::new(camera.center, moved.center);
                let (lo, hi) = frame.bounds(lower, upper);
                for y in 0..=8 {
                    for x in 0..=8 {
                        for h in [-500., 0., 4500.] {
                            let world = [
                                (bounds[0] + (bounds[2] - bounds[0]) * x as f64 / 8.) * WORLD,
                                (bounds[1] + (bounds[3] - bounds[1]) * y as f64 / 8.) * WORLD,
                            ];
                            let p = crate::globe::local(moved.center, world, h);
                            assert!(
                                p.cmpge(lo).all() && p.cmple(hi).all(),
                                "{p:?} / {lo:?} / {hi:?}"
                            );
                        }
                    }
                }
            }
        }
    }
    use super::*;
    #[test]
    fn removed_edge_spline_settings_restore_to_edge_mode() {
        assert_eq!(
            serde_json::from_str::<Interpolation>("\"edge-spline\"").unwrap(),
            Interpolation::Edge
        );
        assert!(!Interpolation::ALL.iter().any(|mode| *mode as u32 == 3));
    }

    #[test]
    fn interpolation_preserves_legacy_choices_and_roundtrips_all_modes() {
        assert_eq!(
            serde_json::from_str::<Interpolation>("true").unwrap(),
            Interpolation::Edge
        );
        assert_eq!(
            serde_json::from_str::<Interpolation>("false").unwrap(),
            Interpolation::Bilinear
        );
        for retired in ["edge7", "edge-multi", "edge-curve", "edge-bilinear"] {
            let json = serde_json::to_string(retired).unwrap();
            let mode = serde_json::from_str::<Interpolation>(&json).unwrap();
            assert_eq!(mode, Interpolation::Edge);
            assert_eq!(serde_json::to_string(&mode).unwrap(), "\"edge\"");
        }
        for mode in Interpolation::ALL {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(serde_json::from_str::<Interpolation>(&json).unwrap(), mode);
        }
    }

    #[test]
    fn flat_zoomed_map_scissors_use_zero_elevation_and_cover_each_tile() {
        let size = [2686, 1564];
        let mut camera = Camera::default();
        camera.globe = true;
        camera.height = 0.;
        camera.set_map_view(
            [0.8844271266197067, 0.3963378090889512],
            19.,
            0.,
            0.,
            size.map(f64::from),
        );
        let side = 1. / 262144.;
        let cx = (camera.center[0] / WORLD / side).floor() as i32;
        let cy = (camera.center[1] / WORLD / side).floor() as i32;
        let back = camera.back();
        let right = glam::DVec3::new(1., 0., 0.);
        let up = back.cross(right);
        let mut old_pixels = 0u64;
        let mut new_pixels = 0u64;
        for y in cy - 2..=cy + 2 {
            for x in cx - 3..=cx + 3 {
                let bounds = [
                    x as f64 * side,
                    y as f64 * side,
                    (x + 1) as f64 * side,
                    (y + 1) as f64 * side,
                ];
                let old = scissor(bounds, &camera, size);
                let new = scissor_at_heights(bounds, &camera, size, [0., 0.]);
                old_pixels += u64::from(old[2]) * u64::from(old[3]);
                new_pixels += u64::from(new[2]) * u64::from(new[3]);
                for sy in 0..=4 {
                    for sx in 0..=4 {
                        let world = [
                            (x as f64 + sx as f64 / 4.) * side * WORLD,
                            (y as f64 + sy as f64 / 4.) * side * WORLD,
                        ];
                        let q = crate::globe::local(camera.center, world, 0.)
                            - glam::DVec3::Y * camera.height;
                        let depth = camera.distance - q.dot(back);
                        let scale = size[1] as f64 / (2. * camera.tan_half_fov() * depth);
                        let p = [
                            size[0] as f64 / 2. + q.dot(right) * scale,
                            size[1] as f64 / 2. - q.dot(up) * scale,
                        ];
                        if p[0] >= 0.
                            && p[0] < size[0] as f64
                            && p[1] >= 0.
                            && p[1] < size[1] as f64
                        {
                            assert!(p[0] >= new[0] as f64 && p[0] <= (new[0] + new[2]) as f64);
                            assert!(p[1] >= new[1] as f64 && p[1] <= (new[1] + new[3]) as f64);
                        }
                    }
                }
            }
        }
        eprintln!("平面タイル描画面積: {old_pixels} → {new_pixels} px/フレーム");
        assert!(new_pixels * 10 < old_pixels);
        assert!(new_pixels < u64::from(size[0]) * u64::from(size[1]) * 2);
    }
    #[test]
    fn globe_tile_scissors_cover_curved_edges_without_shading_the_whole_frame() {
        let mut c = Camera::default();
        c.globe = true;
        let size = [3357, 1953];
        c.center = [35572623.49885807, 15783474.539974594];
        c.height = 500.;
        c.distance = 867787.0600352235;
        c.yaw = 0.02880050428211689;
        c.pitch = 1.1057327250626425;
        c.viewport = size.map(f64::from);
        let back = c.back();
        let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
        let up = back.cross(right);
        let mut pixels = 0u64;
        let mut tiles = 0u64;
        for z in [6, 8, 10] {
            let side = WORLD / 2f64.powi(z);
            let cx = (c.center[0] / side) as i32;
            let cy = (c.center[1] / side) as i32;
            for y in cy - 2..=cy + 2 {
                for x in cx - 2..=cx + 2 {
                    let bounds = [
                        x as f64 * side / WORLD,
                        y as f64 * side / WORLD,
                        (x + 1) as f64 * side / WORLD,
                        (y + 1) as f64 * side / WORLD,
                    ];
                    let clip = scissor(bounds, &c, size);
                    pixels += u64::from(clip[2]) * u64::from(clip[3]);
                    tiles += 1;
                    for iy in 0..=16 {
                        for ix in 0..=16 {
                            for h in [-500., 0., 2500., 4500.] {
                                let world = [
                                    (x as f64 + ix as f64 / 16.) * side,
                                    (y as f64 + iy as f64 / 16.) * side,
                                ];
                                let d = crate::globe::local(c.center, world, h)
                                    - glam::DVec3::Y * c.height;
                                let depth = c.distance - d.dot(back);
                                if depth <= 0. {
                                    continue;
                                }
                                let scale = size[1] as f64 / (2. * c.tan_half_fov() * depth);
                                let p = [
                                    size[0] as f64 / 2. + d.dot(right) * scale,
                                    size[1] as f64 / 2. - d.dot(up) * scale,
                                ];
                                if p[0] >= 0.
                                    && p[0] < size[0] as f64
                                    && p[1] >= 0.
                                    && p[1] < size[1] as f64
                                {
                                    assert!(
                                        p[0] >= clip[0] as f64
                                            && p[0] < (clip[0] + clip[2]) as f64
                                            && p[1] >= clip[1] as f64
                                            && p[1] < (clip[1] + clip[3]) as f64,
                                        "球面の端が欠ける: z{z}, p={p:?}, clip={clip:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            pixels < tiles * u64::from(size[0]) * u64::from(size[1]) / 5,
            "元タイルごとに全面を描画している"
        );
    }
    #[test]
    fn globe_scissors_remain_conservative_near_the_horizon_poles_and_date_line() {
        let size = [1280, 800];
        for center in [[0.885, 0.395], [0.99999, 0.4], [0.5, 0.0001], [0.5, 0.9999]] {
            for (distance, z) in [(100., 18), (10000., 13), (867787., 8), (3000000., 5)] {
                for pitch in [0.1, 0.8, 1.56] {
                    let mut c = Camera::default();
                    c.globe = true;
                    c.center = center.map(|v| v * WORLD);
                    c.height = 1000.;
                    c.distance = distance;
                    c.pitch = pitch;
                    c.yaw = 0.7;
                    c.viewport = size.map(f64::from);
                    let n = 2f64.powi(z);
                    let x = (center[0] * n).floor();
                    let y = (center[1] * n).floor();
                    let area = [x / n, y / n, (x + 1.) / n, (y + 1.) / n];
                    let clip = scissor(area, &c, size);
                    let back = c.back();
                    let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
                    let up = back.cross(right);
                    for iy in 0..=12 {
                        for ix in 0..=12 {
                            for h in [-500., 0., 4500.] {
                                let w = [
                                    (x + ix as f64 / 12.) / n * WORLD,
                                    (y + iy as f64 / 12.) / n * WORLD,
                                ];
                                let d =
                                    crate::globe::local(c.center, w, h) - glam::DVec3::Y * c.height;
                                let depth = c.distance - d.dot(back);
                                if depth <= 0. {
                                    continue;
                                }
                                let scale = size[1] as f64 / (2. * c.tan_half_fov() * depth);
                                let p = [
                                    size[0] as f64 / 2. + d.dot(right) * scale,
                                    size[1] as f64 / 2. - d.dot(up) * scale,
                                ];
                                if p[0] >= 0.
                                    && p[0] < size[0] as f64
                                    && p[1] >= 0.
                                    && p[1] < size[1] as f64
                                {
                                    assert!(
                                        p[0] >= clip[0] as f64
                                            && p[0] < (clip[0] + clip[2]) as f64
                                            && p[1] >= clip[1] as f64
                                            && p[1] < (clip[1] + clip[3]) as f64,
                                        "画像範囲を切り過ぎた: center={center:?} z{z} pitch={pitch} p={p:?} clip={clip:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    #[ignore = "全面描画とタイル別描画範囲を同じGPUで計測する"]
    fn benchmark_globe_tile_scissors() -> anyhow::Result<()> {
        benchmark_tile_scissors(false)
    }
    #[test]
    #[ignore = "指定スナップショットの平面描画範囲を同じGPUで比較する"]
    fn benchmark_flat_zoomed_tile_scissors() -> anyhow::Result<()> {
        benchmark_tile_scissors(true)
    }
    fn benchmark_tile_scissors(flat: bool) -> anyhow::Result<()> {
        let backend = if cfg!(windows) {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::PRIMARY
        };
        let Some(app) = crate::remembered_scene_tests::app_with_backend(backend) else {
            return Ok(());
        };
        let device = &app.renderer.state.device;
        let queue = &app.renderer.state.queue;
        let renderer = Renderer::new(device);
        let size = if flat { [2686, 1564] } else { [1280, 768] };
        let mut camera = Camera::default();
        camera.globe = true;
        camera.center = [35572623.49885807, 15783474.539974594];
        camera.height = 500.;
        camera.distance = 867787.0600352235;
        camera.yaw = 0.02880050428211689;
        camera.pitch = 1.1057327250626425;
        camera.viewport = size.map(f64::from);
        if flat {
            camera.height = 0.;
            camera.set_map_view(
                [0.8844271266197067, 0.3963378090889512],
                19.,
                0.,
                0.,
                size.map(f64::from),
            );
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let positions = texture(
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        // 実際の球面への投影と同じ地理座標を使う。
        let projection = crate::Projection {
            camera: camera.clone(),
        };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32));
        let mut values = Vec::<[f32; 4]>::new();
        for y in 0..size[1] {
            for x in 0..size[0] {
                values.push(
                    projection
                        .world_at(rect, egui::pos2(x as f32 + 0.5, y as f32 + 0.5))
                        .map_or([0.; 4], |w| {
                            [
                                (w[0] * WORLD - camera.center[0]) as f32,
                                (w[1] * WORLD - camera.center[1]) as f32,
                                1.,
                                10000.,
                            ]
                        }),
                );
            }
        }
        queue.write_texture(
            positions.as_image_copy(),
            bytemuck::cast_slice(&values),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 16),
                rows_per_image: None,
            },
            extent,
        );
        let view = positions.create_view(&Default::default());
        let target = texture(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let lighting = texture(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let side = if flat { 1. / 262144. } else { 1. / 256. };
        let cx = (camera.center[0] / WORLD / side) as i32;
        let cy = (camera.center[1] / WORLD / side) as i32;
        let mut commands = Vec::new();
        let ry = if flat { 2 } else { 8 };
        let rx = if flat { 3 } else { 10 };
        for y in cy - ry..=cy + ry {
            for x in cx - rx..=cx + rx {
                commands.push(Quad {
                    bounds: [
                        x as f64 * side,
                        y as f64 * side,
                        (x + 1) as f64 * side,
                        (y + 1) as f64 * side,
                    ],
                    uv: [egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)); 2],
                    textures: [None, None],
                    amount: 1.,
                    tint: [0.3, 0.5, 0.8, 1.],
                    shade: false,
                    gamma_shade: false,
                    opaque: false,
                    surface: None,
                    edge_filter: Interpolation::Bilinear,
                    border: false,
                });
            }
        }
        let mut outputs = Vec::new();
        for restricted in [false, true] {
            let begin = std::time::Instant::now();
            for _ in 0..12 {
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target.create_view(&Default::default()),
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                }
                renderer.render_region(
                    device,
                    queue,
                    &mut encoder,
                    &target,
                    &lighting,
                    &view,
                    &camera,
                    size,
                    &commands,
                    restricted,
                    if flat { [0., 0.] } else { [-500., 4500.] },
                );
                queue.submit([encoder.finish()]);
                let _ = device.poll(wgpu::Maintain::Wait);
            }
            eprintln!(
                "描画範囲制限={restricted}: {:.2} ms/フレーム（{}タイル）",
                begin.elapsed().as_secs_f64() * 1000. / 12.,
                commands.len()
            );
            let row_bytes = (size[0] * 4).div_ceil(256) * 256;
            let bytes = u64::from(row_bytes) * u64::from(size[1]);
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row_bytes),
                        rows_per_image: None,
                    },
                },
                extent,
            );
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            read.slice(..)
                .map_async(wgpu::MapMode::Read, move |v| send.send(v).unwrap());
            let _ = device.poll(wgpu::Maintain::Wait);
            receive.recv()??;
            outputs.push(read.slice(..).get_mapped_range().to_vec());
        }
        assert_eq!(
            outputs[0], outputs[1],
            "球面のタイル描画範囲を制限した結果、画像に欠けが生じた"
        );
        Ok(())
    }
    #[test]
    #[ignore = "GPU設定の再利用を多数の連続描画で検証する"]
    fn continuous_drawing_reuses_gpu_descriptors_and_uniform_storage() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: if cfg!(windows) {
                wgpu::Backends::DX12
            } else {
                wgpu::Backends::PRIMARY
            },
            ..Default::default()
        });
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let renderer = Renderer::new(&device);
        let extent = wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        };
        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let positions = texture(
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let coordinates = vec![[WORLD as f32 / 2., WORLD as f32 / 2., 1., 1.]; 64];
        queue.write_texture(
            positions.as_image_copy(),
            bytemuck::cast_slice(&coordinates),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8 * 16),
                rows_per_image: None,
            },
            extent,
        );
        let positions = positions.create_view(&Default::default());
        let target = texture(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let lighting = texture(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let tile_count = std::env::var("YAMAMUKI_GPU_STRESS_TILES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(192)
            .max(16);
        let mut commands = (0..tile_count)
            .map(|i| {
                let source = texture(
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                );
                queue.write_texture(
                    source.as_image_copy(),
                    &vec![i as u8; 8 * 8 * 4],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(32),
                        rows_per_image: None,
                    },
                    extent,
                );
                let view = source.create_view(&Default::default());
                Quad {
                    bounds: [0., 0., 1., 1.],
                    uv: [egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)); 2],
                    textures: [Some(view.clone()), Some(view)],
                    amount: 1.,
                    tint: [1.; 4],
                    shade: false,
                    gamma_shade: false,
                    opaque: false,
                    surface: None,
                    edge_filter: Interpolation::Bilinear,
                    border: false,
                }
            })
            .collect::<Vec<_>>();
        let camera = Camera {
            center: [0.; 2],
            distance: 1.,
            pitch: 0.,
            ..Default::default()
        };
        let frames = std::env::var("YAMAMUKI_GPU_STRESS_FRAMES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1500);
        let started = std::time::Instant::now();
        for frame in 0..frames {
            commands[0].amount = (frame % 100) as f32 / 100.;
            let mut encoder = device.create_command_encoder(&Default::default());
            renderer.render(
                &device,
                &queue,
                &mut encoder,
                &target,
                &lighting,
                &positions,
                &camera,
                [8, 8],
                &commands,
            );
            queue.submit([encoder.finish()]);
        }
        let _ = device.poll(wgpu::Maintain::Wait);
        {
            let cache = renderer.cache.lock().unwrap();
            assert_eq!(
                cache.groups_created,
                tile_count as u64 + 1,
                "同じタイルの設定がフレームごとに増えている"
            );
            assert_eq!(
                cache.buffers_created, 1,
                "パラメーターバッファをフレームごとに作っている"
            );
        }
        commands.truncate(16);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &target,
            &lighting,
            &positions,
            &camera,
            [8, 8],
            &commands,
        );
        queue.submit([encoder.finish()]);
        let _ = device.poll(wgpu::Maintain::Wait);
        let cache = renderer.cache.lock().unwrap();
        assert_eq!(
            cache.groups.len(),
            17,
            "使わなくなったタイルの設定が残っている"
        );
        assert_eq!(cache.groups_created, tile_count as u64 + 1);
        eprintln!(
            "連続描画 {frames}フレーム / {tile_count}タイル: {:.2}秒、生成した設定 {}件、共通バッファ {}本",
            started.elapsed().as_secs_f64(),
            cache.groups_created,
            cache.buffers_created
        );
    }
    #[test]
    fn native_pixels_crossfade_and_hillshade_on_real_gpu() {
        check_material_pixels(false);
    }
    #[test]
    fn background_compilation_preserves_basic_draws_and_switches_to_filtered_draws() {
        check_material_pixels(true);
    }
    fn check_material_pixels(background: bool) {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: if cfg!(windows) {
                wgpu::Backends::DX12
            } else {
                wgpu::Backends::PRIMARY
            },
            ..Default::default()
        });
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            eprintln!("GPUなし: 直接貼付の実描画を省略");
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let renderer = if background {
            Renderer::new_background(&device, &egui::Context::default())
        } else {
            Renderer::new(&device)
        };
        let width = 384u32;
        let extent = wgpu::Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let positions = texture(
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let source = |values: Vec<u8>| {
            let tex = texture(
                wgpu::TextureFormat::Rgba8UnormSrgb,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            queue.write_texture(
                tex.as_image_copy(),
                &values,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: None,
                },
                extent,
            );
            tex.create_view(&Default::default())
        };
        let checker = source(
            (0..width)
                .flat_map(|x| {
                    let v = if x % 2 == 0 { 0 } else { 255 };
                    [v, v, v, 255]
                })
                .collect(),
        );
        let black = source((0..width).flat_map(|_| [0, 0, 0, 255]).collect());
        let white = source(vec![255; width as usize * 4]);
        let mut camera = Camera {
            center: [0.; 2],
            pitch: 0.,
            ..Default::default()
        };
        // scissorの保守的な全面描画を使い、実際の地理座標は位置テクスチャで与える。
        camera.distance = 1.;
        let quad = |old: wgpu::TextureView, new: wgpu::TextureView, amount| Quad {
            bounds: [0., 0., 1., 1.],
            uv: [egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)); 2],
            textures: [Some(old), Some(new)],
            amount,
            tint: [1.; 4],
            shade: false,
            gamma_shade: false,
            opaque: false,
            surface: None,
            edge_filter: Interpolation::Bilinear,
            border: false,
        };
        let shade = texture(
            wgpu::TextureFormat::Rg8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            shade.as_image_copy(),
            &(0..width).flat_map(|_| [64, 0]).collect::<Vec<u8>>(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 2),
                rows_per_image: None,
            },
            extent,
        );
        let shade_view = shade.create_view(&Default::default());
        let gray = source((0..width).flat_map(|_| [64, 64, 64, 255]).collect());
        let mid = source((0..width).flat_map(|_| [188, 188, 188, 255]).collect());
        for mode in (0..29).filter(|mode| *mode != 15) {
            if background && mode == 16 {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
                while renderer.interpolation_pending() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                assert!(
                    !renderer.interpolation_pending(),
                    "高品質補間が準備完了しない"
                );
                assert!(
                    renderer.interpolation_error().is_none(),
                    "{:?}",
                    renderer.interpolation_error()
                );
            }

            let mut q = match mode {
                0 | 14 | 16 | 17 => quad(checker.clone(), checker.clone(), 1.),
                1 => quad(black.clone(), white.clone(), 0.5),
                6 => quad(gray.clone(), gray.clone(), 1.),
                7 => quad(mid.clone(), mid.clone(), 1.),
                _ => quad(white.clone(), white.clone(), 1.),
            };
            q.edge_filter = if mode == 17 {
                Interpolation::BSpline
            } else if mode == 16 {
                Interpolation::Bicubic
            } else if mode == 14 {
                Interpolation::Nearest
            } else {
                (mode >= 13).into()
            };
            if mode >= 13 {
                // 拡大時の補間経路でも単色が変色しないことを確認する。
                q.uv = [egui::Rect::from_min_max(egui::pos2(0.25, 0.25), egui::pos2(0.5, 0.5)); 2];
            }
            if (8..10).contains(&mode) || mode == 26 {
                q.border = true;
                q.textures = [None, None];
                if mode == 26 {
                    q.uv =
                        [egui::Rect::from_min_max(egui::pos2(0.5, 0.25), egui::pos2(1., 0.75)); 2];
                }
                if mode == 9 {
                    q.uv =
                        [egui::Rect::from_min_max(egui::pos2(0.25, 0.25), egui::pos2(0.75, 0.75));
                            2];
                }
            }
            camera.globe = (10..=12).contains(&mode);
            camera.height = if mode == 12 {
                60000.
            } else if (10..=12).contains(&mode) {
                3000.
            } else {
                0.
            };
            let coordinates = (0..width)
                .map(|x| {
                    if (10..=12).contains(&mode) {
                        return [
                            ((if mode == 10 { 1. } else { 100000. }) / camera.scale()) as f32,
                            0.,
                            1.,
                            if mode == 12 { 70000. } else { 11000. },
                        ];
                    }
                    [
                        (x as f32 + 0.5) / width as f32 * WORLD as f32,
                        WORLD as f32 / 2.,
                        if mode == 2 || mode == 6 {
                            0.5
                        } else if mode == 7 {
                            1.3
                        } else {
                            1.
                        },
                        if mode == 3 && x < width / 2 { 0. } else { 1. },
                    ]
                })
                .collect::<Vec<_>>();
            queue.write_texture(
                positions.as_image_copy(),
                bytemuck::cast_slice(&coordinates),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 16),
                    rows_per_image: None,
                },
                extent,
            );
            let target = texture(
                wgpu::TextureFormat::Rgba8UnormSrgb,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let target_view = target.create_view(&Default::default());
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(width * 4),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
            let mut commands = vec![q];
            if (18..26).contains(&mode) {
                let coefficient = |gain: f32| {
                    let tex = texture(
                        wgpu::TextureFormat::R32Float,
                        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    );
                    let values = (0..width)
                        .map(|x| {
                            if mode == 24 {
                                0.5 + x as f32 / (width - 1) as f32
                            } else if mode == 25 {
                                if x % 2 == 0 { 0.5 } else { 1.5 }
                            } else {
                                gain
                            }
                        })
                        .collect::<Vec<_>>();
                    queue.write_texture(
                        tex.as_image_copy(),
                        bytemuck::cast_slice(&values),
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(width * 4),
                            rows_per_image: None,
                        },
                        extent,
                    );
                    tex.create_view(&Default::default())
                };
                let gains = if mode == 19 {
                    [1.5, 1.5]
                } else if mode == 22 {
                    [0.5, 1.5]
                } else {
                    [0.5, 0.5]
                };
                let mut merged = quad(
                    black.clone(),
                    black.clone(),
                    if mode == 22 { 0.5 } else { 1. },
                );
                merged.textures = if mode == 20 {
                    let color = source((0..width).flat_map(|_| [64, 0, 0, 128]).collect());
                    [Some(color.clone()), Some(color)]
                } else if mode == 21 {
                    let color = source((0..width).flat_map(|_| [64, 0, 0, 255]).collect());
                    [Some(color.clone()), Some(color)]
                } else {
                    [None, None]
                };
                merged.surface = Some(SurfaceBlend {
                    textures: gains.map(|g| Some(coefficient(g))),
                    uv: merged.uv,
                    group: 1,
                });
                if mode == 25 {
                    merged.edge_filter = Interpolation::Nearest;
                    merged.surface.as_mut().unwrap().uv =
                        [egui::Rect::from_min_max(egui::pos2(0.25, 0.), egui::pos2(0.5, 1.)); 2];
                }
                let background = source((0..width).flat_map(|_| [128, 128, 128, 255]).collect());
                commands = vec![quad(background.clone(), background, 1.), merged];
                if mode == 23 {
                    let mut other = commands[1].clone();
                    other.surface.as_mut().unwrap().group = 2;
                    commands.push(other);
                }
            }

            // 高さのある元画像で、隠面省略を実際に有効にした状態を検査する。
            if mode >= 27 {
                let tall = |rgba: [u8; 4]| {
                    let size = wgpu::Extent3d {
                        height: 8,
                        ..extent
                    };
                    let tex = device.create_texture(&wgpu::TextureDescriptor {
                        label: None,
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    let values = (0..width * 8).flat_map(|_| rgba).collect::<Vec<_>>();
                    queue.write_texture(
                        tex.as_image_copy(),
                        &values,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(width * 4),
                            rows_per_image: Some(8),
                        },
                        size,
                    );
                    tex.create_view(&Default::default())
                };
                let white = tall([255; 4]);
                let black = tall([0, 0, 0, 255]);
                let mut base = quad(white.clone(), white.clone(), 1.);
                base.opaque = true;
                let mut front = quad(white, black, 0.5);
                front.opaque = true;
                commands = vec![base, front];
                if mode == 28 {
                    // 半透明の後続画像は下地を遮蔽してはならない。
                    let transparent = tall([0, 0, 0, 128]);
                    commands.push(quad(transparent.clone(), transparent, 1.));
                }
            }

            if (4..=5).contains(&mode) {
                let mut overlay = quad(shade_view.clone(), shade_view.clone(), 1.);
                overlay.shade = true;
                overlay.gamma_shade = mode == 5;
                commands.push(overlay);
            }
            // 多数の画像がある経路でも、空・半透明・陰影・フェードの値を維持する。
            for _ in 0..8 {
                let mut invisible = commands[0].clone();
                invisible.tint = [0.; 4];
                commands.push(invisible);
            }
            renderer.render(
                &device,
                &queue,
                &mut encoder,
                &target,
                &texture(
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                ),
                &positions.create_view(&Default::default()),
                &camera,
                [width, 1],
                &commands,
            );
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: None,
                    },
                },
                extent,
            );
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            read.slice(..)
                .map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
            let _ = device.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().unwrap();
            let bytes = read.slice(..).get_mapped_range();
            for (x, pixel) in bytes.chunks_exact(4).enumerate() {
                if mode == 11 {
                    assert!(
                        pixel[0] < 248 && pixel[2] > pixel[0] + 3,
                        "遠景が青い霞になっていない: {pixel:?}"
                    );
                    continue;
                }
                let expected = match mode {
                    0 => {
                        if x % 2 == 0 {
                            0
                        } else {
                            255
                        }
                    }
                    1 => 128,
                    2 => 128,
                    4 => 225,
                    5 => 192,
                    6 => 32,
                    7 => 244,
                    8 => {
                        if x < 2 || x >= width as usize - 2 {
                            255
                        } else {
                            0
                        }
                    }
                    9 => 0,
                    10 | 12 | 13 | 15 => 255,
                    16 | 17 => {
                        // 周期2の白黒画素にCatmull–Romを適用したCPU参照値。
                        let p = width as f32 * 0.25 + (x as f32 + 0.5) * 0.25 - 0.5;
                        let u = p - p.floor();
                        let v = u * u * (3. - 2. * u);
                        let v = if mode == 17 { (1. + v) / 3. } else { v };
                        let linear = if p.floor() as u32 % 2 == 0 { v } else { 1. - v };
                        let gamma = if linear <= 0.0031308 {
                            linear * 12.92
                        } else {
                            1.055 * linear.powf(1. / 2.4) - 0.055
                        };
                        (gamma * 255.).round() as u8
                    }
                    18 => 64,
                    19 => 192,
                    20 => 96,
                    21 => 64,
                    22 => 128,
                    23 => 32,
                    24 => (64. + 128. * x as f32 / (width - 1) as f32).round() as u8,
                    27 => 128,
                    28 => 92,
                    26 => {
                        if x >= width as usize - 2 {
                            255
                        } else {
                            0
                        }
                    }
                    25 => {
                        if (x / 4) % 2 == 0 {
                            64
                        } else {
                            192
                        }
                    }
                    14 => {
                        if (x / 4) % 2 == 0 {
                            0
                        } else {
                            255
                        }
                    }
                    _ => {
                        if x < width as usize / 2 {
                            0
                        } else {
                            255
                        }
                    }
                };
                assert!(
                    pixel[0].abs_diff(expected) <= 2,
                    "mode={mode} x={x}: {pixel:?}, expected={expected}"
                );
                if mode == 20 {
                    assert!(
                        pixel[1].abs_diff(32) <= 2 && pixel[2].abs_diff(32) <= 2,
                        "半透明地表の背景係数が不正: {pixel:?}"
                    );
                }
                if mode == 21 {
                    assert_eq!(&pixel[1..3], &[0, 0], "不透明な地表へ背景陰影が漏れた");
                }
                assert_eq!(pixel[3], 255);
            }
        }
    }
}
