use crate::geometry::Mesh;
use std::collections::HashMap;
use wgpu::util::DeviceExt;

#[derive(Clone)]
pub struct Control {
    pub words: Vec<u32>,
    pub edges: u32,
}
pub fn control(vertices: usize, indices: &[u32]) -> Control {
    let faces = indices.len() / 3;
    let mut map = HashMap::new();
    let mut owners = vec![0u32; vertices];
    let mut face_edges = Vec::with_capacity(indices.len());
    let mut edge_owners = Vec::new();
    let mut endpoints = Vec::new();
    for (face, f) in indices.chunks_exact(3).enumerate() {
        for &v in f {
            owners[v as usize] = face as u32;
        }
        for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
            let key = (a.min(b), a.max(b));
            let id = *map.entry(key).or_insert_with(|| {
                let id = edge_owners.len() as u32;
                edge_owners.push(face as u32);
                endpoints.extend([key.0, key.1]);
                id
            });
            face_edges.push(id);
        }
    }
    let edges = edge_owners.len() as u32;
    let mut words = Vec::with_capacity(6 * faces + vertices + 3 * edges as usize);
    words.extend_from_slice(indices);
    words.extend(face_edges);
    words.extend(owners);
    words.extend(edge_owners);
    words.extend(endpoints);
    Control { words, edges }
}
pub fn vertex_count(v: u32, e: u32, f: u32, d: u32) -> u32 {
    v + e * (d - 1) + f * if d < 3 { 0 } else { (d - 1) * (d - 2) / 2 }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Point {
    pub base: [f32; 4],
    pub curved: [f32; 4],
}
pub struct Surface {
    pub source: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub adjacency: wgpu::Buffer,
    pub vertices: u32,
    pub triangles: u32,
    pub divisions: u32,
    pub bytes: usize,
}
pub struct Engine {
    indices: wgpu::ComputePipeline,
    adaptive_indices: wgpu::ComputePipeline,
    adjacency: wgpu::ComputePipeline,
    points: wgpu::ComputePipeline,
}
fn index_shader() -> String {
    format!(
        "{}\n{}",
        include_str!("loop_common.wgsl"),
        include_str!("loop_indices.wgsl")
    )
}
fn point_shader() -> String {
    format!(
        "{}\n{}",
        include_str!("loop_common.wgsl"),
        include_str!("loop_points.wgsl")
    )
}
fn buffer(d: &wgpu::Device, label: &str, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: if data.is_empty() { &[0; 4] } else { data },
        usage,
    })
}
fn storage(d: &wgpu::Device, label: &str, size: usize, extra: wgpu::BufferUsages) -> wgpu::Buffer {
    d.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(4) as u64,
        usage: wgpu::BufferUsages::STORAGE | extra,
        mapped_at_creation: false,
    })
}
impl Engine {
    pub fn new(d: &wgpu::Device) -> Self {
        let pipeline = |label, source: String| {
            let m = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module: &m,
                entry_point: Some("cs"),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            indices: pipeline("GPU三角形分割", index_shader()),
            adaptive_indices: pipeline(
                "GPU画面適応分割",
                format!(
                    "{}\n{}",
                    include_str!("loop_common.wgsl"),
                    include_str!("loop_adaptive_indices.wgsl")
                ),
            ),
            adjacency: pipeline("GPU接続リスト", include_str!("loop_adjacency.wgsl").into()),
            points: pipeline("GPU Loop位置", point_shader()),
        }
    }
    fn dispatch(
        d: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        buffers: &[&wgpu::Buffer],
        count: u32,
    ) {
        if count == 0 {
            return;
        }
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &buffers
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("GPU Loop分割"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(count.div_ceil(128), 1, 1);
    }
    pub fn prepare(&self, d: &wgpu::Device, q: &wgpu::Queue, mesh: &Mesh) -> Surface {
        let v = mesh.vertices.len() as u32;
        let f = (mesh.indices.len() / 3) as u32;
        let e = mesh.control.edges;
        let points: Vec<_> = mesh
            .vertices
            .iter()
            .map(|p| Point {
                base: [p.base[0], p.base[1], p.base[2], 0.],
                curved: [p.curved[0], p.curved[1], p.curved[2], 0.],
            })
            .collect();
        let mut source = buffer(
            d,
            "制御頂点",
            if points.is_empty() {
                &[0; 32]
            } else {
                bytemuck::cast_slice(&points)
            },
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let mut indices = buffer(
            d,
            "制御面",
            bytemuck::cast_slice(&mesh.indices),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_SRC,
        );
        let control = buffer(
            d,
            "制御接続情報",
            bytemuck::cast_slice(&mesh.control.words),
            wgpu::BufferUsages::STORAGE,
        );
        let mut adjacency = storage(
            d,
            "制御面接続",
            (v + 3 * f) as usize * 4,
            wgpu::BufferUsages::COPY_DST,
        );
        let mut encoder = d.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("GPU曲面生成"),
        });
        let adaptive = mesh.adaptive.as_ref().map(|plan| {
            buffer(
                d,
                "適応分割テンプレート",
                bytemuck::cast_slice(&plan.records),
                wgpu::BufferUsages::STORAGE,
            )
        });
        let mut division = 1;
        let mut previous_n = v;
        let params = buffer(
            d,
            "制御設定",
            bytemuck::cast_slice(&[v, e, f, 1, 1, v, v, f]),
            wgpu::BufferUsages::UNIFORM,
        );
        encoder.clear_buffer(&adjacency, 0, None);
        Self::dispatch(
            d,
            &mut encoder,
            &self.adjacency,
            &[&params, &indices, &adjacency],
            f,
        );
        while division < mesh.divisions {
            let next = division * 2;
            let n = vertex_count(v, e, f, next);
            let final_adaptive = next == mesh.divisions && adaptive.is_some();
            let faces = if final_adaptive {
                mesh.adaptive.as_ref().unwrap().records.len() as u32
            } else {
                f * next * next
            };
            let parameters = buffer(
                d,
                "分割設定",
                bytemuck::cast_slice(&[v, e, f, division, next, previous_n, n, faces]),
                wgpu::BufferUsages::UNIFORM,
            );
            let next_source = storage(
                d,
                "分割頂点",
                (n as usize * 32).max(32),
                wgpu::BufferUsages::COPY_SRC,
            );
            let next_indices = storage(
                d,
                "分割面",
                faces as usize * 12,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_SRC,
            );
            let next_adj = storage(
                d,
                "分割接続",
                (n + 3 * faces) as usize * 4,
                wgpu::BufferUsages::COPY_DST,
            );
            if final_adaptive {
                Self::dispatch(
                    d,
                    &mut encoder,
                    &self.adaptive_indices,
                    &[
                        &parameters,
                        &control,
                        adaptive.as_ref().unwrap(),
                        &next_indices,
                    ],
                    faces,
                );
            } else {
                Self::dispatch(
                    d,
                    &mut encoder,
                    &self.indices,
                    &[&parameters, &control, &next_indices],
                    faces,
                );
            }
            Self::dispatch(
                d,
                &mut encoder,
                &self.points,
                &[
                    &parameters,
                    &control,
                    &source,
                    &indices,
                    &adjacency,
                    &next_source,
                ],
                n,
            );
            encoder.clear_buffer(&next_adj, 0, None);
            Self::dispatch(
                d,
                &mut encoder,
                &self.adjacency,
                &[&parameters, &next_indices, &next_adj],
                faces,
            );
            source = next_source;
            indices = next_indices;
            adjacency = next_adj;
            division = next;
            previous_n = n;
        }
        q.submit([encoder.finish()]);
        let triangles = if division > 1 {
            mesh.adaptive
                .as_ref()
                .map_or(f * division * division, |p| p.records.len() as u32)
        } else {
            f
        };
        let vertices = vertex_count(v, e, f, division);
        let bytes = source.size() as usize + indices.size() as usize + adjacency.size() as usize;
        Surface {
            source,
            indices,
            adjacency,
            vertices,
            triangles,
            divisions: division,
            bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shaders_are_valid() {
        for s in [
            index_shader(),
            point_shader(),
            format!(
                "{}\n{}",
                include_str!("loop_common.wgsl"),
                include_str!("loop_adaptive_indices.wgsl")
            ),
            include_str!("loop_adjacency.wgsl").into(),
        ] {
            let m = naga::front::wgsl::parse_str(&s).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&m)
            .unwrap();
        }
    }
    #[test]
    fn shared_edge_is_stored_once() {
        let c = control(4, &[0, 1, 2, 2, 1, 3]);
        assert_eq!(c.edges, 5);
        assert_eq!(vertex_count(4, c.edges, 2, 4), 25);
        assert_eq!(c.words[7], c.words[9]);
    }
}

#[cfg(test)]
#[path = "loop_reference.rs"]
mod reference;
#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::geometry::Vertex;
    fn read(d: &wgpu::Device, q: &wgpu::Queue, b: &wgpu::Buffer, size: u64) -> Vec<u8> {
        let out = d.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = d.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(b, 0, &out, 0, size);
        q.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        out.slice(..).map_async(wgpu::MapMode::Read, move |v| {
            tx.send(v).unwrap();
        });
        let _ = d.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let data = out.slice(..).get_mapped_range().to_vec();
        out.unmap();
        data
    }
    #[test]
    fn gpu_loop_matches_cpu_and_keeps_shared_edges() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: if cfg!(target_os = "windows") {
                wgpu::Backends::DX12
            } else {
                wgpu::Backends::PRIMARY
            },
            ..Default::default()
        });
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("GPU照合には実行可能なGPUが必要です");
        let (d, q) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .unwrap();
        let engine = Engine::new(&d);
        for heights in [[0., 0., 0., 0., 0.], [0., 4., 9., 2., 30.]] {
            let points = [
                [0., 0., 0.],
                [100., 0., 0.],
                [100., 0., 100.],
                [0., 0., 100.],
                [50., 0., 50.],
            ];
            let vertices: Vec<_> = points
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let p = [p[0], heights[i], p[2]];
                    Vertex { base: p, curved: p }
                })
                .collect();
            let indices = vec![0, 4, 1, 1, 4, 2, 2, 4, 3, 3, 4, 0];
            for divisions in [1, 2, 4, 8] {
                let control = control(vertices.len(), &indices);
                let mesh = Mesh {
                    vertices: vertices.clone(),
                    indices: indices.clone(),
                    control,
                    source_faces: 4,
                    tiles: 1,
                    divisions,
                    adaptive: None,
                    levels: vec![14],
                };
                let surface = engine.prepare(&d, &q, &mesh);
                let raw = read(&d, &q, &surface.source, surface.vertices as u64 * 32);
                let got: &[Point] = bytemuck::cast_slice(&raw);
                let raw_indices = read(&d, &q, &surface.indices, surface.triangles as u64 * 12);
                let got_indices: &[u32] = bytemuck::cast_slice(&raw_indices);
                let (expected, expected_indices) =
                    reference::subdivide(&vertices, &indices, divisions);
                assert_eq!(got.len(), expected.len());
                assert_eq!(got_indices.len(), expected_indices.len());
                let mut remap = Vec::new();
                for v in &expected {
                    let (index, other) = got
                        .iter()
                        .enumerate()
                        .find(|(_, p)| (0..3).all(|i| (p.base[i] - v.base[i]).abs() < 0.001))
                        .expect("GPU基底点が一致しません");
                    for i in 0..3 {
                        assert!(
                            (other.curved[i] - v.curved[i]).abs() < 0.001,
                            "GPU曲面差: D{divisions} {:?} {:?}",
                            other,
                            v.base
                        );
                    }
                    remap.push(index as u32);
                }
                let canonical = |f: &[u32]| {
                    let mut p = [f[0], f[1], f[2]];
                    p.sort();
                    p
                };
                let want: std::collections::BTreeSet<_> = expected_indices
                    .chunks_exact(3)
                    .map(|f| canonical(&f.iter().map(|&i| remap[i as usize]).collect::<Vec<_>>()))
                    .collect();
                let actual: std::collections::BTreeSet<_> =
                    got_indices.chunks_exact(3).map(canonical).collect();
                assert_eq!(want, actual);
                if divisions == 8 {
                    let mut mixed = mesh.clone();
                    let plan = crate::adaptive_curve::make(&mixed, &[400., 10., 40., 80.], 24., 8);
                    assert_eq!(&**plan.levels, &[8, 1, 2, 4]);
                    let records = plan.records.clone();
                    mixed.adaptive = Some(std::sync::Arc::new(plan));
                    let adaptive = engine.prepare(&d, &q, &mixed);
                    let bytes = read(&d, &q, &adaptive.indices, adaptive.triangles as u64 * 12);
                    let indices: &[u32] = bytemuck::cast_slice(&bytes);
                    let bytes = read(&d, &q, &adaptive.source, adaptive.vertices as u64 * 32);
                    let positions: &[Point] = bytemuck::cast_slice(&bytes);
                    assert!(adaptive.triangles < surface.triangles);
                    let mut edges = std::collections::HashMap::new();
                    for (face, record) in indices.chunks_exact(3).zip(records.iter()) {
                        let original =
                            &mixed.indices[record[0] as usize * 3..record[0] as usize * 3 + 3];
                        for k in 0..3 {
                            let code = record[k + 1];
                            let i = (code & 15) as f32 / 8.;
                            let j = (code >> 4) as f32 / 8.;
                            for axis in 0..3 {
                                let expected = mixed.vertices[original[0] as usize].base[axis]
                                    * (1. - i - j)
                                    + mixed.vertices[original[1] as usize].base[axis] * i
                                    + mixed.vertices[original[2] as usize].base[axis] * j;
                                assert!(
                                    (positions[face[k] as usize].base[axis] - expected).abs()
                                        < 0.001
                                );
                            }
                        }
                        for k in 0..3 {
                            let mut edge = [face[k], face[(k + 1) % 3]];
                            edge.sort();
                            *edges.entry(edge).or_insert(0) += 1;
                        }
                    }
                    for ([a, b], count) in edges {
                        assert!(count <= 2);
                        if count == 1 {
                            let a = positions[a as usize].base;
                            let b = positions[b as usize].base;
                            assert!(
                                [0, 2]
                                    .into_iter()
                                    .any(|axis| (a[axis] - b[axis]).abs() < 0.001
                                        && (a[axis].abs() < 0.001
                                            || (a[axis] - 100.).abs() < 0.001)),
                                "適応分割の内部に未接続辺があります"
                            );
                        }
                    }
                }
                println!(
                    "GPU Loop D{divisions}: {}頂点・{}面、CPU照合成功",
                    got.len(),
                    actual.len()
                );
            }
        }
    }
}
