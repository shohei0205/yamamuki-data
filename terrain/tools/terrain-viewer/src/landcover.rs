use crate::shared_data::{Key, WORLD, bounds};
use anyhow::{Context, Result, ensure};
use eframe::egui;
use flate2::read::GzDecoder;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
};

pub type Bounds = [f64; 4];
type Point = [f64; 2];
type Ring = Vec<Point>;
type Polygon = (u8, Vec<Ring>);
const PALETTE: [[u8; 4]; 4] = [
    [224, 216, 196, 255],
    [111, 148, 88, 255],
    [185, 152, 113, 255],
    [114, 172, 204, 255],
];
const MAX_PAYLOAD_BYTES: u64 = 32 * 1024 * 1024;

struct Decoder<'a>(&'a [u8]);

impl Decoder<'_> {
    fn byte(&mut self) -> Result<u8> {
        let (&value, rest) = self.0.split_first().context("Truncated BZF2")?;
        self.0 = rest;
        Ok(value)
    }

    fn integer(&mut self) -> Result<usize> {
        let mut value = 0;
        for shift in (0..35).step_by(7) {
            let byte = self.byte()?;
            value |= usize::from(byte & 127) << shift;
            if byte < 128 {
                return Ok(value);
            }
        }
        anyhow::bail!("Invalid BZF2 integer")
    }

    fn count(&mut self) -> Result<usize> {
        let value = self.integer()?;
        ensure!(value <= self.0.len(), "Invalid BZF2 count");
        Ok(value)
    }

    fn coordinate(&mut self, previous: f64) -> Result<f64> {
        let tag = usize::from(self.byte()?);
        if tag == 0 {
            return Ok(previous);
        }
        let (low, size) = (tag % 8, tag / 8);
        ensure!(
            size > 0 && low + size <= 8,
            "Invalid BZF2 coordinate window"
        );
        let mut bits = previous.to_le_bytes();
        for byte in &mut bits[low..low + size] {
            *byte ^= self.byte()?;
        }
        let value = f64::from_le_bytes(bits);
        ensure!(value.is_finite(), "Invalid BZF2 coordinate");
        Ok(value)
    }

    fn point(&mut self, previous: Point) -> Result<Point> {
        Ok([self.coordinate(previous[0])?, self.coordinate(previous[1])?])
    }
}

fn decode(payload: &[u8]) -> Result<Vec<Polygon>> {
    let mut bytes = Vec::new();
    GzDecoder::new(payload)
        .take(MAX_PAYLOAD_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_PAYLOAD_BYTES,
        "Landcover BZF2 exceeds size limit"
    );
    let mut input = Decoder(
        bytes
            .strip_prefix(b"BZF2")
            .context("Expected BZF2 landcover")?,
    );
    let paths = input.count()?;
    let polygons = input.count()?;
    let mut endpoints = Vec::new();
    let mut previous = [0.; 2];
    for _ in 0..input.count()? {
        previous = input.point(previous)?;
        endpoints.push(previous);
    }
    let mut chains = Vec::new();
    for _ in 0..paths {
        let count = input.count()?;
        ensure!(count > 0, "Empty BZF2 path");
        let mut point = *endpoints
            .get(input.integer()?)
            .context("BZF2 start endpoint")?;
        let last = *endpoints
            .get(input.integer()?)
            .context("BZF2 end endpoint")?;
        let mut path = vec![point];
        for i in 0..count {
            ensure!(
                input.byte()? == 0,
                "Expected straight segments in national BZF2"
            );
            point = if i + 1 == count {
                last
            } else {
                input.point(point)?
            };
            path.push(point);
        }
        chains.push(path);
    }
    let mut result = Vec::new();
    for _ in 0..polygons {
        let class = input.byte()?;
        ensure!((1..=3).contains(&class), "Invalid landcover class");
        let mut rings = Vec::new();
        for _ in 0..input.count()? {
            let mut ring = Vec::new();
            for _ in 0..input.count()? {
                let id = input.integer()?;
                let path = chains.get(id / 2).context("BZF2 path reference")?;
                if id % 2 == 0 {
                    ring.extend(path.iter().take(path.len() - 1).copied());
                } else {
                    ring.extend(path.iter().rev().take(path.len() - 1).copied());
                }
                ensure!(
                    ring.len() <= MAX_PAYLOAD_BYTES as usize / 16,
                    "BZF2 ring too large"
                );
            }
            ensure!(ring.len() >= 3, "Degenerate BZF2 ring");
            ring.push(ring[0]);
            rings.push(ring);
        }
        ensure!(!rings.is_empty(), "Empty BZF2 polygon");
        result.push((class, rings));
    }
    ensure!(input.0.is_empty(), "Trailing BZF2 data");
    Ok(result)
}

