use crate::{geometry::Mesh, selection::Camera};
use eframe::{
    egui,
    egui_wgpu::{Callback, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor},
};
use glam::{Mat4, Vec3};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Instant,
};
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Uniform {
    matrix: [[f32; 4]; 4],
    offset: [f32; 4],
    params: [f32; 4],
}
static NEXT_MESH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
pub struct GpuMesh {
    id: u64,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    bind: wgpu::BindGroup,
    count: u32,
    last_retention: std::sync::atomic::AtomicU32,
    pub bytes: usize,
    pub vertex_count: u32,
    pub triangles: u32,
    pub divisions: u32,
}
pub struct Renderer {
    pub state: RenderState,
    pipeline: wgpu::RenderPipeline,
    fallback_pipeline: wgpu::RenderPipeline,
    fallback_uniform: wgpu::Buffer,
    fallback_bind: wgpu::BindGroup,
    compute: wgpu::ComputePipeline,
    loop_engine: crate::gpu_loop::Engine,
    render_bind: wgpu::BindGroup,
    metrics: Arc<Mutex<Metrics>>,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    color: wgpu::Texture,
    depth: wgpu::Texture,
    pub texture: egui::TextureId,
    size: [u32; 2],
}
const SHADER: &str = r#"
struct Uniform { matrix:mat4x4<f32>, offset:vec4<f32>, params:vec4<f32> }
@group(0) @binding(0) var<uniform> u:Uniform;
struct VOut { @builtin(position) clip:vec4<f32>, @location(0) p:vec3<f32>, @location(1) n:vec3<f32> }
@vertex fn vs(@location(0) p:vec3<f32>,@location(1) n:vec3<f32>)->VOut{
 var out:VOut;out.p=p+u.offset.xyz;out.n=n;out.clip=u.matrix*vec4(out.p,1);return out;
}
fn linear_from_srgb(c:vec3<f32>)->vec3<f32>{
 let low=c/vec3(12.92);let high=pow((c+vec3(0.055))/vec3(1.055),vec3(2.4));return select(high,low,c<=vec3(0.04045));
}
@fragment fn fs(v:VOut)->@location(0) vec4<f32>{
 var n=normalize(cross(dpdx(v.p),dpdy(v.p)));if u.params.y>0.5{n=normalize(v.n);}if n.y<0{n=-n;}
 let light=normalize(vec3(-0.5,1,-0.6));let shade=clamp(0.82+(dot(n,light)-light.y)*u.params.z,0.15,1.2);
 let base=mix(vec3(0.40,0.49,0.32),vec3(0.69,0.65,0.51),clamp(v.p.y/3800.0,0.0,1.0));return vec4(linear_from_srgb(clamp(base*shade,vec3(0.0),vec3(1.0))),1);
}
"#;
const COMPUTE: &str = r#"
struct Uniform { matrix:mat4x4<f32>, offset:vec4<f32>, params:vec4<f32> }
@group(0) @binding(0) var<uniform> u:Uniform;
struct Point {base:vec4<f32>,curved:vec4<f32>}
@group(0) @binding(1) var<storage,read> source:array<Point>;
@group(0) @binding(2) var<storage,read> topology:array<u32>;
@group(0) @binding(3) var<storage,read> faces:array<u32>;
struct Vertex { p:vec4<f32>, n:vec4<f32> }
@group(0) @binding(4) var<storage,read_write> out:array<Vertex>;
fn pos(i:u32)->vec3<f32>{return mix(source[i].curved.xyz,source[i].base.xyz,u.params.x);}
@compute @workgroup_size(128) fn cs(@builtin(global_invocation_id) id:vec3<u32>){
 let i=id.x;let count=arrayLength(&source);if i>=count{return;}var node=topology[i];var n=vec3(0.0);
 loop {if node==0u{break;}let at=((node-1u)/3u)*3u;let a=pos(faces[at]);let b=pos(faces[at+1u]);let c=pos(faces[at+2u]);n+=cross(b-a,c-a);node=topology[count+node-1u];}
 if dot(n,n)<0.00000001{n=vec3(0,1,0);}out[i].p=vec4(pos(i),1);out[i].n=vec4(normalize(n),0);
}

