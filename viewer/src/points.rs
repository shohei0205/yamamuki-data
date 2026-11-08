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
    points: Vec<Box<RawValue>>,
}

pub fn project(latitude: f64, longitude: f64) -> [f64; 2] {
    let lat = latitude.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    [
        (longitude / 360. + 0.5).clamp(0., 1.),
        (0.5 - lat.tan().asinh() / std::f64::consts::TAU).clamp(0., 1.),
    ]
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

pub struct Points {
    entries: Vec<Entry>,
    pub bounds: [[f64; 2]; 2],
}

impl Points {
    pub fn open(path: &Path) -> Result<Self> {
        Self::read(File::open(path)?).with_context(|| format!("地点ファイル {}", path.display()))
    }

    pub(crate) fn read(input: impl Read) -> Result<Self> {
        let mut file = BufReader::new(input);
        let input: Box<dyn Read> = if file.fill_buf()?.starts_with(&[0x1f, 0x8b]) {
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
        let points: Vec<Box<RawValue>> = match first {
            b'[' => serde_json::from_reader(input)?,
            b'{' => {
                let doc: Document = serde_json::from_reader(input)?;
                ensure!(
                    doc.schema_format == "yamamuki-points" && doc.schema_version == 6.,
                    "地点JSONの形式または版に対応していません"
                );
                doc.points
            }
            _ => anyhow::bail!("地点JSONの配列または版6のオブジェクトが必要です"),
        };
        ensure!(!points.is_empty(), "地点がありません");
        let mut bounds = [[1., 1.], [0., 0.]];
        let mut entries = Vec::with_capacity(points.len());
        for (source_index, original) in points.into_iter().enumerate() {
            let mut point: Point = serde_json::from_str(original.get())?;
            point.original = Some(original);
            ensure!(
                !point.id.trim().is_empty()
                    && !point.name.trim().is_empty()
                    && (-90.0..=90.0).contains(&point.latitude)
                    && (-180.0..=180.0).contains(&point.longitude),
                "地点ID、名称、緯度、経度を確認してください"
            );
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
        Ok(Self { entries, bounds })
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