struct Tile {
    kind: i32,
    mask: u16,
    polygons: Vec<Polygon>,
}

pub struct Database {
    connection: Connection,
    pub extent: Bounds,
    cache: HashMap<Key, Option<Tile>>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("地表SQLiteを開く {}", path.display()))?;
        let metadata: String =
            connection.query_row("SELECT value FROM metadata WHERE key='manifest'", [], |r| {
                r.get(0)
            })?;
        let metadata: Value = serde_json::from_str(&metadata)?;
        ensure!(
            metadata["version"] == 1
                && metadata["format"] == "BZF2+gzip"
                && metadata["worldWidthM"].as_f64() == Some(WORLD),
            "全国地表SQLite v1 が必要です"
        );
        let [xmin, ymin, xmax, ymax]: [i32; 4] = connection.query_row(
            "SELECT min(x),min(y),max(x),max(y) FROM tiles WHERE z=8",
            [],
            |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]),
        )?;
        let left = bounds((8, xmin, ymin));
        let right = bounds((8, xmax, ymax));
        Ok(Self {
            connection,
            extent: [left[0], left[1], right[2], right[3]],
            cache: HashMap::new(),
        })
    }

    fn tile(&mut self, key: Key) -> Result<Option<&Tile>> {
        use rusqlite::OptionalExtension;
        if !self.cache.contains_key(&key) {
            if self.cache.len() >= 512 {
                self.cache.clear();
            }
            let (z, x, y) = key;
            let row = self
                .connection
                .query_row(
                    "SELECT kind,coverage_mask,payload FROM tiles WHERE z=? AND x=? AND y=?",
                    params![z, x, y],
                    |r| {
                        Ok((
                            r.get::<_, i32>(0)?,
                            r.get::<_, u16>(1)?,
                            r.get::<_, Option<Vec<u8>>>(2)?,
                        ))
                    },
                )
                .optional()?;
            let tile = row
                .map(|(kind, mask, payload)| -> Result<Tile> {
                    let polygons = match (kind, payload) {
                        (-1, Some(payload)) => decode(&payload)?,
                        (0 | 3, None) => Vec::new(),
                        _ => anyhow::bail!("Invalid landcover tile {key:?}"),
                    };
                    ensure!(
                        mask != 0 && (z == 6 || mask == 1),
                        "Invalid landcover coverage mask"
                    );
                    Ok(Tile {
                        kind,
                        mask,
                        polygons,
                    })
                })
                .transpose()?;
            self.cache.insert(key, tile);
        }
        Ok(self.cache[&key].as_ref())
    }

    pub fn image(&mut self, request: Request) -> Result<Image> {
        let z = request.level;
        ensure!(LEVELS.contains(&z), "Invalid landcover level");
        let output = request.for_blur();
        let [pad_x, pad_y] = output.blur_pixels();
        if [pad_x, pad_y] == [0; 2] {
            return self.rasterize(request);
        }
        let mut blurred = self.rasterize(output.padded([pad_x, pad_y]))?;
        soften(&mut blurred.pixels, blurred.request.size, [pad_x, pad_y]);

        if output.size != request.size {
            // Keep water and coverage at the original display resolution, even
            // when the land blur is evaluated on a coarser raster.
            let mut image = self.rasterize(request)?;
            let [width, height] = request.size;
            let [blur_width, blur_height] = output.size;
            for (y, row) in image.pixels.chunks_exact_mut(width * 4).enumerate() {
                let source_y = (2 * y + 1) * blur_height / (2 * height) + pad_y;
                for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                    if is_land(pixel) {
                        let source_x = (2 * x + 1) * blur_width / (2 * width) + pad_x;
                        let start = (source_y * blurred.request.size[0] + source_x) * 4;
                        let sample = &blurred.pixels[start..start + 4];
                        if is_land(sample) {
                            pixel.copy_from_slice(sample);
                        }
                    }
                }
            }
            return Ok(image);
        }

        let [width, height] = output.size;
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in pad_y..pad_y + height {
            let start = (y * blurred.request.size[0] + pad_x) * 4;
            pixels.extend_from_slice(&blurred.pixels[start..start + width * 4]);
        }
        Ok(Image {
            request: output,
            z,
            pixels,
        })
    }

    fn rasterize(&mut self, request: Request) -> Result<Image> {
        let z = request.level;
        let mut image = Image {
            request,
            z,
            pixels: vec![0; request.size[0] * request.size[1] * 4],
        };
        let side = WORLD / f64::from(1u32 << z);
        let index = |v: f64| (v / side).floor().clamp(0., f64::from((1u32 << z) - 1)) as i32;
        let [left, top, right, bottom] = request.bounds;
        let [extent_left, extent_top, extent_right, extent_bottom] = self.extent;
        for y in index(top.max(extent_top))..=index(bottom.min(extent_bottom)) {
            for x in index(left.max(extent_left))..=index(right.min(extent_right)) {
                let key = (z, x, y);
                let Some(tile) = self.tile(key)? else {
                    continue;
                };
                let tile_bounds = bounds(key);
                let [x0, y0, _, _] = tile_bounds;
                let parts = if z == 6 { 4 } else { 1 };
                let part_side = side / f64::from(parts);
                for iy in 0..parts {
                    for ix in 0..parts {
                        if tile.mask & (1 << (ix + iy * parts)) == 0 {
                            continue;
                        }
                        let l = x0 + f64::from(ix) * part_side;
                        let t = y0 + f64::from(iy) * part_side;
                        image.fill_box(
                            [l, t, l + part_side, t + part_side],
                            if tile.kind == 3 { 3 } else { 0 },
                        );
                    }
                }
                if tile.kind == -1 {
                    // Classification priority follows the source layering.
                    for class in 1..=3 {
                        for (_, rings) in tile.polygons.iter().filter(|(code, _)| *code == class) {
                            image.fill_polygon(rings, tile_bounds, tile.mask, parts, class);
                        }
                    }
                }
            }
        }
        Ok(image)
    }
}

