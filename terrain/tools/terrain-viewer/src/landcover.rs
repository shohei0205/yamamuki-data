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
    sync::{Arc, mpsc},
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
const ASTC_SIDE: usize = 256;
const ASTC_BYTES: usize = ASTC_SIDE * ASTC_SIDE / 4;

#[derive(Clone, Copy)]
enum Format {
    Vector,
    Astc,
    Png,
    PngOrWebp,
    LandMixIndex,
    LandSplitGray4,
}

fn decode_astc(payload: &[u8]) -> Result<Vec<u8>> {
    ensure!(payload.len() == ASTC_BYTES, "Invalid ASTC 8×8 tile length");
    let mut decoded = vec![0u32; ASTC_SIDE * ASTC_SIDE];
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        texture2ddecoder::decode_astc_8_8(payload, ASTC_SIDE, ASTC_SIDE, &mut decoded)
    }))
    .map_err(|_| anyhow::anyhow!("Invalid ASTC 8×8 block"))?
    .map_err(anyhow::Error::msg)?;
    Ok(decoded
        .into_iter()
        .flat_map(|color| {
            let [b, g, r, a] = color.to_le_bytes();
            // The source alpha is a binary coverage mask.
            if a >= 128 { [r, g, b, 255] } else { [0; 4] }
        })
        .collect())
}

fn land_mix_palette() -> [[u8; 4]; 253] {
    let mut colors = [[0; 4]; 253];
    let mut index = 0;
    for forest in 0..=21u16 {
        for bare in 0..=21 - forest {
            let weights = [21 - forest - bare, forest, bare];
            for channel in 0..3 {
                let sum: u16 = weights
                    .into_iter()
                    .zip(PALETTE)
                    .map(|(weight, color)| weight * u16::from(color[channel]))
                    .sum();
                colors[index][channel] = ((sum + 10) / 21) as u8;
            }
            colors[index][3] = 255;
            index += 1;
        }
    }
    colors
}

fn decode_land_gray(payload: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(payload));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    ensure!(
        info.width == ASTC_SIDE as u32
            && info.height == ASTC_SIDE as u32
            && matches!(info.bit_depth, png::BitDepth::Two | png::BitDepth::Four)
            && info.color_type == png::ColorType::Grayscale,
        "Invalid packed land layer"
    );
    let mut pixels = vec![0; ASTC_SIDE * ASTC_SIDE];
    reader.next_frame(&mut pixels)?;
    Ok(pixels)
}

enum SplitLandAux<'a> {
    Legacy(Option<&'a [u8]>),
    Unified(Option<&'a [u8]>),
}

fn decode_land_missing(payload: &[u8]) -> Result<Vec<u8>> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(payload)).read_info()?;
    let info = reader.info();
    ensure!(
        info.width == ASTC_SIDE as u32
            && info.height == ASTC_SIDE as u32
            && info.bit_depth == png::BitDepth::One
            && info.color_type == png::ColorType::Grayscale,
        "Invalid 1-bit land missing tile"
    );
    let mut packed = vec![0; ASTC_SIDE * ASTC_SIDE / 8];
    reader.next_frame(&mut packed)?;
    Ok(packed)
}

fn decode_land_split(
    forest: &[u8],
    bare: &[u8],
    aux: SplitLandAux<'_>,
    visible: [bool; 2],
    bare_overlay: bool,
) -> Result<Vec<u8>> {
    let forest = decode_land_gray(forest)?;
    let bare = decode_land_gray(bare)?;
    let (underlay, missing, legacy) = match aux {
        SplitLandAux::Legacy(payload) => (payload.map(decode_land_gray).transpose()?, None, true),
        SplitLandAux::Unified(payload) => {
            (None, payload.map(decode_land_missing).transpose()?, false)
        }
    };
    let mut rgba = Vec::with_capacity(forest.len() * 4);
    for (i, (forest, bare)) in forest.into_iter().zip(bare).enumerate() {
        if (legacy && forest == 255 && bare == 255)
            || missing
                .as_ref()
                .is_some_and(|mask| mask[i / 8] & (0x80 >> (i % 8)) != 0)
        {
            rgba.extend_from_slice(&[0; 4]);
            continue;
        }
        if bare_overlay {
            let [r, g, b, _] = PALETTE[2];
            rgba.extend_from_slice(&[r, g, b, bare]);
            continue;
        }
        let f = if visible[0] {
            u16::from((forest ^ underlay.as_ref().map_or(0, |delta| delta[i])) / 17)
        } else {
            0
        };
        let b = if visible[1] { u16::from(bare / 17) } else { 0 };
        // Other is the opaque base, then forest, then bare. Water is composited separately.
        let weights = [(15 - f) * (15 - b), f * (15 - b), 15 * b];
        let rgb = std::array::from_fn::<_, 3, _>(|channel| {
            let sum: u16 = weights
                .into_iter()
                .zip(PALETTE)
                .map(|(weight, color)| weight * u16::from(color[channel]))
                .sum();
            ((sum + 112) / 225) as u8
        });
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Ok(rgba)
}

#[test]
fn forest_underlay_preserves_coverage_and_layer_order() {
    fn tile(value: u8, depth: png::BitDepth) -> Vec<u8> {
        let mut payload = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut payload, ASTC_SIDE as u32, ASTC_SIDE as u32);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(depth);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&vec![value; ASTC_SIDE * ASTC_SIDE * depth as usize / 8])
                .unwrap();
        }
        payload
    }
    let forest = tile(0x0f, png::BitDepth::Four);
    let bare = tile(0xff, png::BitDepth::Four);
    let underlay = tile(0xf0, png::BitDepth::Four);
    let normal = decode_land_split(
        &forest,
        &bare,
        SplitLandAux::Legacy(None),
        [true, true],
        false,
    )
    .unwrap();
    let filled = decode_land_split(
        &forest,
        &bare,
        SplitLandAux::Legacy(Some(&underlay)),
        [true, false],
        false,
    )
    .unwrap();
    assert_eq!(normal[..8], [PALETTE[2], [0; 4]].concat());
    assert_eq!(filled[..8], [PALETTE[1], [0; 4]].concat());
    let forest = tile(0xff, png::BitDepth::Four);
    let missing = tile(0x40, png::BitDepth::One);
    let with_bare = decode_land_split(
        &forest,
        &bare,
        SplitLandAux::Unified(Some(&missing)),
        [true, true],
        false,
    )
    .unwrap();
    let without_bare = decode_land_split(
        &forest,
        &bare,
        SplitLandAux::Unified(Some(&missing)),
        [true, false],
        false,
    )
    .unwrap();
    assert_eq!(with_bare[..8], [PALETTE[2], [0; 4]].concat());
    assert_eq!(without_bare[..8], [PALETTE[1], [0; 4]].concat());
}

