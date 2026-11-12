use anyhow::{Context, Result, ensure};
use flate2::read::MultiGzDecoder;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
};

const INDEX_DEPTH: u32 = 20;
const AXIS: f64 = (1u32 << INDEX_DEPTH) as f64;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub id: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub elevation_m: Option<f64>,
    pub name_reading: Option<NameReading>,
    pub aliases: Option<Vec<Alias>>,
    pub tags: Option<Vec<String>>,
    pub related_ids: Option<Vec<String>>,
    pub provenance: Option<serde_json::Value>,
    pub graphic: Option<Graphic>,
    pub osm_id: Option<serde_json::Number>,
    #[serde(skip)]
    pub original: Option<Box<RawValue>>,
}

pub const PROVENANCE_FIELDS: [(&str, &str); 3] = [
    ("dataSource", "出典"),
    ("classification", "取得元での分類"),
    ("elevationSource", "標高の出典"),
];

impl Point {
    pub fn provenance_codes(&self, field: &str) -> impl Iterator<Item = &str> {
        self.provenance
            .as_ref()
            .and_then(|value| value.get(field))
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .split(';')
            .map(str::trim)
    }
}

#[derive(Deserialize)]
pub struct Graphic {
    pub svg: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Alias {
    Name(String),
    Detail {
        name: String,
        #[serde(rename = "nameReading")]
        name_reading: Option<NameReading>,
    },
}

impl Alias {
    pub fn name(&self) -> &str {
        match self {
            Self::Name(name) | Self::Detail { name, .. } => name,
        }
    }

