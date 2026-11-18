//! 2Dのタイル選択・合成順を引き継ぎ、元テクスチャを地形へ直接渡す。
use eframe::{egui, egui_wgpu::RenderState};
use yamamuki_terrain_view::direct_material::Interpolation;
use yamamuki_terrain_view::direct_material::{Capture, Quad};
pub struct BorderCapture {
    pub rect: egui::Rect,
    pub color: egui::Color32,
    pub uv: egui::Rect,
    pub fill: bool,
}
pub fn tile_border(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &crate::map::View,
    tile: crate::map::Tile,
    material: bool,
    color: egui::Color32,
) {
    clipped_tile_border(painter, rect, view, tile, tile, material, color);
}
/// 表示区画で切り取られた元タイルの、本来の外周だけを描く。
pub fn clipped_tile_border(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &crate::map::View,
    source: crate::map::Tile,
    region: crate::map::Tile,
    material: bool,
    color: egui::Color32,
) {
    let Some((tile, uv)) = clipped_border_area(source, region) else {
        return;
    };
    if uv.left() > 0. && uv.right() < 1. && uv.top() > 0. && uv.bottom() < 1. {
        return;
    }
    let corners = view.tile_corners(rect, tile);
    if material {
        let rect = egui::Rect::from_points(&corners);
        painter.add(egui::Shape::Callback(egui::PaintCallback {
            rect,
            callback: std::sync::Arc::new(BorderCapture {
                rect,
                color,
                uv,
                fill: false,
            }),
        }));
    } else {
        // UVが元タイルの0または1に一致する辺だけが本物の外周。
        for (visible, a, b) in [
            (uv.top() == 0., 0, 1),
            (uv.right() == 1., 1, 2),
            (uv.bottom() == 1., 2, 3),
            (uv.left() == 0., 3, 0),
        ] {
            if visible {
                painter.line_segment([corners[a], corners[b]], egui::Stroke::new(1.5_f32, color));
            }
        }
    }
}
/// 元タイルと表示区画の交差だけを塗り、粗タイル内の細タイルを偽のレベルにしない。
pub fn tile_level_color(z: u32) -> egui::Color32 {
    let hue = (z as f32 * 0.618034).fract();
    egui::ecolor::Hsva::new(hue, 0.7, 0.95, 0.48).into()
}
pub fn clipped_tile_fill(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &crate::map::View,
    source: crate::map::Tile,
    region: crate::map::Tile,
    material: bool,
    weight: f32,
) {
    let Some((tile, _)) = clipped_border_area(source, region) else {
        return;
    };
    let color = tile_level_color(source.z).gamma_multiply(weight.clamp(0., 1.));
    let corners = view.tile_corners(rect, tile);
    if material {
        let area = egui::Rect::from_points(&corners);
        painter.add(egui::Shape::Callback(egui::PaintCallback {
            rect: area,
            callback: std::sync::Arc::new(BorderCapture {
                rect: area,
                color,
                uv: crate::lod_fade::FULL_UV,
                fill: true,
            }),
        }));
    } else {
        painter.add(egui::Shape::convex_polygon(
            corners.to_vec(),
            color,
            egui::Stroke::NONE,
        ));
    }
}
fn clipped_border_area(
    source: crate::map::Tile,
    region: crate::map::Tile,
) -> Option<(crate::map::Tile, egui::Rect)> {
    if source.z <= region.z {
        let (parent, uv) = crate::lod_fade::sample(region, source.z, crate::lod_fade::FULL_UV);
        (parent == source).then_some((region, uv))
    } else {
        let (parent, _) = crate::lod_fade::sample(source, region.z, crate::lod_fade::FULL_UV);
        (parent == region).then_some((source, crate::lod_fade::FULL_UV))
    }
}
const COVERAGE_MARGIN: f64 = 1.02;
/// 0は通常の2D、2は3D近景。画像合成用の倍率をLOD選択へ混ぜない。
pub fn lod_zoom(screen_zoom: f64, material_zoom: f64, pass: usize) -> f64 {
    if matches!(pass, 0 | 2) {
        screen_zoom
    } else {
        screen_zoom.min(material_zoom)
    }
}
pub fn minimum_span(size: f64, density: f64) -> f64 {
    size / (256. * density.exp2()) / COVERAGE_MARGIN
}
pub fn map_view(area: [f64; 4], size: f64) -> crate::map::View {
    let extent = (area[2] - area[0]).max(area[3] - area[1]) * COVERAGE_MARGIN;
    crate::map::View {
        center: [(area[0] + area[2]) * 0.5, (area[1] + area[3]) * 0.5],
        // 元画像の最大密度と一致するとき、丸め誤差だけで一段粗いLODへ落とさない。
        zoom: (size / (256. * extent)).log2() + 1e-9,
        orientation: crate::map::Orientation::default(),
    }
}

