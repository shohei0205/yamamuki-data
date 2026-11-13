//! 提供範囲を地図に重ねる。穴も輪郭だけを描き、地表や地点を塗りつぶさない。

use crate::{map::View, points::project};
use anyhow::Result;
use eframe::egui::{self, Color32, Rect, Stroke};
use serde_json::Value;

const OVERALL: Color32 = Color32::from_rgb(214, 112, 25);
const FINEST: Color32 = Color32::from_rgb(0, 155, 181);
type Ring = Vec<[f64; 2]>;

#[derive(Default)]
pub struct Coverage {
    overall: Vec<Ring>,
    finest: Vec<Ring>,
    regions: Vec<String>,
}

#[derive(Default)]
pub struct Visibility {
    pub overall: bool,
    pub finest: bool,
}

impl Coverage {
    pub fn from_metadata(metadata: &Value, surface: bool) -> Result<Self> {
        yamamuki_data_sqlite::validate_coverage_metadata(metadata, surface)?;
        let coverage = &metadata["coverage"];
        let read = |geometry: &Value| -> Vec<Ring> {
            let polygons: Vec<&Value> = match geometry["type"].as_str() {
                Some("Polygon") => vec![&geometry["coordinates"]],
                Some("MultiPolygon") => {
                    geometry["coordinates"].as_array().unwrap().iter().collect()
                }
                _ => return Vec::new(),
            };
            polygons
                .into_iter()
                .flat_map(|p| p.as_array().unwrap())
                .map(|ring| {
                    ring.as_array()
                        .unwrap()
                        .iter()
                        .map(|p| project(p[1].as_f64().unwrap(), p[0].as_f64().unwrap()))
                        .collect()
                })
                .collect()
        };
        Ok(Self {
            overall: read(&coverage["geometry"]),
            finest: read(&coverage["finest"]["geometry"]),
            regions: coverage["regions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        })
    }

    pub fn bounds(&self) -> Option<[[f64; 2]; 2]> {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        let mut found = false;
        for point in self.overall.iter().flatten() {
            found = true;
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        found.then_some([min, max])
    }

    pub fn menu(
        &self,
        ui: &mut egui::Ui,
        visibility: &mut Visibility,
        name: &str,
    ) -> Option<[[f64; 2]; 2]> {
        ui.strong(name);
        if !self.regions.is_empty() {
            ui.label(format!("対象地域：{}", self.regions.join("・")));
        }
        if self.overall.is_empty() {
            ui.label("提供範囲の図形なし");
            return None;
        }
        ui.checkbox(
            &mut visibility.overall,
            egui::RichText::new("全体の提供範囲（実線）").color(OVERALL),
        );
        if !self.finest.is_empty() {
            ui.checkbox(
                &mut visibility.finest,
                egui::RichText::new("最も精細な提供範囲（破線）").color(FINEST),
            );
        }
        if ui.button("提供範囲に移動").clicked() {
            visibility.overall = true;
            return self.bounds();
        }
        None
    }

    pub fn paint(&self, painter: &egui::Painter, rect: Rect, view: &View, visibility: &Visibility) {
        let painter = painter.with_clip_rect(rect);
        for (shown, rings, color, width) in [
            (visibility.overall, &self.overall, OVERALL, 2.0),
            (visibility.finest, &self.finest, FINEST, 2.5),
        ] {
            if !shown {
                continue;
            }
            for ring in rings {
                let positions: Vec<_> =
                    ring.iter().map(|&world| view.screen(rect, world)).collect();
                if Rect::from_points(&positions).intersects(rect) {
                    // 同じ範囲でも判別できるよう、精細範囲を破線にする。
                    if color == FINEST {
                        // 拡大時に画面外まで破線を生成しない。
                        for edge in positions.windows(2) {
                            if let Some(segment) =
                                clip_segment(rect.expand(width + 2.), [edge[0], edge[1]])
                            {
                                painter.extend(egui::Shape::dashed_line(
                                    &segment,
                                    Stroke::new(width + 2.0, Color32::WHITE),
                                    8.,
                                    5.,
                                ));
                                painter.extend(egui::Shape::dashed_line(
                                    &segment,
                                    Stroke::new(width, color),
                                    8.,
                                    5.,
                                ));
                            }
                        }
                    } else {
                        painter.add(egui::Shape::line(
                            positions.clone(),
                            Stroke::new(width + 2.0, Color32::WHITE),
                        ));
                        painter.add(egui::Shape::line(positions, Stroke::new(width, color)));
                    }
                }
            }
        }
    }
}

fn clip_segment(rect: Rect, [a, b]: [egui::Pos2; 2]) -> Option<[egui::Pos2; 2]> {
    let delta = b - a;
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for (p, q) in [
        (-delta.x, a.x - rect.left()),
        (delta.x, rect.right() - a.x),
        (-delta.y, a.y - rect.top()),
        (delta.y, rect.bottom() - a.y),
    ] {
        if p == 0. {
            if q < 0. {
                return None;
            }
        } else if p < 0. {
            low = low.max(q / p);
        } else {
            high = high.min(q / p);
        }
        if low > high {
            return None;
        }
    }
    Some([a + delta * low, a + delta * high])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn menu_switches_both_ranges_on_and_off_and_moves_to_bounds() {
        let geometry = json!({"type":"Polygon","coordinates":[[[130,30],[140,30],[140,40],[130,40],[130,30]]]});
        let coverage = Coverage::from_metadata(
            &json!({"coverage":{"geometry":geometry,"finest":{"geometry":geometry}}}),
            true,
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut visibility = Visibility::default();
        let frame = |events: Vec<egui::Event>, visibility: &mut Visibility| {
            let mut fit = None;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600., 400.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        fit = coverage.menu(ui, visibility, "表示テスト");
                    });
                },
            );
            (output, fit)
        };
        let (mut output, _) = frame(Vec::new(), &mut visibility);
        for (label, overall, finest) in [
            ("全体の提供範囲（実線）", true, false),
            ("全体の提供範囲（実線）", false, false),
            ("最も精細な提供範囲（破線）", false, true),
            ("最も精細な提供範囲（破線）", false, false),
            ("提供範囲に移動", true, false),
        ] {
            let position = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos + egui::vec2(3., 3.))
                    }
                    _ => None,
                })
                .expect("操作できる項目が表示される");
            let mut fit = None;
            for pressed in [true, false] {
                (output, fit) = frame(
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    &mut visibility,
                );
            }
            assert_eq!(
                (visibility.overall, visibility.finest),
                (overall, finest),
                "{label}"
            );
            if label == "提供範囲に移動" {
                assert_eq!(fit, coverage.bounds());
            }
        }
    }

    #[test]
    fn point_json_gzip_and_sqlite_keep_declared_range() {
        use crate::points::Points;
        use std::io::Write;
        let geometry = json!({"type":"Polygon","coordinates":[[[130,30],[140,30],[140,40],[130,40],[130,30]]]});
        let metadata = json!({"coverage":{"regions":["JP"],"geometry":geometry},"language":"ja"});
        let point = json!({"id":"sample:1","name":"山","latitude":35.,"longitude":139.});
        let body=serde_json::to_vec(&json!({"schemaFormat":"yamamuki-points","schemaVersion":6,"metadata":metadata,"points":[point]})).unwrap();
        let expected = Coverage::from_metadata(&metadata, false).unwrap().bounds();
        let mut compressed =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        compressed.write_all(&body).unwrap();
        for input in [body, compressed.finish().unwrap()] {
            let points = Points::read(input.as_slice()).unwrap();
            assert_eq!(points.info.coverage.bounds(), expected);
        }
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("points.sqlite3");
        let db = yamamuki_data_sqlite::rusqlite::Connection::open(&path).unwrap();
        yamamuki_data_sqlite::initialize(&db).unwrap();
        db.execute(
            "INSERT INTO datasets VALUES(1,'sample','points','山','1',6,?1)",
            [metadata.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO point_records VALUES(1,0,'sample:1',?1)",
            [point.to_string()],
        )
        .unwrap();
        assert_eq!(
            Points::open(&path).unwrap().info.coverage.bounds(),
            expected
        );
    }

    #[test]
    fn clips_dashes_before_drawing_at_high_zoom() {
        let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600., 400.));
        let clipped = clip_segment(rect, [egui::pos2(-1e7, 200.), egui::pos2(1e7, 200.)]).unwrap();
        assert!(clipped[0].distance(clipped[1]) <= rect.width() + 2.);
        assert!(clip_segment(rect, [egui::pos2(-100., -100.), egui::pos2(700., -100.)]).is_none());
    }

    #[test]
    fn keeps_islands_and_holes_and_toggles_each_outline() {
        let overall = json!({"type":"MultiPolygon","coordinates":[
            [[[130,30],[140,30],[140,40],[130,40],[130,30]],[[133,33],[133,37],[137,37],[137,33],[133,33]]],
            [[[145,40],[150,40],[150,45],[145,45],[145,40]]]
        ]});
        let finest = json!({"type":"Polygon","coordinates":[[[131,31],[132,31],[132,32],[131,32],[131,31]]]});
        let coverage = Coverage::from_metadata(
            &json!({"coverage":{"regions":["JP"],"geometry":overall,"finest":{"geometry":finest}}}),
            true,
        )
        .unwrap();
        assert_eq!(coverage.overall.len(), 3);
        assert_eq!(coverage.finest.len(), 1);
        assert_eq!(
            coverage.bounds(),
            Some([project(45., 130.), project(30., 150.)])
        );
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600., 400.));
        let mut view = View {
            center: project(37., 140.),
            zoom: 4.,
            orientation: Default::default(),
        };
        view.fit(rect, coverage.bounds().unwrap());
        let mut counts = Vec::new();
        for (overall, finest) in [(false, false), (true, false), (false, true), (true, true)] {
            let output = ctx.run(Default::default(), |ctx| {
                coverage.paint(
                    &ctx.layer_painter(egui::LayerId::background()),
                    rect,
                    &view,
                    &Visibility { overall, finest },
                );
            });
            counts.push(output.shapes.len());
        }
        assert_eq!(counts[0], 0);
        assert_eq!(counts[1], 6);
        assert!(counts[2] > 0);
        assert_eq!(counts[3], counts[1] + counts[2]);
    }

    #[test]
    fn old_metadata_has_no_invented_geometry() {
        let coverage =
            Coverage::from_metadata(&json!({"coverage":{"regions":["JP"]}}), false).unwrap();
        assert!(coverage.bounds().is_none());
        assert!(
            Coverage::from_metadata(&json!({}), false)
                .unwrap()
                .overall
                .is_empty()
        );
        assert!(Coverage::from_metadata(&json!({"coverage":{"geometry":null}}), false).is_err());
    }
}