fn decode_land_mix(payload: &[u8]) -> Result<Vec<u8>> {
    static COLORS: std::sync::OnceLock<[[u8; 4]; 253]> = std::sync::OnceLock::new();
    let colors = COLORS.get_or_init(land_mix_palette);
    let mut reader = png::Decoder::new(std::io::Cursor::new(payload)).read_info()?;
    let info = reader.info();
    ensure!(
        info.width == ASTC_SIDE as u32
            && info.height == ASTC_SIDE as u32
            && info.bit_depth == png::BitDepth::Eight
            && info.color_type == png::ColorType::Grayscale,
        "Invalid land mixture tile"
    );
    let mut indices = vec![0; ASTC_SIDE * ASTC_SIDE];
    reader.next_frame(&mut indices)?;
    let mut rgba = Vec::with_capacity(indices.len() * 4);
    for index in indices {
        match index {
            255 => rgba.extend_from_slice(&[0; 4]),
            253..=254 => anyhow::bail!("Reserved land mixture index {index}"),
            _ => rgba.extend_from_slice(&colors[index as usize]),
        }
    }
    Ok(rgba)
}

fn decode_land_image(payload: &[u8]) -> Result<Vec<u8>> {
    if !payload.starts_with(b"RIFF") {
        return decode_png(payload);
    }
    let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(payload))?;
    let side = ASTC_SIDE as u32;
    ensure!(
        decoder.dimensions() == (side, side),
        "Invalid WebP landcover dimensions"
    );
    decoder.set_memory_limit(8 * 1024 * 1024);
    let mut pixels = vec![0; decoder.output_buffer_size().context("WebP tile size")?];
    decoder.read_image(&mut pixels)?;
    if decoder.has_alpha() {
        Ok(pixels)
    } else {
        Ok(pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect())
    }
}

fn decode_png(payload: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(payload));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    ensure!(
        info.width == ASTC_SIDE as u32
            && info.height == ASTC_SIDE as u32
            && (info.bit_depth == png::BitDepth::Eight
                || (info.color_type == png::ColorType::Grayscale
                    && matches!(info.bit_depth, png::BitDepth::Two | png::BitDepth::Four)))
            && matches!(
                info.color_type,
                png::ColorType::Rgb | png::ColorType::Rgba | png::ColorType::Grayscale
            ),
        "Invalid PNG landcover tile"
    );
    let mut pixels = vec![0; reader.output_buffer_size().context("PNG tile size")?];
    let frame = reader.next_frame(&mut pixels)?;
    if frame.color_type == png::ColorType::Grayscale {
        let [r, g, b, _] = PALETTE[3];
        Ok(pixels
            .into_iter()
            .flat_map(|alpha| [r, g, b, alpha])
            .collect())
    } else if frame.color_type == png::ColorType::Rgb {
        Ok(pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect())
    } else {
        Ok(pixels)
    }
}

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

fn landcover_l3_l4(connection: &Connection, table: &str) -> Result<[u32; 2]> {
    let (has_z11, has_z10, has_z9): (bool, bool, bool) = connection.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE z=11), EXISTS(SELECT 1 FROM {table} WHERE z=10), EXISTS(SELECT 1 FROM {table} WHERE z=9)"),
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    Ok(match (has_z11, has_z10, has_z9) {
        (false, true, true) => [10, 9],
        (_, true, _) => [11, 10],
        _ => [11, 9],
    })
}

fn has_z7(connection: &Connection, table: &str) -> Result<bool> {
    Ok(connection.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE z=7)"),
        [],
        |row| row.get(0),
    )?)
}

