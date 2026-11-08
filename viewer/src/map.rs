use anyhow::{Context, Result, ensure};
use eframe::egui::{self, ColorImage, Pos2, Rect, TextureHandle};
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

pub const MIN_ZOOM: f64 = 5.;
pub const MAX_ZOOM: f64 = 18.;
pub const TILE_SIZE: usize = 256;
const MAX_PENDING: usize = 6;
const MAX_CACHE: usize = 160;
const ATTRIBUTION: &str = "https://maps.gsi.go.jp/development/ichiran.html";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tile {
    z: u32,
    x: u32,
    y: u32,
}

impl Tile {
    fn url(self) -> String {
        format!(
            "https://cyberjapandata.gsi.go.jp/xyz/std/{}/{}/{}.png",
            self.z, self.x, self.y
        )
    }
}

pub fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("yamamuki-viewer/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .build()?)
}

fn fetch(client: &reqwest::blocking::Client, tile: Tile) -> Result<Option<ColorImage>> {
    let response = client.get(tile.url()).send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    response
        .error_for_status()?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "地図タイルが大きすぎます");
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    ensure!(
        info.width as usize == TILE_SIZE && info.height as usize == TILE_SIZE,
        "地図タイルの寸法が異なります"
    );
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .context("地図タイルの容量が不正です")?
    ];
    let frame = reader.next_frame(&mut pixels)?;
    let size = [TILE_SIZE; 2];
    let image = match frame.color_type {
        png::ColorType::Rgb => ColorImage::from_rgb(size, &pixels),
        png::ColorType::Rgba => ColorImage::from_rgba_unmultiplied(size, &pixels),
        png::ColorType::Grayscale => ColorImage::from_gray(size, &pixels),
        png::ColorType::GrayscaleAlpha => ColorImage::from_rgba_unmultiplied(
            size,
            &pixels
                .chunks_exact(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect::<Vec<_>>(),
        ),
        png::ColorType::Indexed => anyhow::bail!("地図タイルの色形式に対応していません"),
    };
    Ok(Some(image))
}

pub fn check() -> Result<()> {
    let tile = Tile {
        z: 8,
        x: 226,
        y: 101,
    };
    let image = fetch(&client()?, tile)?.context("確認用の地理院タイルが見つかりません")?;
    println!(
        "地理院タイル {}×{} を取得しました",
        image.size[0], image.size[1]
    );
    Ok(())
}

pub struct View {
    pub center: [f64; 2],
    pub zoom: f64,
}

impl View {
    pub fn scale(&self) -> f64 {
        TILE_SIZE as f64 * self.zoom.exp2()
    }

    pub fn world_at(&self, rect: Rect, pos: Pos2) -> [f64; 2] {
        let d = pos - rect.center();
        [
            self.center[0] + f64::from(d.x) / self.scale(),
            self.center[1] + f64::from(d.y) / self.scale(),
        ]
    }

    pub fn screen(&self, rect: Rect, [x, y]: [f64; 2]) -> Pos2 {
        rect.center()
            + egui::vec2(
                ((x - self.center[0]) * self.scale()) as f32,
                ((y - self.center[1]) * self.scale()) as f32,
            )
    }

    pub fn area(&self, rect: Rect) -> [[f64; 2]; 2] {
        [self.world_at(rect, rect.min), self.world_at(rect, rect.max)]
    }

    pub fn zoom_at(&mut self, rect: Rect, pointer: Pos2, delta: f64) {
        let anchor = self.world_at(rect, pointer);
        self.zoom = (self.zoom + delta).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.world_at(rect, pointer);
        for axis in 0..2 {
            self.center[axis] = (self.center[axis] + anchor[axis] - after[axis]).clamp(0., 1.);
        }
    }

    pub fn fit(&mut self, rect: Rect, [min, max]: [[f64; 2]; 2]) {
        self.center = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
        let dx = (max[0] - min[0]).max(1e-8);
        let dy = (max[1] - min[1]).max(1e-8);
        let scale = (f64::from(rect.width()) / dx).min(f64::from(rect.height()) / dy) * 0.8;
        self.zoom = (scale / TILE_SIZE as f64).log2().clamp(MIN_ZOOM, 14.);
    }

    fn tiles(&self, rect: Rect) -> Vec<Tile> {
        let z = self.zoom.floor() as u32;
        let side = 1u32 << z;
        let index = |x: f64| (x * side as f64).floor().clamp(0., (side - 1) as f64) as u32;
        let [min, max] = self.area(rect);
        let mut tiles = Vec::new();
        for y in index(min[1])..=index(max[1]) {
            for x in index(min[0])..=index(max[0]) {
                tiles.push(Tile { z, x, y });
            }
        }
        tiles.sort_by(|a, b| {
            let d = |tile: &Tile| {
                ((tile.x as f64 + 0.5) / side as f64 - self.center[0]).powi(2)
                    + ((tile.y as f64 + 0.5) / side as f64 - self.center[1]).powi(2)
            };
            d(a).total_cmp(&d(b))
        });
        tiles
    }

    fn tile_rect(&self, rect: Rect, tile: Tile) -> Rect {
        let side = (1u32 << tile.z) as f64;
        Rect::from_min_max(
            self.screen(rect, [tile.x as f64 / side, tile.y as f64 / side]),
            self.screen(
                rect,
                [(tile.x + 1) as f64 / side, (tile.y + 1) as f64 / side],
            ),
        )
    }
}

