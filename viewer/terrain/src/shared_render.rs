use crate::{
    selection::Camera,
    shared_data::{Key, Packet, WORLD},
};
use eframe::{
    egui,
    egui_wgpu::{Callback, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor},
};
use glam::{Mat4, Vec3};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicUsize, Ordering},
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct View {
    matrix: [[f32; 4]; 4],
    offset: [f32; 4],
    settings: [f32; 4],
    flags: [u32; 4],
    extra: [u32; 4],
}
struct SourceBuffers {
    sections: Vec<wgpu::Buffer>,
    bytes: usize,
}
#[derive(Default)]
struct UploadBudget {
    jobs: AtomicUsize,
    samples: AtomicUsize,
}
impl UploadBudget {
    fn available(&self, n: u32) -> bool {
        self.jobs.load(Ordering::Acquire) < 4
            && self.samples.load(Ordering::Acquire) + (n as usize + 1).pow(2) <= (513usize).pow(2)
    }
    fn start(&self, n: u32) {
        self.jobs.fetch_add(1, Ordering::AcqRel);
        self.samples
            .fetch_add((n as usize + 1).pow(2), Ordering::AcqRel);
    }
    fn finish(&self, n: u32) {
        self.samples
            .fetch_sub((n as usize + 1).pow(2), Ordering::AcqRel);
        self.jobs.fetch_sub(1, Ordering::AcqRel);
    }
}
pub struct Tile {
    pub key: Key,
    pub bytes: usize,
    pub controls: usize,
    pub compressed_bytes: usize,
    pub n: u32,
    pub origin: [f64; 2],
    pub side: f64,
    local: bool,
    compute: wgpu::BindGroup,
    _sources: Arc<SourceBuffers>,
    view: wgpu::Buffer,
    output: wgpu::Buffer,
    density: Vec<u32>,
    vertices: wgpu::Buffer,
    triangles: wgpu::Buffer,
    triangle_indices: u32,
}

impl Tile {
    pub fn local_field(&self) -> bool {
        self.local
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LandcoverView {
    areas: [[f32; 4]; 3],
    settings: [f32; 4],
}

struct LandcoverGpu {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    blank: wgpu::TextureView,
    slots: [Option<(wgpu::TextureView, crate::landcover::Bounds)>; 3],
    detail_z: Option<u32>,
    external: bool,
    visible_layers: Option<crate::landcover::LayerVisibility>,
    transition: Option<std::time::Instant>,
    group: wgpu::BindGroup,
}

impl LandcoverGpu {
    fn texture(device: &wgpu::Device, size: [usize; 2]) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("地表分類"),
            size: wgpu::Extent3d {
                width: size[0] as u32,
                height: size[1] as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn new(device: &wgpu::Device) -> Self {
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
            label: Some("地表分類"),
            entries: &[
                texture(0),
                texture(1),
                texture(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("地表分類の座標範囲"),
            size: std::mem::size_of::<LandcoverView>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blank = Self::texture(device, [1, 1]).create_view(&Default::default());
        let group = Self::bind(
            device,
            &layout,
            &sampler,
            &uniform,
            [&blank, &blank, &blank],
        );
        Self {
            layout,
            sampler,
            uniform,
            blank,
            slots: [None, None, None],
            detail_z: None,
            external: false,
            visible_layers: None,
            transition: None,
            group,
        }
    }

    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        uniform: &wgpu::Buffer,
        textures: [&wgpu::TextureView; 3],
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("地表分類"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(textures[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(textures[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(textures[2]),
                },
            ],
        })
    }

    fn refresh(&mut self, device: &wgpu::Device) {
        let textures =
            std::array::from_fn(|i| self.slots[i].as_ref().map_or(&self.blank, |(view, _)| view));
        self.group = Self::bind(device, &self.layout, &self.sampler, &self.uniform, textures);
    }
}
pub struct Renderer {
    pub state: RenderState,
    sources: std::collections::HashMap<Key, Weak<SourceBuffers>>,
    uploads: Arc<UploadBudget>,
    wake: egui::Context,
    landcover: LandcoverGpu,
    pub direct: Option<Vec<crate::direct_material::Quad>>,
    pub point_state: Arc<std::sync::Mutex<crate::point_visibility::State>>,
    pub hit_state: Arc<std::sync::Mutex<crate::hit_test::State>>,
    point_renderer: Arc<crate::point_visibility::Renderer>,
    direct_renderer: Arc<crate::direct_material::Renderer>,
    positions: wgpu::Texture,
    positions_view: wgpu::TextureView,
    lighting_source: wgpu::Texture,
    pub texture: egui::TextureId,
    color: wgpu::Texture,
    depth: wgpu::Texture,
    size: [u32; 2],
    compute: wgpu::ComputePipeline,
    indices: Arc<std::collections::HashMap<u32, wgpu::Buffer>>,

    pipeline: wgpu::RenderPipeline,
    fallback: wgpu::RenderPipeline,
    control: wgpu::RenderPipeline,
    control_fallback: wgpu::RenderPipeline,
}
fn storage_layout(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Record {
    bounds: [f32; 4],
    data: [u32; 4],
    neighbors: [u32; 4],
}
struct CachedScene {
    tiles: Vec<Arc<Tile>>,
    _atlas: wgpu::Buffer,
    _links: wgpu::Buffer,
    metadata: wgpu::Buffer,
    bindings: Vec<wgpu::BindGroup>,
    patches: Vec<wgpu::Buffer>,
    records: Vec<Record>,
    plans: Vec<crate::shared_patches::Plan>,
    last_plan: std::time::Instant,
    camera: [f64; 10],
    triangles: u64,
    visible_patches: usize,
}
fn atlas_fits(current: u64, previous: u64, limit: u32) -> bool {
    current.saturating_add(previous).saturating_mul(32) <= u64::from(limit)
}
fn lighting_target(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("陰影適用前の画面色"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
fn position_target(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("画面ピクセルの地理座標"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
fn target(device: &wgpu::Device, size: [u32; 2], depth: bool) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("全国共有曲面"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if depth {
            wgpu::TextureFormat::Depth32Float
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        },
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | if depth {
                wgpu::TextureUsages::empty()
            } else {
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC
            },
        view_formats: &[],
    })
}
impl Renderer {
    pub fn new(state: RenderState, ctx: &egui::Context) -> Self {
        let d = &state.device;
        let source = include_str!("sibson_gpu_controls.wgsl");
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sibson GPU"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let compute = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("制御点から曲面を評価"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let render_shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("共有曲面描画"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shared_render.wgsl").into()),
        });
        let bind_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("共有曲面描画データ"),
            entries: &[
                storage_layout(0),
                storage_layout(2),
                storage_layout(3),
                storage_layout(4),
                storage_layout(5),
                storage_layout(6),
                storage_layout(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let landcover = LandcoverGpu::new(d);
        let direct_renderer = Arc::new(crate::direct_material::Renderer::new(d));
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_layout, &landcover.layout],
            push_constant_ranges: &[],
        });
        let mut descriptor = wgpu::RenderPipelineDescriptor {
            label: Some("全国共有曲面"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            multiview: None,
            cache: None,
        };
        let pipeline = d.create_render_pipeline(&descriptor);
        let fallback = pipeline.clone();
        descriptor.vertex.entry_point = Some("control_vs");
        descriptor.fragment.as_mut().unwrap().entry_point = Some("control_fs");
        descriptor.depth_stencil.as_mut().unwrap().bias = wgpu::DepthBiasState {
            constant: 2,
            slope_scale: 1.,
            clamp: 0.,
        };
        let control = d.create_render_pipeline(&descriptor);
        let control_fallback = control.clone();
        let size = [1, 1];
        let color = target(d, size, false);
        let depth = target(d, size, true);
        let texture = state.renderer.write().register_native_texture(
            d,
            &color.create_view(&Default::default()),
            wgpu::FilterMode::Linear,
        );
        let indices = Arc::new(
            (0..=9)
                .map(|level| {
                    let n = 1u32 << level;
                    let mut ids = Vec::with_capacity((6 * n * n) as usize);
                    for y in 0..n {
                        for x in 0..n {
                            let a = y * (n + 1) + x;
                            ids.extend([a, a + 1, a + n + 2, a, a + n + 2, a + n + 1]);
                        }
                    }
                    (
                        n,
                        d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("共有分割テンプレート"),
                            contents: bytemuck::cast_slice(&ids),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                    )
                })
                .collect(),
        );
        let positions = position_target(d, size);
        let positions_view = positions.create_view(&Default::default());
        Self {
            sources: Default::default(),
            uploads: Default::default(),
            wake: ctx.clone(),
            indices,
            landcover,
            direct: None,
            point_state: Default::default(),
            hit_state: Default::default(),
            point_renderer: Arc::new(crate::point_visibility::Renderer::new(d)),
            direct_renderer,
            positions,
            positions_view,
            lighting_source: lighting_target(d, size),
            state,
            texture,
            color,
            depth,
            size,
            compute,

            pipeline,
            fallback,
            control,
            control_fallback,
        }
    }
    pub fn set_external_materials(
        &mut self,
        views: [Option<(wgpu::TextureView, crate::landcover::Bounds)>; 3],
    ) {
        self.landcover.slots = views;
        self.landcover.external = true;
        self.landcover.transition = None;
        self.landcover.visible_layers = None;
        self.landcover.refresh(&self.state.device);
    }
    pub fn upload_landcover(
        &mut self,
        mut image: crate::landcover::Image,
        overview: bool,
        animate: bool,
    ) {
        let size = image.request.size;
        let visible = image.request.visibility();
        let visibility_changed = self
            .landcover
            .visible_layers
            .is_some_and(|old| old != visible);
        if visibility_changed {
            self.clear_landcover();
        }
        self.landcover.visible_layers = Some(visible);
        // 乗算済みアルファの地表色をシェーダーで地形へ合成する。
        image.premultiply_for_display();
        let texture = LandcoverGpu::texture(&self.state.device, size);
        self.state.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] as u32 * 4),
                rows_per_image: Some(size[1] as u32),
            },
            texture.size(),
        );
        if !overview {
            let old_z = self
                .landcover
                .detail_z
                .or_else(|| self.landcover.slots[0].as_ref().map(|_| 6));
            if animate && !visibility_changed && old_z.is_some_and(|z| z != image.lod()) {
                self.landcover.slots[2] = self.landcover.slots[1].take();
                self.landcover.transition = Some(std::time::Instant::now());
            }
            self.landcover.detail_z = Some(image.lod());
        }
        self.landcover.slots[usize::from(!overview)] = Some((
            texture.create_view(&Default::default()),
            image.request.bounds,
        ));
        self.landcover.refresh(&self.state.device);
    }

