use crate::{
    selection::Camera,
    shared_data::{Key, Packet, WORLD},
};
use eframe::{
    egui,
    egui_wgpu::{Callback, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor},
};
use glam::{Mat4, Vec3};
use std::sync::Arc;
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
pub struct Tile {
    pub key: Key,
    pub bytes: usize,
    pub controls: usize,
    pub compressed_bytes: usize,
    pub n: u32,
    pub origin: [f64; 2],
    pub side: f64,
    compute: wgpu::BindGroup,
    view: wgpu::Buffer,
    output: wgpu::Buffer,
    density: Vec<u32>,
    vertices: wgpu::Buffer,
    triangles: wgpu::Buffer,
    triangle_indices: u32,
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
    landcover: LandcoverGpu,
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
    metadata: wgpu::Buffer,
    bindings: Vec<wgpu::BindGroup>,
    patches: Vec<wgpu::Buffer>,
    records: Vec<Record>,
    plans: Vec<crate::shared_patches::Plan>,
    last_plan: std::time::Instant,
    camera: [f64; 7],
    triangles: u64,
    visible_patches: usize,
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
            wgpu::TextureFormat::Depth24PlusStencil8
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        },
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | if depth {
                wgpu::TextureUsages::empty()
            } else {
                wgpu::TextureUsages::TEXTURE_BINDING
            },
        view_formats: &[],
    })
}
impl Renderer {
    pub fn new(state: RenderState) -> Self {
        let d = &state.device;
        let source = include_str!("../../terrain/sibson_gpu_controls.wgsl");
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
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState {
                    front: wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::Always,
                        pass_op: wgpu::StencilOperation::Replace,
                        ..Default::default()
                    },
                    back: wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::Always,
                        pass_op: wgpu::StencilOperation::Replace,
                        ..Default::default()
                    },
                    read_mask: 255,
                    write_mask: 255,
                },
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        };
        let pipeline = d.create_render_pipeline(&descriptor);
        let stencil = &mut descriptor.depth_stencil.as_mut().unwrap().stencil;
        stencil.front.compare = wgpu::CompareFunction::Equal;
        stencil.back.compare = wgpu::CompareFunction::Equal;
        let fallback = d.create_render_pipeline(&descriptor);
        descriptor.vertex.entry_point = Some("control_vs");
        descriptor.fragment.as_mut().unwrap().entry_point = Some("control_fs");
        descriptor.depth_stencil.as_mut().unwrap().bias = wgpu::DepthBiasState {
            constant: -2,
            slope_scale: -1.,
            clamp: 0.,
        };
        let control_fallback = d.create_render_pipeline(&descriptor);
        descriptor
            .depth_stencil
            .as_mut()
            .unwrap()
            .stencil
            .front
            .compare = wgpu::CompareFunction::Always;
        descriptor
            .depth_stencil
            .as_mut()
            .unwrap()
            .stencil
            .back
            .compare = wgpu::CompareFunction::Always;
        let control = d.create_render_pipeline(&descriptor);
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
        Self {
            indices,
            landcover,
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
        // The shader composites premultiplied texture colors over terrain.
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
        self.landcover.slots = [None, None, None];
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
    pub fn upload(&mut self, p: &Packet, n: u32) -> anyhow::Result<Arc<Tile>> {
        let d = &self.state.device;
        let limit = d.limits().max_storage_buffer_binding_size as usize;
        anyhow::ensure!(
            p.sections.iter().all(|v| v.len().max(4) <= limit),
            "GPU補助情報が機器の上限を超えます"
        );
        let controls = p
            .sections
            .iter()
            .map(|bytes| {
                d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("制御点と補助情報"),
                    contents: if bytes.is_empty() { &[0; 4] } else { bytes },
                    usage: wgpu::BufferUsages::STORAGE,
                })
            })
            .collect::<Vec<_>>();
        let size = (n as u64 + 1).pow(2) * 32;
        let output = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU曲面"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        // bounds, lookup, counts, settings are four aligned vec4 records.
        let mut params = Vec::new();
        for v in [0f32, 0., p.side as f32 / 1000., p.side as f32 / 1000.] {
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
            bytes: p.bytes() + size as usize,
            controls: p.controls,
            compressed_bytes: p.compressed_bytes,
            n,
            origin: p.origin,
            side: p.side,
            compute,
            view,
            output,
            density: p.density.clone(),
            vertices: d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("制御点XYZ"),
                contents: bytemuck::cast_slice(&p.vertices),
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
        self.state.queue.submit([encoder.finish()]);
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
        previous: Vec<Arc<Tile>>,
        morph: f32,
        tin: bool,
        rect: egui::Rect,
        ppp: f32,
        landcover_enabled: bool,
        landcover_animate: bool,
        landcover_bilinear: bool,
    ) -> egui::PaintCallback {
        let tiles: Vec<_> = tiles
            .into_iter()
            .filter(|t| camera.projected(t.key).0)
            .collect();
        let roots: Vec<_> = roots
            .into_iter()
            .filter(|t| camera.projected(t.key).0)
            .collect();
        let size = [
            (rect.width() * ppp).max(1.) as u32,
            (rect.height() * ppp).max(1.) as u32,
        ];
        if size != self.size {
            self.state.renderer.write().free_texture(&self.texture);
            self.color = target(&self.state.device, size, false);
            self.depth = target(&self.state.device, size, true);
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
                tiles,
                landcover_group: self.landcover.group.clone(),
                landcover_blend: self.landcover_blend(landcover_enabled && landcover_animate),
                landcover_water_only: self.landcover.visible_layers.is_some_and(|(layers, _, _)| layers == [false, true]),
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
    landcover_group: wgpu::BindGroup,
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
                    f32::from(self.landcover_water_only),
                    f32::from(!self.landcover_bilinear),
                    0.,
                ],
            }),
        );
        let eye = camera.back() * camera.distance + glam::DVec3::new(0., camera.height, 0.);
        let view = Mat4::look_at_rh(
            eye.as_vec3(),
            Vec3::new(0., camera.height as f32, 0.),
            Vec3::Y,
        );
        let far = (camera.distance * 8. + 8_000_000.) as f32;
        let projection = Mat4::perspective_rh(
            std::f32::consts::FRAC_PI_4,
            (camera.viewport[0] / camera.viewport[1]) as f32,
            (camera.distance / 10000.).max(0.2) as f32,
            far,
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
            return Vec::new();
        }
        let current = self.tiles.len() + self.roots.len();
        let inputs = all[..current]
            .iter()
            .map(|t| crate::shared_patches::Input {
                origin: t.origin,
                side: t.side,
                n: t.n,
                density: t.density.clone(),
            })
            .collect::<Vec<_>>();
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
                    neighbors: [0; 4],
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
                metadata,
                bindings,
                patches,
                records,
                plans: Vec::new(),
                last_plan: std::time::Instant::now() - std::time::Duration::from_secs(1),
                camera: [f64::NAN; 7],
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
        ];
        if cache.plans.is_empty()
            || (pose != cache.camera
                && cache.last_plan.elapsed() >= std::time::Duration::from_millis(100))
        {
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
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
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
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Store,
                }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_stencil_reference(1);
        pass.set_bind_group(1, &self.landcover_group, &[]);
        for (i, plan) in cache.plans.iter().enumerate() {
            let fallback = i >= self.tiles.len();
            pass.set_stencil_reference(if fallback { 0 } else { 1 });
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
    fn shared_gpu_shaders_validate() {
        for source in [
            include_str!("../../terrain/sibson_gpu_controls.wgsl"),
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
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("実GPU");
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
        // Two adjacent current fields at different cache resolutions, and their previous versions.
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