"#;
fn srgb_linear(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
fn texture(device: &wgpu::Device, size: [u32; 2], depth: bool) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("地形描画"),
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
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC
            },
        view_formats: &[],
    })
}
impl Renderer {
    pub fn new(state: RenderState) -> Self {
        let device = &state.device;
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("カメラ"),
            contents: bytemuck::bytes_of(&Uniform {
                matrix: Mat4::IDENTITY.to_cols_array_2d(),
                offset: [0.; 4],
                params: [0.5, 1., 1., 0.],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("地形"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&render_layout],
            push_constant_ranges: &[],
        });
        let mut descriptor = wgpu::RenderPipelineDescriptor {
            label: Some("地形"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 16,
                            shader_location: 1,
                        },
                    ],
                }],
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
                    read_mask: 0xff,
                    write_mask: 0xff,
                },
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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
        let pipeline = device.create_render_pipeline(&descriptor);
        descriptor.depth_stencil.as_mut().unwrap().stencil = wgpu::StencilState {
            front: wgpu::StencilFaceState {
                compare: wgpu::CompareFunction::Equal,
                ..Default::default()
            },
            back: wgpu::StencilFaceState {
                compare: wgpu::CompareFunction::Equal,
                ..Default::default()
            },
            read_mask: 0xff,
            write_mask: 0,
        };
        let fallback_pipeline = device.create_render_pipeline(&descriptor);
        let fallback_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("z6背景カメラ"),
            size: 96,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fallback_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &render_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: fallback_uniform.as_entire_binding(),
            }],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &(0..5)
                .map(|i| wgpu::BindGroupLayoutEntry {
                    binding: i,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if i == 0 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage { read_only: i != 4 }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
        let cp = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("曲面と法線"),
            source: wgpu::ShaderSource::Wgsl(COMPUTE.into()),
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&cp),
            module: &shader,
            entry_point: Some("cs"),
            compilation_options: Default::default(),
            cache: None,
        });
        let size = [1, 1];
        let color = texture(device, size, false);
        let depth = texture(device, size, true);
        let id = state.renderer.write().register_native_texture(
            device,
            &color.create_view(&Default::default()),
            wgpu::FilterMode::Linear,
        );
        let render_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &render_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            fallback_pipeline,
            fallback_uniform,
            fallback_bind,
            render_bind,
            metrics: Arc::new(Mutex::new(Metrics::default())),
            loop_engine: crate::gpu_loop::Engine::new(device),
            state,
            pipeline,
            compute,
            layout,
            uniform,
            color,
            depth,
            texture: id,
            size,
        }
    }
    pub fn upload(&self, mesh: &Mesh) -> GpuMesh {
        let d = &self.state.device;
        let surface = self.loop_engine.prepare(d, &self.state.queue, mesh);
        let source = surface.source;
        let indices = surface.indices;
        let topology = surface.adjacency;
        let vertices = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU曲面"),
            size: (surface.vertices as usize * 32).max(32) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[&self.uniform, &source, &topology, &indices, &vertices]
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        });
        GpuMesh {
            id: NEXT_MESH.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            vertices,
            indices,
            bind,
            count: surface.triangles * 3,
            vertex_count: surface.vertices,
            triangles: surface.triangles,
            divisions: surface.divisions,
            last_retention: std::sync::atomic::AtomicU32::new(f32::NAN.to_bits()),
            bytes: surface.bytes + (surface.vertices as usize * 32).max(32),
        }
    }
    pub fn stats(&self) -> PaintStats {
        self.metrics.lock().unwrap().snapshot()
    }
    pub fn frame(
        &mut self,
        mesh: Arc<GpuMesh>,
        camera: &Camera,
        origin: [f64; 2],
        size: [u32; 2],
        retention: f32,
        smooth: bool,
        strength: f32,
        rect: egui::Rect,
        fallback: Option<(Arc<GpuMesh>, [f64; 2])>,
    ) -> egui::PaintCallback {
        if self.size != size {
            self.state.renderer.write().free_texture(&self.texture);
            self.color = texture(&self.state.device, size, false);
            self.depth = texture(&self.state.device, size, true);
            self.texture = self.state.renderer.write().register_native_texture(
                &self.state.device,
                &self.color.create_view(&Default::default()),
                wgpu::FilterMode::Linear,
            );
            self.size = size;
        }
        Callback::new_paint_callback(
            rect,
            FrameJob {
                mesh,
                fallback,
                fallback_pipeline: self.fallback_pipeline.clone(),
                fallback_uniform: self.fallback_uniform.clone(),
                fallback_bind: self.fallback_bind.clone(),
                camera: camera.clone(),
                origin,
                size,
                retention,
                smooth,
                strength,
                metrics: self.metrics.clone(),
                pipeline: self.pipeline.clone(),
                compute: self.compute.clone(),
                render_bind: self.render_bind.clone(),
                uniform: self.uniform.clone(),
                color: self.color.create_view(&Default::default()),
                depth: self.depth.create_view(&Default::default()),
            },
        )
    }
}