fn is_land(pixel: &[u8]) -> bool {
    pixel[3] != 0 && pixel != PALETTE[3]
}

fn soften(pixels: &mut [u8], [width, height]: [usize; 2], [rx, ry]: [usize; 2]) {
    if rx == 0 && ry == 0 {
        return;
    }
    fn pass(
        src: &[u8],
        dst: &mut [u8],
        lines: usize,
        len: usize,
        line_step: usize,
        step: usize,
        radius: usize,
    ) {
        for line in 0..lines {
            let start = line * line_step;
            let mut sum = [0; 4];
            let update = |sum: &mut [i32; 4], position: usize, sign: i32| {
                let p = start + position * step;
                if is_land(&src[p..p + 4]) {
                    for channel in 0..3 {
                        sum[channel] += i32::from(src[p + channel]) * sign;
                    }
                    sum[3] += sign;
                }
            };
            for x in 0..radius.min(len) {
                update(&mut sum, x, 1);
            }
            for x in 0..len {
                if x + radius < len {
                    update(&mut sum, x + radius, 1);
                }
                let p = start + x * step;
                if is_land(&src[p..p + 4]) {
                    for channel in 0..3 {
                        dst[p + channel] = ((sum[channel] + sum[3] / 2) / sum[3]) as u8;
                    }
                    dst[p + 3] = src[p + 3];
                } else {
                    dst[p..p + 4].copy_from_slice(&src[p..p + 4]);
                }
                if x >= radius {
                    update(&mut sum, x - radius, -1);
                }
            }
        }
    }
    let mut horizontal = vec![0; pixels.len()];
    pass(pixels, &mut horizontal, height, width, width * 4, 4, rx);
    pass(&horizontal, pixels, width, height, 4, width * 4, ry);
}

