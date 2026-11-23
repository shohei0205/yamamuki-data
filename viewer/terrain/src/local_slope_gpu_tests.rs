//! 実機GPUで固定係数1の標高と勾配をCPUの倍精度計算に照合する。
use wgpu::util::DeviceExt;
fn gpu_matches_cpu() -> anyhow::Result<()> {
    use yamamuki_terrain::{
        local_slope::Prepared,
        natural_regional_surface::{Fast, Work},
    };
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12 | wgpu::Backends::METAL,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .ok_or_else(|| anyhow::anyhow!("GPUがありません"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default(), None))?;
    let model = Fast::new(
        (0..9)
            .flat_map(|y| {
                (0..9).map(move |x| {
                    [
                        x as f64 * 10.,
                        y as f64 * 10.,
                        (x * x + 2 * y * y + x * y) as f64,
                    ]
                })
            })
            .collect(),
    )?;
    let prepared = Prepared::new(&model);
    let bounds = [22., 23., 57., 58.];
    let (bytes, info) = yamamuki_terrain::natural_regional_gpu::packet(
        &model,
        &prepared.gradients,
        bounds,
        serde_json::json!({}),
    )?;
    let mut offset = 4 + u32::from_le_bytes(bytes[..4].try_into()?) as usize;
    let mut buffers = Vec::new();
    for name in ["pointBytes", "cellBytes", "nodeBytes", "referenceBytes"] {
        let length = info[name].as_u64().unwrap() as usize;
        buffers.push(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &bytes[offset..offset + length],
                usage: wgpu::BufferUsages::STORAGE,
            }),
        );
        offset += length;
    }
    let n = 8u32;
    let size = u64::from((n + 1) * (n + 1)) * 32;
    buffers.push(device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    }));
    let mut params = Vec::new();
    for v in bounds {
        params.extend(((v / 1000.) as f32).to_le_bytes());
    }
    for v in info["indexBounds"].as_array().unwrap() {
        params.extend(((v.as_f64().unwrap() / 1000.) as f32).to_le_bytes());
    }
    for v in [n, 0, 0, 0] {
        params.extend(v.to_le_bytes());
    }
    for v in [1000f32, 1., 0., 0.] {
        params.extend(v.to_le_bytes());
    }
    buffers.push(
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM,
        }),
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(include_str!("sibson_gpu_controls.wgsl").into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let entries = buffers
        .iter()
        .enumerate()
        .map(|(i, b)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: b.as_entire_binding(),
        })
        .collect::<Vec<_>>();
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(2, 2, 1);
    }
    encoder.copy_buffer_to_buffer(&buffers[4], 0, &read, 0, size);
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    read.slice(..)
        .map_async(wgpu::MapMode::Read, move |v| send.send(v).unwrap());
    device.poll(wgpu::Maintain::Wait);
    receive.recv()??;
    let range = read.slice(..).get_mapped_range();
    let output = bytemuck::cast_slice::<u8, f32>(&range);
    let mut work = Work::default();
    for y in 0..=n {
        for x in 0..=n {
            let q = [
                bounds[0] + (bounds[2] - bounds[0]) * f64::from(x) / f64::from(n),
                bounds[1] + (bounds[3] - bounds[1]) * f64::from(y) / f64::from(n),
            ];
            let sample = &output[((y * (n + 1) + x) * 8) as usize..][..8];
            let pair = prepared.pair(q, model.basis_borrowed(q, &mut work)?)?;
            assert!((f64::from(sample[0]) - pair.original).abs() < 0.01);
            assert!(
                (f64::from(sample[4]) - pair.value(1.)).abs() < 0.01,
                "{q:?}: {sample:?} != {pair:?}"
            );
            assert_eq!(sample[7], 1.);
            for axis in 0..2 {
                let (mut a, mut b) = (q, q);
                a[axis] += 1.;
                b[axis] -= 1.;
                let derivative = (prepared.evaluate(a, 1., &mut work)?
                    - prepared.evaluate(b, 1., &mut work)?)
                    / 2.;
                assert!((f64::from(sample[5 + axis]) - derivative).abs() < 0.01);
            }
        }
    }
    drop(range);
    read.unmap();
    Ok(())
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "実機GPUによる係数1固定の検査")]
fn fixed_slope_gpu_matches_cpu() -> anyhow::Result<()> {
    gpu_matches_cpu()
}