enum Cached {
    Texture(TextureHandle),
    Missing,
    Failed(Instant),
}

type Reply = (Tile, Result<Option<ColorImage>>);

pub struct Tiles {
    queue: mpsc::Sender<Tile>,
    replies: mpsc::Receiver<Reply>,
    pending: HashSet<Tile>,
    cache: HashMap<Tile, (Cached, u64)>,
    frame: u64,
}

impl Tiles {
    pub fn new(ctx: &egui::Context, client: reqwest::blocking::Client) -> Self {
        let (queue, jobs) = mpsc::channel();
        let (send, replies) = mpsc::channel();
        let jobs = Arc::new(Mutex::new(jobs));
        for _ in 0..4 {
            let (jobs, send, ctx, client) =
                (jobs.clone(), send.clone(), ctx.clone(), client.clone());
            std::thread::spawn(move || {
                loop {
                    let job = jobs.lock().expect("tile queue").recv();
                    let Ok(tile) = job else { break };
                    if send.send((tile, fetch(&client, tile))).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Self {
            queue,
            replies,
            pending: HashSet::new(),
            cache: HashMap::new(),
            frame: 0,
        }
    }

    pub fn paint(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        view: &View,
        dragging: bool,
    ) -> bool {
        self.frame += 1;
        let limit = if dragging { 1 } else { MAX_PENDING };
        let mut processed = 0;
        // 取得とPNGの展開はワーカーで行う。ドラッグ中はGPUへ渡すタイル数を抑える。
        for (tile, result) in self.replies.try_iter().take(limit) {
            processed += 1;
            self.pending.remove(&tile);
            let entry = match result {
                Ok(Some(image)) => Cached::Texture(ui.ctx().load_texture(
                    tile.url(),
                    image,
                    egui::TextureOptions::LINEAR,
                )),
                Ok(None) => Cached::Missing,
                Err(_) => Cached::Failed(Instant::now()),
            };
            self.cache.insert(tile, (entry, self.frame));
        }
        if processed == limit && !self.pending.is_empty() {
            ui.ctx().request_repaint();
        }
        let tiles = view.tiles(rect);
        let mut failed = false;
        for tile in tiles {
            if let Some((entry, touched)) = self.cache.get_mut(&tile) {
                *touched = self.frame;
                match entry {
                    Cached::Texture(texture) => {
                        painter.image(
                            texture.id(),
                            view.tile_rect(rect, tile),
                            Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
                            egui::Color32::WHITE,
                        );
                        continue;
                    }
                    Cached::Missing => continue,
                    Cached::Failed(at) => {
                        failed = true;
                        let retry = Duration::from_secs(15).saturating_sub(at.elapsed());
                        if !retry.is_zero() {
                            ui.ctx().request_repaint_after(retry);
                            continue;
                        }
                    }
                }
            }
            if self.pending.len() < MAX_PENDING
                && !self.pending.contains(&tile)
                && self.queue.send(tile).is_ok()
            {
                self.pending.insert(tile);
            }
        }
        while self.cache.len() > MAX_CACHE {
            if let Some(key) = self
                .cache
                .iter()
                .min_by_key(|(_, (_, touched))| *touched)
                .map(|(&key, _)| key)
            {
                self.cache.remove(&key);
            }
        }
        failed
    }

    pub fn attribution(ui: &mut egui::Ui) {
        ui.hyperlink_to("出典：国土地理院（標準地図）", ATTRIBUTION);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_spreads_ready_tile_uploads_over_frames() {
        let ctx = egui::Context::default();
        let (queue, _jobs) = mpsc::channel();
        let (send, replies) = mpsc::channel();
        let pending: HashSet<_> = (0..MAX_PENDING as u32)
            .map(|x| Tile { z: 5, x, y: 0 })
            .collect();
        for &tile in &pending {
            send.send((
                tile,
                Ok(Some(ColorImage::new([TILE_SIZE; 2], egui::Color32::WHITE))),
            ))
            .unwrap();
        }
        let mut tiles = Tiles {
            queue,
            replies,
            pending,
            cache: HashMap::new(),
            frame: 0,
        };
        let view = View {
            center: [0.5; 2],
            zoom: MIN_ZOOM,
        };
        let mut frame = |dragging| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    tiles.paint(
                        ui,
                        ui.painter(),
                        ui.available_rect_before_wrap(),
                        &view,
                        dragging,
                    );
                });
            });
            tiles.cache.len()
        };

        assert_eq!(frame(true), 1);
        assert_eq!(frame(true), 2);
        assert_eq!(frame(false), MAX_PENDING);
    }

    #[test]
    fn cursor_anchor_survives_zoom_and_fuji_uses_expected_xyz() {
        let mut view = View {
            center: crate::points::project(35.36, 138.73),
            zoom: 8.,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800., 600.));
        assert_eq!(
            view.tiles(rect)[0],
            Tile {
                z: 8,
                x: 226,
                y: 101
            }
        );
        let p = egui::pos2(600., 340.);
        let world = view.world_at(rect, p);
        view.zoom_at(rect, p, 2.);
        assert_eq!(view.world_at(rect, p), world);
    }
}