fn auto_level(metres: f64) -> u32 {
    if metres < 100. {
        11
    } else if metres < 400. {
        9
    } else if metres < 1200. {
        8
    } else {
        6
    }
}

pub fn fade(start: Option<std::time::Instant>, enabled: bool) -> f32 {
    let t = start
        .filter(|_| enabled)
        .map_or(1., |start| (start.elapsed().as_secs_f32() / 0.45).min(1.));
    t * t * (3. - 2. * t)
}

const LEVELS: [u32; 4] = [11, 9, 8, 6];
// Projected metres, matching the L3–L6 generator fit tolerances.
const FIT_TOLERANCE_M: [u32; 4] = [200, 500, 2000, 4000];
const DEFAULT_BLUR_M: [u32; 4] = [100, 250, 1000, 2000];

fn level_index(z: u32) -> usize {
    LEVELS
        .iter()
        .position(|&level| level == z)
        .expect("supported landcover level")
}

pub struct Style {
    level: Option<u32>,
    quality: f64,
    blur: [u32; LEVELS.len()],
    pub animate: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            level: None,
            quality: 1.,
            blur: DEFAULT_BLUR_M,
            animate: true,
        }
    }
}

impl Style {
    pub fn request(&self, center: Point, size: [usize; 2], metres: f64) -> Request {
        let mut request = Request::around(
            center,
            size,
            metres,
            Some(
                self.level
                    .unwrap_or_else(|| auto_level(metres / self.quality)),
            ),
        );
        request.blur = self.blur[level_index(request.level)];
        request.overview_blur = self.blur[level_index(6)];
        request
    }

    pub fn controls(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_id_salt("landcover-level")
            .selected_text(
                self.level
                    .map_or_else(|| "地表 自動LOD".into(), |z| format!("地表 z{z}")),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.level, None, "地表 自動LOD");
                for (i, z) in LEVELS.into_iter().enumerate() {
                    ui.selectable_value(&mut self.level, Some(z), format!("L{} / z{z}", i + 3));
                }
            });
        ui.add_enabled(
            self.level.is_none(),
            egui::Slider::new(&mut self.quality, 0.5..=4.)
                .step_by(0.25)
                .text("地表LOD精細度"),
        );
        ui.menu_button("陸地ぼかし", |ui| {
            for (i, (radius, z)) in self.blur.iter_mut().zip(LEVELS).enumerate() {
                ui.add(
                    egui::Slider::new(radius, 0..=FIT_TOLERANCE_M[i])
                        .suffix(" m")
                        .text(format!("L{} / z{z} 半径", i + 3)),
                );
            }
        });
        ui.checkbox(&mut self.animate, "地表LODを滑らかに切替");
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Request {
    pub bounds: Bounds,
    pub size: [usize; 2],
    level: u32,
    blur: u32,
    overview_blur: u32,
}

impl Request {
    fn blur_pixels(self) -> [usize; 2] {
        std::array::from_fn(|axis| {
            let extent = self.bounds[axis + 2] - self.bounds[axis];
            let radius = f64::from(self.blur) * self.size[axis] as f64 / extent;
            radius.round() as usize
        })
    }

    fn for_blur(mut self) -> Self {
        // A heavily blurred layer can use fewer texels when zoomed far in.
        // Limit the padding cost while keeping the radius in projected metres.
        if self.blur != 0 {
            for axis in 0..2 {
                let extent = self.bounds[axis + 2] - self.bounds[axis];
                let limit = (extent * 32. / f64::from(self.blur)).floor() as usize;
                self.size[axis] = self.size[axis].min(limit.max(1));
            }
        }
        self
    }

    fn padded(mut self, padding: [usize; 2]) -> Self {
        for (axis, pad) in padding.into_iter().enumerate() {
            let extent = self.bounds[axis + 2] - self.bounds[axis];
            let distance = extent / self.size[axis] as f64 * pad as f64;
            self.bounds[axis] -= distance;
            self.bounds[axis + 2] += distance;
            self.size[axis] += 2 * pad;
        }
        self
    }

