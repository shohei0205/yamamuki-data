use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    collections::{BTreeSet, HashMap},
    io::Read,
    sync::Mutex,
    time::Duration,
};

pub const SOURCE: &str = "https://github.com/kyodo-official/japan-choropleth#行政区域";
const TOPOJSON: &str = "https://raw.githubusercontent.com/kyodo-official/japan-choropleth/main/data/topojson/municipalities.topojson";

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    pub code: String,
    pub label: String,
}

#[derive(Deserialize)]
struct Topology {
    arcs: Vec<Vec<[f64; 2]>>,
    transform: Option<Transform>,
    objects: Objects,
}

#[derive(Deserialize)]
struct Transform {
    scale: [f64; 2],
    translate: [f64; 2],
}

#[derive(Deserialize)]
struct Objects {
    municipalities: Municipalities,
}

#[derive(Deserialize)]
struct Municipalities {
    geometries: Vec<Geometry>,
}

#[derive(Deserialize)]
struct Geometry {
    id: String,
    properties: Properties,
    #[serde(flatten)]
    shape: Shape,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "arcs")]
enum Shape {
    Polygon(Vec<Vec<i32>>),
    MultiPolygon(Vec<Vec<Vec<i32>>>),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Properties {
    prefecture: String,
    display_name: Option<String>,
    county: Option<String>,
    municipality: Option<String>,
    ward: Option<String>,
}

struct Polygon {
    area: usize,
    rings: Vec<Vec<[f64; 2]>>,
    bounds: [[f64; 2]; 2],
}

#[derive(Default)]
pub struct BoundaryCache(Mutex<Option<BoundaryIndex>>);

impl BoundaryCache {
    pub fn lookup(
        &self,
        client: &reqwest::blocking::Client,
        points: &[[f64; 2]],
    ) -> Result<Vec<Location>> {
        let mut cache = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("行政区域の処理に失敗しました"))?;
        if cache.is_none() {
            let mut bytes = Vec::new();
            client
                .get(TOPOJSON)
                .timeout(Duration::from_secs(25))
                .send()?
                .error_for_status()?
                .take(20 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= 20 * 1024 * 1024,
                "行政区域データが大きすぎます"
            );
            *cache = Some(BoundaryIndex::parse(&bytes)?);
        }
        let index = cache.as_ref().unwrap();
        Ok(points.iter().map(|&point| index.lookup(point)).collect())
    }
}

struct BoundaryIndex {
    areas: Vec<Location>,
    polygons: Vec<Polygon>,
    grid: HashMap<(i32, i32), Vec<usize>>,
}

fn cell(point: [f64; 2]) -> (i32, i32) {
    (
        (point[0] * 2.).floor() as i32,
        (point[1] * 2.).floor() as i32,
    )
}

