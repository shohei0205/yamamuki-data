use crate::{
    appearance,
    point_list::{self, PointList},
    points::{PROVENANCE_FIELDS, Point, Points},
};
use eframe::egui;
use serde_json::Value;

#[derive(Default)]
pub struct Detail {
    index: Option<usize>,
    json: String,
    provenance: [String; 3],
    graphic: Option<(egui::TextureHandle, egui::Vec2)>,
    graphic_scale: f32,
    nearby_open: bool,
    nearby_filtered: bool,
    nearby_key: Option<(usize, Option<u64>)>,
    nearby: Vec<(usize, f64)>,
}

pub enum Action {
    Close,
    Select(usize),
}

impl Detail {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        points: &Points,
        list: &PointList,
        index: usize,
    ) -> Option<Action> {
        let point = points.point(index);
        let point_changed = self.index != Some(index);
        if point_changed {
            self.index = Some(index);
            let original = point.original.as_ref().map(|raw| raw.get()).unwrap_or("{}");
            let json: Value = serde_json::from_str(original).unwrap_or_default();
            self.json = serde_json::to_string_pretty(&json).unwrap_or_else(|_| original.into());
            self.provenance = PROVENANCE_FIELDS.map(|(key, _)| {
                point
                    .provenance_codes(key)
                    .map(|code| {
                        let label = match code {
                            "unknown" => return "出典情報なし".into(),
                            "mountainList" => "国土地理院：山岳一覧",
                            "vector" => "国土地理院：ベクトルタイル",
                            "natural" => "国土地理院：自然地名",
                            "osm" => "OpenStreetMap",
                            "triangulation" => "三角点",
                            "spotHeight" => "標高点",
                            "supplement" => "補足による補正",
                            "dem1a_png" => "DEM1A",
                            "dem5a_png" => "DEM5A",
                            "dem5b_png" => "DEM5B",
                            "dem5c_png" => "DEM5C",
                            "dem_png" => "DEM10B",
                            "natural=peak" => "山頂",
                            "natural=volcano" => "火山",
                            _ => return code.to_owned(),
                        };
                        format!("[{code}] {label}")
                    })
                    .collect::<Vec<_>>()
                    .join("、")
            });
        }

        let pixels_per_point = ctx.pixels_per_point();
        if point_changed || pixels_per_point != self.graphic_scale {
            self.graphic_scale = pixels_per_point;
            self.graphic = point
                .graphic
                .as_ref()
                .and_then(|graphic| svg_image(&graphic.svg, pixels_per_point))
                .map(|(image, display_size)| {
                    let texture =
                        ctx.load_texture("地点のSVG", image, egui::TextureOptions::LINEAR);
                    (texture, display_size)
                });
        }

        let available_height = ctx.available_rect().height().max(0.);
        let map_min_height = 120_f32.min(available_height / 3.);
        let max_height = available_height - map_min_height;
        let mut action = None;
        egui::TopBottomPanel::bottom("point")
            .resizable(true)
            .frame(appearance::panel(appearance::PAPER, 14, 8))
            .default_height(230.)
            .height_range(110_f32.min(max_height)..=max_height)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("地点の詳細")
                        .color(appearance::ACCENT)
                        .small()
                        .strong(),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.heading(egui::RichText::new(&point.name).strong());
                    ui.label(
                        egui::RichText::new(point_list::reading(point)).color(appearance::MUTED),
                    );
                    if ui.button("近くの山").clicked() {
                        self.nearby_open = true;
                    }
                    if ui.button("閉じる").clicked() {
                        action = Some(Action::Close);
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([true, false])
                    .id_salt(index)
                    .show(ui, |ui| {
                        ui.label(format!(
                            "{}　{:.6}°, {:.6}°　{}",
                            list.location_label(index),
                            point.latitude,
                            point.longitude,
                            point
                                .elevation_m
                                .map_or("標高不明".into(), |value| format!("{value} m"))
                        ));
                        for ((_, label), value) in
                            PROVENANCE_FIELDS.into_iter().zip(&self.provenance)
                        {
                            ui.label(format!("{label}　{value}"));
                        }
                        ui.label(format!("ID　{}", point.id));
                        if let Some(osm) = point.osm_id.as_ref().and_then(|value| value.as_u64()) {
                            ui.hyperlink_to(
                                "OSM の地図・登録情報",
                                format!("https://www.openstreetmap.org/node/{osm}"),
                            );
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.label("関連する地点 ID");
                            if let Some(ids) =
                                point.related_ids.as_ref().filter(|ids| !ids.is_empty())
                            {
                                for id in ids {
                                    if ui.link(id).clicked() {
                                        if let Some(target) =
                                            (0..points.len()).find(|&i| points.point(i).id == *id)
                                        {
                                            action = Some(Action::Select(target));
                                        } else if let Some(osm) = id
                                            .strip_prefix("osm:")
                                            .filter(|s| s.parse::<u64>().is_ok())
                                        {
                                            ctx.open_url(egui::OpenUrl::new_tab(format!(
                                                "https://www.openstreetmap.org/node/{osm}"
                                            )));
                                        } else {
                                            ctx.copy_text(id.clone());
                                        }
                                    }
                                }
                            } else {
                                ui.label("なし");
                            }
                        });
                        for (label, value) in [
                            ("タグ", point_list::tags(point)),
                            ("別名", point_list::aliases(point)),
                        ] {
                            ui.label(format!(
                                "{label}　{}",
                                if value.is_empty() { "なし" } else { &value }
                            ));
                        }
                        if let Some((texture, display_size)) = &self.graphic {
                            ui.image((texture.id(), *display_size));
                        } else if point.graphic.is_some() {
                            ui.label("SVGをプレビューできません");
                        }
                        ui.collapsing("元の地点 JSON", |ui| {
                            if ui.button("コピー").clicked() {
                                ctx.copy_text(self.json.clone());
                            }
                            ui.add(
                                egui::TextEdit::multiline(&mut self.json.as_str())
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                    });
            });

        if self.nearby_open {
            egui::Window::new("近くの山")
                .open(&mut self.nearby_open)
                .default_width(800.)
                .vscroll(true)
                .hscroll(true)
                .show(ctx, |ui| {
                    ui.label(format!("基準　{}（直線距離が近い20地点）", point.name));
                    ui.checkbox(&mut self.nearby_filtered, "絞り込み中の地点だけを表示");
                    let key = (index, self.nearby_filtered.then_some(list.filter_revision));
                    if self.nearby_key != Some(key) {
                        self.nearby = nearby_points(points, index, |i| {
                            !self.nearby_filtered || list.matches(i)
                        });
                        self.nearby_key = Some(key);
                    }
                    let nearby = &self.nearby;
                    let mut table = egui_extras::TableBuilder::new(ui)
                        .striped(true)
                        .vscroll(false)
                        .sense(egui::Sense::click());
                    for width in [85., 125., 70., 140., 135., 100., 125., 120., 45., 135.] {
                        table = table.column(egui_extras::Column::initial(width).clip(true));
                    }
                    table
                        .header(22., |mut row| {
                            for title in [
                                "距離 (km)",
                                "地点名",
                                "標高 (m)",
                                "所在地（参考）",
                                "緯度・経度",
                                "よみがな",
                                "別名",
                                "タグ",
                                "SVG",
                                "ID",
                            ] {
                                row.col(|ui| {
                                    ui.label(title);
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(24., nearby.len(), |mut row| {
                                let (target, distance) = nearby[row.index()];
                                row.col(|ui| {
                                    ui.label(format!("{distance:.3}"));
                                });
                                point_list::point_cells(
                                    &mut row,
                                    points.point(target),
                                    list.location_label(target),
                                );
                                if row.response().clicked() {
                                    action = Some(Action::Select(target));
                                }
                            })
                        });
                });
            if matches!(action, Some(Action::Select(_))) {
                self.nearby_open = false;
            }
        }
        action
    }
}

fn nearby_points(
    points: &Points,
    index: usize,
    matches: impl Fn(usize) -> bool,
) -> Vec<(usize, f64)> {
    let origin = points.point(index);
    let mut rows: Vec<_> = (0..points.len())
        .filter(|&i| i != index && matches(i))
        .map(|i| (i, distance_km(origin, points.point(i))))
        .collect();
    let compare = |a: &(usize, f64), b: &(usize, f64)| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0));
    if rows.len() > 20 {
        rows.select_nth_unstable_by(20, compare);
        rows.truncate(20);
    }
    rows.sort_unstable_by(compare);
    rows
}

fn distance_km(origin: &Point, point: &Point) -> f64 {
    let dlat = (point.latitude - origin.latitude).to_radians();
    let dlon = (point.longitude - origin.longitude).to_radians();
    let a = (dlat / 2.).sin().powi(2)
        + origin.latitude.to_radians().cos()
            * point.latitude.to_radians().cos()
            * (dlon / 2.).sin().powi(2);
    6371.0088 * 2. * a.clamp(0., 1.).sqrt().asin()
}

fn svg_image(svg: &str, pixels_per_point: f32) -> Option<(egui::ColorImage, egui::Vec2)> {
    use resvg::{tiny_skia, usvg};

    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    if svg.len() > 100_000 {
        return None;
    }
    let tree = usvg::Tree::from_str(svg, &options).ok()?;
    let size = tree.size();
    let logical_scale = 120. / size.width().max(size.height());
    let display_size = egui::vec2(
        (size.width() * logical_scale).ceil().max(1.),
        (size.height() * logical_scale).ceil().max(1.),
    );
    let width = (display_size.x * pixels_per_point).ceil().max(1.) as u32;
    let height = (display_size.y * pixels_per_point).ceil().max(1.) as u32;
    let pixel_scale = logical_scale * pixels_per_point;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(pixel_scale, pixel_scale),
        &mut pixmap.as_mut(),
    );
    let image =
        egui::ColorImage::from_rgba_premultiplied([width as usize, height as usize], pixmap.data());
    Some((image, display_size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rerenders_preview_when_the_same_point_moves_to_a_higher_dpi() {
        let points = Points::read(
            br#"[{"id":"gsi:1","name":"Peak","latitude":35.0,"longitude":138.0,"graphic":{"svg":"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 20 10\"><path d=\"M0 0H20V10H0Z\"/></svg>"}}]"#.as_slice(),
        )
        .unwrap();
        let ctx = egui::Context::default();
        let list = PointList::default();
        let mut detail = Detail::default();
        let mut frame = |density| {
            let mut input = egui::RawInput::default();
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(density);
            let _ = ctx.run(input, |ctx| {
                detail.show(ctx, &points, &list, 0);
            });
            let (texture, display_size) = detail.graphic.as_ref().unwrap();
            (texture.id(), texture.size(), *display_size)
        };
        let (original_texture, original_pixels, original_size) = frame(1.);
        let (new_texture, new_pixels, new_size) = frame(2.);
        assert_ne!(new_texture, original_texture);
        assert_eq!(original_pixels, [120, 60]);
        assert_eq!(new_pixels, [240, 120]);
        assert_eq!(new_size, original_size);
        assert_eq!(frame(2.).0, new_texture);
    }

    #[test]
    fn detail_can_be_dragged_beyond_the_height_of_its_contents() {
        let ctx = egui::Context::default();
        let points = Points::read(
            br#"[{"id":"gsi:1","name":"Peak","latitude":35.0,"longitude":138.0}]"#.as_slice(),
        )
        .unwrap();
        let list = PointList::default();
        let mut detail = Detail::default();
        let mut frame = |events| {
            let mut divider = 0.;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 800.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    detail.show(ctx, &points, &list, 0);
                    divider = ctx.available_rect().bottom();
                },
            );
            divider
        };
        let start = egui::pos2(650., frame(vec![]));
        let end = egui::pos2(650., 300.);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(start)]);
        frame(vec![button(start, true)]);
        frame(vec![egui::Event::PointerMoved(end)]);
        frame(vec![button(end, false)]);
        let divider = frame(vec![]);
        assert!((divider - end.y).abs() <= 2., "divider at {divider}");
    }

    #[test]
    fn renders_point_svg_at_display_density_without_changing_layout_size() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><path d="M0 0H20V10H0Z"/></svg>"#;
        for (density, dimensions) in [(1., [120, 60]), (1.25, [150, 75]), (2., [240, 120])] {
            let (image, display_size) = svg_image(svg, density).unwrap();
            assert_eq!(image.size, dimensions);
            assert_eq!(display_size, egui::vec2(120., 60.));
            assert!(image.pixels.iter().all(|color| color.a() == 255));
        }
    }
}