    pub fn landcover_blend(&self, animate: bool) -> f32 {
        crate::landcover::fade(self.landcover.transition, animate)
    }

    pub fn finish_landcover_transition(&mut self) {
        self.landcover.transition = None;
        if self.landcover.slots[2].take().is_some() {
            self.landcover.refresh(&self.state.device);
        }
    }

    pub fn clear_landcover(&mut self) {
        if !self.landcover.external
            && self.landcover.slots.iter().all(Option::is_none)
            && self.landcover.visible_layers.is_none()
        {
            return;
        }
        self.landcover.slots = [None, None, None];
        self.landcover.external = false;
        self.landcover.detail_z = None;
        self.landcover.visible_layers = None;
        self.landcover.transition = None;
        self.landcover.refresh(&self.state.device);
    }

    pub fn metrics(&self) -> (u64, usize, usize) {
        let renderer = self.state.renderer.read();
        renderer
            .callback_resources
            .get::<CachedScene>()
            .map(|cache| {
                (
                    cache.triangles,
                    cache.visible_patches,
                    cache._atlas.size() as usize
                        + cache.metadata.size() as usize
                        + cache
                            .patches
                            .iter()
                            .map(|b| b.size() as usize)
                            .sum::<usize>(),
                )
            })
            .unwrap_or((0, 0, 0))
    }
    pub fn sample_budget(&self) -> u64 {
        // 新旧曲面を同時に保持し、残り1/8を背景と接続用に残す。
        (u64::from(self.state.device.limits().max_storage_buffer_binding_size) / 32 * 7 / 8 / 2)
            .min(2_000_000)
    }
    pub fn can_upload(&self, n: u32) -> bool {
        let _ = self.state.device.poll(wgpu::Maintain::Poll);
        self.uploads.available(n)
    }
    pub fn source_bytes(&self) -> usize {
        self.sources
            .values()
            .filter_map(Weak::upgrade)
            .map(|s| s.bytes)
            .sum()
    }
    pub fn reset_sources(&mut self) {
        self.sources.clear();
    }
    pub fn upload(&mut self, p: &Packet, n: u32) -> anyhow::Result<Arc<Tile>> {
        self.upload_field(p, n, crate::shared_field::Field::default())
    }
    pub fn upload_field(
        &mut self,
        p: &Packet,
        n: u32,
        field: crate::shared_field::Field,
    ) -> anyhow::Result<Arc<Tile>> {
        let [ox, oy, side] = field.relative(p.side);
        let vertices = p
            .vertices
            .iter()
            .map(|v| [v[0] - ox as f32, v[1] - oy as f32, v[2], v[3]])
            .collect::<Vec<_>>();
        let mut density = vec![0u32; 1024];
        for v in &vertices {
            if v[0] >= 0. && v[1] >= 0. && v[0] as f64 <= side && v[1] as f64 <= side {
                let x = (v[0] as f64 / side * 32.).floor().clamp(0., 31.) as usize;
                let y = (v[1] as f64 / side * 32.).floor().clamp(0., 31.) as usize;
                density[y * 32 + x] += 1;
            }
        }
        let d = &self.state.device;
        let limit = d.limits().max_storage_buffer_binding_size as usize;
        anyhow::ensure!(
            p.sections.iter().all(|v| v.len().max(4) <= limit),
            "GPU補助情報が機器の上限を超えます"
        );
        let density = if field.depth == 0 {
            p.density.clone()
        } else {
            density
        };
        let sources = if let Some(buffers) = self.sources.get(&p.key).and_then(Weak::upgrade) {
            buffers
        } else {
            let buffers = Arc::new(SourceBuffers {
                sections: p
                    .sections
                    .iter()
                    .map(|bytes| {
                        d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("共有する制御点・補助情報"),
                            contents: if bytes.is_empty() { &[0; 4] } else { bytes },
                            usage: wgpu::BufferUsages::STORAGE,
                        })
                    })
                    .collect(),
                bytes: p.sections.iter().map(|b| b.len().max(4)).sum(),
            });
            self.sources.insert(p.key, Arc::downgrade(&buffers));
            buffers
        };
        let controls = &sources.sections;
        let size = (n as u64 + 1).pow(2) * 32;
        let output = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU曲面"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        // bounds・lookup・counts・settingsを16byte境界の4レコードに並べる。
        let mut params = Vec::new();
        for v in [
            ox as f32 / 1000.,
            oy as f32 / 1000.,
            (ox + side) as f32 / 1000.,
            (oy + side) as f32 / 1000.,
        ] {
            params.extend(v.to_le_bytes());
        }
        for v in p.index_bounds {
            params.extend(v.to_le_bytes());
        }
        for v in [n, 0, 0, 0u32] {
            params.extend(v.to_le_bytes());
        }
        for v in [1000f32, 1f32, 0., 0.] {
            params.extend(v.to_le_bytes());
        }
        let uniform = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("補間設定"),
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let compute = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.compute.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: controls[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: controls[1].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: controls[2].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: controls[3].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let view = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("タイルカメラ"),
            size: std::mem::size_of::<View>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tile = Arc::new(Tile {
            key: p.key,
            bytes: p.vertices.len() * 16
                + p.triangles.len() * 4
                + size as usize
                + std::mem::size_of::<View>()
                + 64,
            controls: p.controls,
            compressed_bytes: p.compressed_bytes,
            n,
            origin: [p.origin[0] + ox, p.origin[1] + oy],
            side,
            local: field.depth > 0,
            compute,
            _sources: sources,
            view,
            output,
            density,
            vertices: d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("制御点XYZ"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::STORAGE,
            }),
            triangles: d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("制御点TIN"),
                contents: bytemuck::cast_slice(&p.triangles),
                usage: wgpu::BufferUsages::STORAGE,
            }),
            triangle_indices: p.triangles.len() as u32,
        });
        let mut encoder = d.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("曲面生成"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("自然近傍補間"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &tile.compute, &[]);
            pass.dispatch_workgroups((tile.n + 1).div_ceil(8), (tile.n + 1).div_ceil(8), 1);
        }
        self.uploads.start(n);
        self.state.queue.submit([encoder.finish()]);
        let uploads = self.uploads.clone();
        let ctx = self.wake.clone();
        self.state.queue.on_submitted_work_done(move || {
            uploads.finish(n);
            ctx.request_repaint();
        });
        Ok(tile)
    }
    pub fn frame(
        &mut self,
        tiles: Vec<Arc<Tile>>,
        roots: Vec<Arc<Tile>>,
        camera: &Camera,
        settings: [f32; 3],
        smooth: bool,
        wire: u32,
        borders: bool,
        colors: bool,
        join: bool,
        mut previous: Vec<Arc<Tile>>,
        mut morph: f32,
        tin: bool,
        rect: egui::Rect,
        ppp: f32,
        landcover_enabled: bool,
        landcover_animate: bool,
        landcover_bilinear: bool,
    ) -> egui::PaintCallback {
        let tiles: Vec<_> = tiles
            .into_iter()
            .filter(|t| {
                camera
                    .visible_depth([
                        t.origin[0],
                        t.origin[1],
                        t.origin[0] + t.side,
                        t.origin[1] + t.side,
                    ])
                    .is_some()
            })
            .collect();
        let mut unique = std::collections::HashSet::new();
        let mut roots: Vec<_> = roots
            .into_iter()
            .filter(|t| {
                unique.insert((
                    t.key,
                    t.origin[0].to_bits(),
                    t.origin[1].to_bits(),
                    t.side.to_bits(),
                ))
            })
            .filter(|t| {
                !tiles.iter().any(|current| {
                    current.key == t.key && current.origin == t.origin && current.side == t.side
                })
            })
            .filter(|t| {
                camera
                    .visible_depth([
                        t.origin[0],
                        t.origin[1],
                        t.origin[0] + t.side,
                        t.origin[1] + t.side,
                    ])
                    .is_some()
            })
            .collect();
        roots.sort_by(|a, b| {
            b.key
                .0
                .cmp(&a.key.0)
                .then_with(|| a.side.total_cmp(&b.side))
        });
        previous.retain(|t| {
            camera
                .visible_depth([
                    t.origin[0],
                    t.origin[1],
                    t.origin[0] + t.side,
                    t.origin[1] + t.side,
                ])
                .is_some()
        });
        let current_samples = tiles
            .iter()
            .chain(&roots)
            .map(|tile| u64::from(tile.n + 1).pow(2))
            .sum::<u64>();
        let old_samples = previous
            .iter()
            .map(|tile| u64::from(tile.n + 1).pow(2))
            .sum::<u64>();
        if !atlas_fits(
            current_samples,
            old_samples,
            self.state.device.limits().max_storage_buffer_binding_size,
        ) {
            // 精細化やLOD変更前の曲面だけで上限を超える場合は、今回の切替を完了させる。
            eprintln!(
                "GPU曲面アトラス上限: current={current_samples}, old={old_samples}。旧曲面を解放して切替"
            );
            previous.clear();
            morph = 1.;
        }
        let size = [
            (rect.width() * ppp).max(1.) as u32,
            (rect.height() * ppp).max(1.) as u32,
        ];
        if size != self.size {
            self.state.renderer.write().free_texture(&self.texture);
            self.color = target(&self.state.device, size, false);
            self.depth = target(&self.state.device, size, true);
            self.positions = position_target(&self.state.device, size);
            self.positions_view = self.positions.create_view(&Default::default());
            self.lighting_source = lighting_target(&self.state.device, size);
            self.texture = self.state.renderer.write().register_native_texture(
                &self.state.device,
                &self.color.create_view(&Default::default()),
                wgpu::FilterMode::Linear,
            );
            self.size = size;
        }
        Callback::new_paint_callback(
            rect,
            Frame {
                point_state: self.point_state.clone(),
                hit_state: self.hit_state.clone(),
                hit_texture: self.positions.clone(),
                point_renderer: self.point_renderer.clone(),
                direct: self.direct.clone(),
                direct_renderer: self.direct_renderer.clone(),
                positions: self.positions_view.clone(),
                color_texture: self.color.clone(),
                lighting_source: self.lighting_source.clone(),
                size: self.size,
                tiles,
                landcover_group: self.landcover.group.clone(),
                landcover_external: self.landcover.external,
                landcover_blend: self.landcover_blend(landcover_enabled && landcover_animate),
                landcover_water_only: self
                    .landcover
                    .visible_layers
                    .is_some_and(|(layers, _, _, _)| layers == [false, true]),
                landcover_uniform: self.landcover.uniform.clone(),
                landcover_bilinear,
                landcover_bounds: std::array::from_fn(|i| {
                    if landcover_enabled {
                        self.landcover.slots[i].as_ref().map(|(_, b)| *b)
                    } else {
                        None
                    }
                }),
                roots,
                camera: camera.clone(),
                settings,
                smooth,
                wire,
                borders,
                colors,
                join,
                previous,
                morph,
                tin,
                color: self.color.create_view(&Default::default()),
                depth: self.depth.create_view(&Default::default()),
                pipeline: self.pipeline.clone(),
                indices: self.indices.clone(),
                fallback: self.fallback.clone(),
                control: self.control.clone(),
                control_fallback: self.control_fallback.clone(),
            },
        )
    }
}
struct Frame {
    hit_state: Arc<std::sync::Mutex<crate::hit_test::State>>,
    hit_texture: wgpu::Texture,
    point_state: Arc<std::sync::Mutex<crate::point_visibility::State>>,
    point_renderer: Arc<crate::point_visibility::Renderer>,
    direct: Option<Vec<crate::direct_material::Quad>>,
    direct_renderer: Arc<crate::direct_material::Renderer>,
    positions: wgpu::TextureView,
    color_texture: wgpu::Texture,
    lighting_source: wgpu::Texture,
    size: [u32; 2],
    landcover_group: wgpu::BindGroup,
    landcover_external: bool,
    landcover_uniform: wgpu::Buffer,
    landcover_bounds: [Option<crate::landcover::Bounds>; 3],
    landcover_blend: f32,
    landcover_water_only: bool,
    landcover_bilinear: bool,
    indices: Arc<std::collections::HashMap<u32, wgpu::Buffer>>,
    tiles: Vec<Arc<Tile>>,
    roots: Vec<Arc<Tile>>,
    camera: Camera,
    settings: [f32; 3],
    smooth: bool,
    wire: u32,
    borders: bool,
    colors: bool,
    join: bool,
    previous: Vec<Arc<Tile>>,
    morph: f32,
    tin: bool,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    fallback: wgpu::RenderPipeline,
    control: wgpu::RenderPipeline,
    control_fallback: wgpu::RenderPipeline,
}
impl CallbackTrait for Frame {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // 描画が続く間も、完了済みのバインドグループやバッファを回収する。
        let _ = device.poll(wgpu::Maintain::Poll);
        let camera = &self.camera;
        let areas = self.landcover_bounds.map(|bounds| {
            bounds.map_or([0.; 4], |[l, t, r, b]| {
                [
                    (l - camera.center[0]) as f32,
                    (t - camera.center[1]) as f32,
                    (r - l) as f32,
                    (b - t) as f32,
                ]
            })
        });
        queue.write_buffer(
            &self.landcover_uniform,
            0,
            bytemuck::bytes_of(&LandcoverView {
                areas,
                settings: [
                    self.landcover_blend,
                    if self.landcover_external {
                        -1.
                    } else {
                        f32::from(self.landcover_water_only)
                    },
                    f32::from(!self.landcover_bilinear),
                    f32::from(self.direct.is_some()),
                ],
            }),
        );
        let world_eye = camera.eye();
        let eye = glam::DVec3::new(
            (world_eye[0] - camera.center[0]) * camera.scale(),
            world_eye[2],
            (world_eye[1] - camera.center[1]) * camera.scale(),
        );
        let view = Mat4::look_at_rh(
            eye.as_vec3(),
            Vec3::new(0., camera.height as f32, 0.),
            Vec3::Y,
        );
        let far = camera.far_depth() as f32;
        let projection = Mat4::perspective_rh(
            camera.fov_y as f32,
            (camera.viewport[0] / camera.viewport[1]) as f32,
            far,
            (camera.distance / 10000.).max(0.2) as f32,
        );
        let latitude = (std::f64::consts::PI * (1. - 2. * camera.center[1] / WORLD))
            .sinh()
            .atan();
        let scale = latitude.cos() as f32;
        let all = self
            .tiles
            .iter()
            .chain(&self.roots)
            .chain(&self.previous)
            .cloned()
            .collect::<Vec<_>>();
        if all.is_empty() {
            crate::hit_test::miss(&self.hit_state);
            return Vec::new();
        }
        let current = self.tiles.len() + self.roots.len();
        let rebuild = resources.get::<CachedScene>().is_none_or(|cache| {
            cache.tiles.len() != all.len()
                || cache
                    .tiles
                    .iter()
                    .zip(&all)
                    .any(|(a, b)| !Arc::ptr_eq(a, b))
        });
        if rebuild {
            let mut records = Vec::<Record>::new();
            let mut count = 0u32;
            for (i, t) in all.iter().enumerate() {
                records.push(Record {
                    bounds: [
                        (t.origin[0] - camera.center[0]) as f32,
                        (t.origin[1] - camera.center[1]) as f32,
                        t.side as f32,
                        if i >= self.tiles.len() && i < current {
                            1.
                        } else {
                            0.
                        },
                    ],
                    data: [count, t.n, t.key.0, 0],
                    neighbors: [0, 0, 0, 0],
                });
                count += (t.n + 1).pow(2);
            }
            let mut links = Vec::<u32>::new();
            for i in 0..records.len() {
                let a = records[i].bounds;
                records[i].neighbors[0] = links.len() as u32;
                for (j, b) in records.iter().enumerate() {
                    let pad = (a[2] / 8.)
                        .min(2. * a[2] / all[i].n as f32)
                        .max((b.bounds[2] / 8.).min(2. * b.bounds[2] / all[j].n as f32))
                        .max(1.);
                    if a[0] - pad <= b.bounds[0] + b.bounds[2]
                        && a[0] + a[2] + pad >= b.bounds[0]
                        && a[1] - pad <= b.bounds[1] + b.bounds[2]
                        && a[1] + a[2] + pad >= b.bounds[1]
                    {
                        links.push(j as u32);
                    }
                }
                records[i].data[3] = links.len() as u32 - records[i].neighbors[0];
            }
            // 背景の被覆判定には、細分化された全パッチでなく外側の矩形だけを渡す。
            for i in 0..current {
                if records[i].bounds[3] == 0. {
                    continue;
                }
                let a = records[i];
                let mut candidates = (0..current)
                    .filter(|&j| {
                        let b = records[j];
                        let preferred = b.bounds[3] < a.bounds[3]
                            || (b.bounds[3] == a.bounds[3]
                                && (b.data[2] > a.data[2]
                                    || (b.data[2] == a.data[2] && b.bounds[2] < a.bounds[2])));
                        j != i
                            && preferred
                            && a.bounds[0] < b.bounds[0] + b.bounds[2]
                            && a.bounds[0] + a.bounds[2] > b.bounds[0]
                            && a.bounds[1] < b.bounds[1] + b.bounds[2]
                            && a.bounds[1] + a.bounds[2] > b.bounds[1]
                    })
                    .collect::<Vec<_>>();
                candidates.sort_by(|&a, &b| records[b].bounds[2].total_cmp(&records[a].bounds[2]));
                let mut chosen = Vec::<usize>::new();
                for j in candidates {
                    let b = records[j].bounds;
                    if !chosen.iter().any(|&k| {
                        let c = records[k].bounds;
                        b[0] >= c[0]
                            && b[1] >= c[1]
                            && b[0] + b[2] <= c[0] + c[2]
                            && b[1] + b[2] <= c[1] + c[2]
                    }) {
                        chosen.push(j);
                    }
                }
                records[i].neighbors[2] = links.len() as u32;
                records[i].neighbors[3] = chosen.len() as u32;
                links.extend(chosen.into_iter().map(|j| j as u32));
            }
            let atlas = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("共有曲面アトラス"),
                size: u64::from(count) * 32,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            for (t, r) in all.iter().zip(&records) {
                encoder.copy_buffer_to_buffer(
                    &t.output,
                    0,
                    &atlas,
                    u64::from(r.data[0]) * 32,
                    u64::from((t.n + 1).pow(2)) * 32,
                );
            }
            let metadata = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("曲面接合情報"),
                contents: bytemuck::cast_slice(&records),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            let refs = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("局所接合候補"),
                contents: bytemuck::cast_slice(&links),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let mut bindings = Vec::new();
            let mut patches = Vec::new();
            for tile in &all[..current] {
                let patch = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("画面分割パッチ"),
                    size: 1024 * std::mem::size_of::<crate::shared_patches::Patch>() as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                bindings.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("GPUパッチと接合"),
                    layout: &self.pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: tile.output.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: tile.view.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: patch.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: atlas.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: metadata.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: refs.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: tile.vertices.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: tile.triangles.as_entire_binding(),
                        },
                    ],
                }));
                patches.push(patch);
            }
            resources.insert(CachedScene {
                tiles: all.clone(),
                _atlas: atlas,
                _links: refs,
                metadata,
                bindings,
                patches,
                records,
                plans: Vec::new(),
                last_plan: std::time::Instant::now() - std::time::Duration::from_secs(1),
                camera: [f64::NAN; 10],
                triangles: 0,
                visible_patches: 0,
            });
        }
        let cache = resources.get_mut::<CachedScene>().unwrap();
        let pose = [
            camera.center[0],
            camera.center[1],
            camera.distance,
            camera.yaw,
            camera.pitch,
            camera.viewport[0],
            camera.viewport[1],
            camera.mesh_pixels,
            camera.fov_y,
            camera.height,
        ];
        if cache.plans.is_empty()
            || (pose != cache.camera
                && cache.last_plan.elapsed() >= std::time::Duration::from_millis(100))
        {
            let inputs = all[..current]
                .iter()
                .map(|t| crate::shared_patches::Input {
                    origin: t.origin,
                    side: t.side,
                    n: t.n,
                    density: t.density.clone(),
                })
                .collect::<Vec<_>>();
            cache.plans = crate::shared_patches::plan(&inputs[..self.tiles.len()], camera);
            cache.plans.extend(crate::shared_patches::plan(
                &inputs[self.tiles.len()..],
                camera,
            ));
            for (plan, buffer) in cache.plans.iter().zip(&cache.patches) {
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(&plan.patches));
            }
            cache.last_plan = std::time::Instant::now();
            cache.camera = pose;
        }
        for (r, t) in cache.records.iter_mut().zip(&all) {
            r.bounds[0] = (t.origin[0] - camera.center[0]) as f32;
            r.bounds[1] = (t.origin[1] - camera.center[1]) as f32;
        }
        queue.write_buffer(&cache.metadata, 0, bytemuck::cast_slice(&cache.records));
        for (i, tile) in all[..current].iter().enumerate() {
            let u = View {
                matrix: (projection * view).to_cols_array_2d(),
                offset: [
                    ((tile.origin[0] - camera.center[0]) * scale as f64) as f32,
                    ((tile.origin[1] - camera.center[1]) * scale as f64) as f32,
                    scale,
                    0.,
                ],
                settings: [
                    tile.side as f32,
                    self.settings[0],
                    self.settings[1],
                    self.morph,
                ],
                flags: [tile.n, u32::from(self.smooth), self.wire, i as u32],
                extra: [
                    current as u32,
                    u32::from(self.tin),
                    u32::from(self.join),
                    u32::from(self.borders) | u32::from(self.colors) * 2,
                ],
            };
            queue.write_buffer(&tile.view, 0, bytemuck::bytes_of(&u));
        }
        cache.triangles = 0;
        cache.visible_patches = 0;
        if !self.tin && self.wire != 4 {
            cache.triangles = cache
                .plans
                .iter()
                .flat_map(|p| &p.groups)
                .map(|(n, r)| 2 * u64::from(*n).pow(2) * u64::from(r.end - r.start))
                .sum();
            cache.visible_patches = cache.plans.iter().map(|p| p.patches.len()).sum();
        }
        if self.tin || self.wire >= 3 {
            cache.triangles += all[..current]
                .iter()
                .map(|t| u64::from(t.triangle_indices) / 3)
                .sum::<u64>();
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("全国地形"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.color,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.787412,
                            g: 0.879100,
                            b: 0.955105,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.positions,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.landcover_group, &[]);
        for (i, plan) in cache.plans.iter().enumerate() {
            let fallback = i >= self.tiles.len();
            pass.set_bind_group(0, &cache.bindings[i], &[]);
            if !self.tin && self.wire != 4 {
                pass.set_pipeline(if fallback {
                    &self.fallback
                } else {
                    &self.pipeline
                });
                for (n, range) in &plan.groups {
                    pass.set_index_buffer(self.indices[n].slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..6 * n * n, 0, range.clone());
                }
            }
            if self.tin || self.wire >= 3 {
                pass.set_pipeline(if fallback {
                    &self.control_fallback
                } else {
                    &self.control
                });
                pass.draw(0..all[i].triangle_indices, 0..1);
            }
        }
        drop(pass);
        if let Some(commands) = &self.direct {
            self.direct_renderer.render(
                device,
                queue,
                encoder,
                &self.color_texture,
                &self.lighting_source,
                &self.positions,
                camera,
                self.size,
                commands,
            );
        }
        if let Some(tile) = all.first() {
            // メッシュ描画後の位置情報と、同じ曲面・境界・モーフの補間を使う。
            self.point_renderer.encode(
                device,
                encoder,
                &self.point_state,
                &self.positions,
                &cache._atlas,
                &cache.metadata,
                &cache._links,
                &tile.view,
            );
        }
        crate::hit_test::encode(&self.hit_state, device, encoder, &self.hit_texture, camera);
        Vec::new()
    }
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        _: &mut wgpu::RenderPass<'static>,
        _: &CallbackResources,
    ) {
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn transition_atlas_respects_actual_gpu_binding_limit() {
        let limit = 128 * 1024 * 1024;
        assert!(super::atlas_fits(2_000_000, 2_000_000, limit));
        assert!(!super::atlas_fits(2_200_000, 2_000_000, limit));
        assert!(super::atlas_fits(2_200_000, 0, limit));
        assert!(!super::atlas_fits(u64::MAX, 1, limit));
    }
    #[test]
    fn pending_gpu_work_cannot_grow_without_completion() {
        let budget = super::UploadBudget::default();
        assert!(budget.available(512));
        budget.start(512);
        assert!(!budget.available(32));
        budget.finish(512);
        assert!(budget.available(512));
        for _ in 0..4 {
            assert!(budget.available(32));
            budget.start(32);
        }
        assert!(!budget.available(32));
        for _ in 0..4 {
            budget.finish(32);
        }
        assert!(budget.available(512));
    }
    #[test]
    fn shared_gpu_shaders_validate() {
        for source in [
            include_str!("sibson_gpu_controls.wgsl"),
            include_str!("shared_render.wgsl"),
        ] {
            let module = naga::front::wgsl::parse_str(source).expect("WGSL parse");
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("WGSL validation");
        }
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    #[test]
    fn gpu_mixed_seams_and_morph_are_continuous() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: if cfg!(target_os = "windows") {
                wgpu::Backends::DX12
            } else {
                wgpu::Backends::PRIMARY
            },
            ..Default::default()
        });
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            eprintln!("GPUアダプターがないため実描画試験はスキップ（WGSL検査は別途実行）");
            return;
        };
        let (d, q) = pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let shader = format!(
            "{}\n{}",
            include_str!("shared_render.wgsl"),
            r#"
 @group(0) @binding(8) var<storage,read_write> result:array<vec4<f32>>;
 @compute @workgroup_size(1)fn check(@builtin(global_invocation_id)id:vec3<u32>){
  let point=vec2(100.,f32(id.x/2u)*12.5);
  result[id.x]=vec4(surface(point,id.x%2u),1.);
 }
 "#
        );
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("check"),
            compilation_options: Default::default(),
            cache: None,
        });
        let make = |bytes: &[u8], usage| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytes,
                usage,
            })
        };
        let mut records = Vec::new();
        let mut atlas = Vec::<[f32; 8]>::new();
        let mut links = Vec::<u32>::new();
        // 解像度の異なる隣接2曲面と、それぞれの直前の曲面を用意する。
        for (i, (x, n, h)) in [
            (0., 8u32, 100.),
            (100., 4u32, 120.),
            (0., 4u32, 50.),
            (100., 4u32, 50.),
        ]
        .into_iter()
        .enumerate()
        {
            records.push(Record {
                bounds: [x, 0., 100., 0.],
                data: [atlas.len() as u32, n, 8, 4],
                neighbors: [links.len() as u32, 0, 0, 0],
            });
            links.extend(0..4);
            for y in 0..=n {
                for ix in 0..=n {
                    let z = h
                        + (x + ix as f32 * 100. / n as f32) * 0.1
                        + y as f32 * 100. / n as f32 * 0.2;
                    atlas.push([z, 0.1, 0.2, 1., z, 0.1, 0.2, 1.]);
                }
            }
            assert_eq!(records.len(), i + 1);
        }
        let atlas = make(bytemuck::cast_slice(&atlas), wgpu::BufferUsages::STORAGE);
        let metadata = make(bytemuck::cast_slice(&records), wgpu::BufferUsages::STORAGE);
        let refs = make(bytemuck::cast_slice(&links), wgpu::BufferUsages::STORAGE);
        let out = d.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 18 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = d.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 18 * 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for amount in [0., 0.25, 0.5, 0.75, 1.] {
            let view = View {
                matrix: Mat4::IDENTITY.to_cols_array_2d(),
                offset: [0.; 4],
                settings: [100., 0.25, 1., amount],
                flags: [8, 1, 0, 0],
                extra: [2, 0, 1, 0],
            };
            let uniform = make(bytemuck::bytes_of(&view), wgpu::BufferUsages::UNIFORM);
            let entries = [
                (1, &uniform),
                (3, &atlas),
                (4, &metadata),
                (5, &refs),
                (8, &out),
            ]
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            });
            let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let mut encoder = d.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(18, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&out, 0, &read, 0, 18 * 16);
            q.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            read.slice(..)
                .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
            let _ = d.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().unwrap();
            {
                let bytes = read.slice(..).get_mapped_range();
                let values: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
                for pair in values.chunks_exact(2) {
                    for j in 0..3 {
                        assert!(
                            (pair[0][j] - pair[1][j]).abs() < 0.0001,
                            "seam amount={amount}: {pair:?}"
                        );
                    }
                }
                let expected = 60. + 60. * amount;
                assert!(
                    (values[0][0] - expected).abs() < 0.001,
                    "morph {:?}",
                    values[0]
                );
            }
            read.unmap();
        }
    }
}