pub struct Database {
    connection: Connection,
    pub extent: Bounds,
    cache: HashMap<Key, Option<Tile>>,
    format: Format,
    water: Option<Box<Database>>,
    reads_water_table: bool,
    forest_underlay: bool,
    unified_forest: bool,
    default_bare_on_top: bool,
    l3_zoom: u32,
    l4_zoom: u32,
    has_z7: bool,
    raster_cache: HashMap<(Key, [bool; 2], bool), Option<Arc<[u8]>>>,
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
            metadata["version"] == 1 && metadata["worldWidthM"].as_f64() == Some(WORLD),
            "全国地表SQLite v1 が必要です"
        );
        if metadata["layer"] == "water" {
            let land = path
                .parent()
                .and_then(Path::parent)
                .context("Water layer directory")?
                .join("land/landcover-national-png.sqlite3");
            return Self::open(&land);
        }
        let water = if metadata["layer"] == "combined" {
            Some(Box::new(Self::open_single_water(path, true)?))
        } else if metadata["layer"] == "land" {
            let water_path = path
                .parent()
                .and_then(Path::parent)
                .context("Land layer directory")?
                .join("water/landcover-national-png.sqlite3");
            Some(Box::new(Self::open_single_water(&water_path, false)?))
        } else {
            None
        };
        let format = match metadata["format"].as_str() {
            Some("BZF2+gzip") => Format::Vector,
            Some("ASTC_8x8_UNORM")
                if metadata["tilePixels"] == ASTC_SIDE
                    && metadata["block"] == serde_json::json!([8, 8]) =>
            {
                Format::Astc
            }
            Some("PNG") if metadata["tilePixels"] == ASTC_SIDE => {
                match metadata["landEncoding"].as_str() {
                    Some("webp-lossless-or-png") => Format::PngOrWebp,
                    Some("mix21-index8") => Format::LandMixIndex,
                    Some(
                        "forest-bare-over-gray4"
                        | "forest-bare-unified-gray4"
                        | "forest-bare-unified-gray2",
                    ) => Format::LandSplitGray4,
                    _ => Format::Png,
                }
            }
            _ => anyhow::bail!("全国地表SQLiteの形式に対応していません"),
        };
        let [xmin, ymin, xmax, ymax]: [i32; 4] = connection.query_row(
            "SELECT min(x),min(y),max(x),max(y) FROM tiles WHERE z=8",
            [],
            |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]),
        )?;
        let [l3_zoom, l4_zoom] = landcover_l3_l4(&connection, "tiles")?;
        let has_z7 = has_z7(&connection, "tiles")?;
        ensure!(
            water.as_ref().is_none_or(|water| [water.l3_zoom, water.l4_zoom] == [l3_zoom, l4_zoom] && water.has_z7 == has_z7),
            "陸地と水域のズーム構成が一致していません"
        );
        let left = bounds((8, xmin, ymin));
        let right = bounds((8, xmax, ymax));
        Ok(Self {
            connection,
            extent: [left[0], left[1], right[2], right[3]],
            cache: HashMap::new(),
            format,
            water,
            reads_water_table: false,
            forest_underlay: metadata["forestUnderlay"] == "source-lpf-gray4-xor",
            unified_forest: matches!(
                metadata["landEncoding"].as_str(),
                Some("forest-bare-unified-gray4" | "forest-bare-unified-gray2")
            ),
            default_bare_on_top: metadata["compositeOrder"] == "forest-water-bare",
            l3_zoom,
            l4_zoom,
            has_z7,
            raster_cache: HashMap::new(),
        })
    }

    fn open_single_water(path: &Path, combined: bool) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let metadata: String =
            connection.query_row("SELECT value FROM metadata WHERE key='manifest'", [], |r| {
                r.get(0)
            })?;
        let metadata: Value = serde_json::from_str(&metadata)?;
        ensure!(
            metadata["format"] == "PNG"
                && metadata["layer"] == if combined { "combined" } else { "water" }
                && metadata["worldWidthM"].as_f64() == Some(WORLD)
                && metadata["tilePixels"] == ASTC_SIDE,
            "Invalid water layer"
        );
        let extent_sql = if combined {
            "SELECT min(x),min(y),max(x),max(y) FROM water_tiles WHERE z=8"
        } else {
            "SELECT min(x),min(y),max(x),max(y) FROM tiles WHERE z=8"
        };
        let [xmin, ymin, xmax, ymax]: [i32; 4] = connection.query_row(extent_sql, [], |r| {
            Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?])
        })?;
        let table = if combined { "water_tiles" } else { "tiles" };
        let [l3_zoom, l4_zoom] = landcover_l3_l4(&connection, table)?;
        let has_z7 = has_z7(&connection, table)?;
        let a = bounds((8, xmin, ymin));
        let b = bounds((8, xmax, ymax));
        Ok(Self {
            connection,
            extent: [a[0], a[1], b[2], b[3]],
            format: Format::Png,
            water: None,
            reads_water_table: combined,
            forest_underlay: false,
            unified_forest: false,
            default_bare_on_top: false,
            l3_zoom,
            l4_zoom,
            has_z7,
            cache: HashMap::new(),
            raster_cache: HashMap::new(),
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

    fn raster_tile(
        &mut self,
        key: Key,
        split_layers: [bool; 2],
        bare_overlay: bool,
    ) -> Result<Option<Arc<[u8]>>> {
        use rusqlite::OptionalExtension;
        let split_layers = if matches!(self.format, Format::LandSplitGray4) {
            split_layers
        } else {
            [true; 2]
        };
        let cache_key = (key, split_layers, bare_overlay);
        if !self.raster_cache.contains_key(&cache_key) {
            if self.raster_cache.len() >= 512 {
                self.raster_cache.clear();
            }
            let (z, x, y) = key;
            let payload: Option<Vec<u8>> = self
                .connection
                .query_row(
                    if self.reads_water_table {
                        "SELECT payload FROM water_tiles WHERE z=? AND x=? AND y=?"
                    } else {
                        "SELECT payload FROM tiles WHERE z=? AND x=? AND y=?"
                    },
                    params![z, x, y],
                    |row| row.get(0),
                )
                .optional()?;
            let pixels = payload
                .as_deref()
                .map(|payload| match self.format {
                    Format::Png => decode_png(payload),
                    Format::PngOrWebp => decode_land_image(payload),
                    Format::LandMixIndex => decode_land_mix(payload),
                    Format::LandSplitGray4 => {
                        let bare: Vec<u8> = self.connection.query_row(
                            "SELECT payload FROM bare_tiles WHERE z=? AND x=? AND y=?",
                            params![z, x, y],
                            |row| row.get(0),
                        )?;
                        let aux_payload: Option<Vec<u8>> = if self.unified_forest {
                            self.connection.query_row(
                                "SELECT payload FROM land_missing_tiles WHERE z=? AND x=? AND y=?",
                                params![z, x, y],
                                |row| row.get(0),
                            ).optional()?
                        } else if self.forest_underlay && split_layers == [true, false] {
                            self.connection.query_row(
                                "SELECT payload FROM forest_underlay_tiles WHERE z=? AND x=? AND y=?",
                                params![z, x, y],
                                |row| row.get(0),
                            ).optional()?
                        } else {
                            None
                        };
                        let aux = if self.unified_forest {
                            SplitLandAux::Unified(aux_payload.as_deref())
                        } else {
                            SplitLandAux::Legacy(aux_payload.as_deref())
                        };
                        decode_land_split(payload, &bare, aux, split_layers, bare_overlay)
                    }
                    Format::Astc => decode_astc(payload),
                    Format::Vector => unreachable!("vector tiles are decoded separately"),
                })
                .transpose()
                .with_context(|| format!("地表画像 z{z}/{x}/{y}"))?;
            self.raster_cache.insert(cache_key, pixels.map(Arc::from));
        }
        Ok(self.raster_cache[&cache_key].clone())
    }

    pub fn image(&mut self, request: Request) -> Result<Image> {
        let z = request.level;
        ensure!(LEVELS.contains(&z) || z == 10, "Invalid landcover level");
        if self.water.is_some() {
            let bare_on_top = self.unified_forest
                && request.bare_on_top
                && request.visible_layers == [true, true]
                && request.split_layers[1];
            let mut image = if request.visible_layers[0] {
                self.rasterize(request)?
            } else {
                Image::empty(request)
            };
            if !request.visible_layers[1] {
                image.water_z = Some(request.water_level);
                return Ok(image);
            }
            let water = self.water.as_mut().unwrap();
            let mut water_request = request;
            water_request.level = request.water_level;
            water_request.water_layer = true;
            let mut overlay = water.rasterize(water_request)?;
            if !request.visible_layers[0] {
                overlay.z = z;
                overlay.request = request;
                overlay.water_z = Some(request.water_level);
                return Ok(overlay);
            }
            let bare = if bare_on_top {
                let mut bare_request = request;
                bare_request.bare_overlay = true;
                bare_request.water_layer = true;
                Some(self.rasterize(bare_request)?)
            } else {
                None
            };
            composite_water(
                &mut image.pixels,
                &overlay.pixels,
                bare.as_ref().map(|image| image.pixels.as_slice()),
            );
            image.water_z = Some(request.water_level);
            return Ok(image);
        }
        if !matches!(self.format, Format::Vector) {
            return self.rasterize(request);
        }
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
            water_z: None,
            pixels,
        })
    }

    fn rasterize(&mut self, request: Request) -> Result<Image> {
        let z = request.level;
        let mut image = Image::empty(request);
        let side = WORLD / f64::from(1u32 << z);
        let index = |v: f64| (v / side).floor().clamp(0., f64::from((1u32 << z) - 1)) as i32;
        let [left, top, right, bottom] = request.bounds;
        let [extent_left, extent_top, extent_right, extent_bottom] = self.extent;
        for y in index(top.max(extent_top))..=index(bottom.min(extent_bottom)) {
            for x in index(left.max(extent_left))..=index(right.min(extent_right)) {
                let key = (z, x, y);
                let tile_bounds = bounds(key);
                if !matches!(self.format, Format::Vector) {
                    let area = image.raster_area(tile_bounds);
                    if area[0] < area[2] && area[1] < area[3] {
                        if let Some(tile) =
                            self.raster_tile(key, request.split_layers, request.bare_overlay)?
                        {
                            let mut neighbors = [const { None }; 9];
                            neighbors[4] = Some(tile);
                            if request.bilinear {
                                for (i, slot) in neighbors.iter_mut().enumerate() {
                                    if i != 4 {
                                        *slot = self.raster_tile(
                                            (z, x + (i % 3) as i32 - 1, y + (i / 3) as i32 - 1),
                                            request.split_layers,
                                            request.bare_overlay,
                                        )?;
                                    }
                                }
                            }
                            image.blit_raster(tile_bounds, area, &neighbors);
                        }
                    }
                    continue;
                }
                let Some(tile) = self.tile(key)? else {
                    continue;
                };
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

fn composite_water(land: &mut [u8], water: &[u8], bare_on_top: Option<&[u8]>) {
    for (i, (land, water)) in land
        .chunks_exact_mut(4)
        .zip(water.chunks_exact(4))
        .enumerate()
    {
        let alpha = i32::from(water[3]);
        if land[3] == 0 {
            land.copy_from_slice(water);
        } else if alpha != 0 {
            let overlap = bare_on_top.map_or(0, |bare| i32::from(bare[i * 4 + 3]) * alpha);
            for c in 0..3 {
                let top = i32::from(water[c]);
                let base = i32::from(land[c]);
                // Swapping two source-over layers changes only their overlap.
                let correction = (i32::from(PALETTE[2][c]) - top) * overlap;
                let color =
                    ((base * (255 - alpha) + top * alpha) * 255 + correction + 32512) / 65025;
                land[c] = color.clamp(0, 255) as u8;
            }
        }
    }
}

fn sample_raster(
    tiles: &[Option<Arc<[u8]>>; 9],
    (x, fx): (i32, f32),
    (y, fy): (i32, f32),
    water_layer: bool,
    bilinear: bool,
) -> [u8; 4] {
    let side = ASTC_SIDE as i32;
    let texel = |x: i32, y: i32| {
        let (tile, sx, sy) = if (0..side).contains(&x) && (0..side).contains(&y) {
            (&tiles[4], x, y)
        } else {
            let index = ((y.div_euclid(side) + 1) * 3 + x.div_euclid(side) + 1) as usize;
            if tiles[index].is_some() {
                (&tiles[index], x.rem_euclid(side), y.rem_euclid(side))
            } else {
                (&tiles[4], x.clamp(0, side - 1), y.clamp(0, side - 1))
            }
        };
        let offset = (sy as usize * ASTC_SIDE + sx as usize) * 4;
        &tile.as_deref().expect("center raster tile")[offset..offset + 4]
    };
    let nearest = texel(x + i32::from(fx >= 0.5), y + i32::from(fy >= 0.5));
    if !bilinear {
        return nearest.try_into().expect("RGBA raster texel");
    }
    // Keep the binary coverage of the nearest texel, while interpolating color
    // over the known samples so missing pixels cannot darken valid colors.
    if !water_layer && nearest[3] == 0 {
        return [0; 4];
    }
    let mut rgb = [0.; 3];
    let mut weight = 0.;
    for (dx, wx) in [(0, 1. - fx), (1, fx)] {
        for (dy, wy) in [(0, 1. - fy), (1, fy)] {
            let texel = texel(x + dx, y + dy);
            if texel[3] != 0 {
                let w = wx * wy * f32::from(texel[3]) / 255.;
                weight += w;
                for (color, &channel) in rgb.iter_mut().zip(texel) {
                    *color += f32::from(channel) * w;
                }
            }
        }
    }
    if weight == 0. {
        return [0; 4];
    }
    let [r, g, b] = rgb.map(|c| (c / weight).round() as u8);
    [
        r,
        g,
        b,
        if water_layer {
            (weight * 255.).round() as u8
        } else {
            255
        },
    ]
}

fn is_land(pixel: &[u8]) -> bool {
    pixel[3] != 0 && pixel != PALETTE[3]
}

fn palette_distance(pixel: &[u8], color: [u8; 4]) -> i32 {
    (0..3)
        .map(|i| (i32::from(pixel[i]) - i32::from(color[i])).pow(2))
        .sum()
}

fn soften(pixels: &mut [u8], [width, height]: [usize; 2], [rx, ry]: [usize; 2]) {
    if rx == 0 && ry == 0 {
        return;
    }
    fn pass(
        src: &[u8],
        dst: &mut [u8],
        land: &[bool],
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
                if land[p / 4] {
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
                if land[p / 4] {
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
    let mask: Vec<_> = pixels.chunks_exact(4).map(is_land).collect();
    let mut horizontal = vec![0; pixels.len()];
    pass(
        pixels,
        &mut horizontal,
        &mask,
        height,
        width,
        width * 4,
        4,
        rx,
    );
    pass(&horizontal, pixels, &mask, width, height, 4, width * 4, ry);
}

fn auto_level(metres: f64, l3_zoom: u32, l4_zoom: u32, has_z7: bool) -> u32 {
    if metres < if l3_zoom == 10 { 200. } else { 100. } {
        11
    } else if metres < if l4_zoom == 10 { 300. } else { 400. } {
        9
    } else if metres < 1200. {
        8
    } else if has_z7 && metres < 2400. {
        7
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

const LEVELS: [u32; 5] = [11, 9, 8, 7, 6];
// Projected metres, matching the L3–L6 generator fit tolerances.
const FIT_TOLERANCE_M: [u32; 5] = [200, 500, 2000, 3000, 4000];
const DEFAULT_BLUR_M: [u32; 5] = [100, 250, 1000, 1500, 2000];

fn level_index(z: u32) -> usize {
    LEVELS
        .iter()
        .position(|&level| level == if z == 10 { 9 } else { z })
        .expect("supported landcover level")
}

pub struct Style {
    level: Option<u32>,
    water_level: Option<u32>,
    pub l3_zoom: u32,
    pub l4_zoom: u32,
    pub has_z7: bool,
    visible_layers: [bool; 2],
    split_layers: [bool; 2],
    pub bare_on_top: bool,
    quality: f64,
    blur: [u32; LEVELS.len()],
    pub animate: bool,
    pub bilinear: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            level: None,
            water_level: None,
            l3_zoom: 11,
            l4_zoom: 9,
            has_z7: false,
            visible_layers: [true; 2],
            split_layers: [true; 2],
            bare_on_top: false,
            quality: 1.,
            blur: DEFAULT_BLUR_M,
            animate: true,
            bilinear: true,
        }
    }
}

impl Style {
    fn actual_zoom(&self, zoom: u32) -> u32 {
        match zoom {
            11 => self.l3_zoom,
            9 => self.l4_zoom,
            _ => zoom,
        }
    }

    pub fn request(&self, center: Point, size: [usize; 2], metres: f64) -> Request {
        let mut request = Request::around(
            center,
            size,
            metres,
            Some(
                self.actual_zoom(
                    self.level
                        .filter(|&zoom| zoom != 7 || self.has_z7)
                        .unwrap_or_else(|| auto_level(metres / self.quality, self.l3_zoom, self.l4_zoom, self.has_z7)),
                ),
            ),
        );
        request.water_level = self.actual_zoom(
            self.water_level
                .filter(|&zoom| zoom != 7 || self.has_z7)
                .unwrap_or_else(|| auto_level(metres / self.quality / 2., self.l3_zoom, self.l4_zoom, self.has_z7)),
        );
        request.visible_layers = self.visible_layers;
        request.split_layers = self.split_layers;
        request.bare_on_top = self.bare_on_top;
        request.blur = self.blur[level_index(if request.level == self.l3_zoom { 11 } else { request.level })];
        request.overview_blur = self.blur[level_index(6)];
        request.bilinear = self.bilinear;
        request
    }

    pub fn level_picker(
        &mut self,
        ui: &mut egui::Ui,
        layered: bool,
        split_land: bool,
        unified_land: bool,
    ) {
        if split_land {
            ui.checkbox(&mut self.visible_layers[0], "陸地");
            ui.add_enabled_ui(self.visible_layers[0], |ui| {
                ui.checkbox(&mut self.split_layers[0], "森林");
                ui.checkbox(&mut self.split_layers[1], "砂礫地");
            });
        } else if layered {
            ui.checkbox(&mut self.visible_layers[0], "森林・砂礫地");
        } else {
            ui.label("地表レベル");
        }
        ui.selectable_value(&mut self.level, None, "自動");
        let has_z7 = self.has_z7;
        let zooms = || LEVELS.into_iter().filter(move |&z| z != 7 || has_z7);
        for (i, z) in zooms().enumerate() {
            let label = format!("L{} / z{}", i + 3, self.actual_zoom(z));
            ui.selectable_value(&mut self.level, Some(z), label);
        }
        if unified_land {
            ui.checkbox(&mut self.bare_on_top, "砂礫地を水域の上に表示");
        }
        if layered {
            ui.separator();
            ui.checkbox(&mut self.visible_layers[1], "水域");
            ui.selectable_value(&mut self.water_level, None, "自動");
            for (i, z) in zooms().enumerate() {
                let label = format!("L{} / z{}", i + 3, self.actual_zoom(z));
                ui.selectable_value(&mut self.water_level, Some(z), label);
            }
        }
    }

    pub fn controls(&mut self, ui: &mut egui::Ui, supports_blur: bool) {
        ui.add_enabled(
            self.level.is_none(),
            egui::Slider::new(&mut self.quality, 0.5..=4.)
                .step_by(0.25)
                .text("地表LOD精細度"),
        );
        if supports_blur {
            let zooms = LEVELS.map(|zoom| self.actual_zoom(zoom));
            ui.menu_button("陸地ぼかし", |ui| {
                for (i, (radius, z)) in self.blur.iter_mut().zip(zooms).enumerate() {
                    if z == 7 && !self.has_z7 {
                        continue;
                    }
                    let level = if z == 6 && !self.has_z7 { i + 2 } else { i + 3 };
                    ui.add(
                        egui::Slider::new(radius, 0..=FIT_TOLERANCE_M[i])
                            .suffix(" m")
                            .text(format!("L{level} / z{z} 半径")),
                    );
                }
            });
        }
        ui.checkbox(&mut self.animate, "地表LODを滑らかに切替");
        ui.checkbox(&mut self.bilinear, "地表バイリニア補間");
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Request {
    pub bounds: Bounds,
    pub size: [usize; 2],
    level: u32,
    water_level: u32,
    water_layer: bool,
    pub visible_layers: [bool; 2],
    split_layers: [bool; 2],
    bare_overlay: bool,
    bare_on_top: bool,
    blur: u32,
    overview_blur: u32,
    bilinear: bool,
}

pub type LayerVisibility = ([bool; 2], [bool; 2], bool);

impl Request {
    pub fn visibility(self) -> LayerVisibility {
        (self.visible_layers, self.split_layers, self.bare_on_top)
    }

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
        let level = level.unwrap_or_else(|| auto_level(metres_per_pixel, 11, 9, false));
        Self {
            bounds: [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ],
            size,
            level,
            water_level: level,
            water_layer: false,
            visible_layers: [true; 2],
            split_layers: [true; 2],
            bare_overlay: false,
            bare_on_top: false,
            blur: DEFAULT_BLUR_M[level_index(level)],
            overview_blur: DEFAULT_BLUR_M[level_index(6)],
            bilinear: true,
        }
    }
}

pub struct Image {
    pub request: Request,
    pub z: u32,
    pub water_z: Option<u32>,
    pub pixels: Vec<u8>,
}

impl Image {
    fn empty(request: Request) -> Self {
        Self {
            request,
            z: request.level,
            water_z: None,
            pixels: vec![0; request.size[0] * request.size[1] * 4],
        }
    }

    pub fn premultiply_for_display(&mut self) {
        for pixel in self.pixels.chunks_exact_mut(4) {
            if pixel[3] < 255 {
                let alpha = u16::from(pixel[3]);
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                }
            }
        }
    }

    pub fn lod(&self) -> u32 {
        self.water_z.map_or(self.z, |z| self.z * 16 + z)
    }
    pub fn status(&self) -> String {
        self.water_z.map_or_else(
            || format!("地表 z{}", self.z),
            |z| format!("森林・砂礫地 z{} ／ 水域 z{z}", self.z),
        )
    }

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

    fn raster_area(&self, [l, t, r, b]: Bounds) -> [usize; 4] {
        let [l, t] = self.pixel(l, t);
        let [r, b] = self.pixel(r, b);
        let [w, h] = self.request.size;
        [
            Self::first_pixel(l, w),
            Self::first_pixel(t, h),
            Self::first_pixel(r, w),
            Self::first_pixel(b, h),
        ]
    }

    fn blit_raster(
        &mut self,
        [l, t, r, b]: Bounds,
        [x0, y0, x1, y1]: [usize; 4],
        tiles: &[Option<Arc<[u8]>>; 9],
    ) {
        let [l, t] = self.pixel(l, t);
        let [r, b] = self.pixel(r, b);
        let sample = |pixel: usize, start: f64, end: f64| {
            let position = ((pixel as f64 + 0.5 - start) * ASTC_SIDE as f64 / (end - start))
                .clamp(0., ASTC_SIDE as f64)
                - 0.5;
            let base = position.floor();
            (base as i32, (position - base) as f32)
        };
        let width = self.request.size[0];
        let columns: Vec<_> = (x0..x1).map(|x| sample(x, l, r)).collect();
        for y in y0..y1 {
            let row = sample(y, t, b);
            let dst = (y * width + x0) * 4;
            for (pixel, &column) in self.pixels[dst..dst + columns.len() * 4]
                .chunks_exact_mut(4)
                .zip(&columns)
            {
                pixel.copy_from_slice(&sample_raster(
                    tiles, column, row, self.request.water_layer, self.request.bilinear,
                ));
            }
        }
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
    Extent {
        bounds: Bounds,
        supports_blur: bool,
        layered: bool,
        split_land: bool,
        unified_land: bool,
        default_bare_on_top: bool,
        l3_zoom: u32,
        l4_zoom: u32,
        has_z7: bool,
    },
    Overview(Image),
    Detail(Image),
    Error(String),
}

pub struct Loader {
    pub extent: Option<Bounds>,
    pub supports_blur: bool,
    pub layered: bool,
    pub split_land: bool,
    pub unified_land: bool,
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
                send(Message::Extent {
                    bounds: database.extent,
                    supports_blur: matches!(database.format, Format::Vector),
                    layered: database.water.is_some(),
                    split_land: matches!(database.format, Format::LandSplitGray4),
                    unified_land: database.unified_forest,
                    default_bare_on_top: database.default_bare_on_top,
                    l3_zoom: database.l3_zoom,
                    l4_zoom: database.l4_zoom,
                    has_z7: database.has_z7,
                });
                let mut overview_request = overview.then(|| {
                    let side = (r - l).max(b - t) * 1.04;
                    let mut request = Request::around(
                        [(l + r) / 2., (t + b) / 2.],
                        [1024, 1024],
                        side / 1024.,
                        Some(6),
                    );
                    request.bare_on_top = database.default_bare_on_top;
                    request
                });
                if let Some(request) = overview_request {
                    send(Message::Overview(database.image(request)?));
                }
                while let Ok(mut request) = requests.recv() {
                    while let Ok(next) = requests.try_recv() {
                        request = next;
                    }
                    if let Some(base) = &mut overview_request {
                        if base.blur != request.overview_blur
                            || base.visibility() != request.visibility()
                            || base.bilinear != request.bilinear
                        {
                            base.blur = request.overview_blur;
                            base.visible_layers = request.visible_layers;
                            base.split_layers = request.split_layers;
                            base.bare_on_top = request.bare_on_top;
                            base.bilinear = request.bilinear;
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
            supports_blur: false,
            layered: false,
            split_land: false,
            unified_land: false,
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

const MAP_BACKGROUND: u8 = 100;

fn map_color_image(mut image: Image) -> egui::ColorImage {
    for pixel in image.pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel =
                ((u16::from(*channel) * alpha + u16::from(MAP_BACKGROUND) * (255 - alpha) + 127)
                    / 255) as u8;
        }
        pixel[3] = 255;
    }
    egui::ColorImage::from_rgba_premultiplied(image.request.size, &image.pixels)
}

fn map_fade_tint(opacity: f32) -> egui::Color32 {
    // egui's gamma framebuffer blends premultiplied RGB directly.
    egui::Color32::WHITE.gamma_multiply(opacity)
}

type MapTexture = (egui::TextureHandle, Bounds, u32, LayerVisibility);

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
                    Message::Extent {
                        bounds,
                        supports_blur,
                        layered,
                        split_land,
                        unified_land,
                        default_bare_on_top,
                        l3_zoom,
                        l4_zoom,
                        has_z7,
                    } => {
                        self.loader.extent = Some(bounds);
                        self.loader.supports_blur = supports_blur;
                        self.loader.layered = layered;
                        self.loader.split_land = split_land;
                        self.loader.unified_land = unified_land;
                        self.style.bare_on_top = default_bare_on_top;
                        self.style.l3_zoom = l3_zoom;
                        self.style.l4_zoom = l4_zoom;
                        self.style.has_z7 = has_z7;
                    }
                    Message::Detail(image) => next = Some(image),
                    Message::Error(error) => self.status = error,
                    Message::Overview(_) => {}
                }
            }
            if let Some(image) = next {
                let bounds = image.request.bounds;
                let lod = image.lod();
                let visible_layers = image.request.visibility();
                self.status = format!("{} ／ ESA WorldCover 2021", image.status());
                // Flatten each LOD over the map background before fading,
                // so partially covered water is not applied twice.
                let filter = if image.request.bilinear {
                    egui::TextureOptions::LINEAR
                } else {
                    egui::TextureOptions::NEAREST
                };
                let texture = ctx.load_texture("地表", map_color_image(image), filter);
                if self
                    .texture
                    .as_ref()
                    .is_some_and(|entry| entry.3 != visible_layers)
                {
                    self.previous = None;
                    self.transition = None;
                }
                if self.style.animate
                    && self
                        .texture
                        .as_ref()
                        .is_some_and(|entry| entry.2 != lod && entry.3 == visible_layers)
                {
                    self.previous = self.texture.take();
                    self.transition = Some(std::time::Instant::now());
                }
                self.texture = Some((texture, bounds, lod, visible_layers));
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
            });
            ui.horizontal_wrapped(|ui| {
                self.style.level_picker(
                    ui,
                    self.loader.layered,
                    self.loader.split_land,
                    self.loader.unified_land,
                );
                self.style.controls(ui, self.loader.supports_blur);
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
                .rect_filled(rect, 0., egui::Color32::from_gray(MAP_BACKGROUND));
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
            if let Some((texture, bounds, _, _)) = &self.texture {
                let area = screen_bounds(*bounds);
                if blend < 1. {
                    if let Some((previous, bounds, _, _)) = &self.previous {
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
    fn astc_bilinear_crosses_tile_edges_without_mixing_missing_color() {
        let solid = |color: [u8; 4]| Arc::from(color.repeat(ASTC_SIDE * ASTC_SIDE));
        let mut tiles = [const { None }; 9];
        tiles[4] = Some(solid([60, 80, 100, 255]));
        tiles[5] = Some(solid([100, 120, 140, 255]));
        let edge = ASTC_SIDE as i32 - 1;
        assert_eq!(
            sample_raster(&tiles, (edge, 0.5), (4, 0.25), false, true),
            [80, 100, 120, 255]
        );
        tiles[5] = Some(solid([0; 4]));
        assert_eq!(
            sample_raster(&tiles, (edge, 0.25), (4, 0.25), false, true),
            [60, 80, 100, 255]
        );
        assert_eq!(
            sample_raster(&tiles, (edge, 0.75), (4, 0.25), false, true),
            [0; 4]
        );
    }

    #[test]
    fn nearest_raster_preserves_source_colors_and_water_coverage() {
        let mut tiles = [const { None }; 9];
        let mut center = vec![0; ASTC_SIDE * ASTC_SIDE * 4];
        center[..8].copy_from_slice(&[60, 80, 100, 255, 100, 120, 140, 85]);
        tiles[4] = Some(Arc::from(center));
        assert_eq!(
            sample_raster(&tiles, (0, 0.25), (0, 0.), true, false),
            [60, 80, 100, 255]
        );
        assert_eq!(
            sample_raster(&tiles, (0, 0.75), (0, 0.), true, false),
            [100, 120, 140, 85]
        );
        assert_eq!(
            sample_raster(&tiles, (1, 0.75), (0, 0.), false, false),
            [0; 4]
        );
    }

    #[test]
    fn astc_blocks_decode_to_rgba_and_preserve_missing_pixels() {
        fn constant([r, g, b, a]: [u8; 4]) -> [u8; 16] {
            [
                0xfc, 0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, r, r, g, g, b, b, a, a,
            ]
        }
        let mut payload = constant([24, 80, 160, 255]).repeat(ASTC_BYTES / 16);
        payload[16..32].copy_from_slice(&constant([22, 33, 44, 0]));
        let image = decode_astc(&payload).unwrap();
        assert_eq!(&image[..4], &[24, 80, 160, 255]);
        assert_eq!(&image[8 * 4..9 * 4], &[0; 4]);
        assert!(decode_astc(&payload[..16]).is_err());
    }

    #[test]
    fn land_and_water_levels_are_independent() {
        let mut style = Style::default();
        let request = style.request([0.; 2], [32; 2], 150.);
        assert_eq!((request.level, request.water_level), (9, 11));
        style.l4_zoom = 10;
        let request = style.request([0.; 2], [32; 2], 150.);
        assert_eq!((request.level, request.water_level), (10, 11));
        let request = style.request([0.; 2], [32; 2], 350.);
        assert_eq!((request.level, request.water_level), (8, 10));
        style.level = Some(6);
        style.water_level = Some(11);
        let request = style.request([0.; 2], [32; 2], 1000.);
        assert_eq!((request.level, request.water_level), (6, 11));
    }

    #[test]
    fn compact_zooms_map_fixed_and_automatic_l3_to_z10() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE tiles(z INTEGER); INSERT INTO tiles VALUES (10),(9),(8),(6)").unwrap();
        let [l3_zoom, l4_zoom] = landcover_l3_l4(&db, "tiles").unwrap();
        assert_eq!([l3_zoom, l4_zoom], [10, 9]);
        let mut style = Style { l3_zoom, l4_zoom, blur: [10, 20, 30, 35, 40], ..Style::default() };
        assert_eq!(style.request([0.; 2], [32; 2], 150.).level, 10);
        assert_eq!(style.request([0.; 2], [32; 2], 150.).blur, 10);
        style.level = Some(9);
        assert_eq!(style.request([0.; 2], [32; 2], 150.).level, 9);
        db.execute("INSERT INTO tiles VALUES (11)", []).unwrap();
        assert_eq!(landcover_l3_l4(&db, "tiles").unwrap(), [11, 10]);
    }

    #[test]
    fn optional_z7_is_available_for_fixed_and_automatic_selection() {
        let mut style = Style::default();
        let request = |style: &Style, metres| style.request([0.; 2], [32; 2], metres);
        assert_eq!(request(&style, 1800.).level, 6);
        style.has_z7 = true;
        assert_eq!((request(&style, 1800.).level, request(&style, 1800.).water_level), (7, 8));
        assert_eq!(request(&style, 2600.).level, 6);
        style.level = Some(7);
        style.water_level = Some(7);
        assert_eq!((request(&style, 300.).level, request(&style, 300.).water_level), (7, 7));
        style.has_z7 = false;
        assert_eq!(request(&style, 300.).level, 9);
    }

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
        style.blur = [0, 1, 2, 3, 5];
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
    fn half_covered_water_does_not_brighten_the_edge() {
        let mut image = Image::empty(Request::around([0.; 2], [1; 2], 1., Some(6)));
        image.pixels.copy_from_slice(&[114, 172, 204, 128]);
        let color = map_color_image(image);
        assert_eq!(color.pixels[0].to_array(), [107, 136, 152, 255]);
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
    let mut overview_request = Request::around(center, [800, 800], side / 800., Some(6));
    overview_request.bare_on_top = database.default_bare_on_top;
    let overview = database.image(overview_request)?;
    let fuji = crate::selection::lonlat(138.727, 35.36);
    let mut detail_request = Request::around(fuji, [768, 768], 30., Some(database.l3_zoom));
    detail_request.bare_on_top = database.default_bare_on_top;
    let detail = database.image(detail_request)?;
    println!(
        "Default composite order: {}",
        if database.default_bare_on_top {
            "forest-water-bare"
        } else {
            "forest-bare-water"
        }
    );
    if database.water.is_some() {
        let mut request = Request::around(fuji, [512, 512], 250., Some(database.l4_zoom));
        let l4 = database.image(request)?;
        ensure!(
            l4.pixels.chunks_exact(4).any(|p| p[3] != 0),
            "Empty L4 image"
        );
        println!("Land and water L4 decoded at z{}", l4.z);
        if database.has_z7 {
            let z7 = database.image(Request::around(fuji, [512, 512], 1200., Some(7)))?;
            ensure!(z7.z == 7 && z7.water_z == Some(7) && z7.pixels.chunks_exact(4).any(|p| p[3] != 0),
                "Empty z7 image");
        }
        request.level = 8;
        request.water_level = 6;
        let coarse_water = database.image(request)?;
        request.water_level = database.l3_zoom;
        let fine_water = database.image(request)?;
        ensure!(
            coarse_water.pixels != fine_water.pixels,
            "Water LOD had no effect"
        );
        ensure!(
            coarse_water.lod() != fine_water.lod(),
            "Water LOD did not start a transition"
        );
        request.level = 6;
        let coarse_land = database.image(request)?;
        ensure!(
            coarse_land.pixels != fine_water.pixels,
            "Land LOD had no effect"
        );
        request.visible_layers = [true, false];
        let land = database.image(request)?;
        ensure!(
            land.pixels == database.rasterize(request)?.pixels,
            "Land-only image changed"
        );
        request.visible_layers = [false, true];
        let water = database.image(request)?;
        ensure!(
            water.pixels.chunks_exact(4).any(|pixel| pixel[3] > 0)
                && water
                    .pixels
                    .chunks_exact(4)
                    .all(|pixel| pixel[3] == 0 || pixel[..3] == PALETTE[3][..3]),
            "Water-only image contains land colors"
        );
        request.visible_layers = [false, false];
        ensure!(
            database.image(request)?.pixels.iter().all(|&v| v == 0),
            "Hidden layers left visible pixels"
        );
        println!("Independent land/water LOD and visibility verified");
        if matches!(database.format, Format::LandSplitGray4) {
            let mut request = Request::around(fuji, [768, 768], 30., Some(database.l3_zoom));
            request.visible_layers = [true, false];
            let all = database.image(request)?;
            request.split_layers = [false, true];
            let no_forest = database.image(request)?;
            request.split_layers = [true, false];
            let no_bare = database.image(request)?;
            request.split_layers = [false, false];
            let background = database.image(request)?;
            request.split_layers = [true, true];
            ensure!(
                all.pixels != no_forest.pixels
                    && all.pixels != no_bare.pixels
                    && no_forest.pixels != background.pixels
                    && no_bare.pixels != background.pixels
                    && all.pixels == database.image(request)?.pixels,
                "Split land visibility had no effect or reused the wrong cache entry"
            );
            ensure!(
                all.pixels
                    .chunks_exact(4)
                    .zip(background.pixels.chunks_exact(4))
                    .all(|(a, b)| { a[3] == b[3] && (b[3] == 0 || b == PALETTE[0]) }),
                "Hidden forest and bare changed the base color or coverage"
            );
            println!("Independent forest and bare visibility verified");
            if database.unified_forest {
                let mut differences = 0;
                for (name, center) in [
                    ("Fuji", fuji),
                    (
                        "Kiso river mouth",
                        crate::selection::lonlat(136.718, 35.046),
                    ),
                    ("Tokyo Bay", crate::selection::lonlat(139.839, 35.598)),
                ] {
                    let mut comparison = Request::around(center, [512, 512], 15., Some(database.l3_zoom));
                    comparison.water_level = database.l3_zoom;
                    let ordinary = database.image(comparison)?;
                    comparison.bare_on_top = true;
                    let reversed = database.image(comparison)?;
                    let changed = ordinary
                        .pixels
                        .chunks_exact(4)
                        .zip(reversed.pixels.chunks_exact(4))
                        .filter(|(a, b)| a != b)
                        .count();
                    differences += changed;
                    println!("{name}: layer order changed {changed} of 262144 pixels");
                    comparison.visible_layers[1] = false;
                    let dry = database.image(comparison)?;
                    comparison.bare_on_top = false;
                    ensure!(
                        dry.pixels == database.image(comparison)?.pixels,
                        "Order changed land-only display"
                    );
                    comparison.visible_layers[1] = true;
                    comparison.split_layers[1] = false;
                    let no_bare = database.image(comparison)?;
                    comparison.bare_on_top = true;
                    ensure!(
                        no_bare.pixels == database.image(comparison)?.pixels,
                        "Order affected hidden bare layer"
                    );
                }
                ensure!(
                    differences != 0,
                    "Layer order had no effect on comparison areas"
                );
            }
        }
    }
    for image in [overview, detail] {
        let mut counts = [0; PALETTE.len()];
        for pixel in image.pixels.chunks_exact(4).filter(|p| p[3] != 0) {
            let index = match database.format {
                Format::Vector => PALETTE.iter().position(|color| pixel == color),
                Format::Astc
                | Format::Png
                | Format::PngOrWebp
                | Format::LandMixIndex
                | Format::LandSplitGray4 => {
                    (0..PALETTE.len()).min_by_key(|&i| palette_distance(pixel, PALETTE[i]))
                }
            };
            if let Some(index) = index {
                counts[index] += 1;
            }
        }
        ensure!(
            counts.iter().filter(|&&n| n > 0).count() >= 3,
            "Missing classes at z{}: {counts:?}",
            image.z
        );
        println!("Landcover z{}: {counts:?}", image.z);
    }
    Ok(())
}
