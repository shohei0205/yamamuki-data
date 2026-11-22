//! 描画途中のメッシュ集合も含め、SQLiteから同じ曲面を固定再生する。
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diagnostics: Option<Diagnostics>,
    pub version: u32,
    pub path: PathBuf,
    pub file_bytes: u64,
    pub generation: String,
    camera: selection::Camera,
    meshes: Vec<Mesh>,
    counts: [usize; 4],
    settings: [f32; 3],
    smooth: bool,
    wire: u32,
    borders: bool,
    colors: bool,
    join: bool,
    morph: f32,
    resolution: u32,
    layout: Option<shared_render::LayoutSnapshot>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Mesh {
    key: Key,
    n: u32,
    field: shared_field::Field,
}
// JSONへの変換は保存ワーカーへ任せ、記録中の毎フレームに小さな文字列を大量確保しない。
#[derive(Clone, Serialize, Deserialize)]
pub struct Diagnostics {
    camera: selection::Camera,
    selection_version: u64,
    queued: bool,
    scene_pending: bool,
    loading: bool,
    settled: bool,
    interpolation_pending: bool,
    wanted: Vec<Key>,
    pending_roots: Vec<Key>,
    packet_cache_count: usize,
    mesh_cache_count: usize,
    remembered_scenes: usize,
    remembered_until_selection: Option<u64>,
    source_bytes: usize,
    shown: Vec<DiagnosticMesh>,
    display: Option<DisplayDiagnostics>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct DisplayDiagnostics {
    pub camera: selection::Camera,
    pub meshes: Vec<DiagnosticMesh>,
    pub primary: usize,
    pub current: usize,
    pub morph: f32,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct DiagnosticMesh {
    key: Key,
    n: u32,
    field: shared_field::Field,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_divisions: Option<std::collections::BTreeMap<u32, usize>>,
}
impl DiagnosticMesh {
    pub(crate) fn new(t: &shared_render::Tile) -> Self {
        Self {
            key: t.key,
            n: t.n,
            field: t.field,
            patch_count: None,
            patch_divisions: None,
        }
    }
}
pub struct Prepared {
    snapshot: Snapshot,
    packets: HashMap<Key, Arc<Packet>>,
}
impl Snapshot {
    pub fn projection(&self) -> Projection {
        Projection {
            camera: self.camera.clone(),
        }
    }
    pub fn observer_projection(&self, yaw: f64, pitch: f64) -> Projection {
        let mut camera = self.camera.clone();
        let eye = camera.eye();
        camera.set_observer(
            [eye[0], eye[1]],
            eye[2],
            yaw,
            pitch,
            camera.fov_y,
            camera.viewport,
        );
        Projection { camera }
    }
    pub fn prepare(self) -> Result<Prepared> {
        anyhow::ensure!(self.version == 1, "未対応の地形スナップショットです");
        anyhow::ensure!(
            std::fs::metadata(&self.path)?.len() == self.file_bytes,
            "元の地形SQLiteのサイズが変わっています"
        );
        anyhow::ensure!(
            self.meshes.len() <= 16384
                && self
                    .counts
                    .iter()
                    .try_fold(0usize, |a, b| a.checked_add(*b))
                    == Some(self.meshes.len()),
            "メッシュ構成が不正です"
        );
        let mut db = Database::open(&self.path)?;
        anyhow::ensure!(
            self.generation.is_empty() || db.generation == self.generation,
            "地形データの版が変わっています"
        );
        let mut packets = HashMap::new();
        for mesh in &self.meshes {
            anyhow::ensure!(
                mesh.n.is_power_of_two() && mesh.n <= 512 && mesh.field.depth <= 24,
                "メッシュ分割が不正です"
            );
            anyhow::ensure!(
                mesh.field.x < (2u32 << mesh.field.depth)
                    && mesh.field.y < (2u32 << mesh.field.depth)
                    && mesh.field.x % 2 == 0
                    && mesh.field.y % 2 == 0,
                "評価区画が不正です"
            );
            if !packets.contains_key(&mesh.key) {
                packets.insert(mesh.key, db.packet(mesh.key)?);
            }
        }
        if let Some(layout) = &self.layout {
            anyhow::ensure!(
                layout.plans.len() == self.counts[..3].iter().sum::<usize>()
                    && layout.camera.iter().all(|v| v.is_finite())
                    && layout.pixels_per_point.is_finite()
                    && layout.pixels_per_point > 0.
                    && layout.pixels_per_point <= 8.,
                "描画計画が不正です"
            );
            anyhow::ensure!(
                self.camera
                    .viewport
                    .iter()
                    .all(|v| v.is_finite() && *v >= 1. && *v <= 32768.),
                "描画サイズが不正です"
            );
            anyhow::ensure!(
                layout.previous.is_empty() || layout.previous.len() == self.counts[3],
                "旧描画パッチの構成が不正です"
            );
            for p in layout
                .plans
                .iter()
                .chain(layout.previous.iter().flatten().map(|p| &p.plan))
            {
                anyhow::ensure!(p.patches.len() <= 1_048_576, "描画パッチが多すぎます");
                for patch in &p.patches {
                    anyhow::ensure!(
                        patch
                            .bounds
                            .iter()
                            .chain(&patch.edges)
                            .chain(&patch.origins)
                            .chain(&patch.lines)
                            .all(|v| v.is_finite())
                            && patch.bounds[2] > 0.
                            && patch.bounds[3] >= 1.
                            && patch.bounds[3] <= 512.
                            && (patch.bounds[3] as u32).is_power_of_two(),
                        "パッチ形状が不正です"
                    );
                }
                for (n, r) in &p.groups {
                    anyhow::ensure!(
                        n.is_power_of_two()
                            && *n <= 512
                            && r.start <= r.end
                            && r.end as usize <= p.patches.len(),
                        "描画範囲が不正です"
                    );
                }
            }
        }
        Ok(Prepared {
            snapshot: self,
            packets,
        })
    }
}
impl TerrainView {
    /// 記録時の非同期処理と実際の描画構成。再生側の処理速度に依存せず比較する。
    pub fn snapshot_diagnostics(&self) -> Diagnostics {
        let a = &self.app;
        Diagnostics {
            camera: a.camera.clone(),
            selection_version: a.version,
            queued: a.queued,
            scene_pending: a.scene_pending,
            loading: a.loading,
            settled: a.settled,
            interpolation_pending: a.renderer.interpolation_pending(),
            wanted: a.wanted.clone(),
            pending_roots: a.pending_roots.iter().copied().collect(),
            packet_cache_count: a.packets.len(),
            mesh_cache_count: a.local_gpu.len(),
            remembered_scenes: a.remembered.len(),
            remembered_until_selection: a.remembered_until_selection,
            source_bytes: a.renderer.source_bytes(),
            shown: a.shown.iter().map(|t| DiagnosticMesh::new(t)).collect(),
            display: a.renderer.snapshot_diagnostics(),
        }
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        self.capture_snapshot(true)
    }
    pub fn snapshot_without_meshes(&self) -> Result<Snapshot> {
        self.capture_snapshot(false)
    }
    fn capture_snapshot(&self, include_meshes: bool) -> Result<Snapshot> {
        let a = &self.app;
        let path = PathBuf::from(&a.path).canonicalize()?;
        let display = if include_meshes {
            Some(
                a.renderer
                    .snapshot_display()
                    .ok_or_else(|| anyhow::anyhow!("描画が完了してから保存してください"))?,
            )
        } else {
            None
        };
        let (camera, counts, meshes, morph, layout) =
            if let Some((tiles, primary, current, layout)) = display {
                let lists = [
                    &tiles[..primary],
                    &tiles[primary..current],
                    &tiles[0..0],
                    &tiles[current..],
                ];
                let meshes = lists
                    .into_iter()
                    .flatten()
                    .map(|m| Mesh {
                        key: m.key,
                        n: m.n,
                        field: m.field,
                    })
                    .collect();
                (
                    layout.render_camera.clone(),
                    lists.map(|v| v.len()),
                    meshes,
                    layout.morph,
                    Some(layout),
                )
            } else {
                (a.camera.clone(), [0; 4], Vec::new(), 1., None)
            };
        Ok(Snapshot {
            diagnostics: Some(self.snapshot_diagnostics()),
            version: 1,
            file_bytes: std::fs::metadata(&path)?.len(),
            path,
            camera,
            generation: a.generation.clone(),
            counts,
            meshes,
            settings: a.settings,
            smooth: a.smooth,
            wire: a.wire,
            borders: a.borders,
            colors: a.colors,
            join: a.join,
            morph,
            resolution: a.resolution,
            layout,
        })
    }
    pub fn from_snapshot(
        ctx: &egui::Context,
        state: &eframe::egui_wgpu::RenderState,
        prepared: Prepared,
    ) -> Result<Self> {
        let s = prepared.snapshot;
        let bytes = s
            .meshes
            .iter()
            .map(|m| u64::from(m.n + 1).pow(2) * 32)
            .sum::<u64>();
        anyhow::ensure!(
            bytes <= u64::from(state.device.limits().max_storage_buffer_binding_size),
            "このGPUでは保存時のメッシュを再現できません"
        );
        let mut view = Self::new(ctx, state, s.path.clone())?;
        view.app.generation = s.generation;
        view.app.packets = prepared.packets;
        let mut meshes = Vec::new();
        for m in &s.meshes {
            while !view.app.renderer.can_upload(m.n) {
                let _ = state.device.poll(eframe::wgpu::Maintain::Wait);
            }
            let tile = if m.field.depth == 0 {
                if let Some(tile) = view.app.gpu.get(&(m.key, m.n)) {
                    tile.clone()
                } else {
                    let tile =
                        view.app
                            .renderer
                            .upload_field(&view.app.packets[&m.key], m.n, m.field)?;
                    view.app.gpu.insert((m.key, m.n), tile.clone());
                    tile
                }
            } else if let Some(tile) = view.app.local_gpu.get(&(m.key, m.n, m.field)) {
                tile.clone()
            } else {
                let tile =
                    view.app
                        .renderer
                        .upload_field(&view.app.packets[&m.key], m.n, m.field)?;
                view.app
                    .local_gpu
                    .insert((m.key, m.n, m.field), tile.clone());
                tile
            };
            meshes.push(tile);
        }
        let mut at = 0;
        let mut take = |n: usize| {
            let v = meshes[at..at + n].to_vec();
            at += n;
            v
        };
        view.app.shown = take(s.counts[0]);
        view.app.local_background = take(s.counts[1]);
        view.app.roots = take(s.counts[2]);
        view.app.previous = take(s.counts[3]);
        view.app.previous_primary =
            s.layout
                .as_ref()
                .filter(|l| !l.previous.is_empty())
                .map_or(s.counts[3], |l| {
                    l.previous
                        .iter()
                        .take_while(|p| !p.as_ref().is_some_and(|p| p.background))
                        .count()
                });
        view.app.camera = s.camera;
        view.app.settings = s.settings;
        view.app.smooth = s.smooth;
        view.app.wire = s.wire;
        view.app.borders = s.borders;
        view.app.colors = s.colors;
        view.app.join = s.join;
        view.app.resolution = s.resolution;
        view.app.frozen_morph = Some(s.morph);
        view.app.renderer.frozen_layout = s.layout;
        view.app.queued = false;
        view.app.loading = false;
        view.app.status = "スナップショットを固定表示中".into();
        Ok(view)
    }
    pub fn replan_frozen(&mut self) {
        if let Some(layout) = &mut self.app.renderer.frozen_layout {
            layout.force_replan = true;
            layout.clipped_background = true;
        }
    }
    pub fn freeze_current(&mut self) {
        self.app.frozen_morph = Some(1.);
        self.app.queued = false;
    }
    pub fn unfreeze(&mut self) {
        if self.app.previous.is_empty() && self.app.frozen_morph.unwrap_or(1.) >= 1. {
            self.app.remember_scene();
        }
        self.app.frozen_morph = None;
        self.app.renderer.frozen_layout = None;
        self.app.previous.clear();
        self.app.transition = Instant::now() - Duration::from_secs(1);
        if !self.app.path.is_empty() {
            self.app.queued = true;
            self.app.scene_pending = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draw(view: &mut TerrainView) -> Result<Vec<u8>> {
        draw_inner(view, true)
    }
    fn draw_inner(view: &mut TerrainView, readback: bool) -> Result<Vec<u8>> {
        use eframe::egui_wgpu::{RenderState, ScreenDescriptor};
        let a = &mut view.app;
        let state: RenderState = a.renderer.state.clone();
        let rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(a.camera.viewport[0] as f32, a.camera.viewport[1] as f32),
        );
        let ppp = a
            .renderer
            .frozen_layout
            .as_ref()
            .map_or(1., |s| s.pixels_per_point);
        let size = a
            .renderer
            .frozen_layout
            .as_ref()
            .filter(|s| s.render_size.iter().all(|v| *v > 0))
            .map(|s| s.render_size)
            .unwrap_or([
                (rect.width() * ppp).round() as u32,
                (rect.height() * ppp).round() as u32,
            ]);
        let [width, height] = size;
        let callback = a.renderer.frame(
            a.shown.clone(),
            a.local_background.iter().chain(&a.roots).cloned().collect(),
            &a.camera,
            a.settings,
            a.smooth,
            a.wire,
            a.borders,
            a.colors,
            a.join,
            a.previous.clone(),
            a.frozen_morph.unwrap_or(0.37),
            a.resolution == 0,
            rect,
            1.,
            false,
            false,
            true,
        );
        let jobs = [egui::ClippedPrimitive {
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
                size_in_pixels: [width, height],
                pixels_per_point: 1.,
            },
        );
        if !readback {
            state
                .queue
                .submit(commands.into_iter().chain([encoder.finish()]));
            let _ = state.device.poll(eframe::wgpu::Maintain::Wait);
            return Ok(Vec::new());
        }
        let actual = a.renderer.positions.size();
        let (width, height) = (actual.width, actual.height);
        let row = (width * 16).div_ceil(256) * 256;
        let buffer = state.device.create_buffer(&eframe::wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row) * u64::from(height),
            usage: eframe::wgpu::BufferUsages::MAP_READ | eframe::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            eframe::wgpu::TexelCopyTextureInfo {
                texture: &a.renderer.positions,
                mip_level: 0,
                origin: eframe::wgpu::Origin3d::ZERO,
                aspect: eframe::wgpu::TextureAspect::All,
            },
            eframe::wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: eframe::wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            eframe::wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        state
            .queue
            .submit(commands.into_iter().chain([encoder.finish()]));
        let (send, receive) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(eframe::wgpu::MapMode::Read, move |v| send.send(v).unwrap());
        let _ = state.device.poll(eframe::wgpu::Maintain::Wait);
        receive.recv()??;
        let bytes = buffer.slice(..).get_mapped_range();
        Ok(bytes
            .chunks_exact(row as usize)
            .flat_map(|r| r[..(width * 16) as usize].iter().copied())
            .collect())
    }
    #[test]
    #[ignore = "操作記録のカメラ履歴で実際の非同期更新を進める"]
    fn external_recorded_camera_frames() -> Result<()> {
        let input: serde_json::Value = serde_json::from_reader(std::fs::File::open(
            std::env::var("YAMAMUKI_OPERATION_TEST_INPUT")?,
        )?)?;
        let recording = &input["recording"];
        let initial: Snapshot = serde_json::from_value(recording["initial"]["terrain"].clone())?;
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let ctx = egui::Context::default();
        let mut replay = TerrainView::from_snapshot(&ctx, &app.renderer.state, initial.prepare()?)?;
        let raw = draw(&mut replay)?;
        let size = replay.app.renderer.positions.size();
        let pixels: &[[f32; 4]] = bytemuck::cast_slice(&raw);
        let mut points = Vec::new();
        for y in 0..12 {
            for x in 0..24 {
                let q = pixels[(((y * size.height / 12 + size.height / 24) * size.width)
                    + (x * size.width / 24 + size.width / 48))
                    as usize];
                if q[3] == 0. {
                    continue;
                }
                let mut world = [
                    replay.app.camera.center[0] + q[0] as f64,
                    replay.app.camera.center[1] + q[1] as f64,
                ];
                if let Some(tile) = replay
                    .app
                    .shown
                    .iter()
                    .filter(|t| {
                        (0..2).all(|a| world[a] >= t.origin[a] && world[a] <= t.origin[a] + t.side)
                    })
                    .max_by_key(|t| t.key.0)
                {
                    let (axis, upper, _) = [
                        (0, false, world[0] - tile.origin[0]),
                        (0, true, tile.origin[0] + tile.side - world[0]),
                        (1, false, world[1] - tile.origin[1]),
                        (1, true, tile.origin[1] + tile.side - world[1]),
                    ]
                    .into_iter()
                    .min_by(|a, b| a.2.total_cmp(&b.2))
                    .unwrap();
                    world[axis] = tile.origin[axis] + if upper { tile.side } else { 0. };
                }
                points.push([
                    world[0] / shared_data::WORLD,
                    world[1] / shared_data::WORLD,
                    0.,
                ]);
            }
        }
        let mut history = Vec::new();
        replay.unfreeze();
        let mut frame = eframe::Frame::_new_kittest();
        let frames = recording["frames"].as_array().unwrap();
        for i in 0..frames.len() + 90 {
            let sample = &frames[i.min(frames.len() - 1)]["after"];
            let v = &sample["view"];
            let rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(
                    sample["canvas"][0].as_f64().unwrap() as f32,
                    sample["canvas"][1].as_f64().unwrap() as f32,
                ),
            );
            let r = &v["orientation"]["rotation"];
            replay.set_map_camera(
                [
                    v["center"][0].as_f64().unwrap(),
                    v["center"][1].as_f64().unwrap(),
                ],
                v["zoom"].as_f64().unwrap(),
                r["s"].as_f64().unwrap().atan2(r["c"].as_f64().unwrap()),
                v["orientation"]["vertical_scale"].as_f64().unwrap().acos(),
                rect,
            );
            replay.app.embedded_rect = Some(rect);
            let start = Instant::now();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    ..Default::default()
                },
                |ctx| {
                    replay.update(ctx, &mut frame);
                },
            );
            let update_ms = start.elapsed().as_secs_f64() * 1000.;
            let start = Instant::now();
            draw_inner(&mut replay, false)?;
            if std::env::var_os("YAMAMUKI_OPERATION_PROBE").is_some() {
                history.push(external_probe_heights(&mut replay, &ctx, &points)?);
            }
            if let Some(folder) = std::env::var_os("YAMAMUKI_OPERATION_IMAGE_DIR") {
                if i % 5 == 0 && i < frames.len() || i % 20 == 0 {
                    let folder = PathBuf::from(folder);
                    std::fs::create_dir_all(&folder)?;
                    let raw = draw(&mut replay)?;
                    if std::env::var("YAMAMUKI_OPERATION_CAPTURE_FRAME")
                        .ok()
                        .and_then(|v| v.parse::<usize>().ok())
                        == Some(i)
                    {
                        std::fs::write(folder.join(format!("frame-{i:03}.positions.bin")), &raw)?;
                        std::fs::write(
                            folder.join(format!("frame-{i:03}.terrain.json")),
                            serde_json::to_vec(&replay.snapshot()?)?,
                        )?;
                    }
                    let size = replay.app.renderer.positions.size();
                    let pixels: &[[f32; 4]] = bytemuck::cast_slice(&raw);
                    let step = 3usize;
                    let mut rgb = Vec::new();
                    for y in (0..size.height as usize).step_by(step) {
                        for x in (0..size.width as usize).step_by(step) {
                            let p = pixels[y * size.width as usize + x];
                            let color = if p[3] == 0. {
                                [235, 244, 252]
                            } else {
                                let h = ((p[3].abs() - 10000.) / 4000.).clamp(0., 1.);
                                let shade = p[2].clamp(0., 1.);
                                [
                                    (70. + 150. * h) * shade,
                                    (130. + 50. * h) * shade,
                                    (60. + 80. * h) * shade,
                                ]
                                .map(|v| v as u8)
                            };
                            rgb.extend(color);
                        }
                    }
                    let mut encoder = png::Encoder::new(
                        std::fs::File::create(folder.join(format!("frame-{i:03}.png")))?,
                        size.width.div_ceil(3),
                        size.height.div_ceil(3),
                    );
                    encoder.set_color(png::ColorType::Rgb);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder.write_header()?.write_image_data(&rgb)?;
                }
            }
            if let Ok(point) = std::env::var("YAMAMUKI_OPERATION_DETAIL_POINT") {
                let point = point
                    .split(',')
                    .map(str::parse::<f64>)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                anyhow::ensure!(point.len() == 2, "経度,緯度を指定してください");
                let world = selection::lonlat(point[0], point[1]);
                let best = replay
                    .app
                    .shown
                    .iter()
                    .filter(|t| {
                        (0..2).all(|axis| {
                            world[axis] >= t.origin[axis] && world[axis] <= t.origin[axis] + t.side
                        })
                    })
                    .map(|t| (t.side / t.n as f64, t.key, t.n, t.field))
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let drawn = replay.app.renderer.snapshot_display().and_then(
                    |(tiles, primary, _, layout)| {
                        tiles
                            .iter()
                            .take(primary)
                            .zip(layout.plans.iter())
                            .flat_map(|(tile, plan)| {
                                plan.patches
                                    .iter()
                                    .filter_map(|patch| {
                                        let b = patch.bounds;
                                        ((0..2).all(|axis| {
                                            world[axis] >= tile.origin[axis] + b[axis] as f64
                                                && world[axis]
                                                    <= tile.origin[axis] + (b[axis] + b[2]) as f64
                                        }))
                                        .then_some(b[2] as f64 / b[3] as f64)
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .min_by(f64::total_cmp)
                    },
                );
                eprintln!("detail frame={i} best={best:?} drawn={drawn:?}");
            }
            let mut counts = std::collections::BTreeMap::new();
            for t in &replay.app.shown {
                *counts.entry((t.key.0, t.n)).or_insert(0usize) += 1;
            }
            eprintln!(
                "連続記録 frame={i} update={update_ms:.2}ms draw={:.2}ms shown={} roots={} pending={} meshes={counts:?}",
                start.elapsed().as_secs_f64() * 1000.,
                replay.app.shown.len(),
                replay.app.roots.len(),
                replay.app.scene_pending
            );
        }
        if let Some(path) = std::env::var_os("YAMAMUKI_OPERATION_PROBE") {
            let largest = (0..points.len())
                .filter_map(|i| {
                    let values = history
                        .iter()
                        .map(|frame| frame[i])
                        .filter(|v| v[3] > 0.)
                        .map(|v| v[2])
                        .collect::<Vec<_>>();
                    if values.is_empty() {
                        return None;
                    }
                    Some(
                        values.iter().copied().fold(f32::NEG_INFINITY, f32::max)
                            - values.iter().copied().fold(f32::INFINITY, f32::min),
                    )
                })
                .fold(0f32, f32::max);
            eprintln!("固定した境界地点の最大標高変動: {largest:.4}m");
            std::fs::write(
                path,
                serde_json::to_vec(&serde_json::json!({"points":points,"history":history}))?,
            )?;
            if let Ok(limit) = std::env::var("YAMAMUKI_OPERATION_MAX_HEIGHT_DELTA") {
                let limit: f32 = limit.parse()?;
                anyhow::ensure!(
                    largest <= limit,
                    "境界の標高変動が許容値を超えた: {largest}m > {limit}m"
                );
            }
        }
        Ok(())
    }

    #[test]
    #[ignore = "実視点で固定座標の描画標高を小さなカメラ移動・再分割の前後で比較する"]
    fn external_small_camera_move_height_stability() -> Result<()> {
        check_external_height_stability(false, false, None)
    }
    #[test]
    #[ignore = "指定のズーム不具合スナップショットで外挿元の安定性を検査する"]
    fn external_zoom_keeps_neighbor_extrapolation_stable() -> Result<()> {
        check_external_height_stability(true, true, Some([0.8827219158358839, 0.3906429162144127]))
    }
    #[test]
    #[ignore = "固定解除後のズームで、境界が更新前と完了後の範囲を超えて変形しないか検査する"]
    fn external_zoom_does_not_temporarily_dent_terrain_seams() -> Result<()> {
        check_external_camera_transition(false)
    }
    #[test]
    #[ignore = "固定解除後のカメラ移動で、境界の一時的な凹みと描画時間を検査する"]
    fn external_pan_does_not_temporarily_dent_terrain_seams() -> Result<()> {
        check_external_camera_transition(true)
    }
    fn check_external_camera_transition(pan: bool) -> Result<()> {
        use anyhow::ensure;
        let snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(std::env::var(
            "YAMAMUKI_SNAPSHOT_TEST_INPUT",
        )?)?)?;
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let ctx = egui::Context::default();
        let mut replay =
            TerrainView::from_snapshot(&ctx, &app.renderer.state, snapshot.prepare()?)?;
        let original = replay.app.camera.clone();
        let raw = draw(&mut replay)?;
        let pixels: &[[f32; 4]] = bytemuck::cast_slice(&raw);
        let size = replay.app.renderer.positions.size();
        let mut points = Vec::new();
        for row in 0..16 {
            for col in 0..32 {
                let x = (col * size.width / 32 + size.width / 64).min(size.width - 1);
                let y = (row * size.height / 16 + size.height / 32).min(size.height - 1);
                let q = pixels[(y * size.width + x) as usize];
                if q[3] != 0. {
                    points.push([
                        (original.center[0] + q[0] as f64) / shared_data::WORLD,
                        (original.center[1] + q[1] as f64) / shared_data::WORLD,
                        0.,
                    ]);
                }
            }
        }
        // 実際に表示されていた位置を評価区画の辺へ寄せ、その両側も照会する。
        let mut borders = Vec::new();
        for p in &points {
            let q = [p[0] * shared_data::WORLD, p[1] * shared_data::WORLD];
            if let Some(tile) = replay
                .app
                .shown
                .iter()
                .filter(|t| (0..2).all(|a| q[a] >= t.origin[a] && q[a] <= t.origin[a] + t.side))
                .max_by(|a, b| {
                    a.key
                        .0
                        .cmp(&b.key.0)
                        .then_with(|| b.side.total_cmp(&a.side))
                })
            {
                let (axis, upper, _) = [
                    (0, false, q[0] - tile.origin[0]),
                    (0, true, tile.origin[0] + tile.side - q[0]),
                    (1, false, q[1] - tile.origin[1]),
                    (1, true, tile.origin[1] + tile.side - q[1]),
                ]
                .into_iter()
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .unwrap();
                for delta in [-1., 0., 1.] {
                    let mut xy = q;
                    xy[axis] = tile.origin[axis] + if upper { tile.side } else { 0. } + delta;
                    borders.push([xy[0] / shared_data::WORLD, xy[1] / shared_data::WORLD, 0.]);
                }
            }
        }
        points.extend(borders);
        replay.unfreeze();
        let mut db = Database::open(std::path::Path::new(&replay.app.path))?;
        replay.app.catalog = db.catalog.clone();
        replay.app.remembered_until_selection = None;
        replay.app.loading = false;
        for factor in if pan {
            vec![0., 10., 30., 50., 100., 300., 500., 1000., 500., 0.]
        } else {
            vec![1., 0.98, 0.95, 0.9, 0.8, 0.7, 0.6]
        } {
            replay.app.camera = original.clone();
            if pan {
                replay.app.camera.center[0] += factor;
                replay.app.camera.center[1] += factor * 0.3;
            } else {
                replay.app.camera.distance *= factor;
                replay.app.camera.center[0] += 0.01;
            }
            replay.app.frozen_morph = Some(1.);
            draw_inner(&mut replay, false)?;
            let before = external_probe_heights(&mut replay, &ctx, &points)?;
            replay.app.wanted = selection::select(&replay.app.catalog, &replay.app.camera);
            for key in replay.app.wanted.clone() {
                if !replay.app.packets.contains_key(&key) {
                    replay.app.packets.insert(key, db.packet(key)?);
                }
            }
            let deadline = Instant::now() + Duration::from_secs(45);
            while !replay.app.scene()? {
                ensure!(Instant::now() < deadline, "曲面準備が完了しない");
                let _ = replay
                    .app
                    .renderer
                    .state
                    .device
                    .poll(eframe::wgpu::Maintain::Wait);
                std::thread::sleep(Duration::from_millis(2));
            }
            eprintln!(
                "曲面切替: 主={} 背景={} 旧={}",
                replay.app.shown.len(),
                replay.app.local_background.len(),
                replay.app.previous.len()
            );
            replay.app.frozen_morph = Some(0.);
            draw_inner(&mut replay, false)?;
            let after = external_probe_heights(&mut replay, &ctx, &points)?;
            let mut largest = (0f32, 0usize);
            let mut screen_jump = 0f64;
            let mut count = 0;
            for (i, (a, b)) in before.iter().zip(&after).enumerate() {
                if a[2] == 0. || b[2] == 0. || a[3] == 0. || b[3] == 0. {
                    continue;
                }
                let dx = f64::from(a[0] - b[0]) * original.viewport[0];
                let dy = f64::from(a[1] - b[1]) * original.viewport[1];
                screen_jump = screen_jump.max(dx.hypot(dy));
                let delta = (a[2] - b[2]).abs();
                if delta > largest.0 {
                    largest = (delta, i);
                }
                count += 1;
            }
            eprintln!(
                "切替開始 ズーム距離倍率={factor} 比較={count}点 最大差={}m 地点={:?} 前={}m 後={}m",
                largest.0, points[largest.1], before[largest.1][2], after[largest.1][2]
            );
            ensure!(count > 0, "表示中の照会点がない");
            eprintln!("切替開始の最大画面差: {screen_jump}px");
            // 同一保存タイルの評価更新は、新面へ直接切り替える。開始位置の
            // 一致でなく、開始から完了までの余分な上下動と境界を検査する。
            let mut stages = Vec::new();
            for t in [0.25f32, 0.5, 0.75, 1.] {
                replay.app.frozen_morph = Some(t);
                let started = Instant::now();
                draw_inner(&mut replay, false)?;
                eprintln!(
                    "遷移描画 t={t}: {:.2}ms",
                    started.elapsed().as_secs_f64() * 1000.
                );
                stages.push((t, external_probe_heights(&mut replay, &ctx, &points)?));
            }
            let end = &stages.last().unwrap().1;
            let projection = replay.projection();
            let rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(original.viewport[0] as f32, original.viewport[1] as f32),
            );
            let mut deviation = 0f32;
            for (t, stage) in &stages[..3] {
                for (i, mid) in stage.iter().enumerate() {
                    let a = after[i];
                    let b = end[i];
                    if a[3] == 0. || b[3] == 0. || mid[3] == 0. {
                        continue;
                    }
                    let expected = a[2] * (1. - t) + b[2] * t;
                    let xy = [points[i][0], points[i][1]];
                    if let (Some(a), Some(b)) = (
                        projection.screen(rect, xy, expected as f64),
                        projection.screen(rect, xy, mid[2] as f64),
                    ) {
                        deviation = deviation.max(a.distance(b));
                    }
                }
            }
            eprintln!("遷移途中の直線的な変化からの最大画面差: {deviation}px");
            ensure!(
                deviation < 0.25,
                "遷移途中で余分な上下動が発生: {deviation}px"
            );
            let mut overshoot = 0f32;
            for stage in std::iter::once(&after).chain(stages.iter().map(|(_, v)| v)) {
                for (i, mid) in stage.iter().enumerate() {
                    let a = before[i];
                    let b = end[i];
                    if a[3] == 0. || b[3] == 0. || mid[3] == 0. {
                        continue;
                    }
                    let expected = mid[2].clamp(a[2].min(b[2]), a[2].max(b[2]));
                    let xy = [points[i][0], points[i][1]];
                    if let (Some(a), Some(b)) = (
                        projection.screen(rect, xy, expected as f64),
                        projection.screen(rect, xy, mid[2] as f64),
                    ) {
                        overshoot = overshoot.max(a.distance(b));
                    }
                }
            }
            eprintln!("更新前と完了後の範囲を超える一時変形: {overshoot}px");
            ensure!(overshoot < 0.25, "境界が一時的に凹んだ: {overshoot}px");
            replay.app.previous.clear();
            replay.app.transition = Instant::now() - Duration::from_secs(1);
        }
        Ok(())
    }
    fn external_probe_heights(
        replay: &mut TerrainView,
        ctx: &egui::Context,
        points: &[[f64; 3]],
    ) -> Result<Vec<[f32; 4]>> {
        *replay.app.renderer.point_state.lock().unwrap() = Default::default();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(values) = replay.point_projection(ctx, points) {
                let state = replay.app.renderer.point_state.lock().unwrap();
                if state.is_settled() {
                    return Ok(values);
                }
            }
            anyhow::ensure!(Instant::now() < deadline, "標高照会が完了しない");
            draw_inner(replay, false)?;
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn check_external_height_stability(
        zoom: bool,
        rebuild: bool,
        probe: Option<[f64; 2]>,
    ) -> Result<()> {
        use crate::shared_data::WORLD;
        use anyhow::ensure;
        let snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(std::env::var(
            "YAMAMUKI_SNAPSHOT_TEST_INPUT",
        )?)?)?;
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let ctx = egui::Context::default();
        let mut replay =
            TerrainView::from_snapshot(&ctx, &app.renderer.state, snapshot.prepare()?)?;
        let original = replay.app.camera.clone();
        let raw = draw(&mut replay)?;
        let values: &[[f32; 4]] = bytemuck::cast_slice(&raw);
        let size = replay.app.renderer.positions.size();
        let mut points = Vec::new();
        for row in 0..16 {
            for col in 0..32 {
                let x = (col * size.width / 32 + size.width / 64).min(size.width - 1);
                let y = (row * size.height / 16 + size.height / 32).min(size.height - 1);
                let q = values[(y * size.width + x) as usize];
                if q[3] != 0. {
                    points.push([
                        (original.center[0] + q[0] as f64) / WORLD,
                        (original.center[1] + q[1] as f64) / WORLD,
                        q[3].abs() as f64 - 10000.,
                    ]);
                }
            }
        }
        if let Some(xy) = probe {
            points.push([xy[0], xy[1], 0.]);
        }
        ensure!(!points.is_empty(), "地形上の照会点がない");
        if rebuild {
            let db = Database::open(std::path::Path::new(&replay.app.path))?;
            replay.app.catalog = db.catalog.clone();
            replay.app.wanted = replay
                .app
                .shown
                .iter()
                .map(|t| t.key)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            replay.app.remembered_until_selection = None;
            replay.app.loading = false;
            replay.app.frozen_morph = Some(1.);
        }
        let mut reference: Option<Vec<[f32; 4]>> = None;
        for replan in [false, true] {
            if replan {
                replay.replan_frozen();
                reference = None;
            }
            for shift in if zoom {
                vec![1., 0.95, 0.9, 0.8, 0.7, 0.6, 0.8, 1.]
            } else {
                vec![0.01, 10., -10., 0.01]
            } {
                replay.app.camera = original.clone();
                if zoom {
                    replay.app.camera.distance *= shift;
                    replay.app.camera.center[0] += 0.01;
                } else {
                    replay.app.camera.center[0] += shift;
                }
                if rebuild && replan {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                    while !replay.app.scene()? {
                        ensure!(
                            std::time::Instant::now() < deadline,
                            "曲面の再準備が完了しない"
                        );
                        let _ = replay
                            .app
                            .renderer
                            .state
                            .device
                            .poll(eframe::wgpu::Maintain::Wait);
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    replay.app.previous.clear();
                }
                draw_inner(&mut replay, false)?;
                if replan {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                    while replay
                        .app
                        .renderer
                        .current_plan_matches_reference()
                        .is_none()
                    {
                        ensure!(
                            std::time::Instant::now() < deadline,
                            "分割計画が追いつかない"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        draw_inner(&mut replay, false)?;
                    }
                }
                let draw_started = std::time::Instant::now();
                draw_inner(&mut replay, false)?;
                eprintln!(
                    "描画計測 {:.2}ms / {:?}",
                    draw_started.elapsed().as_secs_f64() * 1000.,
                    replay.app.renderer.metrics()
                );
                *replay.app.renderer.point_state.lock().unwrap() = Default::default();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                let heights = loop {
                    if let Some(values) = replay.point_projection(&ctx, &points) {
                        break values;
                    }
                    ensure!(std::time::Instant::now() < deadline, "標高照会が完了しない");
                    draw_inner(&mut replay, false)?;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                };
                let baseline = reference.get_or_insert_with(|| heights.clone());
                let mut largest = (0., 0);
                let mut count = 0;
                let mut squares = 0.;
                let mut probe_seen = false;
                for (i, (a, b)) in baseline.iter().zip(&heights).enumerate() {
                    if a[2] == 0.
                        || b[2] == 0.
                        || replay
                            .app
                            .renderer
                            .describe_height_point([points[i][0], points[i][1]])
                            == "ownerなし"
                    {
                        continue;
                    }
                    let delta = (a[2] - b[2]).abs();
                    if probe.is_some() && i + 1 == points.len() {
                        probe_seen = true;
                        eprintln!("注目点の標高: 元={}m 現在={}m 差={}m", a[2], b[2], delta);
                        ensure!(delta < 0.5, "注目点の標高が変わった: {delta}m");
                    }
                    if delta > largest.0 {
                        largest = (delta, i);
                    }
                    squares += delta as f64 * delta as f64;
                    count += 1;
                }
                ensure!(
                    probe.is_none() || probe_seen,
                    "検査対象の山が描画範囲にない"
                );
                if let Some(xy) = probe {
                    eprintln!("注目点: {}", replay.app.renderer.describe_height_point(xy));
                }
                eprintln!(
                    "標高安定性 再分割={replan} ズーム={zoom} 操作量={shift}: {count}点 / 最大{:.3}m / RMS {:.3}m / 位置 {:?}",
                    largest.0,
                    (squares / (count.max(1) as f64)).sqrt(),
                    points[largest.1]
                );
                if !rebuild {
                    ensure!(
                        largest.0 < 0.5,
                        "小さなカメラ移動で描画標高が変わった: {}m",
                        largest.0
                    );
                }
            }
        }
        Ok(())
    }
    #[test]
    #[ignore = "実データの視点変更時の取得・GPU準備・切替待ちを計測する"]
    fn benchmark_external_mesh_switch_latency() -> Result<()> {
        let path = std::env::var("YAMAMUKI_SNAPSHOT_TEST_INPUT")?;
        let snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(path)?)?;
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let mut replay = TerrainView::from_snapshot(
            &egui::Context::default(),
            &app.renderer.state,
            snapshot.prepare()?,
        )?;
        if let Some(layout) = &mut replay.app.renderer.frozen_layout {
            layout.force_replan = true;
            layout.clipped_background = true;
        }
        let a = &mut replay.app;
        let mut db = Database::open(std::path::Path::new(&a.path))?;
        a.catalog = db.catalog.clone();
        a.wanted = selection::select(&a.catalog, &a.camera);
        a.remembered_until_selection = None;
        a.loading = true;
        a.previous = a.shown.clone();
        a.transition = std::time::Instant::now();
        let all_started = std::time::Instant::now();
        let parallelism = std::thread::available_parallelism()
            .map_or(2, usize::from)
            .clamp(1, 8);
        let (cpu_ms, gpu_ms, frames) = if std::env::var_os("YAMAMUKI_MESH_PIPELINED").is_some() {
            // 取得とGPU準備を実際に重ねる。現在の表示は全領域が揃うまで維持する。
            let keys = a.wanted.clone();
            for key in &keys {
                a.packets.remove(key);
            }
            let (send, receive) = std::sync::mpsc::channel();
            let job = std::thread::spawn(move || -> Result<f64> {
                let started = std::time::Instant::now();
                for batch in keys.chunks(parallelism) {
                    db.packet_batch(batch, &|p| {
                        let _ = send.send(p);
                    })?;
                }
                Ok(started.elapsed().as_secs_f64() * 1000.)
            });
            let mut frames = 0;
            loop {
                for p in receive.try_iter() {
                    a.packets.insert(p.key, p);
                }
                if job.is_finished() {
                    a.loading = false;
                }
                if a.scene()? {
                    break;
                }
                let _ = a.renderer.state.device.poll(eframe::wgpu::Maintain::Wait);
                frames += 1;
                if frames % 250 == 0 {
                    let plan = a.preparation_plan();
                    let missing = a
                        .wanted
                        .iter()
                        .filter(|k| !a.packets.contains_key(k))
                        .count();
                    eprintln!(
                        "メッシュ準備進捗: {frames}フレーム / {}区画 / ローカルGPU {} / 不足 {missing} / CPU完了 {}",
                        plan.len(),
                        a.local_gpu.len(),
                        job.is_finished()
                    );
                }
                anyhow::ensure!(frames < 3000, "準備が完了しません");
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            let cpu_ms = job.join().unwrap()?;
            (cpu_ms, all_started.elapsed().as_secs_f64() * 1000., frames)
        } else {
            let mut loaded = Vec::new();
            for batch in a.wanted.chunks(parallelism) {
                let packets = std::sync::Mutex::new(Vec::new());
                db.packet_batch(batch, &|p| packets.lock().unwrap().push(p))?;
                loaded.extend(packets.into_inner().unwrap());
            }
            let mut stages = [0.; 4];
            for p in &loaded {
                for (sum, ms) in stages.iter_mut().zip(p.timings_ms) {
                    *sum += ms;
                }
            }
            eprintln!(
                "CPUステージの合計（並列前）復号/三角形/勾配/補助: {stages:?}ms / {parallelism}並列"
            );
            let cpu_ms = all_started.elapsed().as_secs_f64() * 1000.;
            for packet in loaded {
                a.packets.insert(packet.key, packet);
            }
            a.loading = false;
            let start = std::time::Instant::now();
            let mut frames = 0;
            while !a.scene()? {
                let _ = a.renderer.state.device.poll(eframe::wgpu::Maintain::Wait);
                frames += 1;
                anyhow::ensure!(frames < 1000, "準備が完了しません");
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            (cpu_ms, start.elapsed().as_secs_f64() * 1000., frames)
        };
        let _ = a.renderer.state.device.poll(eframe::wgpu::Maintain::Wait);
        let planned = a.preparation_plan();
        let samples = planned
            .iter()
            .map(|(_, _, n, _, _)| u64::from(*n + 1).pow(2))
            .sum::<u64>();
        eprintln!(
            "表示切替: {}タイル / {}評価区画 / {samples}点 / CPU復号・補助={cpu_ms:.2}ms / GPU準備・遷移待ち={:.2}ms / 全体={:.2}ms / {frames}フレーム",
            a.wanted.len(),
            planned.len(),
            gpu_ms,
            all_started.elapsed().as_secs_f64() * 1000.
        );
        let draw_started = std::time::Instant::now();
        draw_inner(&mut replay, false)?;
        eprintln!(
            "準備後の初回描画完了={:.2}ms / 全体={:.2}ms / 描画量={:?}",
            draw_started.elapsed().as_secs_f64() * 1000.,
            all_started.elapsed().as_secs_f64() * 1000.,
            replay.app.renderer.metrics()
        );
        Ok(())
    }
    #[test]
    #[ignore = "実データの切替後もアトラスに古い高さが混入しないことをGPUで検証する"]
    fn external_scene_switch_preserves_surface_positions() -> Result<()> {
        let path = std::env::var("YAMAMUKI_SNAPSHOT_TEST_INPUT")?;
        let mut snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(path)?)?;
        snapshot.camera.viewport = [512., 320.];
        if let Some(layout) = &mut snapshot.layout {
            layout.force_replan = true;
            layout.clipped_background = true;
            layout.render_size = [512, 320];
            layout.pixels_per_point = 1.;
        }
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let mut replay = TerrainView::from_snapshot(
            &egui::Context::default(),
            &app.renderer.state,
            snapshot.prepare()?,
        )?;
        replay.app.previous.clear();
        replay.app.frozen_morph = Some(1.);
        let tiles = replay.app.shown.clone();
        let reference = draw(&mut replay)?;
        replay.app.previous = tiles.clone();
        replay.app.shown = tiles.iter().step_by(2).cloned().collect();
        replay.app.frozen_morph = Some(0.5);
        draw_inner(&mut replay, false)?;
        replay.app.previous.clear();
        replay.app.shown = tiles;
        replay.app.frozen_morph = Some(1.);
        let restored = draw(&mut replay)?;
        let a: &[f32] = bytemuck::cast_slice(&reference);
        let b: &[f32] = bytemuck::cast_slice(&restored);
        anyhow::ensure!(a.len() == b.len(), "描画サイズが変化しました");
        for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            anyhow::ensure!((a[3] == 0.) == (b[3] == 0.), "切替で穴が生じました");
            anyhow::ensure!(
                a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 0.02),
                "アトラス再利用で位置が変わりました: {a:?} / {b:?}"
            );
        }
        eprintln!(
            "メッシュの半数を外して再追加: {}画素の位置・高さが一致",
            a.len() / 4
        );
        Ok(())
    }
    #[test]
    #[ignore = "共有バッファの切替と連続描画を実データで計測する"]
    fn benchmark_external_scene_switching() -> Result<()> {
        let path = std::env::var("YAMAMUKI_SNAPSHOT_TEST_INPUT")?;
        let mut snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(path)?)?;
        if let Some(s) = &mut snapshot.layout {
            s.force_replan = true;
            s.clipped_background = true;
        }
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let mut replay = TerrainView::from_snapshot(
            &egui::Context::default(),
            &app.renderer.state,
            snapshot.prepare()?,
        )?;
        let distance = replay.app.camera.distance;
        let shown = replay.app.shown.clone();
        for (i, factor) in [1., 1., 2., 2., 4., 4., 1., 1.].into_iter().enumerate() {
            replay.app.camera.distance = distance * factor;
            if i == 2 {
                replay.app.previous = shown.clone();
                replay.app.frozen_morph = Some(0.5);
            }
            if i == 3 {
                replay.app.previous.clear();
                replay.app.frozen_morph = Some(1.);
            }
            if let Some(layout) = &mut replay.app.renderer.frozen_layout {
                layout.force_replan = true;
            }
            // 分割計画の100ms抑制に左右されず、各距離で同じ計画を比較する。
            std::thread::sleep(std::time::Duration::from_millis(120));
            let started = std::time::Instant::now();
            let _ = draw_inner(&mut replay, false)?;
            eprintln!(
                "frame={i} distance={factor} 完了={:.2}ms CPU={:?} 描画={:?}",
                started.elapsed().as_secs_f64() * 1000.,
                replay.app.renderer.prepare_stats(),
                replay.app.renderer.metrics()
            );
        }
        for mode in ["nojoin", "quarter_pixels"] {
            replay.app.join = mode != "nojoin";
            if mode == "quarter_pixels" {
                if let Some(layout) = &mut replay.app.renderer.frozen_layout {
                    layout.render_size = layout.render_size.map(|v| (v / 2).max(1));
                }
            }
            for _ in 0..2 {
                let started = std::time::Instant::now();
                let _ = draw_inner(&mut replay, false)?;
                eprintln!(
                    "mode={mode} 完了={:.2}ms CPU={:?}",
                    started.elapsed().as_secs_f64() * 1000.,
                    replay.app.renderer.prepare_stats()
                );
            }
        }
        Ok(())
    }
    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
    fn shared_control_buffers_keep_local_field_coordinates() -> Result<()> {
        let backend = if cfg!(windows) {
            eframe::wgpu::Backends::DX12
        } else {
            eframe::wgpu::Backends::PRIMARY
        };
        let Some(mut app) = crate::remembered_scene_tests::app_with_backend(backend) else {
            return Ok(());
        };
        let key = (13, 7220, 3200);
        let field = crate::shared_field::Field {
            depth: 1,
            x: 2,
            y: 2,
        };
        let mut packet = crate::shared_data::Packet::constant_plane(key, 1500.)?;
        for v in &mut packet.vertices {
            v[2] = v[0] / packet.side as f32 * 1000.;
        }
        let tile = app.renderer.upload_field(&packet, 32, field)?;
        app.camera.center = tile.origin.map(|v| v + tile.side / 2.);
        app.camera.height = 750.;
        app.camera.distance = 10000.;
        app.camera.pitch = 1.56;
        app.camera.globe = false;
        app.camera.viewport = [64., 64.];
        app.shown = vec![tile];
        app.resolution = 0;
        app.frozen_morph = Some(1.);
        let mut view = TerrainView {
            label: String::new(),
            app,
            automatic_maximum: None,
        };
        let pixels = draw(&mut view)?;
        let p = &bytemuck::cast_slice::<u8, f32>(&pixels)[(32 * 64 + 32) * 4..][..4];
        let height = p[3].abs() - 10000.;
        anyhow::ensure!(
            p[3] != 0. && (650. ..850.).contains(&height),
            "局所領域の制御点座標がずれています: {p:?}"
        );
        Ok(())
    }
    #[test]
    #[ignore = "指定したスナップショットの同じメッシュ・視点で描画時間と出力を比較する"]
    fn benchmark_external_fixed_scene() -> Result<()> {
        let snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(std::env::var(
            "YAMAMUKI_SNAPSHOT_TEST_INPUT",
        )?)?)?;
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let mut replay = TerrainView::from_snapshot(
            &egui::Context::default(),
            &app.renderer.state,
            snapshot.prepare()?,
        )?;
        for _ in 0..3 {
            draw_inner(&mut replay, false)?;
        }
        let start = Instant::now();
        let mut cpu = 0.;
        for _ in 0..16 {
            draw_inner(&mut replay, false)?;
            cpu += replay.app.renderer.prepare_stats().prepare_ms;
        }
        eprintln!(
            "固定視点: 平均={:.3}ms CPU={:.3}ms 描画={:?}",
            start.elapsed().as_secs_f64() * 1000. / 16.,
            cpu / 16.,
            replay.app.renderer.metrics()
        );
        let diagnostic_start = Instant::now();
        for _ in 0..16 {
            std::hint::black_box(replay.snapshot_diagnostics());
        }
        eprintln!(
            "フレーム診断の収集: 平均={:.3}ms",
            diagnostic_start.elapsed().as_secs_f64() * 1000. / 16.
        );
        eprintln!(
            "接合候補（元の表・接合評価用）: {:?}",
            replay.app.renderer.candidate_counts()
        );
        if let Some(path) = std::env::var_os("YAMAMUKI_FIXED_POSITION_OUTPUT") {
            std::fs::write(path, draw(&mut replay)?)?;
        }
        Ok(())
    }