    pub fn around(
        center: Point,
        size: [usize; 2],
        metres_per_pixel: f64,
        level: Option<u32>,
    ) -> Self {
        let half = size.map(|v| v as f64 * metres_per_pixel / 2.);
        let level = level.unwrap_or_else(|| auto_level(metres_per_pixel));
        Self {
            bounds: [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ],
            size,
            level,
            blur: DEFAULT_BLUR_M[level_index(level)],
            overview_blur: DEFAULT_BLUR_M[level_index(6)],
        }
    }
}

pub struct Image {
    pub request: Request,
    pub z: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    fn pixel(&self, x: f64, y: f64) -> Point {
        let [l, t, r, b] = self.request.bounds;
        [
            (x - l) * self.request.size[0] as f64 / (r - l),
            (y - t) * self.request.size[1] as f64 / (b - t),
        ]
    }

    fn first_pixel(value: f64, size: usize) -> usize {
        (value - 0.5).ceil().clamp(0., size as f64) as usize
    }

    fn span(&mut self, row: usize, left: f64, right: f64, class: u8) {
        let width = self.request.size[0];
        let a = Self::first_pixel(left, width);
        let b = Self::first_pixel(right, width);
        if a >= b {
            return;
        }
        for pixel in self.pixels[(row * width + a) * 4..(row * width + b) * 4].chunks_exact_mut(4) {
            pixel.copy_from_slice(&PALETTE[usize::from(class)]);
        }
    }

    fn fill_box(&mut self, [l, t, r, b]: Bounds, class: u8) {
        let [l, t] = self.pixel(l, t);
        let [r, b] = self.pixel(r, b);
        for y in
            Self::first_pixel(t, self.request.size[1])..Self::first_pixel(b, self.request.size[1])
        {
            self.span(y, l, r, class);
        }
    }

    fn fill_polygon(&mut self, rings: &[Ring], tile: Bounds, mask: u16, parts: u32, class: u8) {
        let [tx, ty, rx, by] = tile;
        let points: Vec<Vec<Point>> = rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|p| self.pixel(tx + p[0], ty + p[1]))
                    .collect()
            })
            .collect();
        let (min, max) = points
            .iter()
            .flatten()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p[1]), b.max(p[1]))
            });
        let height = self.request.size[1];
        let start = Self::first_pixel(min, height);
        let end = Self::first_pixel(max, height);
        let mut rows = vec![Vec::new(); end - start];
        for ring in &points {
            for edge in ring.windows(2) {
                let (a, b) = if edge[0][1] < edge[1][1] {
                    (edge[0], edge[1])
                } else {
                    (edge[1], edge[0])
                };
                let first = Self::first_pixel(a[1], height).max(start);
                let last = Self::first_pixel(b[1], height).min(end);
                for row in first..last {
                    let t = (row as f64 + 0.5 - a[1]) / (b[1] - a[1]);
                    rows[row - start].push(a[0] + (b[0] - a[0]) * t);
                }
            }
        }
        let [left, top] = self.pixel(tx, ty);
        let [right, bottom] = self.pixel(rx, by);
        let part_width = (right - left) / f64::from(parts);
        for (row, mut crossings) in rows.into_iter().enumerate() {
            let row = row + start;
            let iy = ((row as f64 + 0.5 - top) * f64::from(parts) / (bottom - top)).floor() as i32;
            if iy < 0 || iy >= parts as i32 {
                continue;
            }
            crossings.sort_unstable_by(f64::total_cmp);
            for pair in crossings.chunks_exact(2) {
                for ix in 0..parts {
                    if mask & (1 << (ix + iy as u32 * parts)) == 0 {
                        continue;
                    }
                    let l = left + f64::from(ix) * part_width;
                    self.span(row, pair[0].max(l), pair[1].min(l + part_width), class);
                }
            }
        }
    }
}

pub enum Message {
    Extent(Bounds),
    Overview(Image),
    Detail(Image),
    Error(String),
}

pub struct Loader {
    pub extent: Option<Bounds>,
    pub receive: mpsc::Receiver<Message>,
    sender: mpsc::Sender<Request>,
    last: Option<Request>,
}

