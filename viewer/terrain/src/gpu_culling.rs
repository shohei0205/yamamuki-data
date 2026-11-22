//! 可視パッチをGPUで詰め、分割数ごとに間接描画する試験経路。
use eframe::wgpu;
use std::ops::Range;
use wgpu::util::DeviceExt;
pub(crate) struct Pipelines {
    bounds: wgpu::ComputePipeline,
    cull: wgpu::ComputePipeline,
}
impl Pipelines {
    pub fn new(device: &wgpu::Device) -> Self {
        let bounds_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("地形の標高範囲"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_culling_bounds.wgsl").into()),
        });
        let cull_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("地形GPUカリング"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("globe.wgsl"),
                    include_str!("gpu_culling.wgsl")
                )
                .into(),
            ),
        });
        let pipeline = |label, module| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            bounds: pipeline("地形標高範囲を集約", &bounds_shader),
            cull: pipeline("可視パッチを選択", &cull_shader),
        }
    }
}
pub(crate) struct Draw {
    pub n: u32,
    pub binding: wgpu::BindGroup,
    pub arguments: wgpu::Buffer,
    cull: wgpu::BindGroup,
    planned: bool,
    input: wgpu::Buffer,
    range: Range<u32>,
}
pub(crate) struct Scene {
    heights: wgpu::Buffer,
    bounds: wgpu::BindGroup,
    pub draws: Vec<Vec<Draw>>,
    count: u32,
    bounds_ready: bool,
    planned_ready: bool,
}
fn binding(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}
impl Scene {
    pub fn new(
        device: &wgpu::Device,
        pipelines: &Pipelines,
        atlas: &wgpu::Buffer,
        records: &wgpu::Buffer,
        count: usize,
    ) -> Self {
        let heights = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("地形標高範囲"),
            size: (count.max(1) * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let bounds = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("地形標高範囲"),
            layout: &pipelines.bounds.get_bind_group_layout(0),
            entries: &[binding(0, atlas), binding(1, records), binding(2, &heights)],
        });
        Self {
            heights,
            bounds,
            draws: (0..count).map(|_| Vec::new()).collect(),
            count: count as u32,
            bounds_ready: false,
            planned_ready: false,
        }
    }
    pub fn group(
        &mut self,
        device: &wgpu::Device,
        pipelines: &Pipelines,
        tile: usize,
        group: usize,
        n: u32,
        range: Range<u32>,
        planned: bool,
        buffers: [&wgpu::Buffer; 8],
        view_offset: u64,
        view_size: u64,
        graphics_layout: &wgpu::BindGroupLayout,
    ) {
        let changed = self.draws[tile].get(group).is_none_or(|old| {
            old.n != n || old.range != range || old.input != *buffers[2] || old.planned != planned
        });
        if !changed {
            return;
        }
        self.planned_ready = false;
        self.draws[tile].truncate(group);
        let visible = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("可視地形パッチ"),
            size: u64::from(range.end - range.start).max(1) * 64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let arguments = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("地形の間接描画命令"),
            contents: bytemuck::cast_slice(&[6 * n * n, 0u32, 0, 0, 0]),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("カリング対象区画"),
            contents: bytemuck::cast_slice(&[
                range.start,
                range.end - range.start,
                n,
                u32::from(planned),
            ]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let view = wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: buffers[1],
            offset: view_offset,
            size: std::num::NonZeroU64::new(view_size),
        });
        let cull = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("パッチのGPU選択"),
            layout: &pipelines.cull.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view.clone(),
                },
                binding(1, buffers[2]),
                binding(2, buffers[4]),
                binding(3, buffers[5]),
                binding(4, &self.heights),
                binding(5, &visible),
                binding(6, &arguments),
                binding(7, &params),
            ],
        });
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if i == 1 {
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: view.clone(),
                    }
                } else if i == 2 {
                    binding(2, &visible)
                } else {
                    binding(i as u32, b)
                }
            })
            .collect();
        let graphics = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GPU選択後の地形"),
            layout: graphics_layout,
            entries: &entries,
        });
        self.draws[tile].push(Draw {
            n,
            binding: graphics,
            arguments,
            cull,
            planned,
            input: buffers[2].clone(),
            range,
        });
    }
    pub fn bytes(&self) -> usize {
        self.heights.size() as usize
            + self
                .draws
                .iter()
                .flatten()
                .map(|draw| (draw.range.end - draw.range.start) as usize * 64 + 36)
                .sum::<usize>()
    }
    pub fn truncate(&mut self, tile: usize, count: usize) {
        self.draws[tile].truncate(count);
    }
    pub fn invalidate_plan(&mut self) {
        self.planned_ready = false;
    }
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &Pipelines,
        planned: bool,
    ) {
        if planned && self.planned_ready {
            return;
        }
        self.planned_ready = planned;
        for draw in self.draws.iter().flatten() {
            encoder.clear_buffer(&draw.arguments, 4, Some(4));
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("地形GPU可視判定"),
            timestamp_writes: None,
        });
        // 元の評価格子は同じCachedSceneの間は不変。視点だけの変更で再集計しない。
        if !planned && !self.bounds_ready {
            pass.set_pipeline(&pipelines.bounds);
            pass.set_bind_group(0, &self.bounds, &[]);
            pass.dispatch_workgroups(self.count, 1, 1);
            self.bounds_ready = true;
        }
        pass.set_pipeline(&pipelines.cull);
        for draw in self.draws.iter().flatten() {
            pass.set_bind_group(0, &draw.cull, &[]);
            pass.dispatch_workgroups((draw.range.end - draw.range.start).div_ceil(64), 1, 1);
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUで可視パッチの選択と間接描画命令を検査する"
    )]
    fn compacts_visible_patches_and_resets_indirect_count() -> anyhow::Result<()> {
        use super::*;
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .ok_or_else(|| anyhow::anyhow!("GPU検査には利用可能なGPUが必要です"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default(), None))?;
        let pipelines = Pipelines::new(&device);
        let buffer = |name, data: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(name),
                contents: data,
                usage,
            })
        };
        let mut view = [0u32; 44];
        for i in [0, 5, 10, 15] {
            view[i] = 1f32.to_bits();
        }
        view[18] = 1f32.to_bits();
        let view_buffer = buffer(
            "試験視点",
            bytemuck::cast_slice(&view),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let samples = buffer(
            "平面の標高",
            bytemuck::cast_slice(&[[0f32, 0., 0., 1., 0., 0., 0., 1.]; 4]),
            wgpu::BufferUsages::STORAGE,
        );
        let mut record = [0u32; 32];
        record[2] = 100f32.to_bits();
        record[5] = 1;
        let records = buffer(
            "試験タイル",
            bytemuck::cast_slice(&record),
            wgpu::BufferUsages::STORAGE,
        );
        let patches = [
            crate::shared_patches::Patch {
                bounds: [0., 0., 0.1, 1.],
                edges: [0.; 4],
                origins: [0.; 4],
                lines: [0.; 4],
            },
            crate::shared_patches::Patch {
                bounds: [10., 0., 0.1, 1.],
                edges: [0.; 4],
                origins: [0.; 4],
                lines: [0.; 4],
            },
        ];
        let patches = buffer(
            "視野内と視野外のパッチ",
            bytemuck::cast_slice(&patches),
            wgpu::BufferUsages::STORAGE,
        );
        let empty = buffer("空の補助情報", &[0; 4], wgpu::BufferUsages::STORAGE);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &(0..8)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: if binding == 1 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage { read_only: true }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
        let mut scene = Scene::new(&device, &pipelines, &samples, &records, 1);
        scene.group(
            &device,
            &pipelines,
            0,
            0,
            1,
            0..2,
            false,
            [
                &samples,
                &view_buffer,
                &patches,
                &samples,
                &records,
                &empty,
                &empty,
                &empty,
            ],
            0,
            176,
            &layout,
        );
        for (expected, planned) in [(1, false), (0, false), (2, true), (2, true)] {
            if planned && !scene.draws[0][0].planned {
                scene.group(
                    &device,
                    &pipelines,
                    0,
                    0,
                    1,
                    0..2,
                    true,
                    [
                        &samples,
                        &view_buffer,
                        &patches,
                        &samples,
                        &records,
                        &empty,
                        &empty,
                        &empty,
                    ],
                    0,
                    176,
                    &layout,
                );
            }
            if expected == 0 {
                view[12] = 10f32.to_bits();
                queue.write_buffer(&view_buffer, 0, bytemuck::cast_slice(&view));
            }
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 20,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            scene.encode(&mut encoder, &pipelines, planned);
            encoder.copy_buffer_to_buffer(&scene.draws[0][0].arguments, 0, &read, 0, 20);
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            read.slice(..)
                .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
            let _ = device.poll(wgpu::Maintain::Wait);
            rx.recv()??;
            let data = read.slice(..).get_mapped_range();
            let args = bytemuck::cast_slice::<u8, u32>(&data);
            assert_eq!(
                args,
                [6, expected, 0, 0, 0],
                "可視数と間接描画命令が一致しない"
            );
        }
        Ok(())
    }
    #[test]
    fn shaders_validate() {
        for source in [
            include_str!("gpu_culling_bounds.wgsl").to_owned(),
            format!(
                "{}\n{}",
                include_str!("globe.wgsl"),
                include_str!("gpu_culling.wgsl")
            ),
        ] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
