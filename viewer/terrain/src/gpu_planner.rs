//! 可視判定・分割・接合制約・三角形予算をGPUで決める比較用計画器。
use crate::{
    selection::Camera,
    shared_data::WORLD,
    shared_patches::{Input, Patch, Plan},
};
use anyhow::{Result, ensure};
use eframe::wgpu;
use std::sync::Mutex;
const ITEMS: usize = 16384;
const FIELDS: usize = 4096;
const COVERS: usize = 65536;
const GROUPS: usize = FIELDS * 10;
const WORK_BYTES: u64 = (ITEMS * 64 * 2 + ITEMS * 4 * 64 + GROUPS * 4 * 2) as u64;
const GROUP_BYTES: u64 = (GROUPS * 8) as u64;
const PATCH_BYTES: u64 = (ITEMS * 64) as u64;
const SCRATCH_BYTES: u64 = ((FIELDS * 1024 + 8) * 4) as u64;
const READ_BYTES: u64 = 32 + GROUP_BYTES + PATCH_BYTES;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Source {
    grid: [u32; 4],
    coverage: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    grid: [u32; 4],
    fraction: [f32; 4],
    earth: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    back: [f32; 4],
    camera: [f32; 4],
    viewport: [f32; 4],
    eye: [f32; 4],
    sphere_eye: [f32; 4],
}
struct Buffers {
    source: wgpu::Buffer,
    covers: wgpu::Buffer,
    params: wgpu::Buffer,
    work: wgpu::Buffer,
    scratch: wgpu::Buffer,
    patches: wgpu::Buffer,
    groups: wgpu::Buffer,
    header: wgpu::Buffer,
    read: wgpu::Buffer,
    binding: wgpu::BindGroup,
    last_count: usize,
    read_bytes: u64,
}
pub(crate) struct Planner {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: std::collections::HashMap<&'static str, wgpu::ComputePipeline>,
    buffers: Mutex<Buffers>,
}
impl Planner {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GPUメッシュ分割計画"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("globe.wgsl"),
                    include_str!("gpu_planner.wgsl")
                )
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GPU計画の共通バインド形式"),
            entries: &(0..8)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if binding == 2 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage {
                                read_only: binding < 2,
                            }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipelines = [
            "prepareSources",
            "generateSeeds",
            "prefixSeeds",
            "prefixFields",
            "scatterItems",
            "edgeInit",
            "edgeUnion",
            "edgeReduce",
            "edgeFinalize",
            "prepareBudget",
            "budgetThreshold",
            "budgetApply",
            "finishBudget",
            "edgeDrawReduce",
            "writePatchEdges",
            "serializeFields",
            "completePlan",
            "prepareTasks",
            "markSplit",
            "prefixSplit",
            "scatterSplit",
            "copySplit",
        ]
        .into_iter()
        .map(|entry| {
            (
                entry,
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some(entry),
                    // 各共有領域は使用前に明示的に初期化し、巨大な共有スタックの
                    // 自動ゼロ初期化によるFXCの展開と区画ごとの無駄な書き込みを省く。
                    compilation_options: wgpu::PipelineCompilationOptions {
                        zero_initialize_workgroup_memory: false,
                        ..Default::default()
                    },
                    cache: None,
                }),
            )
        })
        .collect();
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let source = buffer("計画対象の曲面", (FIELDS * 32) as u64, storage);
        let covers = buffer("背景を覆う曲面", (COVERS * 16) as u64, storage);
        let params = buffer(
            "計画視点",
            std::mem::size_of::<Params>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let work = buffer("GPU分割計画の作業領域", WORK_BYTES, storage);
        let scratch = buffer(
            "並列生成の順序と件数",
            SCRATCH_BYTES,
            storage | wgpu::BufferUsages::INDIRECT,
        );
        let patches = buffer(
            "GPU計画パッチ",
            PATCH_BYTES,
            storage | wgpu::BufferUsages::COPY_SRC,
        );
        let groups = buffer(
            "GPU計画の分割数別範囲",
            GROUP_BYTES,
            storage | wgpu::BufferUsages::COPY_SRC,
        );
        let header = buffer("GPU計画の結果", 32, storage | wgpu::BufferUsages::COPY_SRC);
        let read = buffer(
            "GPU計画の診断と接合情報",
            READ_BYTES,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GPUメッシュ計画"),
            layout: &layout,
            entries: &[
                &source, &covers, &params, &work, &patches, &groups, &header, &scratch,
            ]
            .into_iter()
            .enumerate()
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: b.as_entire_binding(),
            })
            .collect::<Vec<_>>(),
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipelines,
            buffers: Mutex::new(Buffers {
                source,
                covers,
                params,
                work,
                scratch,
                patches,
                groups,
                header,
                read,
                binding,
                last_count: ITEMS,
                read_bytes: 0,
            }),
        }
    }
    pub fn bytes(&self) -> usize {
        (SCRATCH_BYTES
            + WORK_BYTES
            + PATCH_BYTES
            + GROUP_BYTES
            + READ_BYTES
            + 32
            + 160
            + (FIELDS * 32 + COVERS * 16) as u64) as usize
    }
    pub fn plan(&self, inputs: &[Input], camera: &Camera) -> Result<Vec<Plan>> {
        ensure!(
            inputs.len() <= FIELDS,
            "GPU計画の評価区画が上限を超えています"
        );
        let unit = WORLD / (1u64 << 30) as f64;
        let coord = |value: f64| -> Result<u32> {
            let grid = (value / unit).round();
            ensure!(
                value.is_finite()
                    && grid >= 0.
                    && grid <= (1u64 << 30) as f64
                    && (grid * unit - value).abs() < 0.001,
                "GPU計画にはWeb Mercatorの四分木に沿う評価区画が必要です"
            );
            Ok(grid as u32)
        };
        let mut sources = Vec::new();
        let mut covers = Vec::<[u32; 4]>::new();
        for input in inputs {
            let offset = covers.len();
            for b in &input.covered_by {
                covers.push([coord(b[0])?, coord(b[1])?, coord(b[2])?, coord(b[3])?]);
            }
            let step = (input.side / unit).round() as u32;
            ensure!(
                step > 0 && step.is_power_of_two() && input.n.is_power_of_two() && input.n <= 512,
                "評価格子が不正です"
            );
            sources.push(Source {
                grid: [
                    coord(input.origin[0])?,
                    coord(input.origin[1])?,
                    step,
                    input.n,
                ],
                coverage: [offset as u32, (covers.len() - offset) as u32, 0, 0],
            });
        }
        ensure!(
            covers.len() <= COVERS,
            "GPU計画の背景被覆情報が上限を超えています"
        );
        let center = camera.center.map(|v| (v / unit).floor() as u32);
        let fraction = std::array::from_fn::<_, 2, _>(|i| {
            (camera.center[i] - f64::from(center[i]) * unit) as f32
        });
        let latitude = crate::globe::latitude(camera.center[1]);
        let right = glam::DVec3::new(camera.yaw.cos(), 0., -camera.yaw.sin());
        let back = camera.back();
        let up = back.cross(right);
        let eye = camera.eye();
        let eye_scale = crate::globe::latitude(eye[1]).cos();
        let sphere_eye = if camera.globe {
            let local = back * camera.distance + glam::DVec3::Y * camera.height;
            let world = crate::globe::world(camera.center, local);
            [
                world[0],
                world[1],
                (local + glam::DVec3::Y * crate::globe::RADIUS).length() - crate::globe::RADIUS,
            ]
        } else {
            eye
        };
        let params = Params {
            grid: [center[0], center[1], inputs.len() as u32, 0],
            fraction: [
                fraction[0],
                fraction[1],
                unit as f32,
                (unit - f64::from(unit as f32)) as f32,
            ],
            earth: [
                latitude.sin() as f32,
                latitude.cos() as f32,
                crate::globe::RADIUS as f32,
                f32::from(camera.globe),
            ],
            right: [
                right.x as f32,
                right.y as f32,
                right.z as f32,
                (camera.distance / 10000.).max(0.2) as f32,
            ],
            up: [
                up.x as f32,
                up.y as f32,
                up.z as f32,
                camera.far_depth() as f32,
            ],
            back: [
                back.x as f32,
                back.y as f32,
                back.z as f32,
                camera.distance as f32,
            ],
            camera: [
                camera.height as f32,
                camera.tan_half_fov() as f32,
                (camera.viewport[0] / camera.viewport[1]) as f32,
                camera.scale() as f32,
            ],
            viewport: [
                camera.viewport[1] as f32,
                camera.mesh_pixels as f32,
                f32::from(camera.observer),
                eye_scale as f32,
            ],
            eye: [
                (eye[0] - camera.center[0]) as f32,
                (eye[1] - camera.center[1]) as f32,
                eye[2] as f32,
                0.,
            ],
            sphere_eye: [
                (sphere_eye[0] - camera.center[0]) as f32,
                (sphere_eye[1] - camera.center[1]) as f32,
                sphere_eye[2] as f32,
                0.,
            ],
        };
        let mut b = self
            .buffers
            .lock()
            .map_err(|_| anyhow::anyhow!("GPU計画の作業領域が利用できません"))?;
        if !sources.is_empty() {
            self.queue
                .write_buffer(&b.source, 0, bytemuck::cast_slice(&sources));
        }
        if !covers.is_empty() {
            self.queue
                .write_buffer(&b.covers, 0, bytemuck::cast_slice(&covers));
        }
        self.queue
            .write_buffer(&b.params, 0, bytemuck::bytes_of(&params));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&b.work, 0, None);
        encoder.clear_buffer(&b.header, 0, None);
        encoder.clear_buffer(&b.scratch, 0, Some(((inputs.len() * 1024 + 8) * 4) as u64));
        let sources = inputs.len().div_ceil(64) as u32;
        let edges = (ITEMS as u32 * 4).div_ceil(64);
        let patches = (ITEMS as u32).div_ceil(64);
        let stages = [
            ("prepareSources", sources, 1),
            ("prepareTasks", 1, 1),
            ("generateSeeds", 0, 0),
            ("prefixSeeds", sources, 1),
            ("prefixFields", 1, 1),
            ("scatterItems", patches, 1),
            ("edgeInit", edges, 1),
            ("edgeUnion", edges, 1),
            ("edgeReduce", edges, 1),
            ("edgeFinalize", edges, 1),
            ("markSplit", patches, 1),
            ("prefixSplit", 1, 1),
            ("scatterSplit", patches, 1),
            ("copySplit", patches, 1),
            ("edgeInit", edges, 1),
            ("edgeUnion", edges, 1),
            ("edgeReduce", edges, 1),
            ("edgeFinalize", edges, 1),
            ("prepareBudget", patches, 1),
            ("budgetThreshold", 1, 1),
            ("budgetApply", patches, 1),
            ("finishBudget", 1, 1),
            ("edgeDrawReduce", edges, 1),
            ("writePatchEdges", patches, 1),
            ("serializeFields", sources, 1),
            ("completePlan", 1, 1),
        ];
        let profile = std::env::var_os("YAMAMUKI_GPU_PLAN_PROFILE").is_some();
        for (stage, x, y) in stages {
            let started = std::time::Instant::now();
            if stage == "edgeInit" {
                encoder.clear_buffer(&b.scratch, 8 * 4, Some(65536 * 4));
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("並列GPU計画"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines[stage]);
            pass.set_bind_group(0, &b.binding, &[]);
            if stage == "generateSeeds" {
                pass.dispatch_workgroups_indirect(&b.scratch, 16);
            } else {
                pass.dispatch_workgroups(x.max(1), y.max(1), 1);
            }
            if profile {
                drop(pass);
                self.queue.submit([encoder.finish()]);
                let _ = self.device.poll(wgpu::Maintain::Wait);
                eprintln!(
                    "GPU計画区間 stage={stage} {:.3}ms",
                    started.elapsed().as_secs_f64() * 1000.
                );
                encoder = self.device.create_command_encoder(&Default::default());
            }
        }
        // 前回の件数に余裕を付けた範囲だけを読み戻す。急増時だけ追加取得し、
        // 読み戻し量を抑えるために分割数やLODを減らすことはしない。
        let active_groups = (inputs.len() * 10 * 8) as u64;
        let capacity = (b.last_count + b.last_count / 2)
            .max(64)
            .next_power_of_two()
            .min(ITEMS);
        let patch_start = 32 + active_groups;
        let read_size = patch_start + (capacity * 64) as u64;
        encoder.copy_buffer_to_buffer(&b.header, 0, &b.read, 0, 32);
        if active_groups > 0 {
            encoder.copy_buffer_to_buffer(&b.groups, 0, &b.read, 32, active_groups);
        }
        encoder.copy_buffer_to_buffer(&b.patches, 0, &b.read, patch_start, (capacity * 64) as u64);
        self.queue.submit([encoder.finish()]);
        let mut mapped = self.read_range(&b, read_size)?;
        let count = u32::from_le_bytes(mapped[0..4].try_into().unwrap()) as usize;
        b.read_bytes = read_size;
        if count > capacity && count <= ITEMS {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&b.patches, 0, &b.read, 0, (count * 64) as u64);
            self.queue.submit([encoder.finish()]);
            let extra = self.read_range(&b, (count * 64) as u64)?;
            mapped.truncate(patch_start as usize);
            mapped.extend_from_slice(&extra);
            b.read_bytes += (count * 64) as u64;
        }
        let header = bytemuck::cast_slice::<u8, u32>(&mapped[..32]);
        let result = (|| {
            ensure!(
                header[1] == 0 && header[6] == 1,
                "GPU計画が完了しませんでした（状態{}・完了{}）",
                header[1],
                header[6]
            );
            let count = header[0] as usize;
            ensure!(count <= ITEMS, "GPU計画のパッチ数が不正です");
            let groups = bytemuck::cast_slice::<u8, [u32; 2]>(&mapped[32..patch_start as usize]);
            let patches = bytemuck::cast_slice::<u8, Patch>(
                &mapped[patch_start as usize..patch_start as usize + count * 64],
            );
            let mut result = Vec::with_capacity(inputs.len());
            for tile in 0..inputs.len() {
                let start = groups[tile * 10][0];
                let mut end = start;
                let mut spans = Vec::new();
                for level in 0..10 {
                    let [offset, count] = groups[tile * 10 + level];
                    ensure!(
                        (offset + count) as usize <= patches.len(),
                        "GPU計画の範囲が不正です"
                    );
                    if count > 0 {
                        spans.push((1u32 << level, offset - start..offset - start + count));
                    }
                    end = offset + count;
                }
                let converted = patches[start as usize..end as usize]
                    .iter()
                    .map(|p| {
                        // 判定・分割・接合はGPUで完了済み。既存描画形式への座標変換だけを行う。
                        let mut p = *p;
                        for axis in 0..3 {
                            p.bounds[axis] = (f64::from(p.bounds[axis].to_bits()) * unit) as f32;
                        }
                        p.bounds[3] = p.bounds[3].to_bits() as f32;
                        for edge in 0..4 {
                            let axis = if edge < 2 { 1 } else { 0 };
                            p.origins[edge] = (f64::from(p.origins[edge].to_bits()) * unit
                                - camera.center[axis])
                                as f32;
                            p.lines[edge] = (f64::from(p.lines[edge].to_bits()) * unit
                                - camera.center[edge / 2])
                                as f32;
                        }
                        p
                    })
                    .collect();
                result.push(Plan {
                    patches: converted,
                    groups: spans,
                });
            }
            Ok(result)
        })();
        if result.is_ok() {
            b.last_count = count;
        }
        result
    }
    fn read_range(&self, b: &Buffers, size: u64) -> Result<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        b.read
            .slice(0..size)
            .map_async(wgpu::MapMode::Read, move |v| {
                let _ = tx.send(v);
            });
        let _ = self.device.poll(wgpu::Maintain::Wait);
        rx.recv()??;
        let mapped = b.read.slice(0..size).get_mapped_range();
        let copy = mapped.to_vec();
        drop(mapped);
        b.read.unmap();
        Ok(copy)
    }
    #[cfg(test)]
    pub fn last_readback_bytes(&self) -> u64 {
        self.buffers.lock().unwrap().read_bytes
    }
}
#[cfg(test)]
mod tests {
    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUで可視判定・被覆除外・分割・接合計画を比較する"
    )]
    fn plans_match_cpu_across_view_changes() -> anyhow::Result<()> {
        use super::*;
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12 | wgpu::Backends::METAL,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .ok_or_else(|| anyhow::anyhow!("GPU検査には利用可能なGPUが必要です"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None))?;
        eprintln!("GPU計画fixture {:?}", adapter.get_info());
        let planner = Planner::new(&device, &queue);
        let side = WORLD / 256.;
        let origin = [side * 224., side * 98.];
        let inputs = vec![
            Input {
                origin,
                side,
                n: 64,
                covered_by: Vec::new(),
            },
            Input {
                origin: [origin[0] + side, origin[1]],
                side,
                n: 32,
                covered_by: Vec::new(),
            },
            Input {
                origin,
                side: side * 2.,
                n: 64,
                covered_by: vec![
                    [origin[0], origin[1], origin[0] + side, origin[1] + side],
                    [
                        origin[0] + side,
                        origin[1],
                        origin[0] + 2. * side,
                        origin[1] + side,
                    ],
                ],
            },
        ];
        for globe in [false, true] {
            for observer in [false, true] {
                for yaw in [0., 0.9, 2.7] {
                    let camera = Camera {
                        center: [origin[0] + side, origin[1] + side],
                        distance: if observer { 1000. } else { 200000. },
                        height: 1200.,
                        pitch: if observer { 1.45 } else { 0.8 },
                        yaw,
                        globe,
                        observer,
                        ..Default::default()
                    };
                    let cpu = crate::shared_patches::plan(&inputs, &camera);
                    let gpu = planner.plan(&inputs, &camera)?;
                    assert_eq!(cpu.len(), gpu.len());
                    for (a, b) in cpu.iter().zip(&gpu) {
                        assert_eq!(
                            a.groups, b.groups,
                            "globe={globe} observer={observer} yaw={yaw}"
                        );
                        assert_eq!(a.patches.len(), b.patches.len());
                        for (a, b) in bytemuck::cast_slice::<_, f32>(&a.patches)
                            .iter()
                            .zip(bytemuck::cast_slice::<_, f32>(&b.patches))
                        {
                            assert!(
                                (a - b).abs() < 0.005,
                                "globe={globe} observer={observer} yaw={yaw} CPU={a} GPU={b}"
                            );
                        }
                    }
                }
            }
        }
        // 件数急増時の追加読み戻しと、予算超過時の半減優先順位を検査する。
        let dense = vec![Input {
            origin,
            side,
            n: 512,
            covered_by: Vec::new(),
        }];
        let camera = Camera {
            center: [origin[0] + side * 0.5, origin[1] + side * 0.5],
            distance: 200000.,
            viewport: [4800., 3200.],
            mesh_pixels: 0.5,
            ..Default::default()
        };
        let reference = crate::shared_patches::plan(&dense, &camera);
        let expected = reference.iter().map(|p| p.patches.len()).sum::<usize>();
        ensure!(expected > 64, "追加読み戻しを検査できる件数が必要です");
        planner.buffers.lock().unwrap().last_count = 0;
        let result = planner.plan(&dense, &camera)?;
        assert_eq!(
            planner.last_readback_bytes(),
            32 + 80 + 64 * 64 + (expected * 64) as u64
        );
        for (a, b) in reference.iter().zip(&result) {
            assert_eq!(a.groups, b.groups);
            for (a, b) in a.patches.iter().zip(&b.patches) {
                assert_eq!(a.bounds, b.bounds, "密度と区画の選択は一致させる");
                // 元のf64加算と整数格子の変換では数nmの丸め差が出る。
                for (a, b) in bytemuck::cast_slice::<_, f32>(std::slice::from_ref(a))
                    .iter()
                    .zip(bytemuck::cast_slice::<_, f32>(std::slice::from_ref(b)))
                {
                    assert!((a - b).abs() <= 0.0000001, "CPU={a} GPU={b}");
                }
            }
        }
        assert!(planner.plan(&[], &camera)?.is_empty());
        let mut invalid = inputs;
        invalid[0].origin[0] += 0.01;
        assert!(
            planner.plan(&invalid, &Camera::default()).is_err(),
            "不正な格子は明示的なエラーにする"
        );
        Ok(())
    }
    #[test]
    fn shader_validates() {
        let source = format!(
            "{}\n{}",
            include_str!("globe.wgsl"),
            include_str!("gpu_planner.wgsl")
        );
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
