//! 正式な地表SQLiteを背景スレッドで読み、地形用のテクスチャへ合成する。
use anyhow::{Result, ensure};
use eframe::egui;
use std::{collections::HashMap, path::PathBuf, sync::mpsc, time::Instant};
use yamamuki_data_sqlite as db;
pub type Bounds = [f64; 4];
pub type LayerVisibility = ([bool; 2], [bool; 2], bool, bool);
const WORLD: f64 = 40075016.68557849;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Request {
    pub bounds: Bounds,
    pub size: [usize; 2],
    metres: f64,
    levels: [Option<u32>; 3],
    visible: [bool; 3],
    bilinear: bool,
}
impl Request {
    pub fn visibility(self) -> LayerVisibility {
        (
            [self.visible[0], self.visible[2]],
            [self.visible[0], false],
            false,
            self.visible[1],
        )
    }
}
pub struct Image {
    pub request: Request,
    pub pixels: Vec<u8>,
    pub z: u32,
}
impl Image {
    pub fn premultiply_for_display(&mut self) {
        for p in self.pixels.chunks_exact_mut(4) {
            let a = p[3] as u16;
            for v in &mut p[..3] {
                *v = ((*v as u16 * a + 127) / 255) as u8;
            }
        }
    }
    pub fn lod(&self) -> u32 {
        self.z
    }
    pub fn status(&self) -> String {
        format!("地表 z{}", self.z)
    }
}
pub fn fade(start: Option<Instant>, animate: bool) -> f32 {
    if animate {
        start.map_or(1., |v| (v.elapsed().as_secs_f32() / 0.45).min(1.))
    } else {
        1.
    }
}
pub struct Style {
    pub l3_zoom: u32,
    pub l4_zoom: u32,
    pub has_l2: bool,
    pub has_z7: bool,
    pub bare_on_top: bool,
    pub animate: bool,
    pub bilinear: bool,
    levels: [Option<u32>; 3],
    visible: [bool; 3],
    available: Vec<u32>,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            l3_zoom: 10,
            l4_zoom: 9,
            has_l2: true,
            has_z7: true,
            bare_on_top: false,
            animate: true,
            bilinear: true,
            levels: [None; 3],
            visible: [true; 3],
            available: (5..=12).collect(),
        }
    }
}
impl Style {
    pub fn request(&self, center: [f64; 2], size: [usize; 2], metres: f64) -> Request {
        let half = size.map(|v| v as f64 * metres / 2.);
        Request {
            bounds: [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ],
            size,
            metres,
            levels: self.levels,
            visible: self.visible,
            bilinear: self.bilinear,
        }
    }
    pub fn level_picker(
        &mut self,
        ui: &mut egui::Ui,
        _layered: bool,
        _split: bool,
        _unified: bool,
        _bare: bool,
        _shade: bool,
    ) {
        for (i, label) in ["森林", "陰影", "水域"].iter().enumerate() {
            ui.checkbox(&mut self.visible[i], *label);
            egui::ComboBox::from_id_salt(("terrain-surface-level", i))
                .selected_text(self.levels[i].map_or("自動".into(), |z| format!("z{z}")))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.levels[i], None, "自動");
                    for &z in &self.available {
                        ui.selectable_value(&mut self.levels[i], Some(z), format!("z{z}"));
                    }
                });
        }
    }
    pub fn controls(&mut self, ui: &mut egui::Ui, _blur: bool, _layered: bool, _shade: bool) {
        ui.checkbox(&mut self.animate, "地表LODを滑らかに切替");
        ui.checkbox(&mut self.bilinear, "バイリニア補間");
    }
}
pub enum Message {
    Extent {
        bounds: Bounds,
        credits: String,
        supports_blur: bool,
        layered: bool,
        has_hillshade: bool,
        split_land: bool,
        unified_land: bool,
        has_bare: bool,
        default_bare_on_top: bool,
        l3_zoom: u32,
        l4_zoom: u32,
        has_l2: bool,
        has_z7: bool,
    },
    Overview(Image),
    Detail(Image),
    Error(String),
}
pub struct Loader {
    pub extent: Option<Bounds>,
    pub credits: String,
    pub supports_blur: bool,
    pub has_hillshade: bool,
    pub layered: bool,
    pub split_land: bool,
    pub unified_land: bool,
    pub has_bare: bool,
    pub receive: mpsc::Receiver<Message>,
    sender: mpsc::Sender<Request>,
    last: Option<Request>,
}
struct Layer {
    dataset: i64,
    info: db::SurfaceLayer,
    levels: Vec<db::SurfaceLevel>,
    role: usize,
}
struct Surface {
    conn: db::rusqlite::Connection,
    layers: Vec<Layer>,
    extent: Bounds,
    credits: String,
}
impl Surface {
    fn open(path: &std::path::Path) -> Result<Self> {
        let conn = db::open(path)?;
        let mut layers = Vec::new();
        let mut credits = Vec::new();
        let mut extent = [WORLD, WORLD, 0., 0.];
        for dataset in db::datasets(&conn)? {
            if dataset.kind != "surface" || dataset.format_version != 1 {
                continue;
            }
            credits.extend(
                dataset.metadata["sources"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|source| source["attribution"].as_str())
                    .map(str::to_owned),
            );
            for info in db::surface_layers(&conn, dataset.key)? {
                let role = match (info.id.as_str(), info.role.as_str()) {
                    ("forest", "mask") => 0,
                    ("hillshade", "hillshade") => 1,
                    ("water", "mask") => 2,
                    _ => continue,
                };
                let levels = db::surface_levels(&conn, dataset.key, info.key)?;
                let mut stmt=conn.prepare("SELECT z,min(x),min(y),max(x),max(y) FROM surface_tiles WHERE dataset_key=? AND layer_key=? GROUP BY z")?;
                let rows = stmt.query_map([dataset.key, info.key], |r| {
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, f64>(1)?,
                        r.get::<_, f64>(2)?,
                        r.get::<_, f64>(3)?,
                        r.get::<_, f64>(4)?,
                    ))
                })?;
                for row in rows {
                    let (z, x0, y0, x1, y1) = row?;
                    let side = WORLD / 2f64.powi(z as i32);
                    extent[0] = extent[0].min(x0 * side);
                    extent[1] = extent[1].min(y0 * side);
                    extent[2] = extent[2].max((x1 + 1.) * side);
                    extent[3] = extent[3].max((y1 + 1.) * side);
                }
                layers.push(Layer {
                    dataset: dataset.key,
                    info,
                    levels,
                    role,
                });
            }
        }
        ensure!(
            !layers.is_empty(),
            "対応する森林・陰影・水域レイヤーがありません"
        );
        Ok(Self {
            conn,
            layers,
            extent,
            credits: credits.join(" ／ "),
        })
    }
    fn render(&self, request: Request) -> Result<Image> {
        let [width, height] = request.size;
        ensure!(
            width <= 1536 && height <= 1536,
            "地表テクスチャが大きすぎます"
        );
        let mut pixels = vec![0u8; width * height * 4];
        let mut used = 0;
        for layer in &self.layers {
            if !request.visible[layer.role] {
                continue;
            }
            let Some(level) = layer.levels.iter().min_by(|a, b| {
                let score = |l: &db::SurfaceLevel| {
                    if let Some(z) = request.levels[layer.role] {
                        (l.z as f64 - z as f64).abs()
                    } else {
                        (WORLD / (2f64.powi(l.z as i32) * l.pixels as f64) / request.metres)
                            .log2()
                            .abs()
                    }
                };
                score(a).total_cmp(&score(b))
            }) else {
                continue;
            };
            used = used.max(level.z);
            let side = WORLD / 2f64.powi(level.z as i32);
            let n = level.pixels as i64;
            let limit = 1i64 << level.z;
            let mut cache = HashMap::new();
            let mut sample = |x: i64, y: i64| -> Result<f32> {
                let tx = x.div_euclid(n);
                let ty = y.div_euclid(n);
                if tx < 0 || ty < 0 || tx >= limit || ty >= limit {
                    return Ok(0.);
                }
                let key = (tx as u32, ty as u32);
                if !cache.contains_key(&key) {
                    let image = db::surface_tile(
                        &self.conn,
                        layer.dataset,
                        layer.info.key,
                        level.z,
                        key.0,
                        key.1,
                    )?
                    .map(|i| i.to_luma8());
                    cache.insert(key, image);
                }
                Ok(cache[&key].as_ref().map_or(0., |i| {
                    i.get_pixel(x.rem_euclid(n) as u32, y.rem_euclid(n) as u32)
                        .0[0] as f32
                        / 255.
                }))
            };
            for y in 0..height {
                for x in 0..width {
                    let world = [
                        request.bounds[0]
                            + (x as f64 + 0.5) / width as f64
                                * (request.bounds[2] - request.bounds[0]),
                        request.bounds[1]
                            + (y as f64 + 0.5) / height as f64
                                * (request.bounds[3] - request.bounds[1]),
                    ];
                    let uv = world.map(|v| v / side * n as f64 - 0.5);
                    let ix = uv[0].floor() as i64;
                    let iy = uv[1].floor() as i64;
                    let value = if request.bilinear {
                        let fx = (uv[0] - ix as f64) as f32;
                        let fy = (uv[1] - iy as f64) as f32;
                        sample(ix, iy)? * (1. - fx) * (1. - fy)
                            + sample(ix + 1, iy)? * fx * (1. - fy)
                            + sample(ix, iy + 1)? * (1. - fx) * fy
                            + sample(ix + 1, iy + 1)? * fx * fy
                    } else {
                        sample((uv[0] + 0.5).floor() as i64, (uv[1] + 0.5).floor() as i64)?
                    };
                    let p = &mut pixels[(y * width + x) * 4..(y * width + x + 1) * 4];
                    if layer.role == 1 {
                        let shade = (1. + (value * 255. - 136.) / 136. * 0.65).clamp(0., 2.);
                        for c in &mut p[..3] {
                            *c = (*c as f32 * shade).clamp(0., 255.) as u8;
                        }
                    } else {
                        let color = if layer.role == 0 {
                            [111., 148., 88.]
                        } else {
                            [114., 172., 204.]
                        };
                        let a = value;
                        let old = p[3] as f32 / 255.;
                        let out = a + old * (1. - a);
                        if out > 0. {
                            for i in 0..3 {
                                p[i] = ((color[i] * a + p[i] as f32 * old * (1. - a)) / out).round()
                                    as u8;
                            }
                            p[3] = (out * 255.).round() as u8;
                        }
                    }
                }
            }
        }
        Ok(Image {
            request,
            pixels,
            z: used,
        })
    }
}
impl Loader {
    pub fn start(path: PathBuf, ctx: egui::Context, overview: bool) -> Self {
        let (sender, requests) = mpsc::channel::<Request>();
        let (messages, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let send = |m| {
                let _ = messages.send(m);
                ctx.request_repaint();
            };
            let result = (|| -> Result<()> {
                let db = Surface::open(&path)?;
                let bounds = db.extent;
                send(Message::Extent {
                    bounds,
                    credits: db.credits.clone(),
                    supports_blur: false,
                    layered: true,
                    has_hillshade: db.layers.iter().any(|l| l.role == 1),
                    split_land: true,
                    unified_land: true,
                    has_bare: false,
                    default_bare_on_top: false,
                    l3_zoom: 10,
                    l4_zoom: 9,
                    has_l2: true,
                    has_z7: true,
                });
                if overview {
                    let span = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]) * 1.04;
                    send(Message::Overview(db.render(Style::default().request(
                        [(bounds[0] + bounds[2]) / 2., (bounds[1] + bounds[3]) / 2.],
                        [512; 2],
                        span / 512.,
                    ))?));
                }
                while let Ok(mut request) = requests.recv() {
                    while let Ok(latest) = requests.try_recv() {
                        request = latest;
                    }
                    match db.render(request) {
                        Ok(image) => send(Message::Detail(image)),
                        Err(error) => send(Message::Error(format!("{error:#}"))),
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                send(Message::Error(format!("{error:#}")));
            }
        });
        Self {
            extent: None,
            credits: String::new(),
            supports_blur: false,
            has_hillshade: false,
            layered: true,
            split_land: true,
            unified_land: true,
            has_bare: false,
            receive,
            sender,
            last: None,
        }
    }
    pub fn request(&mut self, request: Request) {
        if self.last != Some(request) {
            self.last = Some(request);
            let _ = self.sender.send(request);
        }
    }
}
pub fn write_screenshot(path: &std::path::Path, image: &egui::ColorImage) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(file, image.size[0] as u32, image.size[1] as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let bytes = image
        .pixels
        .iter()
        .flat_map(|p| p.to_array())
        .collect::<Vec<_>>();
    encoder.write_header()?.write_image_data(&bytes)?;
    Ok(())
}