/// 描画用の地図座標を、元テクスチャを保持した地理座標へ戻す。
fn world_rect(view: &crate::map::View, rect: egui::Rect, area: egui::Rect) -> [f64; 4] {
    let point = |p: egui::Pos2| {
        [
            view.center[0] + f64::from(p.x - rect.center().x) / view.scale(),
            view.center[1] + f64::from(p.y - rect.center().y) / view.scale(),
        ]
    };
    let a = point(area.min);
    let b = point(area.max);
    [a[0], a[1], b[0], b[1]]
}
fn mesh_rectangle(vertices: &[egui::epaint::Vertex]) -> (egui::Rect, egui::Rect) {
    (
        egui::Rect::from_points(&vertices.iter().map(|v| v.pos).collect::<Vec<_>>()),
        egui::Rect::from_points(&vertices.iter().map(|v| v.uv).collect::<Vec<_>>()),
    )
}
pub fn collect(
    ctx: &egui::Context,
    state: &RenderState,
    view_cache: &crate::lod_fade::Renderer,
    shapes: Vec<egui::epaint::ClippedShape>,
    view: &crate::map::View,
    rect: egui::Rect,
) -> Vec<Quad> {
    let jobs = ctx.tessellate(shapes, 1.);
    let textures = state.renderer.read();
    let mut result = Vec::new();
    for job in jobs {
        match job.primitive {
            egui::epaint::Primitive::Mesh(mesh) => {
                let Some(texture) =
                    view_cache.color_view(&textures, mesh.texture_id, ctx.cumulative_pass_nr())
                else {
                    ctx.request_repaint();
                    continue;
                };
                for vertices in mesh.vertices.chunks_exact(4) {
                    let (area, uv) = mesh_rectangle(vertices);
                    if !area.is_positive() {
                        continue;
                    }
                    let color = vertices[0].color.to_array().map(|v| f32::from(v) / 255.);
                    result.push(Quad {
                        bounds: world_rect(view, rect, area),
                        uv: [uv; 2],
                        textures: [Some(texture.clone()), Some(texture.clone())],
                        amount: 1.,
                        tint: color,
                        shade: false,
                        gamma_shade: false,
                        opaque: false,
                        surface: None,
                        edge_filter: Interpolation::Bilinear,
                        border: false,
                    });
                }
            }
            egui::epaint::Primitive::Callback(callback) => {
                if let Some(c) = callback.callback.downcast_ref::<BorderCapture>() {
                    result.push(Quad {
                        bounds: world_rect(view, rect, c.rect),
                        uv: [c.uv; 2],
                        textures: [None, None],
                        amount: 1.,
                        tint: c.color.to_array().map(|v| v as f32 / 255.),
                        shade: false,
                        gamma_shade: false,
                        opaque: false,
                        surface: None,
                        edge_filter: Interpolation::Bilinear,
                        border: !c.fill,
                    });
                } else if let Some(c) = callback.callback.downcast_ref::<Capture>() {
                    result.push(Quad {
                        bounds: world_rect(view, rect, c.rect),
                        uv: c.uv,
                        textures: c.textures.clone(),
                        amount: c.amount,
                        tint: [1.; 4],
                        shade: c.shade,
                        gamma_shade: c.gamma_shade,
                        opaque: c.opaque,
                        surface: c.surface.clone(),
                        edge_filter: c.edge_filter,
                        border: false,
                    });
                }
            }
        }
    }
    // 取得範囲はタイルを選ぶためだけに使い、元タイルの途中でLODを切らない。
    result
        .into_iter()
        .filter(|q| intersects_view(q.bounds, world_rect(view, rect, rect)))
        .collect()
}