impl Loader {
    pub fn start(path: PathBuf, ctx: egui::Context, overview: bool) -> Self {
        let (sender, requests) = mpsc::channel::<Request>();
        let (messages, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let send = |message| {
                let _ = messages.send(message);
                ctx.request_repaint();
            };
            let result = (|| -> Result<()> {
                let mut database = Database::open(&path)?;
                let [l, t, r, b] = database.extent;
                send(Message::Extent(database.extent));
                let mut overview_request = overview.then(|| {
                    let side = (r - l).max(b - t) * 1.04;
                    Request::around(
                        [(l + r) / 2., (t + b) / 2.],
                        [1024, 1024],
                        side / 1024.,
                        Some(6),
                    )
                });
                if let Some(request) = overview_request {
                    send(Message::Overview(database.image(request)?));
                }
                while let Ok(mut request) = requests.recv() {
                    while let Ok(next) = requests.try_recv() {
                        request = next;
                    }
                    if let Some(base) = &mut overview_request {
                        if base.blur != request.overview_blur {
                            base.blur = request.overview_blur;
                            send(Message::Overview(database.image(*base)?));
                        }
                    }
                    match database.image(request) {
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
            receive,
            sender,
            last: None,
        }
    }

    pub fn request(&mut self, request: Request) {
        if self.last != Some(request) && self.sender.send(request).is_ok() {
            self.last = Some(request);
        }
    }
}

fn map_fade_tint(opacity: f32) -> egui::Color32 {
    // egui's gamma framebuffer blends premultiplied RGB directly.
    egui::Color32::WHITE.gamma_multiply(opacity)
}

type MapTexture = (egui::TextureHandle, Bounds, u32);

pub struct Map {
    loader: Loader,
    center: Point,
    metres: f64,
    path: String,
    status: String,
    texture: Option<MapTexture>,
    previous: Option<MapTexture>,
    transition: Option<std::time::Instant>,
    style: Style,
    last_request: std::time::Instant,
    needs_fit: bool,
    smoke: Option<PathBuf>,
    screenshot_requested: bool,
}

impl Map {
    pub fn new(ctx: &egui::Context, path: PathBuf, smoke: Option<PathBuf>) -> Self {
        Self {
            loader: Loader::start(path.clone(), ctx.clone(), false),
            center: [0.; 2],
            metres: 0.,
            path: path.to_string_lossy().into(),
            status: "地表SQLiteを読み込み中".into(),
            texture: None,
            previous: None,
            transition: None,
            style: Style::default(),
            last_request: std::time::Instant::now(),
            needs_fit: true,
            smoke,
            screenshot_requested: false,
        }
    }
}

impl eframe::App for Map {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let blend = fade(self.transition, self.style.animate);
        if blend == 1. {
            self.transition = None;
            self.previous = None;
            let mut next = None;
            for message in self.loader.receive.try_iter() {
                match message {
                    Message::Extent(extent) => self.loader.extent = Some(extent),
                    Message::Detail(image) => next = Some(image),
                    Message::Error(error) => self.status = error,
                    Message::Overview(_) => {}
                }
            }
            if let Some(image) = next {
                let color =
                    egui::ColorImage::from_rgba_unmultiplied(image.request.size, &image.pixels);
                let texture = ctx.load_texture("地表", color, egui::TextureOptions::LINEAR);
                if self.style.animate
                    && self
                        .texture
                        .as_ref()
                        .is_some_and(|entry| entry.2 != image.z)
                {
                    self.previous = self.texture.take();
                    self.transition = Some(std::time::Instant::now());
                }
                self.texture = Some((texture, image.request.bounds, image.z));
                self.status = format!("地表 z{} ／ ESA WorldCover 2021", image.z);
            }
        }
        egui::TopBottomPanel::top("landcover").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("全国地表・2D");
                ui.text_edit_singleline(&mut self.path);
                if ui.button("地表SQLiteを開く").clicked() {
                    *self = Self::new(ctx, PathBuf::from(&self.path), self.smoke.take());
                }
                if ui.button("全体").clicked() {
                    self.needs_fit = true;
                }
                self.style.controls(ui);
            });
            ui.horizontal(|ui| {
                for (class, label) in ["その他", "森林", "砂礫地", "水域"].iter().enumerate()
                {
                    let [r, g, b, _] = PALETTE[class];
                    ui.colored_label(egui::Color32::from_rgb(r, g, b), format!("■ {label}"));
                }
                ui.label("灰色は範囲外・原本なし");
                ui.label(&self.status);
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(
                ui.available_size().max(egui::vec2(1., 1.)),
                egui::Sense::drag(),
            );
            ui.painter()
                .rect_filled(rect, 0., egui::Color32::from_gray(100));
            let Some([l, t, r, b]) = self.loader.extent else {
                return;
            };
            if self.needs_fit {
                self.center = [(l + r) / 2., (t + b) / 2.];
                self.metres = ((r - l) / f64::from(rect.width()))
                    .max((b - t) / f64::from(rect.height()))
                    * 1.08;
                self.needs_fit = false;
            }
            if response.dragged() {
                let delta = ctx.input(|i| i.pointer.delta());
                self.center[0] -= f64::from(delta.x) * self.metres;
                self.center[1] -= f64::from(delta.y) * self.metres;
            }
            if response.hovered() {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                let next = (self.metres * (-f64::from(scroll) * 0.002).exp()).clamp(1., 30000.);
                if let Some(pointer) = response.hover_pos() {
                    let offset = pointer - rect.center();
                    self.center[0] += f64::from(offset.x) * (self.metres - next);
                    self.center[1] += f64::from(offset.y) * (self.metres - next);
                }
                self.metres = next;
            }
            if self.last_request.elapsed().as_millis() >= 60 {
                let size = [
                    rect.width().round() as usize,
                    rect.height().round() as usize,
                ];
                let factor = (size[0].max(size[1]) as f64 / 1600.).max(1.);
                let size = size.map(|v| (v as f64 / factor).round().max(1.) as usize);
                self.loader
                    .request(self.style.request(self.center, size, self.metres * factor));
                self.last_request = std::time::Instant::now();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
            }
            let blend = fade(self.transition, self.style.animate);
            if blend < 1. {
                ctx.request_repaint();
            }
            let screen_bounds = |[l, t, r, b]: Bounds| {
                let screen = |x: f64, y: f64| {
                    rect.center()
                        + egui::vec2(
                            ((x - self.center[0]) / self.metres) as f32,
                            ((y - self.center[1]) / self.metres) as f32,
                        )
                };
                egui::Rect::from_min_max(screen(l, t), screen(r, b))
            };
            let paint = |texture: &egui::TextureHandle, area, clip, opacity| {
                ui.painter().with_clip_rect(rect.intersect(clip)).image(
                    texture.id(),
                    area,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    map_fade_tint(opacity),
                );
            };
            if let Some((texture, bounds, _)) = &self.texture {
                let area = screen_bounds(*bounds);
                if blend < 1. {
                    if let Some((previous, bounds, _)) = &self.previous {
                        let old = screen_bounds(*bounds);
                        let shared = area.intersect(rect);
                        if shared.is_positive() {
                            paint(previous, old, shared, 1.);
                            for clip in [
                                egui::Rect::from_min_max(
                                    rect.min,
                                    egui::pos2(shared.left(), rect.bottom()),
                                ),
                                egui::Rect::from_min_max(
                                    egui::pos2(shared.right(), rect.top()),
                                    rect.max,
                                ),
                                egui::Rect::from_min_max(
                                    egui::pos2(shared.left(), rect.top()),
                                    egui::pos2(shared.right(), shared.top()),
                                ),
                                egui::Rect::from_min_max(
                                    egui::pos2(shared.left(), shared.bottom()),
                                    egui::pos2(shared.right(), rect.bottom()),
                                ),
                            ] {
                                if clip.is_positive() {
                                    paint(previous, old, clip, 1. - blend);
                                }
                            }
                        } else {
                            paint(previous, old, rect, 1. - blend);
                        }
                    }
                }
                paint(texture, area, rect, blend);
            }
        });
        if let Some(path) = &self.smoke {
            if !self.screenshot_requested && self.texture.is_some() {
                self.screenshot_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    if let Err(error) = write_screenshot(path, &image) {
                        self.status = format!("{error:#}");
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_detail_selects_a_finer_level_but_fixed_level_is_respected() {
        let mut style = Style::default();
        let request = |style: &Style| style.request([0.; 2], [32; 2], 500.);
        assert_eq!(request(&style).level, 8);
        style.quality = 2.;
        assert_eq!(request(&style).level, 9);
        style.level = Some(6);
        assert_eq!(request(&style).level, 6);
    }

    #[test]
    fn blur_follows_the_selected_level_and_overview_keeps_its_own_radius() {
        let mut style = Style::default();
        style.blur = [0, 1, 2, 5];
        for (metres, radius) in [(50., 0), (200., 1), (500., 2), (1500., 5)] {
            let request = style.request([0.; 2], [32; 2], metres);
            assert_eq!(request.blur, radius);
            assert_eq!(request.overview_blur, 5);
        }
        style.level = Some(9);
        assert_eq!(style.request([0.; 2], [32; 2], 1500.).blur, 1);
    }

    #[test]
    fn blur_radius_uses_geographic_scale_and_half_the_level_tolerance() {
        assert_eq!(DEFAULT_BLUR_M, FIT_TOLERANCE_M.map(|error| error / 2));
        let request = |metres| Request::around([0.; 2], [200; 2], metres, Some(6));
        assert_eq!(request(100.).blur_pixels(), [20; 2]);
        assert_eq!(request(400.).blur_pixels(), [5; 2]);
        let zoomed = request(1.).for_blur();
        assert_eq!(zoomed.size, [3; 2]);
        assert_eq!(zoomed.blur_pixels(), [30; 2]);
        assert_eq!(
            zoomed.padded([30; 2]).bounds,
            [-2100., -2100., 2100., 2100.]
        );
    }

    #[test]
    fn blending_identical_map_colors_keeps_the_same_brightness() {
        for opacity in [0., 0.25, 0.5, 0.75, 1.] {
            let [r, g, b, a] = map_fade_tint(opacity).to_array();
            // The new texture contributes RGB directly; the old texture keeps 1 - alpha.
            assert_eq!(
                [r, g, b].map(|rgb| u16::from(rgb) + 255 - u16::from(a)),
                [255; 3]
            );
        }
    }

    #[test]
    fn blur_preserves_water_and_does_not_mix_water_color_into_land() {
        let forest = PALETTE[1];
        let water = PALETTE[3];
        let mut pixels = [forest, water, forest, water, forest, water].concat();
        let original = pixels.clone();
        soften(&mut pixels, [3, 2], [2, 2]);
        assert_eq!(pixels, original);
    }

    #[test]
    fn blur_softens_colors_and_preserves_missing_data() {
        let original = [40, 60, 80, 255, 60, 80, 100, 255, 0, 0, 0, 0];
        let mut pixels = original;
        soften(&mut pixels, [3, 1], [0, 0]);
        assert_eq!(pixels, original);
        soften(&mut pixels, [3, 1], [2, 0]);
        assert_eq!(pixels, [50, 70, 90, 255, 50, 70, 90, 255, 0, 0, 0, 0]);
    }
}

pub fn write_screenshot(path: &Path, image: &egui::ColorImage) -> Result<()> {
    let mut encoder = png::Encoder::new(
        std::fs::File::create(path)?,
        image.size[0] as u32,
        image.size[1] as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(
        &image
            .pixels
            .iter()
            .flat_map(|p| p.to_array())
            .collect::<Vec<_>>(),
    )?;
    Ok(())
}

pub fn check(path: &Path) -> Result<()> {
    let mut database = Database::open(path)?;
    let [l, t, r, b] = database.extent;
    let center = [(l + r) / 2., (t + b) / 2.];
    let side = (r - l).max(b - t);
    let overview = database.image(Request::around(center, [800, 800], side / 800., Some(6)))?;
    let fuji = crate::selection::lonlat(138.727, 35.36);
    let detail = database.image(Request::around(fuji, [768, 768], 30., Some(11)))?;
    for image in [overview, detail] {
        let counts = PALETTE.map(|color| {
            image
                .pixels
                .chunks_exact(4)
                .filter(|pixel| *pixel == color)
                .count()
        });
        ensure!(
            counts.iter().filter(|&&n| n > 0).count() >= 3,
            "Missing classes at z{}: {counts:?}",
            image.z
        );
        println!("Landcover z{}: {counts:?}", image.z);
    }
    Ok(())
}
