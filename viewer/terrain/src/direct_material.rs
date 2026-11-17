//! 地形の画面上の地理座標から元タイルを直接参照する。合成用の縮小画像は作らない。
use crate::{selection::Camera, shared_data::WORLD};
use eframe::{egui, wgpu};
use std::sync::Arc;

/// 拡大時の補間方式。GPUのモード値と対応する。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Interpolation {
    #[default]
    Bilinear = 0,
    Edge = 1,
    Nearest = 2,
}
impl Interpolation {
    pub const ALL: [Self; 3] = [Self::Bilinear, Self::Edge, Self::Nearest];
    pub fn label(self) -> &'static str {
        match self {
            Self::Bilinear => "バイリニア",
            Self::Edge => "輪郭優先",
            Self::Nearest => "最近傍",
        }
    }
}
impl serde::Serialize for Interpolation {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Self::Bilinear => "bilinear",
            Self::Edge => "edge",
            Self::Nearest => "nearest",
        })
    }
}
impl From<bool> for Interpolation {
    fn from(value: bool) -> Self {
        if value { Self::Edge } else { Self::Bilinear }
    }
}
// 保存済みのboolと旧スナップショットも、当時の見た目のまま復元する。
impl<'de> serde::Deserialize<'de> for Interpolation {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Legacy(bool),
            Name(String),
        }
        match Saved::deserialize(d)? {
            Saved::Legacy(v) => Ok(v.into()),
            Saved::Name(v) => match v.as_str() {
                "bilinear" => Ok(Self::Bilinear),
                "edge" => Ok(Self::Edge),
                "nearest" => Ok(Self::Nearest),
                _ => Err(serde::de::Error::custom("未対応の補間方式")),
            },
        }
    }
}

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
    pub edge_filter: Interpolation,
    pub border: bool,
}
/// 3D用の描画収集時だけ使うマーカー。通常の2D描画へは渡さない。
pub struct Capture {
    pub rect: egui::Rect,
    pub uv: [egui::Rect; 2],
    pub textures: [Option<wgpu::TextureView>; 2],
    pub amount: f32,
    pub shade: bool,
    pub gamma_shade: bool,
    pub edge_filter: Interpolation,
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
    earth: [f32; 4],
    eye: [f32; 4],
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
        let shader = crate::gpu_profile::timed("元タイル直接参照", || {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("元タイル直接参照"),
                source: wgpu::ShaderSource::Wgsl(
                    format!(
                        "{}\n{}\n{}\n{}",
                        include_str!("direct_material.wgsl"),
                        include_str!("globe.wgsl"),
                        include_str!("atmosphere.wgsl"),
                        include_str!("edge_interpolation.wgsl")
                    )
                    .into(),
                ),
            })
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout, &sampler_layout],
            push_constant_ranges: &[],
        });
        let pipeline = |entry, color, mask| {
            crate::gpu_profile::timed(entry, || {
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
            })
        };
        let blend = |src_factor, dst_factor| wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
        // 独立した描画プログラムは同時に作成し、起動時の直列待ちをなくす。
        let (color, darken, brighten, lighting) = std::thread::scope(|scope| {
            let color = scope.spawn(|| {
                pipeline(
                    "color",
                    blend(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
                    wgpu::ColorWrites::ALL,
                )
            });
            let darken = scope.spawn(|| {
                pipeline(
                    "darken",
                    blend(wgpu::BlendFactor::Zero, wgpu::BlendFactor::OneMinusSrc),
                    wgpu::ColorWrites::COLOR,
                )
            });
            let brighten = scope.spawn(|| {
                pipeline(
                    "brighten",
                    blend(wgpu::BlendFactor::Dst, wgpu::BlendFactor::One),
                    wgpu::ColorWrites::COLOR,
                )
            });
            let lighting = pipeline(
                "lighting",
                blend(wgpu::BlendFactor::One, wgpu::BlendFactor::Zero),
                wgpu::ColorWrites::COLOR,
            );
            (
                color.join().unwrap(),
                darken.join().unwrap(),
                brighten.join().unwrap(),
                lighting,
            )
        });
        Self {
            cache: Default::default(),
            color,
            darken,
            brighten,
            lighting,
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
        self.render_region(
            device,
            queue,
            encoder,
            target_texture,
            lighting_source,
            positions,
            camera,
            size,
            commands,
            true,
            [-500., 4500.],
        );
    }
    // 地形OFFでは0mの球面だけを囲み、存在しない山のために全画面を塗らない。
    pub fn render_flat(
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
        self.render_region(
            device,
            queue,
            encoder,
            target_texture,
            lighting_source,
            positions,
            camera,
            size,
            commands,
            true,
            [0., 0.],
        );
    }
    fn render_region(
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
        restrict_pixels: bool,
        heights: [f64; 2],
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
            let mut params = command.map_or(Params::zeroed(), |c| Params {
                earth: [0.; 4],
                eye: [0.; 4],
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
                        + 2. * f32::from(c.gamma_shade)
                        + 8. * c.edge_filter as u32 as f32
                        + 4. * f32::from(c.border),
                ],
            });
            if command.is_none() && camera.globe {
                let latitude = crate::globe::latitude(camera.center[1]);
                let back = camera.back();
                params.earth = [
                    latitude.sin() as f32,
                    latitude.cos() as f32,
                    crate::globe::RADIUS as f32,
                    1.,
                ];
                params.eye = [
                    (back.x * camera.distance) as f32,
                    (camera.height + back.y * camera.distance) as f32,
                    (back.z * camera.distance) as f32,
                    camera.scale() as f32,
                ];
            }
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
                if restrict_pixels {
                    scissor_at_heights(c.bounds, camera, size, heights)
                } else {
                    [0, 0, size[0], size[1]]
                }
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
#[cfg(test)]
fn scissor(bounds: [f64; 4], c: &Camera, size: [u32; 2]) -> [u32; 4] {
    scissor_at_heights(bounds, c, size, [-500., 4500.])
}
fn scissor_at_heights(bounds: [f64; 4], c: &Camera, size: [u32; 2], heights: [f64; 2]) -> [u32; 4] {
    if c.globe {
        return globe_scissor_at_heights(bounds, c, size, heights);
    }
    let back = c.back();
    let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
    let up = back.cross(right);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for x in [bounds[0], bounds[2]] {
        for y in [bounds[1], bounds[3]] {
            for height in heights {
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

// 球面を包む箱を投影し、極側や日付変更線でも曲がった辺を切らない。
fn globe_scissor_at_heights(
    bounds: [f64; 4],
    c: &Camera,
    size: [u32; 2],
    heights: [f64; 2],
) -> [u32; 4] {
    use glam::DVec3;
    let area = bounds.map(|v| v * WORLD);
    let span = (area[2] - area[0]).abs().max((area[3] - area[1]).abs());
    if span > crate::globe::RADIUS * 0.5 {
        return [0, 0, size[0], size[1]];
    }
    let mut lower = DVec3::splat(f64::INFINITY);
    let mut upper = DVec3::splat(f64::NEG_INFINITY);
    const STEPS: u32 = 4;
    for y in 0..=STEPS {
        for x in 0..=STEPS {
            let world = [
                area[0] + (area[2] - area[0]) * x as f64 / STEPS as f64,
                area[1] + (area[3] - area[1]) * y as f64 / STEPS as f64,
            ];
            for h in heights {
                let p = crate::globe::local(c.center, world, h);
                lower = lower.min(p);
                upper = upper.max(p);
            }
        }
    }
    // 格子点の間の球面の膨らみと、Mercator緯度の非線形な変化も含める。
    let dx = (area[2] - area[0]) / STEPS as f64;
    let dy = (area[3] - area[1]) / STEPS as f64;
    let padding = (dx * dx + dy * dy) / crate::globe::RADIUS + 0.1;
    lower -= DVec3::splat(padding);
    upper += DVec3::splat(padding);
    let back = c.back();
    let right = DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
    let up = back.cross(right);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for x in [lower.x, upper.x] {
        for y in [lower.y, upper.y] {
            for z in [lower.z, upper.z] {
                let d = DVec3::new(x, y - c.height, z);
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
    let l = (min[0].floor() - 2.).clamp(0., size[0] as f64) as u32;
    let t = (min[1].floor() - 2.).clamp(0., size[1] as f64) as u32;
    let r = (max[0].ceil() + 2.).clamp(l as f64, size[0] as f64) as u32;
    let b = (max[1].ceil() + 2.).clamp(t as f64, size[1] as f64) as u32;
    [l, t, r - l, b - t]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interpolation_preserves_legacy_choices_and_roundtrips_all_modes() {
        assert_eq!(
            serde_json::from_str::<Interpolation>("true").unwrap(),
            Interpolation::Edge
        );
        assert_eq!(
            serde_json::from_str::<Interpolation>("false").unwrap(),
            Interpolation::Bilinear
        );
        for mode in Interpolation::ALL {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(serde_json::from_str::<Interpolation>(&json).unwrap(), mode);
        }
    }

    #[test]
    fn flat_zoomed_map_scissors_use_zero_elevation_and_cover_each_tile() {
        let size = [2686, 1564];
        let mut camera = Camera::default();
        camera.globe = true;
        camera.height = 0.;
        camera.set_map_view(
            [0.8844271266197067, 0.3963378090889512],
            19.,
            0.,
            0.,
            size.map(f64::from),
        );
        let side = 1. / 262144.;
        let cx = (camera.center[0] / WORLD / side).floor() as i32;
        let cy = (camera.center[1] / WORLD / side).floor() as i32;
        let back = camera.back();
        let right = glam::DVec3::new(1., 0., 0.);
        let up = back.cross(right);
        let mut old_pixels = 0u64;
        let mut new_pixels = 0u64;
        for y in cy - 2..=cy + 2 {
            for x in cx - 3..=cx + 3 {
                let bounds = [
                    x as f64 * side,
                    y as f64 * side,
                    (x + 1) as f64 * side,
                    (y + 1) as f64 * side,
                ];
                let old = scissor(bounds, &camera, size);
                let new = scissor_at_heights(bounds, &camera, size, [0., 0.]);
                old_pixels += u64::from(old[2]) * u64::from(old[3]);
                new_pixels += u64::from(new[2]) * u64::from(new[3]);
                for sy in 0..=4 {
                    for sx in 0..=4 {
                        let world = [
                            (x as f64 + sx as f64 / 4.) * side * WORLD,
                            (y as f64 + sy as f64 / 4.) * side * WORLD,
                        ];
                        let q = crate::globe::local(camera.center, world, 0.)
                            - glam::DVec3::Y * camera.height;
                        let depth = camera.distance - q.dot(back);
                        let scale = size[1] as f64 / (2. * camera.tan_half_fov() * depth);
                        let p = [
                            size[0] as f64 / 2. + q.dot(right) * scale,
                            size[1] as f64 / 2. - q.dot(up) * scale,
                        ];
                        if p[0] >= 0.
                            && p[0] < size[0] as f64
                            && p[1] >= 0.
                            && p[1] < size[1] as f64
                        {
                            assert!(p[0] >= new[0] as f64 && p[0] <= (new[0] + new[2]) as f64);
                            assert!(p[1] >= new[1] as f64 && p[1] <= (new[1] + new[3]) as f64);
                        }
                    }
                }
            }
        }
        eprintln!("平面タイル描画面積: {old_pixels} → {new_pixels} px/フレーム");
        assert!(new_pixels * 10 < old_pixels);
        assert!(new_pixels < u64::from(size[0]) * u64::from(size[1]) * 2);
    }
    #[test]
    fn globe_tile_scissors_cover_curved_edges_without_shading_the_whole_frame() {
        let mut c = Camera::default();
        c.globe = true;
        let size = [3357, 1953];
        c.center = [35572623.49885807, 15783474.539974594];
        c.height = 500.;
        c.distance = 867787.0600352235;
        c.yaw = 0.02880050428211689;
        c.pitch = 1.1057327250626425;
        c.viewport = size.map(f64::from);
        let back = c.back();
        let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
        let up = back.cross(right);
        let mut pixels = 0u64;
        let mut tiles = 0u64;
        for z in [6, 8, 10] {
            let side = WORLD / 2f64.powi(z);
            let cx = (c.center[0] / side) as i32;
            let cy = (c.center[1] / side) as i32;
            for y in cy - 2..=cy + 2 {
                for x in cx - 2..=cx + 2 {
                    let bounds = [
                        x as f64 * side / WORLD,
                        y as f64 * side / WORLD,
                        (x + 1) as f64 * side / WORLD,
                        (y + 1) as f64 * side / WORLD,
                    ];
                    let clip = scissor(bounds, &c, size);
                    pixels += u64::from(clip[2]) * u64::from(clip[3]);
                    tiles += 1;
                    for iy in 0..=16 {
                        for ix in 0..=16 {
                            for h in [-500., 0., 2500., 4500.] {
                                let world = [
                                    (x as f64 + ix as f64 / 16.) * side,
                                    (y as f64 + iy as f64 / 16.) * side,
                                ];
                                let d = crate::globe::local(c.center, world, h)
                                    - glam::DVec3::Y * c.height;
                                let depth = c.distance - d.dot(back);
                                if depth <= 0. {
                                    continue;
                                }
                                let scale = size[1] as f64 / (2. * c.tan_half_fov() * depth);
                                let p = [
                                    size[0] as f64 / 2. + d.dot(right) * scale,
                                    size[1] as f64 / 2. - d.dot(up) * scale,
                                ];
                                if p[0] >= 0.
                                    && p[0] < size[0] as f64
                                    && p[1] >= 0.
                                    && p[1] < size[1] as f64
                                {
                                    assert!(
                                        p[0] >= clip[0] as f64
                                            && p[0] < (clip[0] + clip[2]) as f64
                                            && p[1] >= clip[1] as f64
                                            && p[1] < (clip[1] + clip[3]) as f64,
                                        "球面の端が欠ける: z{z}, p={p:?}, clip={clip:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            pixels < tiles * u64::from(size[0]) * u64::from(size[1]) / 5,
            "元タイルごとに全面を描画している"
        );
    }
    #[test]
    fn globe_scissors_remain_conservative_near_the_horizon_poles_and_date_line() {
        let size = [1280, 800];
        for center in [[0.885, 0.395], [0.99999, 0.4], [0.5, 0.0001], [0.5, 0.9999]] {
            for (distance, z) in [(100., 18), (10000., 13), (867787., 8), (3000000., 5)] {
                for pitch in [0.1, 0.8, 1.56] {
                    let mut c = Camera::default();
                    c.globe = true;
                    c.center = center.map(|v| v * WORLD);
                    c.height = 1000.;
                    c.distance = distance;
                    c.pitch = pitch;
                    c.yaw = 0.7;
                    c.viewport = size.map(f64::from);
                    let n = 2f64.powi(z);
                    let x = (center[0] * n).floor();
                    let y = (center[1] * n).floor();
                    let area = [x / n, y / n, (x + 1.) / n, (y + 1.) / n];
                    let clip = scissor(area, &c, size);
                    let back = c.back();
                    let right = glam::DVec3::new(c.yaw.cos(), 0., -c.yaw.sin());
                    let up = back.cross(right);
                    for iy in 0..=12 {
                        for ix in 0..=12 {
                            for h in [-500., 0., 4500.] {
                                let w = [
                                    (x + ix as f64 / 12.) / n * WORLD,
                                    (y + iy as f64 / 12.) / n * WORLD,
                                ];
                                let d =
                                    crate::globe::local(c.center, w, h) - glam::DVec3::Y * c.height;
                                let depth = c.distance - d.dot(back);
                                if depth <= 0. {
                                    continue;
                                }
                                let scale = size[1] as f64 / (2. * c.tan_half_fov() * depth);
                                let p = [
                                    size[0] as f64 / 2. + d.dot(right) * scale,
                                    size[1] as f64 / 2. - d.dot(up) * scale,
                                ];
                                if p[0] >= 0.
                                    && p[0] < size[0] as f64
                                    && p[1] >= 0.
                                    && p[1] < size[1] as f64
                                {
                                    assert!(
                                        p[0] >= clip[0] as f64
                                            && p[0] < (clip[0] + clip[2]) as f64
                                            && p[1] >= clip[1] as f64
                                            && p[1] < (clip[1] + clip[3]) as f64,
                                        "画像範囲を切り過ぎた: center={center:?} z{z} pitch={pitch} p={p:?} clip={clip:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    #[ignore = "全面描画とタイル別描画範囲を同じGPUで計測する"]
    fn benchmark_globe_tile_scissors() -> anyhow::Result<()> {
        benchmark_tile_scissors(false)
    }
    #[test]
    #[ignore = "指定スナップショットの平面描画範囲を同じGPUで比較する"]
    fn benchmark_flat_zoomed_tile_scissors() -> anyhow::Result<()> {
        benchmark_tile_scissors(true)
    }
    fn benchmark_tile_scissors(flat: bool) -> anyhow::Result<()> {
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
        let renderer = Renderer::new(device);
        let size = if flat { [2686, 1564] } else { [1280, 768] };
        let mut camera = Camera::default();
        camera.globe = true;
        camera.center = [35572623.49885807, 15783474.539974594];
        camera.height = 500.;
        camera.distance = 867787.0600352235;
        camera.yaw = 0.02880050428211689;
        camera.pitch = 1.1057327250626425;
        camera.viewport = size.map(f64::from);
        if flat {
            camera.height = 0.;
            camera.set_map_view(
                [0.8844271266197067, 0.3963378090889512],
                19.,
                0.,
                0.,
                size.map(f64::from),
            );
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
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
        // 実際の球面への投影と同じ地理座標を使う。
        let projection = crate::Projection {
            camera: camera.clone(),
        };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32));
        let mut values = Vec::<[f32; 4]>::new();
        for y in 0..size[1] {
            for x in 0..size[0] {
                values.push(
                    projection
                        .world_at(rect, egui::pos2(x as f32 + 0.5, y as f32 + 0.5))
                        .map_or([0.; 4], |w| {
                            [
                                (w[0] * WORLD - camera.center[0]) as f32,
                                (w[1] * WORLD - camera.center[1]) as f32,
                                1.,
                                10000.,
                            ]
                        }),
                );
            }
        }
        queue.write_texture(
            positions.as_image_copy(),
            bytemuck::cast_slice(&values),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 16),
                rows_per_image: None,
            },
            extent,
        );
        let view = positions.create_view(&Default::default());
        let target = texture(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let lighting = texture(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let side = if flat { 1. / 262144. } else { 1. / 256. };
        let cx = (camera.center[0] / WORLD / side) as i32;
        let cy = (camera.center[1] / WORLD / side) as i32;
        let mut commands = Vec::new();
        let ry = if flat { 2 } else { 8 };
        let rx = if flat { 3 } else { 10 };
        for y in cy - ry..=cy + ry {
            for x in cx - rx..=cx + rx {
                commands.push(Quad {
                    bounds: [
                        x as f64 * side,
                        y as f64 * side,
                        (x + 1) as f64 * side,
                        (y + 1) as f64 * side,
                    ],
                    uv: [egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)); 2],
                    textures: [None, None],
                    amount: 1.,
                    tint: [0.3, 0.5, 0.8, 1.],
                    shade: false,
                    gamma_shade: false,
                    edge_filter: Interpolation::Bilinear,
                    border: false,
                });
            }
        }
        let mut outputs = Vec::new();
        for restricted in [false, true] {
            let begin = std::time::Instant::now();
            for _ in 0..12 {
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target.create_view(&Default::default()),
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                }
                renderer.render_region(
                    device,
                    queue,
                    &mut encoder,
                    &target,
                    &lighting,
                    &view,
                    &camera,
                    size,
                    &commands,
                    restricted,
                    if flat { [0., 0.] } else { [-500., 4500.] },
                );
                queue.submit([encoder.finish()]);
                let _ = device.poll(wgpu::Maintain::Wait);
            }
            eprintln!(
                "描画範囲制限={restricted}: {:.2} ms/フレーム（{}タイル）",
                begin.elapsed().as_secs_f64() * 1000. / 12.,
                commands.len()
            );
            let row_bytes = (size[0] * 4).div_ceil(256) * 256;
            let bytes = u64::from(row_bytes) * u64::from(size[1]);
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row_bytes),
                        rows_per_image: None,
                    },
                },
                extent,
            );
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            read.slice(..)
                .map_async(wgpu::MapMode::Read, move |v| send.send(v).unwrap());
            let _ = device.poll(wgpu::Maintain::Wait);
            receive.recv()??;
            outputs.push(read.slice(..).get_mapped_range().to_vec());
        }
        assert_eq!(
            outputs[0], outputs[1],
            "球面のタイル描画範囲を制限した結果、画像に欠けが生じた"
        );
        Ok(())
    }
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
                    edge_filter: Interpolation::Bilinear,
                    border: false,
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
            edge_filter: Interpolation::Bilinear,
            border: false,
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
        for mode in 0..15 {
            let mut q = match mode {
                0 | 14 => quad(checker.clone(), checker.clone(), 1.),
                1 => quad(black.clone(), white.clone(), 0.5),
                6 => quad(gray.clone(), gray.clone(), 1.),
                7 => quad(mid.clone(), mid.clone(), 1.),
                _ => quad(white.clone(), white.clone(), 1.),
            };
            q.edge_filter = if mode == 14 {
                Interpolation::Nearest
            } else {
                (mode >= 13).into()
            };
            if mode >= 13 {
                // 拡大時の補間経路でも単色が変色しないことを確認する。
                q.uv = [egui::Rect::from_min_max(egui::pos2(0.25, 0.25), egui::pos2(0.5, 0.5)); 2];
            }
            if (8..10).contains(&mode) {
                q.border = true;
                q.textures = [None, None];
                if mode == 9 {
                    q.uv =
                        [egui::Rect::from_min_max(egui::pos2(0.25, 0.25), egui::pos2(0.75, 0.75));
                            2];
                }
            }
            camera.globe = (10..=12).contains(&mode);
            camera.height = if mode == 12 {
                60000.
            } else if (10..=12).contains(&mode) {
                3000.
            } else {
                0.
            };
            let coordinates = (0..width)
                .map(|x| {
                    if (10..=12).contains(&mode) {
                        return [
                            ((if mode == 10 { 1. } else { 100000. }) / camera.scale()) as f32,
                            0.,
                            1.,
                            if mode == 12 { 70000. } else { 11000. },
                        ];
                    }
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
                if mode == 11 {
                    assert!(
                        pixel[0] < 248 && pixel[2] > pixel[0] + 3,
                        "遠景が青い霞になっていない: {pixel:?}"
                    );
                    continue;
                }
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
                    8 => {
                        if x < 2 || x >= width as usize - 2 {
                            255
                        } else {
                            0
                        }
                    }
                    9 => 0,
                    10 | 12 | 13 => 255,
                    14 => {
                        if (x / 4) % 2 == 0 {
                            0
                        } else {
                            255
                        }
                    }
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