fn intersects_view(tile: [f64; 4], view: [f64; 4]) -> bool {
    tile[0] < view[2] && tile[2] > view[0] && tile[1] < view[3] && tile[3] > view[1]
}

#[cfg(test)]
mod tests {
    #[test]
    fn level_fill_uses_source_level_and_only_the_displayed_quadrant() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256., 256.));
        let view = crate::map::View {
            center: [0.5; 2],
            zoom: 0.,
            orientation: Default::default(),
        };
        let source = crate::map::Tile { z: 0, x: 0, y: 0 };
        let region = crate::map::Tile { z: 1, x: 1, y: 0 };
        let output = ctx.run(Default::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            clipped_tile_fill(&painter, rect, &view, source, region, true, 1.);
        });
        let fills = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Callback(c) => c.callback.downcast_ref::<BorderCapture>(),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(fills.len(), 1);
        let fill = fills[0];
        assert!(fill.fill);
        assert_eq!(fill.color, tile_level_color(0));
        assert_ne!(fill.color, tile_level_color(1));
        assert_eq!(
            fill.rect,
            egui::Rect::from_min_max(egui::pos2(128., 0.), egui::pos2(256., 128.))
        );
    }
    use super::*;
    #[test]
    fn partial_coarse_tile_keeps_source_uv_instead_of_region_edges() {
        use crate::map::Tile;
        let coarse = Tile { z: 0, x: 0, y: 0 };
        let detail = Tile { z: 1, x: 1, y: 0 };
        let (tile, uv) = clipped_border_area(coarse, detail).unwrap();
        assert_eq!(tile, detail);
        assert_eq!(
            uv,
            egui::Rect::from_min_max(egui::pos2(0.5, 0.), egui::pos2(1., 0.5))
        );
        assert_eq!(
            [
                uv.top() == 0.,
                uv.right() == 1.,
                uv.bottom() == 1.,
                uv.left() == 0.
            ],
            [true, true, false, false]
        );
        assert_eq!(
            clipped_border_area(detail, coarse),
            Some((detail, crate::lod_fade::FULL_UV))
        );
        assert!(clipped_border_area(detail, Tile { z: 1, x: 0, y: 1 }).is_none());
    }
    #[test]
    fn clipped_border_drawing_omits_the_artificial_subdivision_edges() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256., 256.));
        let view = crate::map::View {
            center: [0.5, 0.5],
            zoom: 0.,
            orientation: Default::default(),
        };
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            clipped_tile_border(
                &painter,
                rect,
                &view,
                crate::map::Tile { z: 0, x: 0, y: 0 },
                crate::map::Tile { z: 1, x: 1, y: 0 },
                false,
                egui::Color32::WHITE,
            );
        });
        let lines = output
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::LineSegment { .. }))
            .count();
        assert_eq!(lines, 2, "粗タイルの部分区画へ偽の4辺を描いた");
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            clipped_tile_border(
                &painter,
                rect,
                &view,
                crate::map::Tile { z: 0, x: 0, y: 0 },
                crate::map::Tile { z: 2, x: 1, y: 1 },
                true,
                egui::Color32::WHITE,
            );
        });
        assert!(output.shapes.is_empty(), "粗タイルの内部に境界を描いた");
    }
    #[test]
    fn one_detailed_quadrant_does_not_put_a_fine_grid_over_the_coarse_remainder() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256., 256.));
        let view = crate::map::View {
            center: [0.5, 0.5],
            zoom: 0.,
            orientation: Default::default(),
        };
        let coarse = crate::map::Tile { z: 0, x: 0, y: 0 };
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            for y in 0..2 {
                for x in 0..2 {
                    let region = crate::map::Tile { z: 1, x, y };
                    let source = if x == 0 && y == 0 { region } else { coarse };
                    clipped_tile_border(
                        &painter,
                        rect,
                        &view,
                        source,
                        region,
                        false,
                        egui::Color32::WHITE,
                    );
                }
            }
        });
        let lines: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::LineSegment { points, .. } = &s.shape {
                    Some(*points)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(lines.len(), 10);
        for [a, b] in lines {
            assert!(
                !(a.x == 128. && b.x == 128. && a.y.min(b.y) >= 128.),
                "粗タイルの右下側に偽の縦線がある"
            );
            assert!(
                !(a.y == 128. && b.y == 128. && a.x.min(b.x) >= 128.),
                "粗タイルの右下側に偽の横線がある"
            );
        }
    }
    #[test]
    fn lod_regions_select_whole_tiles_including_partial_intersections() {
        let tile = [0.25, 0.25, 0.5, 0.5];
        // 詳細描画範囲が元タイルの中央で終わっても、タイル全体を採用する。
        assert!(intersects_view(tile, [0.3, 0.3, 0.4, 0.4]));
        assert!(intersects_view(tile, [0.49, 0.49, 0.6, 0.6]));
        assert!(!intersects_view(tile, [0.5, 0.25, 0.75, 0.5]));
        assert!(!intersects_view(tile, [0., 0., 0.2, 0.2]));
    }

    #[test]
    fn image_rectangles_keep_source_uv_and_native_gutter() {
        let mut mesh = egui::Mesh::with_texture(egui::TextureId::User(17));
        let area = egui::Rect::from_min_max(egui::pos2(100., 200.), egui::pos2(900., 700.));
        let uv = egui::Rect::from_min_max(
            egui::pos2(1. / 386., 1. / 386.),
            egui::pos2(385. / 386., 385. / 386.),
        );
        mesh.add_rect_with_uv(area, uv, egui::Color32::WHITE);
        let (actual_area, actual_uv) = mesh_rectangle(&mesh.vertices);
        assert_eq!(actual_area, area);
        assert_eq!(actual_uv, uv);
        // 元タイル384画素と両端の1画素ガターをそのまま参照する。
        assert!((actual_uv.width() * 386. - 384.).abs() < 0.0001);
        let view = crate::map::View {
            center: [0.5; 2],
            zoom: 12.,
            orientation: Default::default(),
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2560., 1440.));
        let world = world_rect(&view, rect, area);
        assert!(((world[2] - world[0]) * view.scale() - 800.).abs() < 1e-7);
        assert!(((world[3] - world[1]) * view.scale() - 500.).abs() < 1e-7);
    }

    #[test]
    fn terrain_toggle_keeps_online_map_lod_at_the_same_screen_zoom() {
        use crate::map::{Lod, Source};
        for source in Source::GSI_LAYERS.into_iter().chain([
            Source::OpenStreetMap,
            Source::OpenTopoMap,
            Source::CyclOsm,
            Source::Humanitarian,
        ]) {
            for screen in [8., 11.7, 16.25, 25.] {
                for material in [6.3, 10.4, 18.] {
                    for lod in [
                        Lod::default(),
                        Lod {
                            max_level: Some(12),
                            bias: 1.,
                        },
                        Lod {
                            max_level: None,
                            bias: -1.,
                        },
                    ] {
                        assert_eq!(
                            source.tile_zoom(lod_zoom(screen, material, 0), lod),
                            source.tile_zoom(lod_zoom(screen, material, 2), lod)
                        );
                    }
                }
            }
        }
        assert_eq!(lod_zoom(15., 8., 1), 8.);
        assert_eq!(lod_zoom(15., 10., 3), 10.);
    }
    #[test]
    fn source_density_floor_keeps_highest_lod_after_padding() {
        for density in [4., 10., 12. + 1.5f64.log2(), 18.] {
            let span = minimum_span(1536., density);
            let view = map_view([-span / 2., -span / 2., span / 2., span / 2.], 1536.);
            assert!(view.zoom >= density && view.zoom < density + 1e-7);
        }
    }
}