    pub fn reading(&self) -> Option<&NameReading> {
        match self {
            Self::Detail { name_reading, .. } => name_reading.as_ref(),
            Self::Name(_) => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum NameReading {
    Single(String),
    Multiple(Vec<String>),
}

impl NameReading {
    pub fn readings(&self) -> &[String] {
        match self {
            Self::Single(reading) => std::slice::from_ref(reading),
            Self::Multiple(readings) => readings,
        }
    }

    pub fn display(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Single(reading) => std::borrow::Cow::Borrowed(reading),
            Self::Multiple(readings) => std::borrow::Cow::Owned(readings.join(" / ")),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    schema_format: String,
    schema_version: f64,
    metadata: Option<serde_json::Value>,
    points: Vec<Box<RawValue>>,
}

pub fn project(latitude: f64, longitude: f64) -> [f64; 2] {
    let lat = latitude.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    [
        (longitude / 360. + 0.5).clamp(0., 1.),
        (0.5 - lat.tan().asinh() / std::f64::consts::TAU).clamp(0., 1.),
    ]
}

pub fn unproject([x, y]: [f64; 2]) -> [f64; 2] {
    let latitude = ((0.5 - y) * std::f64::consts::TAU)
        .sinh()
        .atan()
        .to_degrees();
    [latitude, (x - 0.5) * 360.]
}

fn morton([x, y]: [f64; 2]) -> u64 {
    let x = (x * AXIS).min(AXIS - 1.) as u32;
    let y = (y * AXIS).min(AXIS - 1.) as u32;
    (0..INDEX_DEPTH).rev().fold(0, |key, bit| {
        (key << 2) | u64::from((x >> bit & 1) | (y >> bit & 1) << 1)
    })
}

struct Entry {
    source_index: usize,
    key: u64,
    world: [f64; 2],
    point: Point,
}

#[derive(Clone, Copy)]
pub struct Mark {
    pub index: usize,
    pub world: [f64; 2],
    pub matched: bool,
}

pub fn is_sqlite(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["sqlite3", "sqlite", "db"]
                .iter()
                .any(|known| known.eq_ignore_ascii_case(ext))
        })
}

pub struct FileInfo {
    pub format: &'static str,
    pub size_bytes: Option<u64>,
    pub dataset: Option<yamamuki_data_sqlite::Dataset>,
    pub metadata_json: Option<String>,
}

pub struct Points {
    pub info: FileInfo,
    entries: Vec<Entry>,
    pub bounds: [[f64; 2]; 2],
}

impl Points {
    pub fn open(path: &Path) -> Result<Self> {
        if is_sqlite(path) {
            return Self::try_open_sqlite(path)?.context("地点データセットがありません");
        }
        let file = File::open(path)?;
        let size_bytes = file.metadata().ok().map(|metadata| metadata.len());
        let mut points =
            Self::read(file).with_context(|| format!("地点ファイル {}", path.display()))?;
        points.info.size_bytes = size_bytes;
        Ok(points)
    }

    pub fn try_open_sqlite(path: &Path) -> Result<Option<Self>> {
        use yamamuki_data_sqlite as sqlite;
        let db = sqlite::open(path)?;
        let mut datasets = sqlite::datasets(&db)?;
        datasets.retain(|dataset| dataset.kind == "points");
        if datasets.is_empty() {
            return Ok(None);
        }
        let dataset = datasets
            .into_iter()
            .find(|dataset| dataset.format_version == 6)
            .context("地点SQLiteの格納形式の版に対応していません")?;
        let mut statement = db.prepare(
            "SELECT point_id, payload_json FROM point_records WHERE dataset_key = ?1 ORDER BY ordinal",
        )?;
        let mut rows = statement.query([dataset.key])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let original = RawValue::from_string(row.get(1)?)?;
            let point = Self::decode_point(original)?;
            ensure!(point.id == id, "地点SQLiteの地点IDが一致しません");
            entries.push(point);
        }
        let metadata_json = Some(serde_json::to_string_pretty(&dataset.metadata)?);
        Ok(Some(Self::from_points(
            entries,
            FileInfo {
                format: "SQLite コンテナ版1",
                size_bytes: std::fs::metadata(path).ok().map(|metadata| metadata.len()),
                dataset: Some(dataset),
                metadata_json,
            },
        )?))
    }

    pub(crate) fn read(input: impl Read) -> Result<Self> {
        let mut file = BufReader::new(input);
        let compressed = file.fill_buf()?.starts_with(&[0x1f, 0x8b]);
        let input: Box<dyn Read> = if compressed {
            Box::new(MultiGzDecoder::new(file))
        } else {
            Box::new(file)
        };
        let mut input = BufReader::new(input);
        if input.fill_buf()?.starts_with(&[0xef, 0xbb, 0xbf]) {
            input.consume(3);
        }
        let first = loop {
            let buf = input.fill_buf()?;
            let i = buf.iter().position(|b| !b.is_ascii_whitespace());
            if let Some(i) = i {
                let first = buf[i];
                input.consume(i);
                break first;
            }
            ensure!(!buf.is_empty(), "地点JSONが空です");
            let len = buf.len();
            input.consume(len);
        };
        let (points, metadata) = match first {
            b'[' => (
                serde_json::from_reader::<_, Vec<Box<RawValue>>>(input)?,
                None,
            ),
            b'{' => {
                let doc: Document = serde_json::from_reader(input)?;
                ensure!(
                    doc.schema_format == "yamamuki-points" && doc.schema_version == 6.,
                    "地点JSONの形式または版に対応していません"
                );
                (doc.points, doc.metadata)
            }
            _ => anyhow::bail!("地点JSONの配列または版6のオブジェクトが必要です"),
        };
        let metadata_json = metadata
            .as_ref()
            .map(serde_json::to_string_pretty)
            .transpose()?;
        let entries = points
            .into_iter()
            .map(Self::decode_point)
            .collect::<Result<_>>()?;
        Self::from_points(
            entries,
            FileInfo {
                format: match (first == b'[', compressed) {
                    (true, true) => "地点JSON 配列形式 (gzip)",
                    (true, false) => "地点JSON 配列形式",
                    (false, true) => "地点JSON 版6 (gzip)",
                    (false, false) => "地点JSON 版6",
                },
                size_bytes: None,
                dataset: None,
                metadata_json,
            },
        )
    }

    fn decode_point(original: Box<RawValue>) -> Result<Point> {
        let mut point: Point = serde_json::from_str(original.get())?;
        point.original = Some(original);
        ensure!(
            !point.id.trim().is_empty()
                && !point.name.trim().is_empty()
                && (-90.0..=90.0).contains(&point.latitude)
                && (-180.0..=180.0).contains(&point.longitude),
            "地点ID、名称、緯度、経度を確認してください"
        );
        Ok(point)
    }

    fn from_points(points: Vec<Point>, info: FileInfo) -> Result<Self> {
        ensure!(!points.is_empty(), "地点がありません");
        let mut bounds = [[1., 1.], [0., 0.]];
        let mut entries = Vec::with_capacity(points.len());
        for (source_index, point) in points.into_iter().enumerate() {
            let world = project(point.latitude, point.longitude);
            for (axis, value) in world.into_iter().enumerate() {
                bounds[0][axis] = f64::min(bounds[0][axis], value);
                bounds[1][axis] = f64::max(bounds[1][axis], value);
            }
            entries.push(Entry {
                source_index,
                key: morton(world),
                world,
                point,
            });
        }
        entries.sort_unstable_by_key(|entry| entry.key);
        Ok(Self {
            entries,
            bounds,
            info,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn source_index(&self, index: usize) -> usize {
        self.entries[index].source_index
    }

    pub fn point(&self, index: usize) -> &Point {
        &self.entries[index].point
    }

    #[cfg(test)]
    pub fn marks(&self, area: [[f64; 2]; 2], scale: f64) -> Vec<Mark> {
        self.filtered_marks(area, scale, &[])
    }

    pub fn filtered_marks(&self, area: [[f64; 2]; 2], scale: f64, prefix: &[usize]) -> Vec<Mark> {
        let mut marks = Vec::new();
        self.visit(
            0..self.entries.len(),
            [0., 0.],
            1.,
            0,
            area,
            scale,
            prefix,
            &mut marks,
        );
        marks
    }

    fn visit(
        &self,
        range: std::ops::Range<usize>,
        origin: [f64; 2],
        side: f64,
        depth: u32,
        area: [[f64; 2]; 2],
        scale: f64,
        prefix: &[usize],
        marks: &mut Vec<Mark>,
    ) {
        if range.is_empty()
            || (0..2)
                .any(|axis| origin[axis] > area[1][axis] || origin[axis] + side < area[0][axis])
        {
            return;
        }
        if range.len() == 1 || side * scale <= 26. || depth == INDEX_DEPTH {
            for index in range {
                let world = self.entries[index].world;
                if (0..2).all(|axis| (area[0][axis]..=area[1][axis]).contains(&world[axis])) {
                    marks.push(Mark {
                        index,
                        world,
                        matched: prefix.is_empty() || prefix[index + 1] > prefix[index],
                    });
                }
            }
            return;
        }
        let shift = (INDEX_DEPTH - depth - 1) * 2;
        let side = side * 0.5;
        let mut start = range.start;
        for q in 0..4 {
            let end = start
                + self.entries[start..range.end].partition_point(|e| (e.key >> shift) & 3 <= q);
            let corner = [
                origin[0] + (q & 1) as f64 * side,
                origin[1] + (q >> 1) as f64 * side,
            ];
            self.visit(
                start..end,
                corner,
                side,
                depth + 1,
                area,
                scale,
                prefix,
                marks,
            );
            start = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const V5: &str =
        r#"[{"id":"fuji","name":"富士山","latitude":35.36,"longitude":138.73,"elevationM":3776}]"#;

    #[test]
    fn opens_legacy_and_gzipped_v6_with_bom() {
        let plain = Points::read(V5.as_bytes()).unwrap();
        let doc = format!(
            "\u{feff}{{\"schemaFormat\":\"yamamuki-points\",\"schemaVersion\":6,\"points\":{V5}}}"
        );
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(doc.as_bytes()).unwrap();
        let zipped = Points::read(gz.finish().unwrap().as_slice()).unwrap();
        assert_eq!(plain.bounds, zipped.bounds);
        assert_eq!(zipped.point(0).elevation_m, Some(3776.));
        assert!(
            Points::read(
                doc.replace("\"schemaVersion\":6", "\"schemaVersion\":6.0")
                    .as_bytes()
            )
            .is_ok()
        );
        assert!(
            Points::read(
                doc.replace("\"schemaVersion\":6", "\"schemaVersion\":7")
                    .as_bytes()
            )
            .is_err()
        );
        assert!(Points::read(V5.replace("35.36", "90.01").as_bytes()).is_err());
    }

    #[test]
    fn reads_a_single_reading_or_multiple_readings() {
        for (reading, expected) in [
            (r#""はくばだけ""#, Some("はくばだけ")),
            (
                r#"["しろうまだけ","はくばだけ"]"#,
                Some("しろうまだけ / はくばだけ"),
            ),
            ("null", None),
        ] {
            let point = V5.replace(
                r#""elevationM":3776"#,
                &format!(r#""elevationM":3776,"nameReading":{reading}"#),
            );
            let json = format!(
                r#"{{"schemaFormat":"yamamuki-points","schemaVersion":6,"points":{point}}}"#
            );
            let points = Points::read(json.as_bytes()).unwrap();
            let text = points
                .point(0)
                .name_reading
                .as_ref()
                .map(NameReading::display);
            assert_eq!(text.as_deref(), expected);
        }
    }

    #[test]
    fn nearby_points_keep_their_own_markers_at_all_zooms() {
        let json = V5.replace(
            "}]",
            r#"},{"id":"neighbor","name":"近傍","latitude":35.361,"longitude":138.731}]"#,
        );
        let points = Points::read(json.as_bytes()).unwrap();
        let all = [[0.; 2], [1.; 2]];
        let overview = points.marks(all, 256.);
        assert_eq!(overview.len(), 2);
        let marks = points.marks(points.bounds, 256. * (1u32 << 18) as f64);
        assert_eq!(marks.len(), 2);
        for mark in overview.into_iter().chain(marks) {
            let point = points.point(mark.index);
            assert_eq!(mark.world, project(point.latitude, point.longitude));
        }
    }
}

#[cfg(test)]
#[path = "point_schema_tests.rs"]
mod point_schema_tests;
