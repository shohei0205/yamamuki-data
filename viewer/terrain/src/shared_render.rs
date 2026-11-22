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
    earth: [f32; 4],
    eye: [f32; 4],
    frame: [f32; 4],
}
struct SourceBuffers {
    sections: Vec<wgpu::Buffer>,
    vertices: wgpu::Buffer,
    triangles: wgpu::Buffer,
    bytes: usize,
}
// 小区画を16件で止めず、点数上限とCPU時間予算で投入量を制御する。
pub(crate) const MAX_UPLOAD_JOBS: usize = 128;
#[derive(Default)]
struct UploadBudget {
    jobs: AtomicUsize,
    samples: AtomicUsize,
}
impl UploadBudget {
    fn available(&self, n: u32) -> bool {
        self.jobs.load(Ordering::Acquire) < MAX_UPLOAD_JOBS
            && self.samples.load(Ordering::Acquire) + (n as usize + 1).pow(2)
                <= 4 * (513usize).pow(2)
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
    pub field: crate::shared_field::Field,
    local: bool,
    compute: wgpu::BindGroup,
    _sources: Arc<SourceBuffers>,
    view: wgpu::Buffer,
    output: wgpu::Buffer,
    seam_band: f32,
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
#[derive(Clone)]
struct SharedPipelines {
    compute: wgpu::ComputePipeline,
    pipeline: wgpu::RenderPipeline,
    fast_pipeline: wgpu::RenderPipeline,
    fallback: wgpu::RenderPipeline,
    control: wgpu::RenderPipeline,
    control_fallback: wgpu::RenderPipeline,
    globe_pipeline: wgpu::RenderPipeline,
    indices: Arc<std::collections::HashMap<u32, wgpu::Buffer>>,
    direct_renderer: Arc<crate::direct_material::Renderer>,
    point_renderer: Arc<crate::point_visibility::LazyRenderer>,
    landcover_layout: wgpu::BindGroupLayout,
}
#[derive(Default)]
struct PipelineCache(Arc<std::sync::OnceLock<SharedPipelines>>);
pub struct Renderer {
    gpu_culler: Arc<std::sync::OnceLock<crate::gpu_culling::Pipelines>>,
    gpu_planner: Arc<std::sync::OnceLock<Arc<crate::gpu_planner::Planner>>>,
    pub state: RenderState,
    sources: std::collections::HashMap<Key, Weak<SourceBuffers>>,
    uploads: Arc<UploadBudget>,
    upload_batch: Option<wgpu::CommandEncoder>,
    upload_batch_sizes: Vec<u32>,
    wake: egui::Context,
    landcover: LandcoverGpu,
    pub direct: Option<Vec<crate::direct_material::Quad>>,
    pub point_state: Arc<std::sync::Mutex<crate::point_visibility::State>>,
    pub hit_state: Arc<std::sync::Mutex<crate::hit_test::State>>,
    point_renderer: Arc<crate::point_visibility::LazyRenderer>,
    direct_renderer: Arc<crate::direct_material::Renderer>,
    pub(crate) positions: wgpu::Texture,
    positions_view: wgpu::TextureView,
    lighting_source: wgpu::Texture,
    pub texture: egui::TextureId,
    pub(crate) color: wgpu::Texture,
    depth: wgpu::Texture,
    size: [u32; 2],
    compute: wgpu::ComputePipeline,
    indices: Arc<std::collections::HashMap<u32, wgpu::Buffer>>,

    pipeline: wgpu::RenderPipeline,
    fast_pipeline: wgpu::RenderPipeline,
    fallback: wgpu::RenderPipeline,
    control: wgpu::RenderPipeline,
    control_fallback: wgpu::RenderPipeline,
    globe_pipeline: wgpu::RenderPipeline,
    pub(crate) frozen_layout: Option<LayoutSnapshot>,
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
    tile: [f32; 4],
    // 元タイルの遠い端点はf64から直接変換し、大きな辺長のf32加算で復元しない。
    tile_end: [f32; 4],
    draw: [u32; 4],
    // 同じ評価曲面が旧表示にもある場合の索引+1。0は探索が必要。
    transition: [u32; 4],
    // 接合用に絞った候補の開始・件数・元タイル終端表。境界探索は従来表を使う。
    blend_data: [u32; 4],
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreviousPlan {
    pub background: bool,
    pub plan: crate::shared_patches::Plan,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct LayoutSnapshot {
    #[serde(default)]
    pub previous: Vec<Option<PreviousPlan>>,
    pub plans: Vec<crate::shared_patches::Plan>,
    pub camera: [f64; 10],
    pub render_camera: Camera,
    pub morph: f32,
    pub pixels_per_point: f32,
    #[serde(default)]
    pub render_size: [u32; 2],
    #[serde(default)]
    pub clipped_background: bool,
    #[serde(default)]
    pub force_replan: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PrepareStats {
    pub rebuild_ms: f64,
    pub plan_ms: f64,
    pub prepare_ms: f64,
    pub atlas_copy_bytes: u64,
    pub allocations: usize,
    pub bind_groups: usize,
}
struct PreparedPlan {
    plans: Result<Vec<crate::shared_patches::Plan>, String>,
    pose: [f64; 10],
    gpu: bool,
    elapsed_ms: f64,
}
// 接合候補を元タイルごとの外接矩形で絞り、全評価区画の総当たりを避ける。
struct SeamGroups {
    groups: Vec<([f32; 4], Vec<usize>)>,
}
impl SeamGroups {
    fn new(records: &[Record], keys: &[Key], densities: &[u32]) -> Self {
        let mut groups = std::collections::HashMap::<Key, ([f32; 4], Vec<usize>)>::new();
        for (i, record) in records.iter().enumerate() {
            let a = record.tile;
            let local = record.bounds;
            let pad = record.tile[3]
                .max((local[2] / 8.).min(2. * local[2] / densities[i] as f32))
                .max(1.);
            let bounds = [a[0] - pad, a[1] - pad, a[0] + a[2] + pad, a[1] + a[2] + pad];
            let entry = groups.entry(keys[i]).or_insert((
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
                Vec::new(),
            ));
            for axis in 0..2 {
                entry.0[axis] = entry.0[axis].min(bounds[axis]);
                entry.0[axis + 2] = entry.0[axis + 2].max(bounds[axis + 2]);
            }
            entry.1.push(i);
        }
        Self {
            groups: groups.into_values().collect(),
        }
    }
    fn candidates(&self, area: [f32; 4]) -> Vec<usize> {
        let mut result = Vec::new();
        for (bounds, ids) in &self.groups {
            if area[0] <= bounds[2]
                && area[2] >= bounds[0]
                && area[1] <= bounds[3]
                && area[3] >= bounds[1]
            {
                result.extend(ids);
            }
        }
        result.sort_unstable();
        result
    }
}
// 並び替えただけで描画設定を作り直さず、同じ曲面の視点設定の場所を保つ。
fn uniform_slots(ids: &[usize], previous: &[(usize, u64)], capacity: u64) -> Vec<u64> {
    let prior = previous
        .iter()
        .copied()
        .collect::<std::collections::HashMap<_, _>>();
    let mut slots = ids
        .iter()
        .map(|id| prior.get(id).copied().filter(|&s| s < capacity))
        .collect::<Vec<_>>();
    let mut used = slots
        .iter()
        .flatten()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let mut free = 0;
    for slot in &mut slots {
        if slot.is_some() {
            continue;
        }
        while used.contains(&free) {
            free += 1;
        }
        assert!(free < capacity, "視点設定の容量が不足");
        *slot = Some(free);
        used.insert(free);
    }
    slots.into_iter().map(Option::unwrap).collect()
}
struct CachedScene {
    gpu_cull: Option<crate::gpu_culling::Scene>,
    stats: PrepareStats,
    owner_texture: egui::TextureId,
    clipped_background: bool,
    plans_uploaded: bool,
    tiles: Vec<Arc<Tile>>,
    primary: usize,
    current: usize,
    render_camera: Camera,
    morph: f32,
    pixels_per_point: f32,
    _atlas: wgpu::Buffer,
    _links: wgpu::Buffer,
    metadata: wgpu::Buffer,
    views: wgpu::Buffer,
    view_bytes: Vec<u8>,
    view_stride: u64,
    view_slots: Vec<u64>,
    bindings: Vec<wgpu::BindGroup>,
    patches: Vec<wgpu::Buffer>,
    bindings_ready: Vec<bool>,
    placeholder_patch: wgpu::Buffer,
    point_patches: wgpu::Buffer,
    records: Vec<Record>,
    plans: Vec<crate::shared_patches::Plan>,
    previous_plans: Vec<Option<PreviousPlan>>,
    plan_inputs: Option<Arc<Vec<crate::shared_patches::Input>>>,
    plan_job: Option<std::sync::Mutex<std::sync::mpsc::Receiver<PreparedPlan>>>,
    plans_from_gpu: bool,
    planning_error: Option<String>,
    last_plan: std::time::Instant,
    camera: [f64; 10],
    triangles: u64,
    visible_patches: usize,
}
fn install_snapshot_layout(cache: &mut CachedScene, layout: &LayoutSnapshot, _: &wgpu::Queue) {
    if cache.plans.is_empty() && layout.plans.len() == cache.patches.len() {
        cache.plans = layout.plans.clone();
        cache.camera = layout.camera;
        cache.plans_uploaded = false;
    }
}
fn source_seam_band(packet: &Packet) -> f32 {
    let density = packet.density.iter().copied().max().unwrap_or(1).max(1) as f64;
    (2. * packet.side / (32. * density.sqrt()))
        .min(packet.side / 8.)
        .max(1.) as f32
}
// 同じ曲面は遷移前後でも一度だけ格納し、残る曲面の番地を維持する。
struct AtlasPlan {
    offsets: Vec<u32>,
    copies: Vec<usize>,
    bytes: u64,
    reuse: bool,
}
fn plan_atlas(
    entries: &[(usize, u32)],
    old: &[(usize, u32, u32)],
    capacity: u64,
    limit: u64,
) -> AtlasPlan {
    use std::collections::HashMap;
    let old = old
        .iter()
        .map(|&(id, offset, size)| (id, (offset, size)))
        .collect::<HashMap<_, _>>();
    let mut placed = HashMap::new();
    let mut ranges = Vec::new();
    for &(id, size) in entries {
        if let Some(&(offset, old_size)) = old.get(&id) {
            if size == old_size && placed.insert(id, offset).is_none() {
                ranges.push((offset, offset + size));
            }
        }
    }
    ranges.sort_unstable();
    let mut offsets = Vec::new();
    let mut copies = Vec::new();
    for (i, &(id, size)) in entries.iter().enumerate() {
        let offset = if let Some(&offset) = placed.get(&id) {
            offset
        } else {
            let mut offset = 0;
            for &(start, end) in &ranges {
                if offset + size <= start {
                    break;
                }
                offset = offset.max(end);
            }
            placed.insert(id, offset);
            ranges.push((offset, offset + size));
            ranges.sort_unstable();
            copies.push(i);
            offset
        };
        offsets.push(offset);
    }
    let mut used = ranges
        .iter()
        .map(|r| u64::from(r.1) * 32)
        .max()
        .unwrap_or(32);
    if used > limit {
        // 断片化した場合だけ詰め直す。既存バッファと別のバッファへコピーする。
        placed.clear();
        let mut cursor = 0;
        for (i, &(id, size)) in entries.iter().enumerate() {
            offsets[i] = *placed.entry(id).or_insert_with(|| {
                let offset = cursor;
                cursor += size;
                offset
            });
        }
        used = u64::from(cursor) * 32;
    }
    let reuse = used <= capacity && ranges.iter().all(|r| u64::from(r.1) * 32 <= limit);
    let bytes = if reuse {
        capacity
    } else {
        used.next_power_of_two().min(limit).max(used)
    };
    if !reuse {
        let mut seen = std::collections::HashSet::new();
        copies = entries
            .iter()
            .enumerate()
            .filter_map(|(i, &(id, _))| seen.insert(id).then_some(i))
            .collect();
    }
    AtlasPlan {
        offsets,
        copies,
        bytes,
        reuse,
    }
}
fn shared_buffer(
    device: &wgpu::Device,
    old: Option<&wgpu::Buffer>,
    bytes: u64,
    label: &str,
) -> (wgpu::Buffer, bool) {
    if let Some(old) = old.filter(|b| b.size() >= bytes) {
        return (old.clone(), true);
    }
    (
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes
                .max(32)
                .saturating_mul(if label == "局所接合候補" { 4 } else { 2 })
                .next_power_of_two()
                .min(u64::from(device.limits().max_storage_buffer_binding_size)),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        false,
    )
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
fn render_shader_source() -> String {
    format!(
        "{}\n{}\n{}",
        include_str!("shared_render.wgsl"),
        include_str!("globe.wgsl"),
        include_str!("atmosphere.wgsl")
    )
}
// 通常表示では旧スナップショットの変形を使わない。分岐をGPUへ残すだけでも
// レジスター使用量が増えるため、同じ接合式だけを使う専用プログラムにする。
fn regular_render_shader_source() -> String {
    let mut source = render_shader_source();
    let start = source.find("fn surfaceFor(").unwrap();
    let end = source[start..].find("fn surface(q:").unwrap() + start;
    source.replace_range(start..end,"fn surfaceFor(q:vec2<f32>,owner:u32,allowMorph:bool)->vec3<f32>{return blend(q,owner,false); }\n");
    source
}

impl Renderer {
    pub fn interpolation_pending(&self) -> bool {
        self.direct_renderer.interpolation_pending()
    }
    pub fn interpolation_error(&self) -> Option<String> {
        self.direct_renderer.interpolation_error()
    }
    pub fn new(state: RenderState, ctx: &egui::Context) -> Self {
        let started = std::time::Instant::now();
        let d = &state.device;
        let mut landcover = LandcoverGpu::new(d);
        let cache = {
            let mut renderer = state.renderer.write();
            if renderer.callback_resources.get::<PipelineCache>().is_none() {
                renderer.callback_resources.insert(PipelineCache::default());
            }
            renderer
                .callback_resources
                .get::<PipelineCache>()
                .unwrap()
                .0
                .clone()
        };
        let shared = cache
            .get_or_init(|| {
                std::thread::scope(|scope| {
                    // 独立したプログラムは並列に準備し、UI側の描画ロックは保持しない。
                    let compute_job = scope.spawn(|| {
                        let source = include_str!("sibson_gpu_controls.wgsl");
                        let shader = crate::gpu_profile::timed("Sibson GPU", || {
                            d.create_shader_module(wgpu::ShaderModuleDescriptor {
                                label: Some("Sibson GPU"),
                                source: wgpu::ShaderSource::Wgsl(source.into()),
                            })
                        });
                        let compute =
                            crate::gpu_profile::timed("制御点から曲面を評価", || {
                                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                                    label: Some("制御点から曲面を評価"),
                                    layout: None,
                                    module: &shader,
                                    entry_point: Some("main"),
                                    compilation_options: Default::default(),
                                    cache: None,
                                })
                            });
                        compute
                    });
                    let direct_job = scope.spawn(|| {
                        Arc::new(crate::direct_material::Renderer::new_background(d, ctx))
                    });
                    let render_shader = crate::gpu_profile::timed("共有曲面描画", || {
                        d.create_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some("共有曲面描画"),
                            source: wgpu::ShaderSource::Wgsl(render_shader_source().into()),
                        })
                    });
                    let bind_layout =
                        d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                    let (pipeline, fast_pipeline) = std::thread::scope(|scope| {
                        let fast = scope.spawn(|| {
                            let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                                label: Some("通常表示の共有曲面"),
                                source: wgpu::ShaderSource::Wgsl(
                                    regular_render_shader_source().into(),
                                ),
                            });
                            let mut regular = descriptor.clone();
                            regular.vertex.module = &shader;
                            regular.fragment.as_mut().unwrap().module = &shader;
                            crate::gpu_profile::timed("地形曲面・通常表示", || {
                                d.create_render_pipeline(&regular)
                            })
                        });
                        let pipeline =
                            crate::gpu_profile::timed("地形曲面・頂点/画素", || {
                                d.create_render_pipeline(&descriptor)
                            });
                        (
                            pipeline,
                            fast.join().expect("通常表示プログラムの準備が停止しました"),
                        )
                    });
                    let fallback = pipeline.clone();
                    descriptor.vertex.entry_point = Some("control_vs");
                    descriptor.fragment.as_mut().unwrap().entry_point = Some("control_fs");
                    descriptor.depth_stencil.as_mut().unwrap().bias = wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 1.,
                        clamp: 0.,
                    };
                    let control = crate::gpu_profile::timed("制御点ワイヤー", || {
                        d.create_render_pipeline(&descriptor)
                    });
                    let control_fallback = control.clone();
                    descriptor.vertex.entry_point = Some("globe_vs");
                    descriptor.fragment.as_mut().unwrap().entry_point = Some("globe_fs");
                    descriptor
                        .depth_stencil
                        .as_mut()
                        .unwrap()
                        .depth_write_enabled = false;
                    descriptor.depth_stencil.as_mut().unwrap().depth_compare =
                        wgpu::CompareFunction::GreaterEqual;
                    descriptor.depth_stencil.as_mut().unwrap().bias = Default::default();
                    let globe_pipeline = crate::gpu_profile::timed("地球・空・大気", || {
                        d.create_render_pipeline(&descriptor)
                    });
                    eprintln!(
                        "地形GPU初期化: 描画まで {:.2}ms",
                        started.elapsed().as_secs_f64() * 1000.
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
                    SharedPipelines {
                        compute: compute_job
                            .join()
                            .expect("曲面評価プログラムの準備が停止しました"),
                        pipeline,
                        fast_pipeline,
                        fallback,
                        control,
                        control_fallback,
                        globe_pipeline,
                        indices,
                        direct_renderer: direct_job
                            .join()
                            .expect("画像合成プログラムの準備が停止しました"),
                        point_renderer: Arc::default(),
                        landcover_layout: landcover.layout.clone(),
                    }
                })
            })
            .clone();
        let SharedPipelines {
            compute,
            pipeline,
            fast_pipeline,
            fallback,
            control,
            control_fallback,
            globe_pipeline,
            indices,
            direct_renderer,
            point_renderer,
            landcover_layout,
        } = shared;
        landcover.layout = landcover_layout;
        landcover.refresh(d);
        eprintln!(
            "地形GPU初期化: 共用プログラム準備 {:.2}ms",
            started.elapsed().as_secs_f64() * 1000.
        );
        let size = [1, 1];
        let color = target(d, size, false);
        let depth = target(d, size, true);
        let texture = state.renderer.write().register_native_texture(
            d,
            &color.create_view(&Default::default()),
            wgpu::FilterMode::Linear,
        );
        let positions = position_target(d, size);
        let positions_view = positions.create_view(&Default::default());
        Self {
            gpu_culler: Default::default(),
            gpu_planner: Default::default(),
            frozen_layout: None,
            sources: Default::default(),
            uploads: Default::default(),
            upload_batch: None,
            upload_batch_sizes: Vec::new(),
            wake: ctx.clone(),
            indices,
            landcover,
            direct: None,
            point_state: Default::default(),
            hit_state: Default::default(),
            point_renderer,
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
            fast_pipeline,
            fallback,
            control,
            control_fallback,
            globe_pipeline,
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

    pub(crate) fn planning_status(&self) -> (bool, Option<String>) {
        let renderer = self.state.renderer.read();
        renderer
            .callback_resources
            .get::<CachedScene>()
            .map(|c| (c.plan_job.is_some(), c.planning_error.clone()))
            .unwrap_or_default()
    }
    #[cfg(test)]
    pub(crate) fn gpu_plan_ready(&self) -> bool {
        let renderer = self.state.renderer.read();
        renderer
            .callback_resources
            .get::<CachedScene>()
            .is_some_and(|c| c.plans_from_gpu && c.plan_job.is_none() && c.planning_error.is_none())
    }
    #[cfg(test)]
    pub(crate) fn prepare_stats(&self) -> PrepareStats {
        self.state
            .renderer
            .read()
            .callback_resources
            .get::<CachedScene>()
            .map(|c| c.stats)
            .unwrap_or_default()
    }
    #[cfg(test)]
    pub(crate) fn current_plan_matches_reference(&self) -> Option<bool> {
        let renderer = self.state.renderer.read();
        let cache = renderer.callback_resources.get::<CachedScene>()?;
        let c = &cache.render_camera;
        let pose = [
            c.center[0],
            c.center[1],
            c.distance,
            c.yaw,
            c.pitch,
            c.viewport[0],
            c.viewport[1],
            c.mesh_pixels,
            c.fov_y,
            c.height,
        ];
        if cache.plan_job.is_some() || cache.camera != pose {
            return None;
        }
        let inputs = cache.plan_inputs.as_ref()?;
        let reference = crate::shared_patches::plan(inputs, c);
        Some(
            reference.len() == cache.plans.len()
                && reference.iter().zip(&cache.plans).all(|(a, b)| {
                    a.groups == b.groups
                        && bytemuck::cast_slice::<_, u8>(&a.patches)
                            == bytemuck::cast_slice::<_, u8>(&b.patches)
                }),
        )
    }
    #[cfg(test)]
    pub(crate) fn describe_height_point(&self, world: [f64; 2]) -> String {
        let renderer = self.state.renderer.read();
        let Some(cache) = renderer.callback_resources.get::<CachedScene>() else {
            return String::new();
        };
        let q = [
            (world[0] * WORLD - cache.render_camera.center[0]) as f32,
            (world[1] * WORLD - cache.render_camera.center[1]) as f32,
        ];
        let owner = (0..cache.current)
            .filter(|&i| {
                let r = cache.records[i];
                r.draw[1] > 0
                    && (0..2).all(|axis| {
                        q[axis] >= r.bounds[axis] - 0.01
                            && q[axis] <= r.bounds[axis] + r.bounds[2] + 0.01
                    })
            })
            .max_by(|&i, &j| {
                cache.records[i].data[2]
                    .cmp(&cache.records[j].data[2])
                    .then_with(|| cache.records[j].bounds[2].total_cmp(&cache.records[i].bounds[2]))
            });
        let Some(i) = owner else {
            return "ownerなし".into();
        };
        let r = cache.records[i];
        let parts = cache.plans[i]
            .patches
            .iter()
            .filter(|p| {
                (0..2).all(|axis| {
                    q[axis] >= r.bounds[axis] + p.bounds[axis] - 0.01
                        && q[axis] <= r.bounds[axis] + p.bounds[axis] + p.bounds[2] + 0.01
                })
            })
            .map(|p| {
                format!(
                    "bounds={:?} edges={:?} origins={:?}",
                    p.bounds, p.edges, p.origins
                )
            })
            .collect::<Vec<_>>();
        let prior = (cache.current..cache.records.len())
            .filter(|&j| {
                let r = cache.records[j];
                (0..2)
                    .all(|a| q[a] >= r.bounds[a] - 0.01 && q[a] <= r.bounds[a] + r.bounds[2] + 0.01)
            })
            .map(|j| {
                format!(
                    "{j}: {:?} {:?} n={} offset={} pad={} maxpad={}",
                    cache.tiles[j].key,
                    cache.tiles[j].field,
                    cache.records[j].data[1],
                    cache.records[j].data[0],
                    cache.records[j].tile[3],
                    f32::from_bits(cache.records[j].draw[3])
                )
            })
            .collect::<Vec<_>>();
        format!(
            "owner={i} key={:?} field={:?} n={} side={} q={q:?} parts={parts:?} prior={prior:?}",
            cache.tiles[i].key, cache.tiles[i].field, r.data[1], r.bounds[2]
        )
    }
    pub fn has_drawn_mesh(&self) -> bool {
        self.state
            .renderer
            .read()
            .callback_resources
            .get::<CachedScene>()
            .is_some_and(|c| c.owner_texture == self.texture && c.triangles > 0)
    }
    #[cfg(test)]
    pub(crate) fn candidate_counts(&self) -> (u64, u64) {
        self.state
            .renderer
            .read()
            .callback_resources
            .get::<CachedScene>()
            .map(|c| {
                c.records.iter().fold((0, 0), |(full, compact), r| {
                    (
                        full + u64::from(r.data[3]),
                        compact + u64::from(r.blend_data[1]),
                    )
                })
            })
            .unwrap_or_default()
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
                        + cache.views.size() as usize
                        + cache._links.size() as usize
                        + cache
                            .patches
                            .iter()
                            .map(|b| b.size() as usize)
                            .sum::<usize>()
                        + cache.gpu_cull.as_ref().map_or(0, |c| c.bytes())
                        + self.gpu_planner.get().map_or(0, |p| p.bytes()),
                )
            })
            .unwrap_or((0, 0, 0))
    }
    pub(crate) fn snapshot_diagnostics(&self) -> Option<crate::snapshot::DisplayDiagnostics> {
        let renderer = self.state.renderer.read();
        let c = renderer.callback_resources.get::<CachedScene>()?;
        if c.owner_texture != self.texture {
            return None;
        }
        let meshes = c
            .tiles
            .iter()
            .enumerate()
            .map(|(i, tile)| {
                let mut mesh = crate::snapshot::DiagnosticMesh::new(tile);
                if let Some(plan) = c.plans.get(i).filter(|_| i < c.current) {
                    let mut divisions = std::collections::BTreeMap::new();
                    for patch in &plan.patches {
                        *divisions.entry(patch.bounds[3] as u32).or_insert(0) += 1;
                    }
                    mesh.patch_count = Some(plan.patches.len());
                    mesh.patch_divisions = Some(divisions);
                }
                mesh
            })
            .collect();
        Some(crate::snapshot::DisplayDiagnostics {
            camera: c.render_camera.clone(),
            meshes,
            primary: c.primary,
            current: c.current,
            morph: c.morph,
        })
    }
    #[cfg(test)]
    pub(crate) fn planning_test_state(
        &self,
    ) -> Option<(Arc<Vec<crate::shared_patches::Input>>, Camera)> {
        let renderer = self.state.renderer.read();
        let cache = renderer.callback_resources.get::<CachedScene>()?;
        Some((
            cache.plan_inputs.as_ref()?.clone(),
            cache.render_camera.clone(),
        ))
    }
    #[cfg(test)]
    pub(crate) fn indirect_arguments(&self) -> Vec<(u32, wgpu::Buffer)> {
        let renderer = self.state.renderer.read();
        renderer
            .callback_resources
            .get::<CachedScene>()
            .and_then(|c| c.gpu_cull.as_ref())
            .map(|c| {
                c.draws
                    .iter()
                    .flatten()
                    .map(|d| (d.n, d.arguments.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(crate) fn snapshot_display(
        &self,
    ) -> Option<(Vec<Arc<Tile>>, usize, usize, LayoutSnapshot)> {
        let renderer = self.state.renderer.read();
        renderer
            .callback_resources
            .get::<CachedScene>()
            .filter(|c| c.owner_texture == self.texture)
            .map(|c| {
                (
                    c.tiles.clone(),
                    c.primary,
                    c.current,
                    LayoutSnapshot {
                        previous: c.previous_plans.clone(),
                        plans: c.plans.clone(),
                        camera: c.camera,
                        render_camera: c.render_camera.clone(),
                        morph: c.morph,
                        pixels_per_point: c.pixels_per_point,
                        render_size: self.size,
                        clipped_background: c.clipped_background,
                        force_replan: false,
                    },
                )
            })
    }
    pub fn sample_budget(&self) -> u64 {
        // 新旧曲面を同時に保持し、残り1/8を背景と接続用に残す。
        (u64::from(self.state.device.limits().max_storage_buffer_binding_size) / 32 * 7 / 8 / 2)
            .min(2_000_000)
    }
    pub fn begin_upload_batch(&mut self) {
        debug_assert!(self.upload_batch.is_none());
        self.upload_batch = Some(self.state.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("曲面評価をまとめて準備"),
            },
        ));
    }
    pub fn flush_upload_batch(&mut self) {
        let Some(encoder) = self.upload_batch.take() else {
            return;
        };
        let sizes = std::mem::take(&mut self.upload_batch_sizes);
        if sizes.is_empty() {
            return;
        }
        self.state.queue.submit([encoder.finish()]);
        let uploads = self.uploads.clone();
        let ctx = self.wake.clone();
        self.state.queue.on_submitted_work_done(move || {
            for n in sizes {
                uploads.finish(n);
            }
            ctx.request_repaint();
        });
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
        let d = &self.state.device;
        let limit = d.limits().max_storage_buffer_binding_size as usize;
        anyhow::ensure!(
            p.sections.iter().all(|v| v.len().max(4) <= limit),
            "GPU補助情報が機器の上限を超えます"
        );
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
                vertices: d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("共有する制御点XYZ"),
                    contents: bytemuck::cast_slice(&p.vertices),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
                triangles: d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("共有する制御点TIN"),
                    contents: bytemuck::cast_slice(&p.triangles),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
                bytes: p.sections.iter().map(|b| b.len().max(4)).sum::<usize>()
                    + p.vertices.len() * 16
                    + p.triangles.len() * 4,
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
            bytes: size as usize + std::mem::size_of::<View>() + 64,
            controls: p.controls,
            compressed_bytes: p.compressed_bytes,
            n,
            origin: [p.origin[0] + ox, p.origin[1] + oy],
            side,
            field,
            local: field.depth > 0,
            compute,
            _sources: sources.clone(),
            view,
            output,
            seam_band: source_seam_band(p),
            vertices: sources.vertices.clone(),
            triangles: sources.triangles.clone(),
            triangle_indices: p.triangles.len() as u32,
        });
        let batching = self.upload_batch.is_some();
        let mut encoder = self.upload_batch.take().unwrap_or_else(|| {
            d.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("曲面生成"),
            })
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
        if batching {
            self.upload_batch = Some(encoder);
            self.upload_batch_sizes.push(n);
        } else {
            self.state.queue.submit([encoder.finish()]);
            let uploads = self.uploads.clone();
            let ctx = self.wake.clone();
            self.state.queue.on_submitted_work_done(move || {
                uploads.finish(n);
                ctx.request_repaint();
            });
        }
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
        let ppp = self
            .frozen_layout
            .as_ref()
            .map_or(ppp, |s| s.pixels_per_point);
        // 視野外へ出るたびに共有シーンの並びを変えると、回転だけで接合情報・
        // GPU設定を全再構築してしまう。容量内なら取得済みの面は保持し、
        // 描画パッチ側でカリングする。大きすぎる場合は従来の可視面だけへ絞る。
        let retain_offscreen = atlas_fits(
            tiles
                .iter()
                .chain(&roots)
                .map(|tile| u64::from(tile.n + 1).pow(2))
                .sum(),
            0,
            self.state.device.limits().max_storage_buffer_binding_size,
        );
        let tiles: Vec<_> = tiles
            .into_iter()
            .filter(|t| {
                retain_offscreen
                    || camera
                        .visible_depth([
                            t.origin[0],
                            t.origin[1],
                            t.origin[0] + t.side,
                            t.origin[1] + t.side,
                        ])
                        .is_some()
            })
            .collect();
        let primary_keys = tiles
            .iter()
            .map(|t| {
                (
                    t.key,
                    t.origin[0].to_bits(),
                    t.origin[1].to_bits(),
                    t.side.to_bits(),
                )
            })
            .collect::<std::collections::HashSet<_>>();
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
                !primary_keys.contains(&(
                    t.key,
                    t.origin[0].to_bits(),
                    t.origin[1].to_bits(),
                    t.side.to_bits(),
                ))
            })
            .filter(|t| {
                retain_offscreen
                    || camera
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
        // 遷移が終わった旧曲面は、次の取得完了を待たず描画と接合候補から外す。
        if morph >= 1. {
            previous.clear();
        }
        previous.retain(|t| {
            retain_offscreen
                || camera
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
        let size = if let Some(layout) = &self.frozen_layout {
            if layout.render_size.iter().all(|v| *v > 0) {
                layout.render_size
            } else {
                [
                    (rect.width() * ppp).round().max(1.) as u32,
                    (rect.height() * ppp).round().max(1.) as u32,
                ]
            }
        } else {
            [
                (rect.width() * ppp).max(1.) as u32,
                (rect.height() * ppp).max(1.) as u32,
            ]
        };
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
        let pipeline = if previous.is_empty() || morph >= 1. {
            self.fast_pipeline.clone()
        } else {
            self.pipeline.clone()
        };
        let fallback = if previous.is_empty() || morph >= 1. {
            self.fast_pipeline.clone()
        } else {
            self.fallback.clone()
        };
        Callback::new_paint_callback(
            rect,
            Frame {
                gpu_culler: self.gpu_culler.clone(),
                gpu_planner: self.gpu_planner.clone(),
                wake: self.wake.clone(),
                owner_texture: self.texture,
                frozen_layout: self.frozen_layout.clone(),
                point_state: self.point_state.clone(),
                hit_state: self.hit_state.clone(),
                hit_texture: self.positions.clone(),
                point_renderer: self.point_renderer.ready_or_start(
                    &self.state.device,
                    &self.point_state,
                    &self.wake,
                ),
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
                pipeline: pipeline.clone(),
                indices: self.indices.clone(),
                fallback,
                control: self.control.clone(),
                control_fallback: self.control_fallback.clone(),
                globe_pipeline: self.globe_pipeline.clone(),
            },
        )
    }
}
struct Frame {
    gpu_culler: Arc<std::sync::OnceLock<crate::gpu_culling::Pipelines>>,
    gpu_planner: Arc<std::sync::OnceLock<Arc<crate::gpu_planner::Planner>>>,
    wake: egui::Context,
    owner_texture: egui::TextureId,
    hit_state: Arc<std::sync::Mutex<crate::hit_test::State>>,
    hit_texture: wgpu::Texture,
    point_state: Arc<std::sync::Mutex<crate::point_visibility::State>>,
    point_renderer: Option<Arc<crate::point_visibility::Renderer>>,
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
    globe_pipeline: wgpu::RenderPipeline,
    frozen_layout: Option<LayoutSnapshot>,
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
        let prepare_started = std::time::Instant::now();
        let mut stats = PrepareStats::default();
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
                    f32::from(self.direct.is_some() || camera.globe),
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
            cache.owner_texture != self.owner_texture
                || cache.primary != self.tiles.len()
                || cache.current != current
                || cache.tiles.len() != all.len()
                || cache
                    .tiles
                    .iter()
                    .zip(&all)
                    .any(|(a, b)| !Arc::ptr_eq(a, b))
        });
        if rebuild {
            let rebuild_started = std::time::Instant::now();
            let old = resources
                .get::<CachedScene>()
                .filter(|c| c.owner_texture == self.owner_texture);
            let entries = all
                .iter()
                .map(|t| (Arc::as_ptr(t) as usize, (t.n + 1).pow(2)))
                .collect::<Vec<_>>();
            let old_entries = old
                .map(|c| {
                    c.tiles
                        .iter()
                        .zip(&c.records)
                        .map(|(t, r)| (Arc::as_ptr(t) as usize, r.data[0], (t.n + 1).pow(2)))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let allocation = plan_atlas(
                &entries,
                &old_entries,
                old.map_or(0, |c| c._atlas.size()),
                u64::from(device.limits().max_storage_buffer_binding_size),
            );
            let old_background = old
                .map(|cache| {
                    cache.tiles[..cache.current]
                        .iter()
                        .zip(&cache.records)
                        .filter(|(_, r)| r.bounds[3] > 0.)
                        .map(|(t, _)| Arc::as_ptr(t) as usize)
                        .collect::<std::collections::HashSet<_>>()
                })
                .unwrap_or_default();
            let mut records = Vec::<Record>::new();
            for (i, t) in all.iter().enumerate() {
                records.push(Record {
                    blend_data: [0; 4],
                    tile_end: {
                        let b = crate::shared_data::bounds(t.key);
                        [
                            (b[2] - camera.center[0]) as f32,
                            (b[3] - camera.center[1]) as f32,
                            0.,
                            0.,
                        ]
                    },
                    transition: [0; 4],
                    draw: [0; 4],
                    bounds: [
                        (t.origin[0] - camera.center[0]) as f32,
                        (t.origin[1] - camera.center[1]) as f32,
                        t.side as f32,
                        if (i >= self.tiles.len() && i < current)
                            || (i >= current
                                && (old_background.contains(&(Arc::as_ptr(t) as usize))
                                    || self
                                        .frozen_layout
                                        .as_ref()
                                        .and_then(|l| l.previous.get(i - current))
                                        .and_then(Option::as_ref)
                                        .is_some_and(|p| p.background)))
                        {
                            1.
                        } else {
                            0.
                        },
                    ],
                    data: [allocation.offsets[i], t.n, t.key.0, 0],
                    neighbors: [0, t.seam_band.to_bits(), 0, 0],
                    tile: {
                        let b = crate::shared_data::bounds(t.key);
                        [
                            (b[0] - camera.center[0]) as f32,
                            (b[1] - camera.center[1]) as f32,
                            (b[2] - b[0]) as f32,
                            t.seam_band,
                        ]
                    },
                });
            }
            // 同じ元タイルの詳細区画が重なっていなければ、内部の参照先は
            // 必ず自分自身になる。GPUで同じ区画群を再探索する必要はない。
            let mut primary_groups = std::collections::HashMap::<Key, Vec<usize>>::new();
            for (i, t) in all[..self.tiles.len()].iter().enumerate() {
                primary_groups.entry(t.key).or_default().push(i);
            }
            for ids in primary_groups.values() {
                for &i in ids {
                    let a = records[i].bounds;
                    records[i].transition[3] = u32::from(!ids.iter().any(|&j| {
                        let b = records[j].bounds;
                        j != i
                            && (0..2).all(|axis| {
                                (a[axis] + a[2]).min(b[axis] + b[2]) - a[axis].max(b[axis]) > 0.01
                            })
                    }));
                }
            }
            let previous_indices = all[current..]
                .iter()
                .enumerate()
                .filter(|(j, _)| records[current + *j].bounds[3] == 0.)
                .map(|(j, t)| (Arc::as_ptr(t) as usize, (current + j + 1) as u32))
                .collect::<std::collections::HashMap<_, _>>();
            for i in 0..self.tiles.len() {
                records[i].transition[0] = previous_indices
                    .get(&(Arc::as_ptr(&all[i]) as usize))
                    .copied()
                    .unwrap_or(0);
            }
            let groups = SeamGroups::new(
                &records,
                &all.iter().map(|t| t.key).collect::<Vec<_>>(),
                &all.iter().map(|t| t.n).collect::<Vec<_>>(),
            );
            let mut links = Vec::<u32>::new();
            for i in 0..records.len() {
                let a = records[i].bounds;
                records[i].neighbors[0] = links.len() as u32;
                let own_pad = (a[2] / 8.).min(2. * a[2] / all[i].n as f32).max(1.);
                for j in groups.candidates([
                    a[0] - own_pad,
                    a[1] - own_pad,
                    a[0] + a[2] + own_pad,
                    a[1] + a[2] + own_pad,
                ]) {
                    let b = &records[j];
                    // GPUで必ず除外される重なった代替面を候補へ入れない。
                    // 大きな代替面の接合幅が詳細面の高速経路まで無効にするのも防ぐ。
                    let own = records[i];
                    let overlap = (0..2).all(|axis| {
                        own.tile_end[axis].min(b.tile_end[axis]) - own.tile[axis].max(b.tile[axis])
                            > 0.01
                    });
                    if (i < current) == (j < current)
                        && b.bounds[3] > 0.
                        && overlap
                        && (own.bounds[3] == 0. || b.bounds[2] > own.bounds[2] * 1.001)
                    {
                        continue;
                    }
                    // 接合幅は元データに対して固定されている。評価区画の幅や
                    // nで候補を切ると、分割だけで必要な隣接曲面が出入りする。
                    let pad = b.tile[3].max(1.);
                    if a[0] - pad <= b.tile_end[0]
                        && a[0] + a[2] + pad >= b.tile[0]
                        && a[1] - pad <= b.tile_end[1]
                        && a[1] + a[2] + pad >= b.tile[1]
                    {
                        links.push(j as u32);
                    }
                }
                let begin = records[i].neighbors[0] as usize;
                // 元タイルごとに並べ、カメラに合わせて分割した領域を重複加算しない。
                links[begin..].sort_by_key(|&j| {
                    let t = &all[j as usize];
                    // 外挿で必ず使う固定格子を先頭に置き、局所区画の探索を省けるようにする。
                    (
                        j as usize >= current,
                        t.key,
                        !(t.field.depth == 0 && t.n == 32),
                    )
                });
                records[i].data[3] = links.len() as u32 - records[i].neighbors[0];
                // 境界から十分離れた頂点は、隣接タイルの探索を省ける。
                let pad = links[begin..]
                    .iter()
                    .map(|&j| all[j as usize].seam_band)
                    .fold(all[i].seam_band, f32::max);
                records[i].draw[3] = pad.to_bits();
            }
            // 背景の被覆判定には、細分化された全パッチでなく外側の矩形だけを渡す。
            for i in 0..current {
                if records[i].bounds[3] == 0. {
                    continue;
                }
                let a = records[i];
                let mut candidates = groups
                    .candidates([
                        a.bounds[0],
                        a.bounds[1],
                        a.bounds[0] + a.bounds[2],
                        a.bounds[1] + a.bounds[2],
                    ])
                    .into_iter()
                    .filter(|&j| j < current)
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
            // 元タイル単位で不要な候補群を一度に飛ばすための終端表。
            for i in 0..records.len() {
                let start = records[i].neighbors[0] as usize;
                let count = records[i].data[3] as usize;
                records[i].neighbors[1] = links.len() as u32;
                let mut begin = 0;
                while begin < count {
                    let first = links[start + begin] as usize;
                    let mut end = begin + 1;
                    while end < count {
                        let id = links[start + end] as usize;
                        if all[id].key != all[first].key || (id >= current) != (first >= current) {
                            break;
                        }
                        end += 1;
                    }
                    links.extend(std::iter::repeat_n(end as u32, end - begin));
                    begin = end;
                }
            }
            // blendの照会点は参照元区画の内部にある。元タイル全体の固定格子、
            // または参照元自身があれば距離0の候補が必ず存在するので、区画と
            // 交差しない局所面は最寄り候補にならない。境界探索用の表は残す。
            for i in 0..records.len() {
                let base = records[i].neighbors[0] as usize;
                let ids = links[base..base + records[i].data[3] as usize].to_vec();
                let mut kept = Vec::new();
                let a = &all[i];
                let mut begin = 0;
                while begin < ids.len() {
                    let first = ids[begin] as usize;
                    let mut end = begin + 1;
                    while end < ids.len()
                        && all[ids[end] as usize].key == all[first].key
                        && (ids[end] as usize >= current) == (first >= current)
                    {
                        end += 1;
                    }
                    let group = &ids[begin..end];
                    let bounded = group.contains(&(i as u32))
                        || group
                            .iter()
                            .any(|&j| all[j as usize].field.depth == 0 && all[j as usize].n == 32);
                    for &j in group {
                        let b = &all[j as usize];
                        // GPUの単精度座標の丸めを含めても交差しない候補だけを省く。
                        let pad = (a.side.max(b.side) * 1e-6).max(WORLD * f32::EPSILON as f64 * 8.);
                        if !bounded
                            || (b.field.depth == 0 && b.n == 32)
                            || (0..2).all(|axis| {
                                a.origin[axis] - pad <= b.origin[axis] + b.side
                                    && a.origin[axis] + a.side + pad >= b.origin[axis]
                            })
                        {
                            kept.push(j);
                        }
                    }
                    begin = end;
                }
                records[i].blend_data = [links.len() as u32, kept.len() as u32, 0, 0];
                links.extend_from_slice(&kept);
                records[i].blend_data[2] = links.len() as u32;
                let mut begin = 0;
                while begin < kept.len() {
                    let first = kept[begin] as usize;
                    let mut end = begin + 1;
                    while end < kept.len()
                        && all[kept[end] as usize].key == all[first].key
                        && (kept[end] as usize >= current) == (first >= current)
                    {
                        end += 1;
                    }
                    links.extend(std::iter::repeat_n(end as u32, end - begin));
                    begin = end;
                }
            }
            // 旧表示で描いたパッチを隣接表の末尾へ保持する。座標は元の評価区画に
            // 対する相対値へ戻し、その後のカメラ移動や再分割には依存させない。
            let old_indices = old
                .map(|c| {
                    c.tiles[..c.current]
                        .iter()
                        .enumerate()
                        .map(|(i, t)| (Arc::as_ptr(t) as usize, i))
                        .collect::<std::collections::HashMap<_, _>>()
                })
                .unwrap_or_default();
            let mut previous_plans = Vec::new();
            for i in current..all.len() {
                let mut previous = self
                    .frozen_layout
                    .as_ref()
                    .and_then(|l| l.previous.get(i - current))
                    .cloned()
                    .flatten();
                if let Some((c, j)) = old.and_then(|c| {
                    old_indices
                        .get(&(Arc::as_ptr(&all[i]) as usize))
                        .map(|&j| (c, j))
                }) {
                    let mut patches = c.plans[j].patches.clone();
                    for patch in &mut patches {
                        for e in 0..4 {
                            let axis = if e < 2 { 1 } else { 0 };
                            patch.origins[e] = (f64::from(patch.origins[e]) + c.camera[axis]
                                - all[i].origin[axis])
                                as f32;
                            let axis = if e < 2 { 0 } else { 1 };
                            patch.lines[e] = (f64::from(patch.lines[e]) + c.camera[axis]
                                - all[i].origin[axis])
                                as f32;
                        }
                    }
                    let mut plan = c.plans[j].clone();
                    plan.patches = patches;
                    previous = Some(PreviousPlan {
                        background: c.records[j].bounds[3] > 0.,
                        plan,
                    });
                }
                if let Some(p) = &previous {
                    records[i].draw[0] = links.len() as u32;
                    records[i].draw[1] = p.plan.patches.len() as u32;
                    records[i].draw[2] = 4;
                    links.extend_from_slice(bytemuck::cast_slice::<
                        crate::shared_patches::Patch,
                        u32,
                    >(&p.plan.patches));
                }
                previous_plans.push(previous);
            }
            let atlas = if allocation.reuse {
                old.unwrap()._atlas.clone()
            } else {
                stats.allocations += 1;
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("共有曲面アトラス"),
                    size: allocation.bytes,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            for &i in &allocation.copies {
                let t = &all[i];
                let bytes = u64::from((t.n + 1).pow(2)) * 32;
                encoder.copy_buffer_to_buffer(
                    &t.output,
                    0,
                    &atlas,
                    u64::from(records[i].data[0]) * 32,
                    bytes,
                );
                stats.atlas_copy_bytes += bytes;
            }
            let (metadata, metadata_reused) = shared_buffer(
                device,
                old.map(|c| &c.metadata),
                std::mem::size_of_val(records.as_slice()) as u64,
                "曲面接合情報",
            );
            let (refs, refs_reused) = shared_buffer(
                device,
                old.map(|c| &c._links),
                std::mem::size_of_val(links.as_slice()) as u64,
                "局所接合候補",
            );
            stats.allocations += usize::from(!metadata_reused) + usize::from(!refs_reused);
            queue.write_buffer(&metadata, 0, bytemuck::cast_slice(&records));
            queue.write_buffer(&refs, 0, bytemuck::cast_slice(&links));
            let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(1);
            let view_stride = (std::mem::size_of::<View>() as u64).div_ceil(alignment) * alignment;
            let view_length = (current.max(1) as u64) * view_stride;
            let prior_views =
                old.filter(|c| c.views.size() >= view_length && c.view_stride == view_stride);
            let views = if let Some(c) = prior_views {
                c.views.clone()
            } else {
                stats.allocations += 1;
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("地形の視点設定をまとめて転送"),
                    size: view_length.next_power_of_two(),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            let previous_slots = old
                .map(|c| {
                    c.tiles[..c.current]
                        .iter()
                        .zip(&c.view_slots)
                        .map(|(t, &slot)| (Arc::as_ptr(t) as usize, slot))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let view_slots = uniform_slots(
                &all[..current]
                    .iter()
                    .map(|t| Arc::as_ptr(t) as usize)
                    .collect::<Vec<_>>(),
                &previous_slots,
                views.size() / view_stride,
            );
            let shared_reused =
                allocation.reuse && metadata_reused && refs_reused && prior_views.is_some();
            // 視野外の面は共通の仮設定で保持し、初めて描くときだけ専用設定を作る。
            let placeholder_patch = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("未表示メッシュ共通の仮パッチ"),
                size: 64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let first = &all[0];
            let buffers = [
                &first.output,
                &first.view,
                &placeholder_patch,
                &atlas,
                &metadata,
                &refs,
                &first.vertices,
                &first.triangles,
            ];
            let entries = buffers
                .iter()
                .enumerate()
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect::<Vec<_>>();
            let placeholder_binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("未表示メッシュ共通の仮設定"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            stats.allocations += 1;
            stats.bind_groups += 1;
            let previous_indices = old
                .map(|c| {
                    c.tiles[..c.current]
                        .iter()
                        .enumerate()
                        .map(|(i, t)| (Arc::as_ptr(t) as usize, i))
                        .collect::<std::collections::HashMap<_, _>>()
                })
                .unwrap_or_default();
            let mut bindings = Vec::new();
            let mut patches = Vec::new();
            let mut bindings_ready = Vec::new();
            for tile in &all[..current] {
                let previous = previous_indices
                    .get(&(Arc::as_ptr(tile) as usize))
                    .and_then(|&i| old.map(|c| (c, i)))
                    .filter(|(c, i)| c.bindings_ready[*i]);
                patches.push(
                    previous
                        .map_or_else(|| placeholder_patch.clone(), |(c, i)| c.patches[i].clone()),
                );
                if let Some((c, i)) = previous.filter(|_| shared_reused) {
                    bindings.push(c.bindings[i].clone());
                    bindings_ready.push(true);
                } else {
                    bindings.push(placeholder_binding.clone());
                    bindings_ready.push(false);
                }
            }
            let point_patches = if let Some(old) = old {
                old.point_patches.clone()
            } else {
                stats.allocations += 1;
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("地点投影用の共有描画パッチ"),
                    size: 64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            stats.rebuild_ms = rebuild_started.elapsed().as_secs_f64() * 1000.;
            resources.insert(CachedScene {
                gpu_cull: None,
                stats,
                owner_texture: self.owner_texture,
                clipped_background: false,
                plans_uploaded: false,
                tiles: all.clone(),
                primary: self.tiles.len(),
                current,
                render_camera: camera.clone(),
                morph: self.morph,
                pixels_per_point: self.size[0] as f32 / camera.viewport[0] as f32,
                _atlas: atlas,
                _links: refs,
                metadata,
                view_bytes: vec![0; views.size() as usize],
                views,
                view_stride,
                view_slots,
                bindings,
                patches,
                bindings_ready,
                placeholder_patch,
                point_patches,
                records,
                plans: Vec::new(),
                previous_plans,
                plan_inputs: None,
                plan_job: None,
                plans_from_gpu: false,
                planning_error: None,
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
        // 完成した計画だけを取り込み、それまでは旧計画で描画を継続する。
        let prepared = cache
            .plan_job
            .as_ref()
            .map(|job| job.lock().unwrap().try_recv());
        if let Some(result) = prepared {
            match result {
                Ok(prepared) => {
                    cache.plans_from_gpu = prepared.gpu;
                    cache.camera = prepared.pose;
                    stats.plan_ms = prepared.elapsed_ms;
                    match prepared.plans {
                        Ok(plans) => {
                            cache.plans = plans;
                            cache.plans_uploaded = false;
                            cache.clipped_background = true;
                            cache.planning_error = None;
                        }
                        Err(error) => {
                            eprintln!("GPU分割計画: {error}");
                            cache.planning_error = Some(error);
                        }
                    }
                    cache.plan_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    cache.plan_job = None;
                    self.wake.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(layout) = self.frozen_layout.as_ref().filter(|s| !s.force_replan) {
            install_snapshot_layout(cache, layout, queue);
            cache.clipped_background = layout.clipped_background;
        } else if cache.plan_job.is_none()
            && (cache.planning_error.is_none() || cache.plans_from_gpu != camera.gpu_planning)
            && (cache.plans_from_gpu != camera.gpu_planning
                || !cache.clipped_background
                || cache.plans.is_empty()
                || (pose != cache.camera
                    && cache.last_plan.elapsed() >= std::time::Duration::from_millis(100)))
        {
            let inputs = cache.plan_inputs.get_or_insert_with(|| {
                Arc::new(
                    all[..current]
                        .iter()
                        .enumerate()
                        .map(|(i, t)| {
                            let covered_by = if i < self.tiles.len() {
                                Vec::new()
                            } else {
                                all[..current]
                                    .iter()
                                    .enumerate()
                                    .filter(|(j, b)| {
                                        *j != i
                                            && (*j < self.tiles.len()
                                                || b.key.0 > t.key.0
                                                || (b.key.0 == t.key.0 && b.side < t.side))
                                            && b.origin[0] < t.origin[0] + t.side
                                            && b.origin[0] + b.side > t.origin[0]
                                            && b.origin[1] < t.origin[1] + t.side
                                            && b.origin[1] + b.side > t.origin[1]
                                    })
                                    .map(|(_, b)| {
                                        [
                                            b.origin[0],
                                            b.origin[1],
                                            b.origin[0] + b.side,
                                            b.origin[1] + b.side,
                                        ]
                                    })
                                    .collect()
                            };
                            crate::shared_patches::Input {
                                origin: t.origin,
                                side: t.side,
                                n: t.n,
                                covered_by,
                            }
                        })
                        .collect::<Vec<_>>(),
                )
            });
            if cache.plans.is_empty() && !camera.gpu_planning {
                // 初回だけは代替計画がないため作成する。視点変更は常に背景で準備する。
                let started = std::time::Instant::now();
                cache.plans = crate::shared_patches::plan(inputs, camera);
                stats.plan_ms = started.elapsed().as_secs_f64() * 1000.;
                cache.plans_uploaded = false;
                cache.clipped_background = true;
                cache.camera = pose;
                cache.plans_from_gpu = false;
                cache.planning_error = None;
            } else {
                let inputs = Arc::clone(inputs);
                let camera = camera.clone();
                let wake = self.wake.clone();
                let (send, receive) = std::sync::mpsc::channel();
                cache.plan_job = Some(std::sync::Mutex::new(receive));
                let planner = self.gpu_planner.clone();
                let device = device.clone();
                let queue = queue.clone();
                std::thread::spawn(move || {
                    let started = std::time::Instant::now();
                    let gpu = camera.gpu_planning;
                    let plans = if gpu {
                        // CPUの可視判定・分割・接合計画は呼ばない。
                        planner
                            .get_or_init(|| {
                                Arc::new(crate::gpu_planner::Planner::new(&device, &queue))
                            })
                            .plan(&inputs, &camera)
                            .map_err(|e| e.to_string())
                    } else {
                        Ok(crate::shared_patches::plan(&inputs, &camera))
                    };
                    let _ = send.send(PreparedPlan {
                        plans,
                        pose,
                        gpu,
                        elapsed_ms: started.elapsed().as_secs_f64() * 1000.,
                    });
                    wake.request_repaint();
                });
            }
            cache.last_plan = std::time::Instant::now();
        }
        if pose != cache.camera && cache.plan_job.is_none() {
            self.wake.request_repaint_after(
                std::time::Duration::from_millis(100).saturating_sub(cache.last_plan.elapsed()),
            );
        }
        if !cache.plans_uploaded {
            if let Some(scene) = &mut cache.gpu_cull {
                scene.invalidate_plan();
            }
            let all_patches = cache
                .plans
                .iter()
                .flat_map(|p| p.patches.iter().copied())
                .collect::<Vec<_>>();
            let bytes = bytemuck::cast_slice(&all_patches);
            if bytes.len() as u64 > cache.point_patches.size() {
                cache.point_patches = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("地点投影用の共有描画パッチ"),
                    size: (bytes.len() as u64).next_power_of_two(),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            if !bytes.is_empty() {
                queue.write_buffer(&cache.point_patches, 0, bytes);
            }
            let mut offset = 0;
            for (r, p) in cache.records.iter_mut().zip(&cache.plans) {
                r.draw = [offset, p.patches.len() as u32, 4, r.draw[3]];
                offset += p.patches.len() as u32;
            }
            for (i, plan) in cache.plans.iter().enumerate() {
                if i != 0 && !self.tin && self.wire < 3 && plan.patches.is_empty() {
                    continue;
                }
                let size = (plan.patches.len().max(1)
                    * std::mem::size_of::<crate::shared_patches::Patch>())
                    as u64;
                let allocate =
                    size > cache.patches[i].size() || cache.patches[i] == cache.placeholder_patch;
                if allocate || !cache.bindings_ready[i] {
                    stats.allocations += usize::from(allocate);
                    stats.bind_groups += 1;
                    let patch = if allocate {
                        device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("境界分割パッチ"),
                            size: size.next_power_of_two(),
                            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        })
                    } else {
                        cache.patches[i].clone()
                    };
                    let tile = &all[i];
                    let buffers = [
                        &tile.output,
                        &tile.view,
                        &patch,
                        &cache._atlas,
                        &cache.metadata,
                        &cache._links,
                        &tile.vertices,
                        &tile.triangles,
                    ];
                    let entries = buffers
                        .iter()
                        .enumerate()
                        .map(|(binding, b)| wgpu::BindGroupEntry {
                            binding: binding as u32,
                            resource: if binding == 1 {
                                wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &cache.views,
                                    offset: cache.view_slots[i] * cache.view_stride,
                                    size: std::num::NonZeroU64::new(
                                        std::mem::size_of::<View>() as u64
                                    ),
                                })
                            } else {
                                b.as_entire_binding()
                            },
                        })
                        .collect::<Vec<_>>();
                    cache.bindings[i] = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("境界分割の描画情報"),
                        layout: &self.pipeline.get_bind_group_layout(0),
                        entries: &entries,
                    });
                    cache.patches[i] = patch;
                    cache.bindings_ready[i] = true;
                }
                if !plan.patches.is_empty() {
                    queue.write_buffer(&cache.patches[i], 0, bytemuck::cast_slice(&plan.patches));
                }
            }
            cache.plans_uploaded = true;
        }
        for (r, t) in cache.records.iter_mut().zip(&all) {
            r.bounds[0] = (t.origin[0] - camera.center[0]) as f32;
            r.bounds[1] = (t.origin[1] - camera.center[1]) as f32;
            let b = crate::shared_data::bounds(t.key);
            r.tile[0] = (b[0] - camera.center[0]) as f32;
            r.tile[1] = (b[1] - camera.center[1]) as f32;
            r.tile_end[0] = (b[2] - camera.center[0]) as f32;
            r.tile_end[1] = (b[3] - camera.center[1]) as f32;
        }
        cache.render_camera = camera.clone();
        cache.morph = self.morph;
        cache.pixels_per_point = self.size[0] as f32 / camera.viewport[0] as f32;
        queue.write_buffer(&cache.metadata, 0, bytemuck::cast_slice(&cache.records));
        let uniform_started = std::time::Instant::now();
        for (i, tile) in all[..current].iter().enumerate() {
            // 地球背景・地点照会は先頭の共通視点を参照する。それ以外は実際に
            // 描画する面だけを更新し、視野外に保持した面へ毎フレーム転送しない。
            if i != 0 && !self.tin && self.wire < 3 && cache.plans[i].patches.is_empty() {
                continue;
            }
            let u = View {
                earth: [
                    latitude.sin() as f32,
                    latitude.cos() as f32,
                    crate::globe::RADIUS as f32,
                    f32::from(camera.globe),
                ],
                eye: [
                    eye.x as f32,
                    eye.y as f32,
                    eye.z as f32,
                    camera.height as f32,
                ],
                frame: [
                    self.size[0] as f32,
                    self.size[1] as f32,
                    camera.tan_half_fov() as f32,
                    camera.center[0] as f32,
                ],
                matrix: (projection * view).to_cols_array_2d(),
                offset: [
                    (cache.camera[0] - camera.center[0]) as f32,
                    (cache.camera[1] - camera.center[1]) as f32,
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
                    u32::from(self.borders)
                        | u32::from(self.colors) * 2
                        | u32::from(!cache.clipped_background) * 4
                        | u32::from(cache.clipped_background) * 8
                        | u32::from(cache.clipped_background) * 16
                        | 32
                        | 64
                        | u32::from(self.settings[2] >= 0.5) * 128,
                ],
            };
            let start = (cache.view_slots[i] * cache.view_stride) as usize;
            let bytes = bytemuck::bytes_of(&u);
            cache.view_bytes[start..start + bytes.len()].copy_from_slice(bytes);
            // 地球背景と地点照会が使う共通視点だけは従来の参照先も更新する。
            if i == 0 {
                queue.write_buffer(&tile.view, 0, bytes);
            }
        }
        queue.write_buffer(&cache.views, 0, &cache.view_bytes);
        let uniform_ms = uniform_started.elapsed().as_secs_f64() * 1000.;
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
        let gpu_culling = (camera.gpu_culling || camera.gpu_planning) && !self.tin && self.wire < 3;
        if gpu_culling {
            let pipelines = self
                .gpu_culler
                .get_or_init(|| crate::gpu_culling::Pipelines::new(device));
            let scene = cache.gpu_cull.get_or_insert_with(|| {
                crate::gpu_culling::Scene::new(
                    device,
                    pipelines,
                    &cache._atlas,
                    &cache.metadata,
                    all.len(),
                )
            });
            for (i, plan) in cache.plans.iter().enumerate() {
                let tile = &all[i];
                for (group, (n, range)) in plan.groups.iter().enumerate() {
                    scene.group(
                        device,
                        pipelines,
                        i,
                        group,
                        *n,
                        range.clone(),
                        camera.gpu_planning,
                        [
                            &tile.output,
                            &cache.views,
                            &cache.patches[i],
                            &cache._atlas,
                            &cache.metadata,
                            &cache._links,
                            &tile.vertices,
                            &tile.triangles,
                        ],
                        cache.view_slots[i] * cache.view_stride,
                        std::mem::size_of::<View>() as u64,
                        &self.pipeline.get_bind_group_layout(0),
                    );
                }
                scene.truncate(i, plan.groups.len());
            }
            scene.encode(encoder, pipelines, camera.gpu_planning);
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
        if camera.globe && !cache.bindings.is_empty() {
            pass.set_pipeline(&self.globe_pipeline);
            pass.set_bind_group(0, &cache.bindings[0], &[]);
            pass.draw(0..3, 0..1);
        }
        for (i, plan) in cache.plans.iter().enumerate() {
            if !self.tin && self.wire < 3 && plan.patches.is_empty() {
                continue;
            }
            let fallback = i >= self.tiles.len();
            pass.set_bind_group(0, &cache.bindings[i], &[]);
            if !self.tin && self.wire != 4 {
                pass.set_pipeline(if fallback {
                    &self.fallback
                } else {
                    &self.pipeline
                });
                if gpu_culling {
                    for draw in &cache.gpu_cull.as_ref().unwrap().draws[i] {
                        pass.set_bind_group(0, &draw.binding, &[]);
                        pass.set_index_buffer(
                            self.indices[&draw.n].slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed_indirect(&draw.arguments, 0);
                    }
                } else {
                    for (n, range) in &plan.groups {
                        pass.set_index_buffer(self.indices[n].slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..6 * n * n, 0, range.clone());
                    }
                }
            }
            if (self.tin || self.wire >= 3) && !(camera.globe && all[i].key == (0, 0, 0)) {
                pass.set_pipeline(if fallback {
                    &self.control_fallback
                } else {
                    &self.control
                });
                pass.draw(0..all[i].triangle_indices, 0..1);
            }
        }
        drop(pass);
        let material_started = std::time::Instant::now();
        if self.direct.is_some() || camera.globe {
            let commands = self.direct.as_deref().unwrap_or(&[]);
            let render = if all.len() == 1 && all[0].key == (0, 0, 0) {
                crate::direct_material::Renderer::render_flat
            } else {
                crate::direct_material::Renderer::render
            };
            render(
                &self.direct_renderer,
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
        if let (Some(tile), Some(point_renderer)) = (all.first(), self.point_renderer.as_ref()) {
            // メッシュ描画後の位置情報と、同じ曲面・境界・モーフの補間を使う。
            point_renderer.encode(
                device,
                encoder,
                &self.point_state,
                &self.positions,
                &cache._atlas,
                &cache.metadata,
                &cache._links,
                &tile.view,
                &cache.point_patches,
            );
        }
        crate::hit_test::encode(&self.hit_state, device, encoder, &self.hit_texture, camera);
        stats.prepare_ms = prepare_started.elapsed().as_secs_f64() * 1000.;
        if std::env::var_os("YAMAMUKI_TERRAIN_PROFILE").is_some()
            && (rebuild || stats.prepare_ms >= 16.)
        {
            eprintln!(
                "地形準備: 全体={:.2}ms 再構築={:.2}ms 分割={:.2}ms 転送={}bytes 確保={} 描画設定={} 視点={:.2}ms 地図={:.2}ms",
                stats.prepare_ms,
                stats.rebuild_ms,
                stats.plan_ms,
                stats.atlas_copy_bytes,
                stats.allocations,
                stats.bind_groups,
                uniform_ms,
                material_started.elapsed().as_secs_f64() * 1000.
            );
        }
        cache.stats = stats;
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
    fn uniform_slots_keep_bindings_when_meshes_reorder() {
        let original = super::uniform_slots(&[10, 20, 30], &[], 4);
        let old = [(10, original[0]), (20, original[1]), (30, original[2])];
        let reordered = super::uniform_slots(&[30, 10, 40], &old, 4);
        assert_eq!(reordered[0], original[2]);
        assert_eq!(reordered[1], original[0]);
        assert_eq!(reordered[2], original[1]);
    }
    #[test]
    fn seam_groups_keep_every_original_neighbor_in_original_order() {
        use bytemuck::Zeroable;
        let mut records = Vec::new();
        let mut keys = Vec::new();
        let mut densities = Vec::new();
        for i in 0..512 {
            let mut record = super::Record::zeroed();
            record.bounds = [
                ((i * 47) % 101) as f32,
                ((i * 71) % 103) as f32,
                (1 << (i % 5)) as f32,
                0.,
            ];
            record.tile = [record.bounds[0], record.bounds[1], record.bounds[2], 1.];
            records.push(record);
            keys.push((13, i / 16, 0));
            densities.push(16 << (i % 5));
        }
        let groups = super::SeamGroups::new(&records, &keys, &densities);
        for (i, a) in records.iter().enumerate() {
            let a = a.bounds;
            let own = (a[2] / 8.).min(2. * a[2] / densities[i] as f32).max(1.);
            let filter = |j: usize| {
                let b = records[j].bounds;
                let pad = own.max((b[2] / 8.).min(2. * b[2] / densities[j] as f32));
                a[0] - pad <= b[0] + b[2]
                    && a[0] + a[2] + pad >= b[0]
                    && a[1] - pad <= b[1] + b[2]
                    && a[1] + a[2] + pad >= b[1]
            };
            let expected = (0..records.len())
                .filter(|&j| filter(j))
                .collect::<Vec<_>>();
            let actual = groups
                .candidates([a[0] - own, a[1] - own, a[0] + a[2] + own, a[1] + a[2] + own])
                .into_iter()
                .filter(|&j| filter(j))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
    }
    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
    fn renderer_reuses_programs_for_flat_and_terrain_views() {
        let backend = if cfg!(windows) {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::PRIMARY
        };
        let Some(app) = crate::remembered_scene_tests::app_with_backend(backend) else {
            return;
        };
        let started = std::time::Instant::now();
        let renderer = super::Renderer::new(
            app.renderer.state.clone(),
            &eframe::egui::Context::default(),
        );
        assert!(std::sync::Arc::ptr_eq(
            &renderer.indices,
            &app.renderer.indices
        ));
        assert!(std::sync::Arc::ptr_eq(
            &renderer.direct_renderer,
            &app.renderer.direct_renderer
        ));
        assert!(std::sync::Arc::ptr_eq(
            &renderer.point_renderer,
            &app.renderer.point_renderer
        ));
        eprintln!(
            "GPUプログラム再利用: {:.2}ms",
            started.elapsed().as_secs_f64() * 1000.
        );
    }
    #[test]
    fn atlas_reuses_slots_and_uploads_only_new_meshes() {
        let plan = super::plan_atlas(&[(2, 4), (3, 4), (2, 4)], &[(1, 0, 4), (2, 4, 4)], 256, 512);
        assert!(plan.reuse);
        assert_eq!(plan.offsets, [4, 0, 4]);
        assert_eq!(plan.copies, [1]);
        assert_eq!(plan.bytes, 256);
    }
    #[test]
    fn atlas_copies_identical_transition_fields_only_once() {
        let plan = super::plan_atlas(&[(1, 4), (1, 4)], &[], 0, 512);
        assert_eq!(plan.offsets, [0, 0]);
        assert_eq!(plan.copies, [0]);
        assert_eq!(plan.bytes, 128);
    }
    #[test]
    fn fragmented_atlas_is_compacted_without_overwriting_retained_fields() {
        let plan = super::plan_atlas(&[(1, 4), (2, 4), (3, 8)], &[(1, 0, 4), (2, 8, 4)], 512, 512);
        assert!(!plan.reuse);
        assert_eq!(plan.offsets, [0, 4, 8]);
        assert_eq!(plan.copies, [0, 1, 2]);
        assert_eq!(plan.bytes, 512);
    }
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
        for _ in 0..4 {
            assert!(budget.available(512));
            budget.start(512);
        }
        assert!(!budget.available(32));
        for _ in 0..4 {
            budget.finish(512);
        }
        assert!(budget.available(512));
        for _ in 0..super::MAX_UPLOAD_JOBS {
            assert!(budget.available(32));
            budget.start(32);
        }
        assert!(!budget.available(32));
        for _ in 0..super::MAX_UPLOAD_JOBS {
            budget.finish(32);
        }
        assert!(budget.available(512));
    }
    #[test]
    fn shared_gpu_shaders_validate() {
        for source in [
            include_str!("sibson_gpu_controls.wgsl").to_string(),
            super::render_shader_source(),
            super::regular_render_shader_source(),
        ] {
            let module = naga::front::wgsl::parse_str(&source).expect("WGSL parse");
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
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
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
            super::render_shader_source(),
            r#"
 @group(0) @binding(8) var<storage,read_write> result:array<vec4<f32>>;
 @compute @workgroup_size(1)fn check(@builtin(global_invocation_id)id:vec3<u32>){
  let point=vec2(100.,f32(id.x/2u)*12.5);
  result[id.x]=vec4(surface(point,id.x%2u),1.);
 }
 "#
        );
        let module = crate::gpu_profile::timed("shared_render.rs:d.create_shader_module", || {
            d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(shader.into()),
            })
        });
        let pipeline =
            crate::gpu_profile::timed("shared_render.rs:d.create_compute_pipeline", || {
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: None,
                    layout: None,
                    module: &module,
                    entry_point: Some("check"),
                    compilation_options: Default::default(),
                    cache: None,
                })
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
                blend_data: [0; 4],
                tile_end: [0.; 4],
                transition: [0; 4],
                draw: [0; 4],
                tile: [x, 0., 100., 12.5],
                bounds: [x, 0., 100., 0.],
                data: [atlas.len() as u32, n, if i < 2 { 8 } else { 7 }, 4],
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
                earth: [0.; 4],
                eye: [0.; 4],
                frame: [0.; 4],
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
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
    fn seam_height_does_not_depend_on_the_number_of_local_fields() -> anyhow::Result<()> {
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
        let shader = crate::gpu_profile::timed(
            "評価区画数によらない接合の検査",
            || {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("評価区画数によらない接合の検査"),
            source: wgpu::ShaderSource::Wgsl(format!(
                "{}\n@group(0) @binding(8) var<storage,read_write> result:array<vec4<f32>>;\n@compute @workgroup_size(1) fn probe(){{result[0]=vec4(blend(vec2(99.,50.),0u,false),1.);result[1]=vec4(blend(vec2(50.,50.),0u,false),1.);result[2]=vec4(blend(vec2(1.,50.),0u,false),1.);}}",
                render_shader_source()).into()),
        })
            },
        );
        let pipeline =
            crate::gpu_profile::timed("shared_render.rs:device.create_compute_pipeline", || {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: None,
                    layout: None,
                    module: &shader,
                    entry_point: Some("probe"),
                    compilation_options: Default::default(),
                    cache: None,
                })
            });
        let mut heights = Vec::new();
        let mut left_heights = Vec::new();
        for configuration in 0..7 {
            let mut fields = vec![([0., 0., 100., 0.], [0., 0., 100., 25.], 100.)];
            if configuration >= 5 {
                // 数十万mの辺長をf32で加算すると、接している辺が3cm重なる。
                // 正確な端点では隣接面として接合し、代替面と誤判定しない。
                fields.push((
                    [-500000., -250000., 500000.03125, 1.],
                    [-500000., -250000., 500000.03125, 25.],
                    200.,
                ));
            } else if configuration >= 3 {
                // ズームで局所区画の評価値が変わっても、隣タイルへの外挿は固定格子を使う。
                fields.push(([100., 0., 100., 1.], [100., 0., 100., 25.], 200.));
                for x in [100., 150.] {
                    for y in [0., 50.] {
                        fields.push((
                            [x, y, 50., 0.],
                            [100., 0., 100., 25.],
                            if configuration == 3 { 200. } else { 900. },
                        ));
                    }
                }
            } else if configuration == 2 {
                // 元タイルは詳細面に重なるが、局所区画だけを見ると隣接している背景。
                fields.push(([100., 0., 50., 1.], [-100., -100., 400., 50.], 10000.));
            } else if configuration == 1 {
                for x in [100., 150.] {
                    for y in [0., 50.] {
                        fields.push(([x, y, 50., 0.], [100., 0., 100., 25.], 200.));
                    }
                }
            } else {
                fields.push(([100., 0., 100., 0.], [100., 0., 100., 25.], 200.));
            }
            let mut atlas = Vec::<[f32; 8]>::new();
            let mut records = Vec::new();
            for (i, (bounds, tile, height)) in fields.iter().enumerate() {
                let n = if configuration >= 3 && i == 1 { 32 } else { 1 };
                records.push(Record {
                    blend_data: [0; 4],
                    tile_end: if configuration >= 5 && i == 1 {
                        [0., 250000., 0., 0.]
                    } else {
                        [tile[0] + tile[2], tile[1] + tile[2], 0., 0.]
                    },
                    transition: [0; 4],
                    draw: [0, 0, 0, 25f32.to_bits()],
                    bounds: *bounds,
                    tile: *tile,
                    data: [atlas.len() as u32, n, 13, fields.len() as u32],
                    neighbors: [0; 4],
                });
                atlas.extend(std::iter::repeat_n(
                    [*height, 0., 0., 1., *height, 0., 0., 1.],
                    (n + 1).pow(2) as usize,
                ));
            }
            let mut view = <View as bytemuck::Zeroable>::zeroed();
            view.extra = [
                fields.len() as u32,
                0,
                1,
                if configuration == 6 { 8 | 32 } else { 8 },
            ];
            let make = |contents: &[u8], usage| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents,
                    usage,
                })
            };
            let buffers = [
                (
                    1,
                    make(bytemuck::bytes_of(&view), wgpu::BufferUsages::UNIFORM),
                ),
                (
                    3,
                    make(bytemuck::cast_slice(&atlas), wgpu::BufferUsages::STORAGE),
                ),
                (
                    4,
                    make(bytemuck::cast_slice(&records), wgpu::BufferUsages::STORAGE),
                ),
                (
                    5,
                    make(
                        bytemuck::cast_slice(&(0..fields.len() as u32).collect::<Vec<_>>()),
                        wgpu::BufferUsages::STORAGE,
                    ),
                ),
                (
                    8,
                    make(
                        &[0u8; 48],
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    ),
                ),
            ];
            let entries = buffers
                .iter()
                .map(|(binding, b)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: b.as_entire_binding(),
                })
                .collect::<Vec<_>>();
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 48,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&buffers[4].1, 0, &read, 0, 48);
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            read.slice(..)
                .map_async(wgpu::MapMode::Read, move |v| send.send(v).unwrap());
            let _ = device.poll(wgpu::Maintain::Wait);
            receive.recv()??;
            let bytes = read.slice(..).get_mapped_range();
            let values = bytemuck::cast_slice::<u8, f32>(&bytes);
            assert!(
                (values[4] - 100.).abs() < 0.0001,
                "境界から離れた頂点の高速経路で標高が変わった: {values:?}"
            );
            heights.push(values[0]);
            left_heights.push(values[8]);
        }
        assert!(
            heights[0] > 140. && heights[0] < 160.,
            "接合されていない: {heights:?}"
        );
        assert!(
            (heights[0] - heights[1]).abs() < 0.0001,
            "評価区画の分割で地形が動いた: {heights:?}"
        );
        assert!(
            (heights[2] - 100.).abs() < 0.0001,
            "重なった背景の局所区画が詳細面に混ざった: {heights:?}"
        );
        assert!((heights[3] - heights[0]).abs() < 0.0001);
        assert!(
            (heights[3] - heights[4]).abs() < 0.0001,
            "局所区画の切替で外挿した標高が変わった: {heights:?}"
        );
        assert!(
            (left_heights[5] - 100.).abs() < 0.001,
            "従来の桁落ち条件を再現できない: {left_heights:?}"
        );
        assert!(
            left_heights[6] > 140. && left_heights[6] < 160.,
            "隣接面を重複面と誤判定した: {left_heights:?}"
        );
        Ok(())
    }

    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
    fn marker_height_uses_the_drawn_triangle_instead_of_bilinear_field() -> anyhow::Result<()> {
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
        let module = crate::gpu_profile::timed(
            "shared_render.rs:device.create_shader_module",
            || {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:Some("地点の実描画標高の検査"),
            source:wgpu::ShaderSource::Wgsl(format!("{}\n@compute @workgroup_size(1) fn probe(){{let m=meshHeight(vec2(50.),0u);result[0]=vec4(m,surface(vec2(50.),0u).x,1.);result[1]=vec4(previousHeight(vec2(50.),1u,blend(vec2(50.),1u,true).x),0.,1.);}}",crate::point_visibility::shader()).into()),
        })
            },
        );
        let pipeline =
            crate::gpu_profile::timed("shared_render.rs:device.create_compute_pipeline", || {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: None,
                    layout: None,
                    module: &module,
                    entry_point: Some("probe"),
                    compilation_options: Default::default(),
                    cache: None,
                })
            });
        for old_level in [12u32, 13] {
            let atlas = [100f32, 100., 100., 300.].map(|h| [h, 0., 0., 1., h, 0., 0., 1.]);
            let record = Record {
                blend_data: [0; 4],
                tile_end: [0.; 4],
                transition: [0; 4],
                bounds: [0., 0., 100., 0.],
                data: [0, 1, 13, 1],
                neighbors: [0; 4],
                tile: [0., 0., 100., 25.],
                draw: [0, 1, 4, 0],
            };
            let patch = crate::shared_patches::Patch {
                bounds: [0., 0., 100., 1.],
                edges: [0.; 4],
                origins: [0.; 4],
                lines: [0., 100., 0., 100.],
            };
            let mut view = <View as bytemuck::Zeroable>::zeroed();
            view.offset[2] = 1.;
            view.settings = [100., 0., 1., 0.5];
            view.extra = [1, 0, 0, 8];
            let mut old_record = record;
            old_record.draw[0] = 1;
            old_record.data[2] = old_level;
            if old_level == 12 {
                old_record.tile[2] = 200.;
            }
            let records = [record, old_record];
            let mut links = vec![1u32];
            links.extend_from_slice(bytemuck::cast_slice::<crate::shared_patches::Patch, u32>(
                &[patch],
            ));
            let make = |bytes: &[u8], usage| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytes,
                    usage,
                })
            };
            let buffers = [
                (
                    1,
                    make(bytemuck::cast_slice(&atlas), wgpu::BufferUsages::STORAGE),
                ),
                (
                    2,
                    make(bytemuck::cast_slice(&records), wgpu::BufferUsages::STORAGE),
                ),
                (
                    3,
                    make(bytemuck::cast_slice(&links), wgpu::BufferUsages::STORAGE),
                ),
                (
                    4,
                    make(bytemuck::bytes_of(&view), wgpu::BufferUsages::UNIFORM),
                ),
                (
                    6,
                    make(
                        &[0u8; 32],
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    ),
                ),
                (
                    7,
                    make(bytemuck::bytes_of(&patch), wgpu::BufferUsages::STORAGE),
                ),
            ];
            let entries = buffers
                .iter()
                .map(|(binding, b)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: b.as_entire_binding(),
                })
                .collect::<Vec<_>>();
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 32,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&buffers[4].1, 0, &read, 0, 32);
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            read.slice(..)
                .map_async(wgpu::MapMode::Read, move |v| send.send(v).unwrap());
            let _ = device.poll(wgpu::Maintain::Wait);
            receive.recv()??;
            let bytes = read.slice(..).get_mapped_range();
            let actual = bytemuck::cast_slice::<u8, f32>(&bytes);
            assert!(
                (actual[0] - 200.).abs() < 0.001 && actual[1] == 1.,
                "実際の三角形上に地点を置けていない: {actual:?}"
            );
            assert!(
                (actual[2] - 175.).abs() < 0.001,
                "固定スナップショットの旧三角形と新面の中間値が不正: z{old_level} {actual:?}"
            );
            assert!(
                (actual[4] - 200.).abs() < 0.001 && actual[5] == 1.,
                "旧表示の三角形の高さを再現できていない: {actual:?}"
            );
        }
        Ok(())
    }

    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
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
            let shader =
                crate::gpu_profile::timed("shared_render.rs:d.create_shader_module", || {
                    d.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: None,
                        source: wgpu::ShaderSource::Wgsl(
                            include_str!("sibson_gpu_controls.wgsl").into(),
                        ),
                    })
                });
            let pipeline =
                crate::gpu_profile::timed("shared_render.rs:d.create_compute_pipeline", || {
                    d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: None,
                        layout: None,
                        module: &shader,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        cache: None,
                    })
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
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
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
        let shader = crate::gpu_profile::timed(
            "shared_render.rs:device.create_shader_module",
            || {
                device.create_shader_module(wgpu::ShaderModuleDescriptor{label:None,source:wgpu::ShaderSource::Wgsl("struct P {depth:vec4<f32>} @group(0) @binding(0) var<uniform> p:P; struct V{@builtin(position) position:vec4<f32>,@location(0) color:vec4<f32>} @vertex fn vs(@builtin(vertex_index)i:u32,@builtin(instance_index)j:u32)->V {let q=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));return V(vec4(q[i],p.depth[j],1.),select(vec4(1.,0.,0.,1.),vec4(0.,0.,1.,1.),j==1u));} @fragment fn fs(v:V)->@location(0)vec4<f32>{return v.color;}".into())})
            },
        );
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
                let pipeline = crate::gpu_profile::timed(
                    "shared_render.rs:device.create_render_pipeline",
                    || {
                        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                        })
                    },
                );
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
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
    fn partial_seam_preserves_xy_and_follows_coarse_edge_on_gpu() -> anyhow::Result<()> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            eprintln!("GPUがないためGPU試験を省略します");
            return Ok(());
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None))?;
        let view = View {
            earth: [0.; 4],
            eye: [0.; 4],
            frame: [0.; 4],
            matrix: Mat4::IDENTITY.to_cols_array_2d(),
            offset: [0., 0., 1., 0.],
            settings: [2., 0., 1., 0.25],
            flags: [8, 1, 0, 0],
            extra: [2, 0, 0, 0],
        };
        let patches = [
            crate::shared_patches::Patch {
                bounds: [0., 0., 1., 4.],
                edges: [0.5; 4],
                origins: [0.; 4],
                lines: [0., 1., 0., 1.],
            },
            // 粗い辺の途中にある短い辺。従来は始点のY=.25がY=.5へ移動した。
            crate::shared_patches::Patch {
                bounds: [0., 0.25, 0.25, 1.],
                edges: [0.5; 4],
                origins: [0.; 4],
                lines: [0., 0.25, 0.25, 0.5],
            },
        ];
        let mut patches = patches.to_vec();
        patches.push(crate::shared_patches::Patch {
            bounds: [0., 0.25, 0.25, 1.],
            edges: [16., 16., 0., 0.],
            origins: [-8.; 4],
            lines: [0., 0.25, 0.25, 0.5],
        });
        patches.push(crate::shared_patches::Patch {
            bounds: [8., 8.25, 0.25, 1.],
            edges: [0.5, 0., 0., 0.],
            origins: [0.; 4],
            lines: [0., 0.25, 0.25, 0.5],
        });
        let mut atlas = Vec::<[f32; 8]>::new();
        for y in 0..=8 {
            for _ in 0..=8 {
                let height = (y as f32 / 4.).powi(2);
                atlas.push([height, 0., y as f32 / 2., 0., height, 0., y as f32 / 2., 0.]);
            }
        }
        // 細タイル外の参照先は標高3の背景であり、細タイル端の傾きを延長しない。
        let records = [
            Record {
                blend_data: [0; 4],
                tile_end: [0.; 4],
                transition: [0; 4],
                draw: [0; 4],
                tile: [0., 0., 2., 0.25],
                bounds: [0., 0., 2., 0.],
                data: [0, 8, 6, 3],
                neighbors: [0; 4],
            },
            Record {
                blend_data: [0; 4],
                tile_end: [0.; 4],
                transition: [0; 4],
                draw: [0; 4],
                tile: [-8., -8., 16., 2.],
                bounds: [-8., -8., 16., 1.],
                data: [81, 1, 0, 3],
                neighbors: [0; 4],
            },
        ];
        let mut records = records.to_vec();
        records.push(Record {
            blend_data: [0; 4],
            tile_end: [0.; 4],
            transition: [0; 4],
            draw: [0; 4],
            tile: [0., 0., 2., 0.25],
            bounds: [0., 0., 2., 0.],
            data: [85, 1, 5, 3],
            neighbors: [0; 4],
        });
        atlas.extend([[3., 0., 0., 0., 3., 0., 0., 0.]; 4]);
        // 旧LODは大きく高さが違う。境界だけは途中の補間率でも新面と一致させる。
        atlas.extend([[50., 0., 0., 0., 50., 0., 0., 0.]; 4]);
        let make = |label, bytes: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage,
            })
        };
        let buffers = [
            (
                1,
                make(
                    "境界試験の視点",
                    bytemuck::bytes_of(&view),
                    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                ),
            ),
            (
                2,
                make(
                    "部分接合パッチ",
                    bytemuck::cast_slice(&patches),
                    wgpu::BufferUsages::STORAGE,
                ),
            ),
            (
                3,
                make(
                    "曲がった曲面",
                    bytemuck::cast_slice(&atlas),
                    wgpu::BufferUsages::STORAGE,
                ),
            ),
            (
                4,
                make(
                    "曲面範囲",
                    bytemuck::cast_slice(&records),
                    wgpu::BufferUsages::STORAGE,
                ),
            ),
            (
                5,
                make(
                    "接合参照",
                    bytemuck::cast_slice(&[0u32, 1, 2]),
                    wgpu::BufferUsages::STORAGE,
                ),
            ),
            (
                8,
                make(
                    "接合頂点出力",
                    &[0u8; 64],
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                ),
            ),
        ];
        let source = format!(
            "{}\n@group(0) @binding(8) var<storage,read_write> result:array<vec4<f32>>;\n@compute @workgroup_size(1) fn probe(){{if (view.extra.w&8u)!=0u{{result[0]=vec4(boundarySample(vec2(0.,0.25)),1.);return;}}if view.flags.w==1u{{result[3]=vec4(gridVertex(0u,3u).position,1.);return;}}result[0]=vec4(gridVertex(5u,0u).position,1.);result[1]=vec4(gridVertex(0u,1u).position,1.);result[2]=vec4(gridVertex(0u,2u).position,1.);}}",
            super::render_shader_source()
        );
        let module =
            crate::gpu_profile::timed("実描画シェーダーの境界頂点試験", || {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("実描画シェーダーの境界頂点試験"),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                })
            });
        let pipeline =
            crate::gpu_profile::timed("shared_render.rs:device.create_compute_pipeline", || {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: None,
                    layout: None,
                    module: &module,
                    entry_point: Some("probe"),
                    compilation_options: Default::default(),
                    cache: None,
                })
            });
        let entries = buffers
            .iter()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&buffers[5].1, 0, &read, 0, 64);
        queue.submit([encoder.finish()]);
        let mut other = view;
        other.flags[3] = 1;
        other.offset = [0., 0., 1., 0.];
        queue.write_buffer(&buffers[0].1, 0, bytemuck::bytes_of(&other));
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&buffers[5].1, 0, &read, 0, 64);
        queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        read.slice(..)
            .map_async(wgpu::MapMode::Read, move |v| send.send(v).unwrap());
        let _ = device.poll(wgpu::Maintain::Wait);
        receive.recv()??;
        let data = read.slice(..).get_mapped_range();
        let points = bytemuck::cast_slice::<u8, [f32; 4]>(&data);
        for point in &points[..2] {
            assert!(
                (point[0]).abs() < 1e-6 && (point[2] - 0.25).abs() < 1e-6,
                "境界のXYが動いた: {point:?}"
            );
            assert!(
                (point[1] - 0.125).abs() < 1e-6,
                "粗い辺の線分に載っていない: {point:?}"
            );
        }
        assert!(
            (points[2][1] - 3.).abs() < 1e-6,
            "細タイル外へ傾きを延長して壁ができた: {:?}",
            points[2]
        );
        assert_eq!(points[0], points[3], "異LOD側の境界頂点が一致しない");
        drop(data);
        read.unmap();
        for morph in [0., 0.25, 0.5, 1.] {
            let mut uniform = view;
            uniform.extra[3] = 8;
            uniform.settings[3] = morph;
            queue.write_buffer(&buffers[0].1, 0, bytemuck::bytes_of(&uniform));
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&buffers[5].1, 0, &read, 0, 64);
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            read.slice(..)
                .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
            let _ = device.poll(wgpu::Maintain::Wait);
            receive.recv()??;
            let data = read.slice(..).get_mapped_range();
            let v = bytemuck::cast_slice::<u8, f32>(&data);
            let expected = 50. * (1. - morph) + 0.0625 * morph;
            assert!(
                (v[0] - expected).abs() < 1e-5,
                "境界だけが先に新しい高さへ飛んだ: {morph} {}",
                v[0]
            );
            drop(data);
            read.unmap();
        }
        Ok(())
    }
    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
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
        for lower_background in [false, true] {
            let fine = renderer.upload(
                &Packet::constant_plane(
                    if lower_background {
                        (13, 7220, 3200)
                    } else {
                        (6, 56, 25)
                    },
                    1500.,
                )?,
                1,
            )?;
            let coarse = renderer.upload(
                &Packet::constant_plane(
                    if lower_background {
                        (5, 28, 12)
                    } else {
                        (0, 0, 0)
                    },
                    if lower_background { 500. } else { 2000. },
                )?,
                if lower_background { 32 } else { 1 },
            )?;
            let mut camera = Camera::default();
            camera.set_observer(
                [
                    fine.origin[0] + fine.side / 2.,
                    fine.origin[1]
                        + if lower_background {
                            fine.side + 1000.
                        } else {
                            fine.side / 2.
                        },
                ],
                if lower_background { 2000. } else { 1502. },
                0.,
                if lower_background { 0.6 } else { 0. },
                0.8,
                [64., 64.],
            );
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64., 64.));
            for tin in [false, true] {
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
                if lower_background {
                    assert!(
                        pixel[67] >= 10499.9 && pixel[67] <= 11500.1,
                        "詳細面の手前で低い背景まで消え、隙間になった: {} (TIN={tin})",
                        pixel[67]
                    );
                } else {
                    assert_eq!(pixel[3], 0., "粗い背景が上空に描画されている: {pixel:?}");
                    assert!(
                        (pixel[67] - 11500.).abs() < 0.1,
                        "細かい地面が消えている: {}",
                        pixel[67]
                    );
                }
                drop(range);
                buffer.unmap();
            }
        }
        Ok(())
    }
    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
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
        let mut renderer = Renderer::new(state.clone(), &eframe::egui::Context::default());
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
        let mut renderer = Renderer::new(state, &eframe::egui::Context::default());
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