    #[test]
    #[ignore = "実スナップショットでGPU分割計画をCPU計画と照合する"]
    fn compare_external_gpu_planner() -> Result<()> {
        let snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(std::env::var(
            "YAMAMUKI_SNAPSHOT_TEST_INPUT",
        )?)?)?;
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let mut replay = TerrainView::from_snapshot(
            &egui::Context::default(),
            &app.renderer.state,
            snapshot.prepare()?,
        )?;
        replay.replan_frozen();
        draw_inner(&mut replay, false)?;
        let (inputs, camera) = replay.app.renderer.planning_test_state().unwrap();
        let start = Instant::now();
        let cpu = crate::shared_patches::plan(&inputs, &camera);
        let cpu_ms = start.elapsed().as_secs_f64() * 1000.;
        let planner =
            crate::gpu_planner::Planner::new(&app.renderer.state.device, &app.renderer.state.queue);
        let start = Instant::now();
        let gpu = planner.plan(&inputs, &camera)?;
        let first_gpu_ms = start.elapsed().as_secs_f64() * 1000.;
        let mut cpu_times = Vec::new();
        let mut gpu_times = Vec::new();
        for _ in 0..8 {
            let start = Instant::now();
            std::hint::black_box(crate::shared_patches::plan(&inputs, &camera));
            cpu_times.push(start.elapsed().as_secs_f64() * 1000.);
        }
        for _ in 0..8 {
            let start = Instant::now();
            let repeat = planner.plan(&inputs, &camera)?;
            gpu_times.push(start.elapsed().as_secs_f64() * 1000.);
            anyhow::ensure!(
                gpu.iter().zip(&repeat).all(|(a, b)| a.groups == b.groups
                    && bytemuck::cast_slice::<_, u8>(&a.patches)
                        == bytemuck::cast_slice::<_, u8>(&b.patches)),
                "GPU並列計画の反復結果が変わりました"
            );
        }
        eprintln!(
            "GPU計画の読み戻し: {} bytes/計画",
            planner.last_readback_bytes()
        );
        cpu_times.sort_by(f64::total_cmp);
        gpu_times.sort_by(f64::total_cmp);
        eprintln!(
            "計画8回の中央値: CPU={:.3}ms GPU={:.3}ms（読み戻し・座標形式変換を含む）初回GPU={first_gpu_ms:.3}ms",
            (cpu_times[3] + cpu_times[4]) * 0.5,
            (gpu_times[3] + gpu_times[4]) * 0.5
        );
        let gpu_ms = first_gpu_ms;
        let a: usize = cpu.iter().map(|p| p.patches.len()).sum();
        let b: usize = gpu.iter().map(|p| p.patches.len()).sum();
        let groups = cpu
            .iter()
            .zip(&gpu)
            .filter(|(a, b)| a.groups != b.groups)
            .count();
        let mut differing = 0;
        let mut maximum = 0f32;
        for (a, b) in cpu.iter().zip(&gpu) {
            for (a, b) in a.patches.iter().zip(&b.patches) {
                let av = bytemuck::cast_slice::<_, f32>(std::slice::from_ref(a));
                let bv = bytemuck::cast_slice::<_, f32>(std::slice::from_ref(b));
                let delta = av
                    .iter()
                    .zip(bv)
                    .map(|(a, b)| (a - b).abs())
                    .fold(0f32, f32::max);
                maximum = maximum.max(delta);
                if delta > 0.005 {
                    eprintln!("GPU計画差 CPU={av:?} GPU={bv:?}");
                }
                differing += usize::from(delta > 0.005);
            }
        }
        eprintln!(
            "GPU計画比較 inputs={} CPU={a}パッチ/{cpu_ms:.3}ms GPU={b}パッチ/{gpu_ms:.3}ms 分割数不一致={groups} 5mm以上の差={differing} 最大差={maximum}",
            inputs.len()
        );
        anyhow::ensure!(
            a == b && groups == 0 && differing == 0,
            "GPU分割計画が従来方式と一致しません"
        );
        // 同じ描画経路へGPU計画を接続し、CPU計画と追加カリングを省いた状態で比較する。
        let reference = draw(&mut replay)?;
        let started = Instant::now();
        for _ in 0..32 {
            draw_inner(&mut replay, false)?;
        }
        let cpu_draw_ms = started.elapsed().as_secs_f64() * 1000. / 32.;
        replay.app.camera.gpu_planning = true;
        let started = Instant::now();
        loop {
            draw_inner(&mut replay, false)?;
            let (_, error) = replay.app.renderer.planning_status();
            anyhow::ensure!(error.is_none(), "GPU計画の統合に失敗: {error:?}");
            if replay.app.renderer.gpu_plan_ready() {
                break;
            }
            anyhow::ensure!(
                started.elapsed().as_secs() < 30,
                "GPU分割計画が時間内に終わりません"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let integrated_ms = started.elapsed().as_secs_f64() * 1000.;
        let pixels = draw(&mut replay)?;
        let before = bytemuck::cast_slice::<u8, f32>(&reference);
        let after = bytemuck::cast_slice::<u8, f32>(&pixels);
        let mut holes = 0;
        let mut positions = 0;
        let mut heights = 0;
        let mut shades = 0;
        for (a, b) in before.chunks_exact(4).zip(after.chunks_exact(4)) {
            holes += usize::from(a[3] != 0. && b[3] == 0.);
            if a[3] != 0. && b[3] != 0. {
                positions += usize::from((a[0] - b[0]).abs() > 0.05 || (a[1] - b[1]).abs() > 0.05);
                heights += usize::from((a[3].abs() - b[3].abs()).abs() > 0.05);
                shades += usize::from((a[2] - b[2]).abs() > 0.001);
            }
        }
        let started = Instant::now();
        for _ in 0..32 {
            draw_inner(&mut replay, false)?;
        }
        eprintln!(
            "GPU計画の統合比較: 描画 CPU={cpu_draw_ms:.3}ms GPU={:.3}ms 切替待ち={integrated_ms:.3}ms 欠落={holes} XY差>5cm={positions} 高さ差>5cm={heights} 陰影差>0.001={shades}",
            started.elapsed().as_secs_f64() * 1000. / 32.
        );
        anyhow::ensure!(
            holes == 0 && positions == 0 && heights == 0 && shades == 0,
            "GPU分割計画で描画結果が変わりました"
        );
        Ok(())
    }
    #[test]
    #[ignore = "実スナップショットでGPUカリングの位置一致と速度を比較する"]
    fn benchmark_external_gpu_culling() -> Result<()> {
        let snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(std::env::var(
            "YAMAMUKI_SNAPSHOT_TEST_INPUT",
        )?)?)?;
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let mut replay = TerrainView::from_snapshot(
            &egui::Context::default(),
            &app.renderer.state,
            snapshot.prepare()?,
        )?;
        let mut reference = Vec::new();
        for enabled in [false, true] {
            replay.app.camera.gpu_culling = enabled;
            for _ in 0..3 {
                draw_inner(&mut replay, false)?;
            }
            let pixels = draw(&mut replay)?;
            if !enabled {
                reference = pixels;
            } else {
                let before = bytemuck::cast_slice::<u8, f32>(&reference);
                let after = bytemuck::cast_slice::<u8, f32>(&pixels);
                let mut holes = 0;
                let mut changed = 0;
                let mut max_height = 0f32;
                let mut positions = 0;
                let mut shades = 0;
                for (a, b) in before.chunks_exact(4).zip(after.chunks_exact(4)) {
                    if a[3] != 0. && b[3] == 0. {
                        holes += 1;
                    }
                    if a[3] != 0. && b[3] != 0. {
                        let delta = (a[3].abs() - b[3].abs()).abs();
                        max_height = max_height.max(delta);
                        changed += usize::from(delta > 0.05);
                        positions +=
                            usize::from((a[0] - b[0]).abs() > 0.05 || (a[1] - b[1]).abs() > 0.05);
                        shades += usize::from((a[2] - b[2]).abs() > 0.001);
                    }
                }
                eprintln!(
                    "GPU選択の描画比較: 欠落={holes} 高さ差>5cm={changed} 最大差={max_height:.6}m XY差>5cm={positions} 陰影差>0.001={shades}"
                );
                anyhow::ensure!(
                    holes == 0 && changed == 0 && positions == 0 && shades == 0,
                    "GPUカリングで描画結果が変わりました"
                );
            }
            if enabled {
                let arguments = replay.app.renderer.indirect_arguments();
                let device = &replay.app.renderer.state.device;
                let read = device.create_buffer(&eframe::wgpu::BufferDescriptor {
                    label: Some("計測用の間接描画命令"),
                    size: (arguments.len().max(1) * 20) as u64,
                    usage: eframe::wgpu::BufferUsages::COPY_DST
                        | eframe::wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                for (i, (_, source)) in arguments.iter().enumerate() {
                    encoder.copy_buffer_to_buffer(source, 0, &read, (i * 20) as u64, 20);
                }
                replay.app.renderer.state.queue.submit([encoder.finish()]);
                let (tx, rx) = std::sync::mpsc::channel();
                read.slice(..)
                    .map_async(eframe::wgpu::MapMode::Read, move |result| {
                        tx.send(result).unwrap()
                    });
                let _ = device.poll(eframe::wgpu::Maintain::Wait);
                rx.recv()??;
                let bytes = read.slice(..).get_mapped_range();
                let values = bytemuck::cast_slice::<u8, u32>(&bytes);
                let visible: u64 = values.chunks_exact(5).map(|v| u64::from(v[1])).sum();
                eprintln!(
                    "GPU可視パッチ={visible} / CPU計画パッチ={}",
                    replay.app.renderer.metrics().1
                );
            }
            let start = Instant::now();
            for _ in 0..32 {
                draw_inner(&mut replay, false)?;
            }
            eprintln!(
                "GPUカリング={enabled} 平均={:.3}ms",
                start.elapsed().as_secs_f64() * 1000. / 32.
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "実スナップショットに地図を重ねてGPU描画時間を測る"]
    fn benchmark_external_online_material() -> Result<()> {
        use crate::direct_material::{Interpolation, Quad};
        let snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(std::env::var(
            "YAMAMUKI_SNAPSHOT_TEST_INPUT",
        )?)?)?;
        let app =
            crate::remembered_scene_tests::app_with_backend(eframe::wgpu::Backends::DX12).unwrap();
        let mut replay = TerrainView::from_snapshot(
            &egui::Context::default(),
            &app.renderer.state,
            snapshot.prepare()?,
        )?;
        let positions = draw(&mut replay)?;
        let camera = replay.app.camera.clone();
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for pixel in positions.chunks_exact(16) {
            let q: [f32; 4] = std::array::from_fn(|i| {
                f32::from_le_bytes(pixel[i * 4..i * 4 + 4].try_into().unwrap())
            });
            if q[3] <= 0. {
                continue;
            }
            for axis in 0..2 {
                let v = (camera.center[axis] + f64::from(q[axis])) / crate::shared_data::WORLD;
                bounds[axis] = bounds[axis].min(v);
                bounds[axis + 2] = bounds[axis + 2].max(v);
            }
        }
        let mut decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(
            std::env::var("YAMAMUKI_MAP_TEST_PNG")?,
        )?));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info()?;
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes)?;
        let channels = info.color_type.samples();
        let rgba = bytes[..info.buffer_size()]
            .chunks_exact(channels)
            .flat_map(|p| match channels {
                4 => [p[0], p[1], p[2], p[3]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[0], p[0], 255],
            })
            .collect::<Vec<_>>();
        let state = replay.app.renderer.state.clone();
        let texture = state
            .device
            .create_texture(&eframe::wgpu::TextureDescriptor {
                label: Some("計測用の保存済み地図"),
                size: eframe::wgpu::Extent3d {
                    width: info.width,
                    height: info.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: eframe::wgpu::TextureDimension::D2,
                format: eframe::wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: eframe::wgpu::TextureUsages::TEXTURE_BINDING
                    | eframe::wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
        state.queue.write_texture(
            texture.as_image_copy(),
            &rgba,
            eframe::wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(info.width * 4),
                rows_per_image: Some(info.height),
            },
            texture.size(),
        );
        let view = texture.create_view(&Default::default());
        let max_zoom = std::env::var("YAMAMUKI_MAP_TEST_ZOOM")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(18);
        let side = (1u32 << max_zoom) as f64;
        let x0 = (bounds[0] * side).floor() as u32;
        let x1 = (bounds[2] * side).floor() as u32;
        let y0 = (bounds[1] * side).floor() as u32;
        let y1 = (bounds[3] * side).floor() as u32;
        let full_regions = std::env::var_os("YAMAMUKI_FULL_MATERIAL_REGIONS").is_some();
        anyhow::ensure!(
            full_regions || (x1 - x0 + 1) * (y1 - y0 + 1) <= 4096,
            "計測範囲が広すぎる: {bounds:?}"
        );
        let mut quads = Vec::new();
        if !full_regions {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    quads.push(Quad {
                        bounds: [
                            x as f64 / side,
                            y as f64 / side,
                            (x + 1) as f64 / side,
                            (y + 1) as f64 / side,
                        ],
                        uv: [egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)); 2],
                        textures: [Some(view.clone()), Some(view.clone())],
                        amount: 1.,
                        tint: [1.; 4],
                        shade: false,
                        gamma_shade: false,
                        opaque: rgba.chunks_exact(4).all(|p| p[3] == 255),
                        surface: None,
                        edge_filter: Interpolation::Bilinear,
                        border: false,
                    });
                }
            }
        }
        if full_regions {
            quads.clear();
            let rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(camera.viewport[0] as f32, camera.viewport[1] as f32),
            );
            let width = camera.viewport[0].max(camera.viewport[1]);
            let areas = replay.projected_material_regions(
                rect,
                width / (256. * (1u32 << max_zoom) as f64) / 1.02,
            );
            let screen_zoom = std::env::var("YAMAMUKI_MAP_SCREEN_ZOOM")
                .ok()
                .and_then(|v| v.parse::<f64>().ok());
            for i in std::iter::once(0).chain((1..areas.len()).rev()) {
                let (area, zoom) = areas[i];
                let zoom = screen_zoom.map_or(zoom, |screen| screen.min(zoom));
                let z = (zoom.floor() as u32).clamp(2, max_zoom);
                let side = (1u32 << z) as f64;
                let half = (area[2] - area[0]).max(area[3] - area[1]) * 1.02 / 2.;
                let cx = (area[0] + area[2]) / 2.;
                let cy = (area[1] + area[3]) / 2.;
                for y in ((cy - half) * side).floor() as u32..=((cy + half) * side).floor() as u32 {
                    for x in
                        ((cx - half) * side).floor() as u32..=((cx + half) * side).floor() as u32
                    {
                        quads.push(Quad {
                            bounds: [
                                x as f64 / side,
                                y as f64 / side,
                                (x + 1) as f64 / side,
                                (y + 1) as f64 / side,
                            ],
                            uv: [egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)); 2],
                            textures: [Some(view.clone()), Some(view.clone())],
                            amount: 1.,
                            tint: [1.; 4],
                            shade: false,
                            gamma_shade: false,
                            opaque: rgba.chunks_exact(4).all(|p| p[3] == 255),
                            surface: None,
                            edge_filter: Interpolation::Bilinear,
                            border: false,
                        });
                    }
                }
            }
        }
        if let Some(dir) = std::env::var_os("YAMAMUKI_SURFACE_TEST_DIR") {
            let dir = PathBuf::from(dir);
            let uv = egui::Rect::from_min_max(
                egui::pos2(3. / info.width as f32, 3. / info.height as f32),
                egui::pos2(1. - 3. / info.width as f32, 1. - 3. / info.height as f32),
            );
            for quad in &mut quads {
                quad.uv = [uv; 2];
            }
            let template = quads.clone();
            for (name, is_shade) in [("hillshade", true), ("water", false)] {
                let mut decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(
                    dir.join(format!("{name}.png")),
                )?));
                decoder.set_transformations(
                    png::Transformations::EXPAND | png::Transformations::STRIP_16,
                );
                let mut reader = decoder.read_info()?;
                let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
                let info = reader.next_frame(&mut bytes)?;
                let texture = state
                    .device
                    .create_texture(&eframe::wgpu::TextureDescriptor {
                        label: Some(name),
                        size: eframe::wgpu::Extent3d {
                            width: info.width,
                            height: info.height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: eframe::wgpu::TextureDimension::D2,
                        format: if is_shade {
                            eframe::wgpu::TextureFormat::Rgba8Unorm
                        } else {
                            eframe::wgpu::TextureFormat::Rgba8UnormSrgb
                        },
                        usage: eframe::wgpu::TextureUsages::TEXTURE_BINDING
                            | eframe::wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                state.queue.write_texture(
                    texture.as_image_copy(),
                    &bytes[..info.buffer_size()],
                    eframe::wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(info.width * 4),
                        rows_per_image: Some(info.height),
                    },
                    texture.size(),
                );
                let view = texture.create_view(&Default::default());
                quads.extend(template.iter().cloned().map(|mut quad| {
                    quad.textures = [Some(view.clone()), Some(view.clone())];
                    quad.shade = is_shade;
                    quad.gamma_shade = is_shade;
                    quad
                }));
            }
        }
        eprintln!(
            "地図範囲 {bounds:?} / {}タイル / 画面 {:?}",
            quads.len(),
            camera.viewport
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        while replay.app.renderer.interpolation_pending() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        anyhow::ensure!(
            !replay.app.renderer.interpolation_pending(),
            "補間準備が終わらない"
        );
        anyhow::ensure!(
            replay.app.renderer.interpolation_error().is_none(),
            "補間準備に失敗"
        );
        replay.app.renderer.direct = Some(Vec::new());
        draw_inner(&mut replay, false)?;
        let baseline = std::time::Instant::now();
        for _ in 0..8 {
            draw_inner(&mut replay, false)?;
        }
        eprintln!(
            "地形のみ: 平均 {:.2}ms",
            baseline.elapsed().as_secs_f64() * 1000. / 8.
        );
        if std::env::var_os("YAMAMUKI_BENCH_CAMERA_MOTION").is_some() {
            replay.replan_frozen();
            for quad in &mut quads {
                quad.edge_filter = Interpolation::Bilinear;
            }
            replay.app.renderer.direct = Some(quads.clone());
            let mut times = Vec::new();
            for i in 0..36 {
                let eye = camera.eye();
                replay.app.camera.set_observer(
                    [eye[0], eye[1]],
                    eye[2],
                    camera.yaw + (i as f64 * 0.2).sin() * 0.12,
                    camera.pitch,
                    camera.fov_y,
                    camera.viewport,
                );
                let started = std::time::Instant::now();
                draw_inner(&mut replay, false)?;
                times.push(started.elapsed().as_secs_f64() * 1000.);
                if i > 0 {
                    let stats = replay.app.renderer.prepare_stats();
                    anyhow::ensure!(
                        stats.rebuild_ms == 0.
                            && stats.plan_ms == 0.
                            && stats.atlas_copy_bytes == 0,
                        "回転だけで共有シーンを再構築した"
                    );
                }
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "連続左右回転: 平均 {:.2}ms / p95 {:.2}ms / 最大 {:.2}ms",
                times.iter().sum::<f64>() / times.len() as f64,
                times[times.len() * 95 / 100],
                times[times.len() - 1]
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                draw_inner(&mut replay, false)?;
                if let Some(matches) = replay.app.renderer.current_plan_matches_reference() {
                    anyhow::ensure!(matches, "非同期計画が同じ視点の同期計画と一致しない");
                    eprintln!("停止後の最終計画: 同期方式と全パッチ・分割数が完全一致");
                    break;
                }
                anyhow::ensure!(
                    std::time::Instant::now() < deadline,
                    "停止後に最新の分割計画へ到達しない"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            return Ok(());
        }
        for mode in [
            Interpolation::Bilinear,
            Interpolation::Bicubic,
            Interpolation::BSpline,
            Interpolation::Edge,
            Interpolation::Nearest,
        ] {
            for quad in &mut quads {
                quad.edge_filter = mode;
            }
            replay.app.renderer.direct = Some(quads.clone());
            draw_inner(&mut replay, false)?;
            let started = std::time::Instant::now();
            for _ in 0..8 {
                draw_inner(&mut replay, false)?;
            }
            eprintln!(
                "地図補間 {mode:?}: 平均 {:.2}ms",
                started.elapsed().as_secs_f64() * 1000. / 8.
            );
            if let Some(dir) = std::env::var_os("YAMAMUKI_MATERIAL_OUTPUT_DIR") {
                let dir = PathBuf::from(dir);
                std::fs::create_dir_all(&dir)?;
                let texture = &replay.app.renderer.color;
                let size = texture.size();
                let row = (size.width * 4).div_ceil(256) * 256;
                let buffer = state.device.create_buffer(&eframe::wgpu::BufferDescriptor {
                    label: Some("画素比較用"),
                    size: u64::from(row) * u64::from(size.height),
                    usage: eframe::wgpu::BufferUsages::COPY_DST
                        | eframe::wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = state.device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    texture.as_image_copy(),
                    eframe::wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: eframe::wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(row),
                            rows_per_image: Some(size.height),
                        },
                    },
                    size,
                );
                state.queue.submit([encoder.finish()]);
                let (tx, rx) = std::sync::mpsc::channel();
                buffer
                    .slice(..)
                    .map_async(eframe::wgpu::MapMode::Read, move |r| {
                        let _ = tx.send(r);
                    });
                let _ = state.device.poll(eframe::wgpu::Maintain::Wait);
                rx.recv()??;
                let mapped = buffer.slice(..).get_mapped_range();
                let rgba = mapped
                    .chunks_exact(row as usize)
                    .flat_map(|r| r[..(size.width * 4) as usize].iter().copied())
                    .collect::<Vec<_>>();
                std::fs::write(dir.join(format!("{mode:?}.rgba")), rgba)?;
                std::fs::write(
                    dir.join("size.json"),
                    serde_json::to_vec(&[size.width, size.height])?,
                )?;
            }
        }
        Ok(())
    }

    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
    fn sea_level_plane_covers_the_screen_at_every_camera_zoom() -> Result<()> {
        let backend = if cfg!(windows) {
            eframe::wgpu::Backends::DX12
        } else {
            eframe::wgpu::Backends::PRIMARY
        };
        let Some(app) = crate::remembered_scene_tests::app_with_backend(backend) else {
            return Ok(());
        };
        let mut flat = TerrainView::flat(&egui::Context::default(), &app.renderer.state)?;
        flat.app.camera.globe = false;
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256., 128.));
        let center = crate::selection::lonlat(138.7274, 35.3606).map(|v| v / shared_data::WORLD);
        for zoom in [6., 12., 18., 22.] {
            for tilt in [0., 1.] {
                flat.set_map_camera(center, zoom, 0.7, tilt, rect);
                if tilt == 0. {
                    let regions = flat.projected_material_regions(rect, 0.001);
                    anyhow::ensure!(
                        regions.len() == 1,
                        "透明な地表レイヤーを同じ範囲へ二重に描いている: {regions:?}"
                    );
                    anyhow::ensure!(
                        (regions[0].1 - zoom).abs() < 1e-8,
                        "地表LODが視点倍率と一致しない"
                    );
                }
                let bytes = draw(&mut flat)?;
                let positions = bytemuck::cast_slice::<u8, f32>(&bytes);
                let holes = positions.chunks_exact(4).filter(|p| p[3] == 0.).count();
                anyhow::ensure!(holes == 0, "平面に穴: z{zoom}, tilt={tilt}, holes={holes}");
                anyhow::ensure!(
                    !flat.app.queued && flat.app.settled && !flat.app.scene_pending,
                    "平面が地形生成を待機している"
                );
            }
        }
        let actual = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1233.1875, 1306.));
        flat.set_map_camera(
            [0.8915569358014047, 0.3926204566666444],
            6.370769965648648,
            0.6374623_f64.atan2(0.7704815),
            0.76122254_f64.acos(),
            actual,
        );
        let regions = flat.material_regions(actual, 0.0001);
        anyhow::ensure!(
            regions[0] == regions[2],
            "平面の画像範囲が視野外へ広がっている"
        );
        let bytes = draw(&mut flat)?;
        let holes = bytemuck::cast_slice::<u8, f32>(&bytes)
            .chunks_exact(4)
            .filter(|p| p[3] == 0.)
            .count();
        anyhow::ensure!(holes == 0, "提供スナップショットの平面に穴: {holes}");
        let positions = bytemuck::cast_slice::<u8, f32>(&bytes);
        for (x, y) in [(100, 100), (600, 600), (1000, 1000)] {
            let q = &positions[((y * 1233 + x) * 4)..((y * 1233 + x) * 4 + 4)];
            let world = [
                q[0] as f64 / shared_data::WORLD + flat.app.camera.center[0] / shared_data::WORLD,
                q[1] as f64 / shared_data::WORLD + flat.app.camera.center[1] / shared_data::WORLD,
            ];
            let screen = flat.projection().screen(actual, world, 0.).unwrap();
            anyhow::ensure!(
                screen.distance(egui::pos2(x as f32 + 0.5, y as f32 + 0.5)) < 2.,
                "GPU地理座標が投影と不一致: {x},{y} → {screen:?}, q={q:?}"
            );
        }

        flat.set_observer_camera(center, 2., 1., 0., 45_f64.to_radians(), rect);
        flat.freeze_current();
        flat.unfreeze();
        anyhow::ensure!(
            !flat.app.queued && !flat.app.scene_pending && flat.app.shown.len() == 1,
            "平面の景色・固定解除が地形読み込みを開始している"
        );
        Ok(())
    }
    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
    fn globe_gpu_matches_cpu_picking_space_and_close_up() -> Result<()> {
        let backend = if cfg!(windows) {
            eframe::wgpu::Backends::DX12
        } else {
            eframe::wgpu::Backends::PRIMARY
        };
        let Some(app) = crate::remembered_scene_tests::app_with_backend(backend) else {
            return Ok(());
        };
        let mut globe = TerrainView::flat(&egui::Context::default(), &app.renderer.state)?;
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(384., 256.));
        let center = crate::selection::lonlat(138.7274, 35.3606).map(|v| v / shared_data::WORLD);
        for zoom in [1., 5., 12., 22.] {
            for tilt in [0., 0.5, 1.2] {
                globe.set_map_camera(center, zoom, 0.7, tilt, rect);
                let bytes = draw(&mut globe)?;
                let p = bytemuck::cast_slice::<u8, f32>(&bytes);
                for y in (4..256).step_by(16) {
                    for x in (4..384).step_by(16) {
                        let pixel = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                        let hit = globe.projection().world_at(rect, pixel);
                        let q = &p[(y * 384 + x) * 4..(y * 384 + x) * 4 + 4];
                        anyhow::ensure!(
                            hit.is_some() == (q[3] != 0.),
                            "球面と宇宙の判定不一致 z{zoom} tilt{tilt} pixel={pixel:?}"
                        );
                        if hit.is_some() {
                            let w = [
                                q[0] as f64 / shared_data::WORLD + center[0],
                                q[1] as f64 / shared_data::WORLD + center[1],
                            ];
                            let screen = globe.projection().screen(rect, w, 0.).unwrap();
                            anyhow::ensure!(
                                screen.distance(pixel) < 2.,
                                "球面座標ずれ z{zoom} tilt{tilt}: {pixel:?} → {screen:?}, q={q:?}, expected={hit:?}"
                            );
                        }
                    }
                }
            }
        }
        for tilt in [0., 0.1, 0.3, 0.5] {
            globe.set_map_camera(center, 15.888889, 0., tilt, rect);
            let regions = globe.projected_material_regions(rect, 0.0001);
            anyhow::ensure!(
                (regions.last().unwrap().0[2] - regions.last().unwrap().0[0]) * shared_data::WORLD
                    < 20000.,
                "近景の画像取得範囲が地球の裏側へ広がっている"
            );
            anyhow::ensure!(
                (regions.last().unwrap().1 - 15.888889).abs() < 1e-6 && regions[0].1 <= 15.888889,
                "緩い傾きで中央以外のLODが低下している"
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "実際の表示スナップショットをGPU描画して位置バッファを保存する"]
    fn reproduce_external_snapshot() -> Result<()> {
        let path = std::env::var("YAMAMUKI_SNAPSHOT_TEST_INPUT")?;
        let out = PathBuf::from(std::env::var("YAMAMUKI_SNAPSHOT_TEST_OUTPUT")?);
        let mut snapshot: Snapshot = serde_json::from_reader(std::fs::File::open(path)?)?;
        if std::env::var_os("YAMAMUKI_SNAPSHOT_REPLAN").is_some() {
            if let Some(s) = &mut snapshot.layout {
                s.force_replan = true;
                s.clipped_background = true;
            }
        }
        let backend = if std::env::var("YAMAMUKI_SNAPSHOT_BACKEND").as_deref() == Ok("dx12") {
            eframe::wgpu::Backends::DX12
        } else {
            eframe::wgpu::Backends::all()
        };
        let Some(app) = crate::remembered_scene_tests::app_with_backend(backend) else {
            anyhow::bail!("GPUが必要です");
        };
        let prepared = snapshot.prepare()?;
        let mut replay =
            TerrainView::from_snapshot(&egui::Context::default(), &app.renderer.state, prepared)?;
        if std::env::var_os("YAMAMUKI_SNAPSHOT_RELOAD").is_some() {
            let mut db = Database::open(std::path::Path::new(&replay.app.path))?;
            replay.app.wanted = selection::select(&db.catalog, &replay.app.camera);
            replay.app.catalog = db.catalog.clone();
            for key in replay.app.wanted.clone() {
                replay.app.packets.insert(key, db.packet(key)?);
            }
            replay.app.remembered_until_selection = None;
            let started = std::time::Instant::now();
            let plan = replay.app.preparation_plan();
            let cold = started.elapsed();
            let cached = std::time::Instant::now();
            for _ in 0..100 {
                std::hint::black_box(replay.app.preparation_plan());
            }
            eprintln!(
                "準備計画: {}区画 初回={:.3}ms 再利用平均={:.3}ms",
                plan.len(),
                cold.as_secs_f64() * 1000.,
                cached.elapsed().as_secs_f64() * 10.
            );
            let mut done = false;
            for i in 0..1000 {
                done = replay.app.scene()?;
                let _ = replay
                    .app
                    .renderer
                    .state
                    .device
                    .poll(eframe::wgpu::Maintain::Wait);
                if i % 25 == 0 || done {
                    eprintln!(
                        "再準備 フレーム={i} 完了={done} 全体メッシュ={} 局所メッシュ={} 元タイル={}",
                        replay.app.gpu.len(),
                        replay.app.local_gpu.len(),
                        replay.app.packets.len()
                    );
                }
                if done {
                    break;
                }
            }
            anyhow::ensure!(done, "GPU曲面の準備が完了しません");
            replay
                .app
                .renderer
                .frozen_layout
                .as_mut()
                .unwrap()
                .force_replan = true;
            replay.app.previous.clear();
            replay.app.frozen_morph = Some(1.);
        }
        let layout = replay.app.renderer.frozen_layout.as_ref().unwrap();
        let [width, height] = if layout.render_size.iter().all(|v| *v > 0) {
            layout.render_size
        } else {
            [
                (replay.app.camera.viewport[0] as f32 * layout.pixels_per_point).round() as u32,
                (replay.app.camera.viewport[1] as f32 * layout.pixels_per_point).round() as u32,
            ]
        };
        let bytes = draw(&mut replay)?;
        eprintln!("再現描画量: {:?}", replay.app.renderer.metrics());
        if std::env::var_os("YAMAMUKI_SNAPSHOT_REQUIRE_NO_BORDERS").is_some() {
            let marked = bytemuck::cast_slice::<u8, f32>(&bytes)
                .chunks_exact(4)
                .filter(|p| p[3] < 0.)
                .count();
            anyhow::ensure!(marked == 0, "タイル内部に境界表示が{marked}画素あります");
        }

        if std::env::var_os("YAMAMUKI_SNAPSHOT_REQUIRE_CLOSED").is_some() {
            let pixels = bytemuck::cast_slice::<u8, f32>(&bytes);
            let holes = pixels
                .chunks_exact(4)
                .skip(width as usize * 8)
                .filter(|p| p[3] == 0.)
                .count();
            anyhow::ensure!(
                holes == 0,
                "再現画像の地形内に{holes}画素の穴が残っています"
            );
        }

        std::fs::write(out.with_extension("positions.bin"), &bytes)?;
        let mut image = Vec::new();
        for p in bytemuck::cast_slice::<u8, f32>(&bytes).chunks_exact(4) {
            if p[3] == 0. {
                image.extend([235, 244, 252, 255]);
            } else {
                let h = ((p[3].abs() - 10000.) / 4000.).clamp(0., 1.);
                let shade = p[2].clamp(0., 1.);
                image.extend(
                    [
                        (70. + 150. * h) * shade,
                        (130. + 50. * h) * shade,
                        (60. + 80. * h) * shade,
                        255.,
                    ]
                    .map(|v| v as u8),
                );
            }
        }
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(&out)?),
            width,
            height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&image)?;
        eprintln!("スナップショット再現: {width}x{height}, {}", out.display());
        Ok(())
    }
    #[test]
    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "実機GPUでの描画検査。--features gpu-tests -- --test-threads=1で実行する"
    )]
    fn snapshot_replays_the_same_gpu_positions_and_keeps_transition_fixed() -> Result<()> {
        let Some(mut app) = crate::remembered_scene_tests::app() else {
            return Ok(());
        };
        let path = std::env::temp_dir().join(format!(
            "terrain-snapshot-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let mut conn = yamamuki_data_sqlite::rusqlite::Connection::open(&path)?;
        yamamuki_data_sqlite::initialize(&conn)?;
        yamamuki_terrain::fixture::create(&mut conn)?;
        drop(conn);
        let mut db = Database::open(&path)?;
        let key = *db.catalog.keys().max_by_key(|k| k.0).unwrap();
        let packet = db.packet(key)?;
        let fresh = app.renderer.upload(&packet, 16)?;
        let old_key = (key.0 - 1, key.1 >> 1, key.2 >> 1);
        let old_packet = db.packet(old_key)?;
        let old = app.renderer.upload(&old_packet, 8)?;
        app.camera.center = [
            packet.origin[0] + packet.side / 2.,
            packet.origin[1] + packet.side / 2.,
        ];
        app.camera.distance = 10000.;
        app.camera.height = 500.;
        app.camera.viewport = [64., 64.];
        app.shown = vec![old.clone()];
        app.frozen_morph = Some(1.);
        app.path = path.to_string_lossy().into();
        app.generation = db.generation.clone();
        let mut source = TerrainView {
            label: "試験".into(),
            app,
            automatic_maximum: None,
        };
        draw(&mut source)?;
        source.app.shown = vec![fresh];
        source.app.previous = vec![old];
        source.app.frozen_morph = Some(0.37);
        source.app.settings[2] = 1.;
        let before = draw(&mut source)?;
        assert!(
            bytemuck::cast_slice::<u8, f32>(&before)
                .chunks_exact(4)
                .any(|p| p[3] != 0.),
            "地形を描画できていない"
        );
        let captured = source.snapshot()?;
        let diagnostics = serde_json::to_value(captured.diagnostics.as_ref().unwrap())?;
        assert_eq!(diagnostics["display"]["meshes"][0]["n"], 16);
        assert_eq!(diagnostics["display"]["morph"], serde_json::json!(0.37f32));
        assert!(
            diagnostics["display"]["meshes"][0]["patch_count"]
                .as_u64()
                .unwrap()
                > 0
        );
        let mut legacy = serde_json::to_value(&captured)?;
        legacy.as_object_mut().unwrap().remove("diagnostics");
        assert!(
            serde_json::from_value::<Snapshot>(legacy)?
                .diagnostics
                .is_none()
        );
        assert_eq!(captured.settings[2], 1., "標高着色モードを保存できていない");
        assert_eq!(captured.morph, 0.37);
        assert_eq!(captured.counts, [1, 0, 0, 1]);
        assert!(
            captured.layout.as_ref().unwrap().previous[0].is_some(),
            "旧表示の三角形を保存できていない"
        );
        let bytes = serde_json::to_vec(&captured)?;
        let decoded: Snapshot = serde_json::from_slice(&bytes)?;
        let prepared = decoded.prepare()?;
        let state = source.app.renderer.state.clone();
        let mut replay = TerrainView::from_snapshot(&egui::Context::default(), &state, prepared)?;
        let after = draw(&mut replay)?;
        assert_eq!(before, after, "保存前後のGPU位置・陰影・高さが違う");
        assert_eq!(replay.app.frozen_morph, Some(0.37));
        replay.app.transition = Instant::now() - Duration::from_secs(30);
        assert_eq!(after, draw(&mut replay)?, "時間経過で固定表示が変わった");
        replay.unfreeze();
        assert!(replay.app.frozen_morph.is_none());
        drop(db);
        drop(source);
        drop(replay);
        let _ = std::fs::remove_file(path);
        Ok(())
    }
}
