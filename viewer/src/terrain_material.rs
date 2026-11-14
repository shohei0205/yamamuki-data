//! 2Dのタイル選択・合成順を引き継ぎ、元テクスチャを地形へ直接渡す。
use eframe::{egui, egui_wgpu::RenderState};
use yamamuki_terrain_view::direct_material::{Capture, Quad};
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
                    });
                }
            }
            egui::epaint::Primitive::Callback(callback) => {
                if let Some(c) = callback.callback.downcast_ref::<Capture>() {
                    result.push(Quad {
                        bounds: world_rect(view, rect, c.rect),
                        uv: c.uv,
                        textures: c.textures.clone(),
                        amount: c.amount,
                        tint: [1.; 4],
                        shade: c.shade,
                        gamma_shade: c.gamma_shade,
                    });
                }
            }
        }
    }
    result
        .into_iter()
        .filter_map(|mut q| {
            let area = q.bounds;
            let clip = world_rect(view, rect, rect);
            let cropped = [
                area[0].max(clip[0]),
                area[1].max(clip[1]),
                area[2].min(clip[2]),
                area[3].min(clip[3]),
            ];
            if cropped[0] >= cropped[2] || cropped[1] >= cropped[3] {
                return None;
            }
            for uv in &mut q.uv {
                let origin = uv.min;
                let size = uv.size();
                let at = |x: f64, y: f64| {
                    origin
                        + size
                            * egui::vec2(
                                ((x - area[0]) / (area[2] - area[0])) as f32,
                                ((y - area[1]) / (area[3] - area[1])) as f32,
                            )
                };
                *uv = egui::Rect::from_min_max(
                    at(cropped[0], cropped[1]),
                    at(cropped[2], cropped[3]),
                );
            }
            q.bounds = cropped;
            Some(q)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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
