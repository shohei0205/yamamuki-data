//! 地形の画面上の地理座標から元タイルを直接参照する。合成用の縮小画像は作らない。
use crate::{selection::Camera, shared_data::WORLD};
use eframe::{egui, wgpu};
use std::sync::Arc;

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
}
/// 3D用の描画収集時だけ使うマーカー。通常の2D描画へは渡さない。
pub struct Capture {
    pub rect: egui::Rect,
    pub uv: [egui::Rect; 2],
    pub textures: [Option<wgpu::TextureView>; 2],
    pub amount: f32,
    pub shade: bool,
    pub gamma_shade: bool,
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
}
#[derive(Default)]
struct Cache {
    uniform: Option<wgpu::Buffer>,
    capacity: usize,
    positions: Option<wgpu::TextureView>,
    lighting: Option<(wgpu::Texture, wgpu::TextureView)>,
    groups: std::collections::HashMap<[Option<wgpu::TextureView>; 2], wgpu::BindGroup>,
    groups_created: u64,
    buffers_created: u64,
}
pub(crate) struct Renderer {
    layout: wgpu::BindGroupLayout,
    color: wgpu::RenderPipeline,
    darken: wgpu::RenderPipeline,
    brighten: wgpu::RenderPipeline,
    lighting: wgpu::RenderPipeline,
    sampler_group: wgpu::BindGroup,
    empty: wgpu::TextureView,
    cache: std::sync::Mutex<Cache>,
}
impl Renderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("元タイル直接参照"),
            source: wgpu::ShaderSource::Wgsl(include_str!("direct_material.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout, &sampler_layout],
            push_constant_ranges: &[],
        });
        let pipeline = |entry, color, mask| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: Some(wgpu::BlendState {
                            color,
                            alpha: wgpu::BlendComponent::OVER,
                        }),
                        write_mask: mask,
                    })],
                }),
                multiview: None,
                cache: None,
            })
        };
        let blend = |src_factor, dst_factor| wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
        Self {
            cache: Default::default(),
            color: pipeline(
                "color",
                blend(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
                wgpu::ColorWrites::ALL,
            ),
            darken: pipeline(
                "darken",
                blend(wgpu::BlendFactor::Zero, wgpu::BlendFactor::OneMinusSrc),
                wgpu::ColorWrites::COLOR,
            ),
            brighten: pipeline(
                "brighten",
                blend(wgpu::BlendFactor::Dst, wgpu::BlendFactor::One),
                wgpu::ColorWrites::COLOR,
            ),
            lighting: pipeline(
                "lighting",
                blend(wgpu::BlendFactor::One, wgpu::BlendFactor::Zero),
                wgpu::ColorWrites::COLOR,
            ),
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
        let _ = device.poll(wgpu::Maintain::Poll);
        // 地形へ貼れない画面外・遠方の画像は、GPU設定を作る前に除く。
        let commands = commands
            .iter()
            .filter(|c| camera.visible_depth(c.bounds.map(|v| v * WORLD)).is_some())
            .collect::<Vec<_>>();
        let target = target_texture.create_view(&Default::default());
        let stride = (std::mem::size_of::<Params>() as u64).div_ceil(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        )) * u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let count = commands.len() + 1;
        let mut cache = self.cache.lock().unwrap();
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
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
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
            .map(|c| c.textures.clone())
            .chain(std::iter::once([None, None]))
            .collect();
        cache.groups.retain(|key, _| active.contains(key));
        let mut needed = std::collections::HashSet::new();
        for (i, command) in commands
            .iter()
            .map(Some)
            .chain(std::iter::once(None))
            .enumerate()
        {
            let params = command.map_or(Params::zeroed(), |c| Params {
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
                tint: c.tint,
                settings: [
                    c.amount,
                    f32::from(c.textures[0].is_some()),
                    f32::from(c.textures[1].is_some()),
                    f32::from(c.textures.iter().all(Option::is_none))
                        + 2. * f32::from(c.gamma_shade),
                ],
            });
            let at = i * stride as usize;
            bytes[at..at + std::mem::size_of::<Params>()]
                .copy_from_slice(bytemuck::bytes_of(&params));
            let key = command.map_or([None, None], |c| c.textures.clone());
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
                scissor(c.bounds, camera, size)
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
        drop(cache);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("元タイルを地形ピクセルへ合成"),
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
        for (group, shade, clip, offset) in &draws {
            if shade.is_none() || clip[2] == 0 || clip[3] == 0 {
                continue;
            }
            pass.set_scissor_rect(clip[0], clip[1], clip[2], clip[3]);
            pass.set_bind_group(0, group, &[*offset]);
            if *shade == Some(true) {
                for pipeline in [&self.darken, &self.brighten] {
                    pass.set_pipeline(pipeline);
                    pass.draw(0..3, 0..1);
                }
            } else {
                pass.set_pipeline(&self.color);
                pass.draw(0..3, 0..1);
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
        let (group, _, _, offset) = draws.last().unwrap();
        pass.set_bind_group(0, group, &[*offset]);
        pass.draw(0..3, 0..1);
    }
}
use bytemuck::Zeroable;
/// 可視標高範囲の投影を囲み、各タイルで画面全体を塗らない。
fn scissor(bounds: [f64; 4], c: &Camera, size: [u32; 2]) -> [u32; 4] {
    let back = c.back();
    let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
    let up = back.cross(right);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for x in [bounds[0], bounds[2]] {
        for y in [bounds[1], bounds[3]] {
            for height in [-500., 4500.] {
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

#[cfg(test)]
mod tests {
    use super::*;
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
        let renderer = Renderer::new(&device);
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
        for mode in 0..8 {
            let q = match mode {
                0 => quad(checker.clone(), checker.clone(), 1.),
                1 => quad(black.clone(), white.clone(), 0.5),
                6 => quad(gray.clone(), gray.clone(), 1.),
                7 => quad(mid.clone(), mid.clone(), 1.),
                _ => quad(white.clone(), white.clone(), 1.),
            };
            let coordinates = (0..width)
                .map(|x| {
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
            if (4..=5).contains(&mode) {
                let mut overlay = quad(shade_view.clone(), shade_view.clone(), 1.);
                overlay.shade = true;
                overlay.gamma_shade = mode == 5;
                commands.push(overlay);
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
                assert_eq!(pixel[3], 255);
            }
        }
    }
}