#[cfg(test)]
mod control_compute_tests {
    use super::*;
    #[test]
    fn gpu_sibson_reproduces_affine_height_and_normal() {
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
            eprintln!("GPUなし: 曲面実演算をスキップ");
            return;
        };
        let (d, q) = pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let points = [
            [-100., -100.],
            [100., -100.],
            [-100., 100.],
            [100., 100.],
            [0., 0.],
        ]
        .into_iter()
        .map(|[x, y]| [x, y, 100. + 0.5 * x + 0.2 * y])
        .collect();
        let model = crate::natural_regional_surface::Fast::certified(points).unwrap();
        // 極値を平坦化する推定器とは分離し、既知の平面勾配でGPU評価座標を検査する。
        let gradients = vec![[0.5, 0.2]; model.base.points.len()];
        let (bytes, info) = crate::natural_regional_gpu::packet(
            &model,
            &gradients,
            [-20., -20., 20., 20.],
            serde_json::json!({}),
        )
        .unwrap();
        let mut at = 4 + u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        let controls = ["pointBytes", "cellBytes", "nodeBytes", "referenceBytes"].map(|key| {
            let n = info[key].as_u64().unwrap() as usize;
            let b = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &bytes[at..at + n],
                usage: wgpu::BufferUsages::STORAGE,
            });
            at += n;
            b
        });
        for bounds in [[-20f32, -20., 20., 20.], [-5., 2., 7., 14.]] {
            let mut params = Vec::new();
            for v in bounds.map(|v| v / 1000.) {
                params.extend(v.to_le_bytes());
            }
            for i in 0..4 {
                params.extend(
                    (info["indexBounds"][i].as_f64().unwrap() as f32 / 1000.).to_le_bytes(),
                );
            }
            for v in [4u32, 0, 0, 0] {
                params.extend(v.to_le_bytes());
            }
            for v in [1000f32, 1., 0., 0.] {
                params.extend(v.to_le_bytes());
            }
            let uniform = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let size = 25 * 32;
            let out = d.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read = d.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(include_str!("sibson_gpu_controls.wgsl").into()),
            });
            let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let entries = [
                &controls[0],
                &controls[1],
                &controls[2],
                &controls[3],
                &out,
                &uniform,
            ]
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect::<Vec<_>>();
            let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let mut encoder = d.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&out, 0, &read, 0, size);
            q.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            read.slice(..).map_async(wgpu::MapMode::Read, move |v| {
                tx.send(v).unwrap();
            });
            let _ = d.poll(wgpu::Maintain::Wait);
            rx.recv().unwrap().unwrap();
            let result = read.slice(..).get_mapped_range();
            let result: &[[f32; 8]] = bytemuck::cast_slice(&result);
            for y in 0..5 {
                for x in 0..5 {
                    let expected = 100.
                        + 0.5 * (bounds[0] + x as f32 / 4. * (bounds[2] - bounds[0]))
                        + 0.2 * (bounds[1] + y as f32 / 4. * (bounds[3] - bounds[1]));
                    let v = result[y * 5 + x];
                    assert!((v[0] - expected).abs() < 0.01, "{v:?}, expected={expected}");
                    assert!(
                        (v[1] - 0.5).abs() < 0.01 && (v[2] - 0.2).abs() < 0.01,
                        "normal {v:?}"
                    );
                    assert_eq!(v[3], 1.);
                    assert!(
                        (v[4] - expected).abs() < 0.01
                            && (v[5] - 0.5).abs() < 0.01
                            && (v[6] - 0.2).abs() < 0.01,
                        "C1 {v:?}, expected {expected}, bounds {bounds:?}"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod upload_roundtrip_tests {
    use super::*;
    #[test]
    fn reverse_depth_keeps_distant_front_faces_in_front() {
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
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
        let shader=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:None,source:wgpu::ShaderSource::Wgsl("struct P {depth:vec4<f32>} @group(0) @binding(0) var<uniform> p:P; struct V{@builtin(position) position:vec4<f32>,@location(0) color:vec4<f32>} @vertex fn vs(@builtin(vertex_index)i:u32,@builtin(instance_index)j:u32)->V {let q=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));return V(vec4(q[i],p.depth[j],1.),select(vec4(1.,0.,0.,1.),vec4(0.,0.,1.,1.),j==1u));} @fragment fn fs(v:V)->@location(0)vec4<f32>{return v.color;}".into())});
        for distance in [1000f32, 10000., 100000.] {
            for reverse in [false, true] {
                let projection = if reverse {
                    Mat4::perspective_rh(0.8, 1., 500000., 0.2)
                } else {
                    Mat4::perspective_rh(0.8, 1., 0.2, 500000.)
                };
                let values = [distance, distance + 0.1].map(|d| {
                    let p = projection * glam::vec4(0., 0., -d, 1.);
                    p.z / p.w
                });
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&[values[0], values[1], 0., 0.]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let format = if reverse {
                    wgpu::TextureFormat::Depth32Float
                } else {
                    wgpu::TextureFormat::Depth24PlusStencil8
                };
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: None,
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format,
                        depth_write_enabled: true,
                        depth_compare: if reverse {
                            wgpu::CompareFunction::GreaterEqual
                        } else {
                            wgpu::CompareFunction::LessEqual
                        },
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview: None,
                    cache: None,
                });
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    }],
                });
                let make = |format, usage| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: None,
                        size: wgpu::Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    })
                };
                let color = make(
                    wgpu::TextureFormat::Rgba8Unorm,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                );
                let depth = make(format, wgpu::TextureUsages::RENDER_ATTACHMENT);
                let read = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 256,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &color.create_view(&Default::default()),
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth.create_view(&Default::default()),
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(if reverse { 0. } else { 1. }),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.draw(0..3, 0..2);
                }
                encoder.copy_texture_to_buffer(
                    color.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &read,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: None,
                        },
                    },
                    color.size(),
                );
                queue.submit([encoder.finish()]);
                let (send, receive) = std::sync::mpsc::channel();
                read.slice(..)
                    .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
                let _ = device.poll(wgpu::Maintain::Wait);
                receive.recv().unwrap().unwrap();
                let pixels = read.slice(..).get_mapped_range();
                if reverse {
                    assert_eq!(
                        &pixels[..4],
                        &[255, 0, 0, 255],
                        "遠方の面が手前を上書き: {distance}"
                    );
                } else if distance >= 10000. {
                    assert_eq!(
                        &pixels[..4],
                        &[0, 0, 255, 255],
                        "従来の深度競合の再現: {distance}"
                    );
                }
                drop(pixels);
                read.unmap();
            }
        }
    }
    #[test]
    fn coarse_background_cannot_cover_sky_above_loaded_fine_ground() -> anyhow::Result<()> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            eprintln!("GPUがないためGPU試験を省略します");
            return Ok(());
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None))?;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let ui = eframe::egui_wgpu::Renderer::new(&device, format, None, 1, false);
        let state = RenderState {
            adapter,
            available_adapters: Vec::new(),
            device,
            queue,
            target_format: format,
            renderer: Arc::new(egui::epaint::mutex::RwLock::new(ui)),
        };
        let ctx = egui::Context::default();
        let mut renderer = Renderer::new(state.clone(), &ctx);
        let fine = renderer.upload(&Packet::constant_plane((6, 56, 25), 1500.)?, 1)?;
        let coarse = renderer.upload(&Packet::constant_plane((0, 0, 0), 2000.)?, 1)?;
        let mut camera = Camera::default();
        camera.set_observer(
            [
                fine.origin[0] + fine.side / 2.,
                fine.origin[1] + fine.side / 2.,
            ],
            1502.,
            0.,
            0.,
            0.8,
            [64., 64.],
        );
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64., 64.));
        for tin in [true, false] {
            let callback = renderer.frame(
                vec![fine.clone()],
                vec![coarse.clone()],
                &camera,
                [0.25, 1., 0.],
                true,
                0,
                false,
                false,
                false,
                Vec::new(),
                1.,
                tin,
                rect,
                1.,
                false,
                false,
                true,
            );
            let jobs = vec![egui::ClippedPrimitive {
                clip_rect: rect,
                primitive: egui::epaint::Primitive::Callback(callback),
            }];
            let mut encoder = state.device.create_command_encoder(&Default::default());
            let commands = state.renderer.write().update_buffers(
                &state.device,
                &state.queue,
                &mut encoder,
                &jobs,
                &ScreenDescriptor {
                    size_in_pixels: [64, 64],
                    pixels_per_point: 1.,
                },
            );
            let buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 512,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &renderer.positions,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 32, y: 16, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &renderer.positions,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 32, y: 48, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 256,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            state.queue.submit(
                commands
                    .into_iter()
                    .chain(std::iter::once(encoder.finish())),
            );
            let (send, receive) = std::sync::mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |v| send.send(v).unwrap());
            let _ = state.device.poll(wgpu::Maintain::Wait);
            receive.recv()?.map_err(|e| anyhow::anyhow!(e))?;
            let range = buffer.slice(..).get_mapped_range();
            let pixel = bytemuck::cast_slice::<u8, f32>(&range);
            assert_eq!(pixel[3], 0., "粗い背景が上空に描画されている: {pixel:?}");
            assert!(
                (pixel[67] - 11500.).abs() < 0.1,
                "細かい地面が消えている: {}",
                pixel[67]
            );
            drop(range);
            buffer.unmap();
        }
        Ok(())
    }
    #[test]
    fn flat_observer_surface_is_zero_on_gpu() -> anyhow::Result<()> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            eprintln!("GPUがないためGPU試験を省略します");
            return Ok(());
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None))?;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let ui = eframe::egui_wgpu::Renderer::new(&device, format, None, 1, false);
        let state = RenderState {
            adapter,
            available_adapters: Vec::new(),
            device,
            queue,
            target_format: format,
            renderer: Arc::new(egui::epaint::mutex::RwLock::new(ui)),
        };
        let mut renderer = Renderer::new(state.clone(), &egui::Context::default());
        let tile = renderer.upload(&Packet::flat()?, 1)?;
        let buffer = state.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 128,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = state.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&tile.output, 0, &buffer, 0, 128);
        state.queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |v| send.send(v).unwrap());
        let _ = state.device.poll(wgpu::Maintain::Wait);
        receive.recv()?.map_err(|e| anyhow::anyhow!(e))?;
        let range = buffer.slice(..).get_mapped_range();
        let samples = bytemuck::cast_slice::<u8, f32>(&range);
        for sample in samples.chunks_exact(8) {
            assert_eq!(sample[0], 0.);
            assert_eq!(sample[4], 0.);
            assert_eq!(sample[3], 1.);
            assert_eq!(sample[7], 1.);
            assert!(sample.iter().all(|v| v.is_finite()));
        }
        drop(range);
        buffer.unmap();
        Ok(())
    }
    #[test]
    #[ignore = "実データでズーム往復時のGPU完了とバッファ共有を検証する"]
    fn real_gpu_zoom_roundtrip() -> anyhow::Result<()> {
        let path = std::env::var("YAMAMUKI_TERRAIN_BENCH_DB")?;
        let mut db = crate::shared_data::Database::open(std::path::Path::new(&path))?;
        let packet = db.packet((6, 56, 25))?;
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
            eprintln!("GPUがないためGPU試験を省略します");
            return Ok(());
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None))?;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let ui = eframe::egui_wgpu::Renderer::new(&device, format, None, 1, false);
        let state = RenderState {
            adapter,
            available_adapters: Vec::new(),
            device,
            queue,
            target_format: format,
            renderer: Arc::new(egui::epaint::mutex::RwLock::new(ui)),
        };
        let mut renderer = Renderer::new(state, &egui::Context::default());
        let mut camera = Camera {
            height: 1800.,
            ..Default::default()
        };
        let center = crate::selection::lonlat(138.7274, 35.3606).map(|v| v / WORLD);
        let mut previous = Vec::<Arc<Tile>>::new();
        for zoom in [8., 16., 24., 28., 24., 16., 8.] {
            camera.set_map_view(center, zoom, 0., 0.8, [1600., 1000.]);
            let start = std::time::Instant::now();
            let mut current = Vec::new();
            for field in crate::shared_field::Field::visible(packet.key, &camera) {
                while !renderer.can_upload(128) {
                    let _ = renderer.state.device.poll(wgpu::Maintain::Wait);
                }
                let tile = renderer.upload_field(&packet, 128, field)?;
                if let Some(first) = current.first().or(previous.first()) {
                    assert!(Arc::ptr_eq(&tile._sources, &first._sources));
                }
                current.push(tile);
            }
            let _ = renderer.state.device.poll(wgpu::Maintain::Wait);
            assert_eq!(renderer.uploads.jobs.load(Ordering::Acquire), 0);
            assert_eq!(renderer.uploads.samples.load(Ordering::Acquire), 0);
            eprintln!(
                "z{zoom}: {}区画 / GPU完了 {:.1} ms / 共有元 {} bytes",
                current.len(),
                start.elapsed().as_secs_f64() * 1000.,
                renderer.source_bytes()
            );
            previous = current;
        }
        drop(previous);
        assert_eq!(renderer.source_bytes(), 0);
        Ok(())
    }
}
