//! GPUで地点の地形高と遮蔽を求め、読み戻しは次フレーム以降に非同期で行う。
use crate::{selection::Camera, shared_data::WORLD};
use eframe::{egui, wgpu};
use std::sync::{Mutex, mpsc};
use wgpu::util::DeviceExt;
#[derive(Default)]
pub struct State {
    request: Option<Request>,
    pending: Option<Pending>,
    result: Option<(u64, Vec<[f32; 4]>)>,
    result_points: u64,
}
struct Request {
    id: u64,
    points_id: u64,
    points: Vec<[f32; 4]>,
}
struct Pending {
    id: u64,
    points_id: u64,
    len: usize,
    buffer: wgpu::Buffer,
    receive: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
}
pub struct Renderer {
    pipeline: wgpu::ComputePipeline,
}
pub fn shader() -> String {
    let source = include_str!("shared_render.wgsl");
    let begin = source.find("fn sampleAt(").unwrap();
    let end = source.find("@vertex fn vs(").unwrap();
    format!(
        "{}\n{}",
        include_str!("point_visibility.wgsl"),
        &source[begin..end]
    )
}
impl Renderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("地点の地形高と遮蔽"),
            source: wgpu::ShaderSource::Wgsl(shader().into()),
        });
        Self {
            pipeline: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("地点の地形高と遮蔽"),
                layout: None,
                module: &module,
                entry_point: Some("query"),
                compilation_options: Default::default(),
                cache: None,
            }),
        }
    }
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        state: &Mutex<State>,
        positions: &wgpu::TextureView,
        atlas: &wgpu::Buffer,
        records: &wgpu::Buffer,
        links: &wgpu::Buffer,
        view: &wgpu::Buffer,
    ) {
        let mut state = state.lock().unwrap();
        if state.pending.is_some() {
            return;
        }
        let Some(request) = state.request.take() else {
            return;
        };
        if request.points.is_empty() {
            state.result = Some((request.id, Vec::new()));
            return;
        }
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("地点座標照会"),
            contents: bytemuck::cast_slice(&request.points),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let size = (request.points.len() * 16) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("地点の遮蔽結果"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("地点の遮蔽の読み戻し"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(positions),
        }];
        for (binding, buffer) in [
            (1, atlas),
            (2, records),
            (3, links),
            (4, view),
            (5, &input),
            (6, &output),
        ] {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            });
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("地点遮蔽の照会"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((request.points.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &read, 0, size);
        state.pending = Some(Pending {
            id: request.id,
            points_id: request.points_id,
            len: request.points.len(),
            buffer: read,
            receive: None,
        });
    }
}
pub fn query(
    device: &wgpu::Device,
    ctx: &egui::Context,
    state: &Mutex<State>,
    camera: &Camera,
    id: u64,
    points: &[[f64; 3]],
) -> Option<Vec<[f32; 4]>> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for p in points {
        for v in p {
            v.to_bits().hash(&mut hash);
        }
    }
    let points_id = hash.finish();
    let mut state = state.lock().unwrap();
    if let Some(pending) = &mut state.pending {
        if pending.receive.is_none() {
            let (send, receive) = mpsc::channel();
            let wake = ctx.clone();
            pending
                .buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = send.send(result);
                    wake.request_repaint();
                });
            pending.receive = Some(receive);
        }
    }
    let _ = device.poll(wgpu::Maintain::Poll);
    let completed = state
        .pending
        .as_ref()
        .and_then(|pending| pending.receive.as_ref()?.try_recv().ok());
    if let Some(result) = completed {
        let pending = state.pending.take().unwrap();
        let values = if result.is_ok() {
            let range = pending.buffer.slice(..).get_mapped_range();
            bytemuck::cast_slice::<u8, [f32; 4]>(&range).to_vec()
        } else {
            vec![[0.; 4]; pending.len]
        };
        pending.buffer.unmap();
        state.result_points = pending.points_id;
        state.result = Some((pending.id, values));
    }
    if let Some((ready, result)) = &state.result {
        if *ready == id {
            return Some(result.clone());
        }
    }
    if !state
        .pending
        .as_ref()
        .is_some_and(|pending| pending.id == id)
    {
        state.request = Some(Request {
            id,
            points_id,
            points: points
                .iter()
                .map(|p| {
                    [
                        (p[0] * WORLD - camera.center[0]) as f32,
                        (p[1] * WORLD - camera.center[1]) as f32,
                        p[2] as f32,
                        0.,
                    ]
                })
                .collect(),
        });
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    // 更新待ちでも直前の遮蔽判定を保持し、位置は現在のカメラで投影し直す。
    // 地点集合が変わった場合は別の地点の結果を使わない。
    if state.result_points == points_id {
        if let Some((_, values)) = &state.result {
            let projection = crate::Projection {
                camera: camera.clone(),
            };
            let rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(camera.viewport[0] as f32, camera.viewport[1] as f32),
            );
            return Some(
                points
                    .iter()
                    .zip(values)
                    .map(|(p, v)| {
                        projection
                            .screen(rect, [p[0], p[1]], v[2] as f64)
                            .map_or([0.; 4], |pos| {
                                [
                                    pos.x / rect.width(),
                                    pos.y / rect.height(),
                                    v[2],
                                    if rect.contains(pos) { v[3] } else { 0. },
                                ]
                            })
                    })
                    .collect(),
            );
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shader_is_valid() {
        let module = naga::front::wgsl::parse_str(&shader()).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
    #[test]
    fn gpu_pipeline_is_valid() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let Some(adapter) = pollster::block_on(instance.request_adapter(&Default::default()))
        else {
            eprintln!("GPUがないためGPU試験を省略します");
            return;
        };
        let (device, _) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                required_limits: adapter.limits(),
                ..Default::default()
            },
            None,
        ))
        .unwrap();
        let _renderer = Renderer::new(&device);
        // 視点の更新結果を待っても、同じ地点の表示を消さない。
        use std::hash::{Hash, Hasher};
        let camera = Camera::default();
        let points = [[camera.center[0] / WORLD, camera.center[1] / WORLD, 0.]];
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for p in &points {
            for v in p {
                v.to_bits().hash(&mut hash);
            }
        }
        let state = Mutex::new(State {
            result: Some((1, vec![[0.5, 0.5, 2., 1.]])),
            result_points: hash.finish(),
            ..Default::default()
        });
        let value = query(
            &device,
            &egui::Context::default(),
            &state,
            &camera,
            2,
            &points,
        )
        .expect("直前の判定を保持");
        assert_eq!(value.len(), 1);
        assert_eq!(value[0][3], 1.);
        assert_eq!(state.lock().unwrap().request.as_ref().unwrap().id, 2);
        assert!(
            query(
                &device,
                &egui::Context::default(),
                &state,
                &camera,
                3,
                &[[0.1, 0.2, 0.]]
            )
            .is_none()
        );
    }
}