impl BoundaryIndex {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let mut doc: Topology = serde_json::from_slice(bytes).context("行政区域TopoJSON")?;
        if let Some(transform) = doc.transform {
            for arc in &mut doc.arcs {
                let mut previous = [0.; 2];
                for point in arc {
                    for i in 0..2 {
                        previous[i] += point[i];
                        point[i] = previous[i] * transform.scale[i] + transform.translate[i];
                    }
                }
            }
        }
        let mut index = Self {
            areas: Vec::new(),
            polygons: Vec::new(),
            grid: HashMap::new(),
        };
        for g in doc.objects.municipalities.geometries {
            let p = g.properties;
            let city = p.display_name.unwrap_or_else(|| {
                [
                    p.county.as_deref().unwrap_or_default(),
                    p.municipality.as_deref().unwrap_or_default(),
                    p.ward.as_deref().unwrap_or_default(),
                ]
                .concat()
            });
            let area = index.areas.len();
            let label = if city.starts_with(&p.prefecture) {
                city
            } else {
                format!("{} {}", p.prefecture, city)
            };
            index.areas.push(Location { code: g.id, label });
            let polygons = match g.shape {
                Shape::Polygon(p) => vec![p],
                Shape::MultiPolygon(p) => p,
            };
            for polygon in polygons {
                let mut rings = Vec::new();
                for ids in polygon {
                    let mut ring = Vec::new();
                    for id in ids {
                        let arc = doc
                            .arcs
                            .get(if id < 0 { !id } else { id } as usize)
                            .context("行政区域の境界線ID")?;
                        let skip = usize::from(!ring.is_empty());
                        if id < 0 {
                            ring.extend(arc.iter().rev().skip(skip).copied());
                        } else {
                            ring.extend(arc.iter().skip(skip).copied());
                        }
                    }
                    rings.push(ring);
                }
                let Some(outer) = rings.first().filter(|ring| !ring.is_empty()) else {
                    continue;
                };
                let mut bounds = [outer[0]; 2];
                for &[x, y] in outer {
                    bounds[0][0] = bounds[0][0].min(x);
                    bounds[0][1] = bounds[0][1].min(y);
                    bounds[1][0] = bounds[1][0].max(x);
                    bounds[1][1] = bounds[1][1].max(y);
                }
                ensure!(
                    bounds[0][0] >= -180.
                        && bounds[1][0] <= 180.
                        && bounds[0][1] >= -90.
                        && bounds[1][1] <= 90.,
                    "行政区域の座標が範囲外です"
                );
                let (x0, y0) = cell(bounds[0]);
                let (x1, y1) = cell(bounds[1]);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        index
                            .grid
                            .entry((x, y))
                            .or_default()
                            .push(index.polygons.len());
                    }
                }
                index.polygons.push(Polygon {
                    area,
                    rings,
                    bounds,
                });
            }
        }
        ensure!(!index.polygons.is_empty(), "行政区域の境界がありません");
        Ok(index)
    }

    fn lookup(&self, [x, y]: [f64; 2]) -> Location {
        let mut areas = BTreeSet::new();
        for &id in self.grid.get(&cell([x, y])).into_iter().flatten() {
            let p = &self.polygons[id];
            let [[west, south], [east, north]] = p.bounds;
            if (west..=east).contains(&x)
                && (south..=north).contains(&y)
                && contains(&p.rings[0], x, y)
                && !p.rings[1..].iter().any(|ring| contains(ring, x, y))
            {
                areas.insert(&self.areas[p.area]);
            }
        }
        let Some(first) = areas.first() else {
            return Location::default();
        };
        Location {
            code: first.code.clone(),
            label: areas
                .iter()
                .map(|a| a.label.as_str())
                .collect::<Vec<_>>()
                .join("・"),
        }
    }
}

fn contains(ring: &[[f64; 2]], x: f64, y: f64) -> bool {
    let Some(mut previous) = ring.last().copied() else {
        return false;
    };
    let mut inside = false;
    for &point in ring {
        let [ax, ay] = previous;
        let [bx, by] = point;
        if ((x - ax) * (by - ay) - (y - ay) * (bx - ax)).abs() < 1e-10
            && x >= ax.min(bx) - 1e-10
            && x <= ax.max(bx) + 1e-10
            && y >= ay.min(by) - 1e-10
            && y <= ay.max(by) + 1e-10
        {
            return true;
        }
        if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
            inside = !inside;
        }
        previous = point;
    }
    inside
}

#[test]
fn polygon_with_hole_and_reversed_delta_arc() {
    let fixture = br#"{"transform":{"scale":[1,1],"translate":[130,30]},"arcs":[[[0,0],[4,0],[0,4],[-4,0],[0,-4]],[[1,1],[1,0],[0,1],[-1,0],[0,-1]]],"objects":{"municipalities":{"geometries":[{"type":"Polygon","arcs":[[0],[-2]],"id":"01","properties":{"prefecture":"A","municipality":"B","county":null,"ward":null}}]}}}"#;
    let index = BoundaryIndex::parse(fixture).unwrap();
    assert_eq!(index.lookup([130.5, 30.5]).label, "A B");
    assert_eq!(index.lookup([131.5, 31.5]), Location::default());
    assert_eq!(index.lookup([135., 30.]), Location::default());
}

#[test]
#[ignore = "requires the public municipality TopoJSON"]
fn looks_up_real_municipalities() {
    let cache = BoundaryCache::default();
    let client = crate::map::client().unwrap();
    let locations = cache
        .lookup(
            &client,
            &[[139.6917, 35.6895], [141.3545, 43.0618], [140., 30.]],
        )
        .unwrap();
    assert_eq!(locations[0].label, "東京都 新宿区");
    assert_eq!(locations[1].label, "北海道 札幌市中央区");
    assert_eq!(locations[2], Location::default());
}