#[derive(Clone, Copy, Default)]
pub struct PaintStats {
    pub count: u64,
    pub interval_ms: f64,
    pub cpu_ms: f64,
    pub fps: f64,
    pub p95_ms: f64,
}
#[derive(Default)]
struct Metrics {
    last_frame: Vec<u64>,
    last_painted: Vec<u64>,
    count: u64,
    cpu_ms: f64,
    recent: VecDeque<Instant>,
    interval_ms: f64,
}
impl Metrics {
    fn painted(&mut self, now: Instant) {
        if self.last_frame == self.last_painted {
            return;
        }
        self.last_painted = self.last_frame.clone();
        self.interval_ms = self
            .recent
            .back()
            .map_or(0., |last| now.duration_since(*last).as_secs_f64() * 1000.);
        self.count += 1;
        self.recent.push_back(now);
        while self
            .recent
            .front()
            .is_some_and(|t| now.duration_since(*t).as_secs_f64() > 1.)
        {
            self.recent.pop_front();
        }
    }

    fn snapshot(&self) -> PaintStats {
        let fps = match (self.recent.front(), self.recent.back()) {
            (Some(first), Some(last))
                if last.elapsed().as_millis() < 250 && self.recent.len() > 1 =>
            {
                (self.recent.len() - 1) as f64
                    / last.duration_since(*first).as_secs_f64().max(0.001)
            }
            _ => 0.,
        };
        let times: Vec<_> = self.recent.iter().copied().collect();
        let mut intervals: Vec<_> = times
            .windows(2)
            .map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1000.)
            .collect();
        intervals.sort_by(f64::total_cmp);
        let p95_ms = intervals
            .get(intervals.len() * 95 / 100)
            .copied()
            .unwrap_or(0.);
        PaintStats {
            p95_ms,
            count: self.count,
            interval_ms: self.interval_ms,
            cpu_ms: self.cpu_ms,
            fps,
        }
    }
}
struct FrameJob {
    mesh: Arc<GpuMesh>,
    fallback: Option<(Arc<GpuMesh>, [f64; 2])>,
    camera: Camera,
    origin: [f64; 2],
    size: [u32; 2],
    retention: f32,
    smooth: bool,
    strength: f32,
    metrics: Arc<Mutex<Metrics>>,
    pipeline: wgpu::RenderPipeline,
    fallback_pipeline: wgpu::RenderPipeline,
    fallback_uniform: wgpu::Buffer,
    fallback_bind: wgpu::BindGroup,
    compute: wgpu::ComputePipeline,
    render_bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
}
impl CallbackTrait for FrameJob {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        _resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let mesh = &self.mesh;
        let camera = &self.camera;
        let size = self.size;
        let origin = self.origin;
        let retention = self.retention;
        let smooth = self.smooth;
        let strength = self.strength;
        let mut stamp = vec![
            mesh.id,
            size[0] as u64,
            size[1] as u64,
            retention.to_bits() as u64,
            smooth as u64,
            strength.to_bits() as u64,
        ];
        if let Some((mesh, origin)) = &self.fallback {
            stamp.extend([mesh.id, origin[0].to_bits(), origin[1].to_bits()]);
        }
        stamp.extend(
            [
                camera.center[0],
                camera.center[1],
                camera.height,
                camera.distance,
                camera.yaw,
                camera.pitch,
                origin[0],
                origin[1],
            ]
            .map(f64::to_bits),
        );
        {
            let mut m = self.metrics.lock().unwrap();
            if stamp == m.last_frame {
                return Vec::new();
            }
            m.last_frame = stamp;
        }
        let started = Instant::now();
        let camera = &self.camera;
        let origin = self.origin;
        let size = self.size;
        let retention = self.retention;
        let smooth = self.smooth;
        let strength = self.strength;
        let back = camera.back().as_vec3();
        let target = Vec3::new(0., camera.height as f32, 0.);
        let eye = target + back * camera.distance as f32;
        let view = Mat4::look_at_rh(eye, target, Vec3::Y);
        let matrix = Mat4::perspective_rh(
            std::f32::consts::PI / 4.,
            size[0] as f32 / size[1] as f32,
            (camera.distance as f32 / 10000.).max(1.),
            (camera.distance as f32 * 5.).max(20000000.),
        ) * view;
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Uniform {
                matrix: matrix.to_cols_array_2d(),
                offset: [
                    (origin[0] - camera.center[0]) as f32,
                    0.,
                    (origin[1] - camera.center[1]) as f32,
                    0.,
                ],
                params: [retention, smooth as u8 as f32, strength, 0.],
            }),
        );
        if let Some((_, origin)) = &self.fallback {
            queue.write_buffer(
                &self.fallback_uniform,
                0,
                bytemuck::bytes_of(&Uniform {
                    matrix: matrix.to_cols_array_2d(),
                    offset: [
                        (origin[0] - camera.center[0]) as f32,
                        0.,
                        (origin[1] - camera.center[1]) as f32,
                        0.,
                    ],
                    params: [retention, smooth as u8 as f32, strength, 0.],
                }),
            );
        }
        for mesh in
            std::iter::once(mesh.as_ref()).chain(self.fallback.as_ref().map(|v| v.0.as_ref()))
        {
            if mesh
                .last_retention
                .load(std::sync::atomic::Ordering::Relaxed)
                != retention.to_bits()
            {
                mesh.last_retention
                    .store(retention.to_bits(), std::sync::atomic::Ordering::Relaxed);
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.compute);
                pass.set_bind_group(0, &mesh.bind, &[]);
                pass.dispatch_workgroups((mesh.vertices.size() / 32).div_ceil(128) as u32, 1, 1);
            }
        }
        {
            let cv = self.color.clone();
            let dv = self.depth.clone();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &cv,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: srgb_linear(0.90),
                            g: srgb_linear(0.945),
                            b: srgb_linear(0.98),
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &dv,
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
            pass.set_stencil_reference(1);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.render_bind, &[]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, 0..1);
            if let Some((background, _)) = &self.fallback {
                pass.set_stencil_reference(0);
                pass.set_pipeline(&self.fallback_pipeline);
                pass.set_bind_group(0, &self.fallback_bind, &[]);
                pass.set_vertex_buffer(0, background.vertices.slice(..));
                pass.set_index_buffer(background.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..background.count, 0, 0..1);
            }
        }
        self.metrics.lock().unwrap().cpu_ms = started.elapsed().as_secs_f64() * 1000.;
        Vec::new()
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        _pass: &mut wgpu::RenderPass<'static>,
        _resources: &CallbackResources,
    ) {
        self.metrics.lock().unwrap().painted(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_ui_paints_do_not_count_as_terrain_frames() {
        let mut m = Metrics::default();
        let start = Instant::now();
        m.last_frame = vec![1];
        m.painted(start);
        m.painted(start + std::time::Duration::from_millis(1));
        assert_eq!(m.count, 1);
        m.last_frame = vec![2];
        m.painted(start + std::time::Duration::from_millis(33));
        assert_eq!(m.count, 2);
        assert_eq!(m.interval_ms, 33.);
        assert_eq!(m.snapshot().p95_ms, 33.);
    }
    #[test]
    fn gpu_shaders_are_valid() {
        for source in [SHADER, COMPUTE] {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
