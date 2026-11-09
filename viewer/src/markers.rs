use crate::points::Mark;
use eframe::egui::{self, Color32, Stroke};

const LARGE_FILE: usize = 20_000;
const LIMIT: usize = 10_000;

pub fn displayed(candidates: Vec<Mark>, count: usize, selected: Option<usize>) -> Vec<Mark> {
    if count < LARGE_FILE || candidates.len() <= LIMIT {
        return candidates;
    }
    let (matching, filtered): (Vec<_>, Vec<_>) =
        candidates.into_iter().partition(|mark| mark.matched);
    let active = selected
        .and_then(|index| matching.iter().chain(&filtered).find(|m| m.index == index))
        .copied();
    let budget = LIMIT - usize::from(active.is_some());
    let filtered_count = filtered.len() - usize::from(active.is_some_and(|mark| !mark.matched));
    let filtered_reserve = filtered_count.min(LIMIT / 5);
    let matching_count = matching.len().min(budget - filtered_reserve);
    let mut result = Vec::with_capacity(LIMIT);
    for (marks, quota, matched) in [
        (&filtered, budget - matching_count, false),
        (&matching, matching_count, true),
    ] {
        let len = marks.len() - usize::from(active.is_some_and(|a| a.matched == matched));
        let quota = quota.min(len);
        if quota != 0 {
            let mut candidates = marks.iter().copied().filter(|m| Some(m.index) != selected);
            let mut previous = 0;
            for i in 0..quota {
                let index = i * len / quota;
                result.push(candidates.nth(index - previous).unwrap());
                previous = index + 1;
            }
        }
    }
    result.extend(active);
    result
}

pub fn in_paint_order(
    marks: &[Mark],
    selected: Option<usize>,
) -> impl DoubleEndedIterator<Item = &Mark> {
    marks
        .iter()
        .filter(|m| !m.matched)
        .chain(marks.iter().filter(|m| m.matched))
        .filter(move |m| selected != Some(m.index))
        .chain(marks.iter().filter(move |m| selected == Some(m.index)))
}

pub fn style(
    zoom: f64,
    matched: bool,
    visible_count: usize,
    active: bool,
) -> (f32, Color32, Stroke) {
    let radius: f32 = if !matched {
        if zoom < 8. {
            2.
        } else if zoom < 11. {
            3.
        } else if zoom < 14. {
            5.
        } else {
            6.
        }
    } else if visible_count > 1000 && zoom < 11. {
        if zoom < 8. { 2. } else { 4. }
    } else {
        7.
    };
    if active {
        return (
            (radius + 4.).max(10.),
            Color32::from_rgb(255, 223, 239),
            Stroke::new(4_f32, Color32::from_rgb(192, 37, 117)),
        );
    }
    let (r, g, b, a) = if matched {
        (23, 107, 82, 217)
    } else {
        (133, 141, 150, 89)
    };
    let weight: f32 = if matched && visible_count <= 1000 {
        3.
    } else {
        1.
    };
    (
        radius,
        Color32::from_rgba_unmultiplied(r, g, b, a),
        Stroke::new(weight, Color32::from_rgb(r, g, b)),
    )
}

pub struct Sprites {
    texture: egui::TextureHandle,
    styles: [(f32, Color32, Stroke); 4],
    pixels_per_point: f32,
    side: f32,
}

impl Sprites {
    pub fn update<'a>(
        cache: &'a mut Option<Self>,
        ctx: &egui::Context,
        zoom: f64,
        count: usize,
    ) -> &'a Self {
        let density = ctx.pixels_per_point();
        let styles = [
            style(zoom, false, count, false),
            style(zoom, true, count, false),
            style(zoom, false, count, true),
            style(zoom, true, count, true),
        ];
        if cache
            .as_ref()
            .is_none_or(|atlas| atlas.pixels_per_point != density || atlas.styles != styles)
        {
            use resvg::tiny_skia as skia;

            let radius = styles
                .iter()
                .map(|(r, _, s)| r + s.width)
                .fold(0_f32, f32::max);
            let side = 2 * ((radius * density).ceil() as u32 + 2);
            let mut pixmap = skia::Pixmap::new(side * 4, side).expect("地点マーカーのテクスチャ");
            let center = side as f32 / 2.;
            let mut paint = skia::Paint {
                anti_alias: true,
                ..Default::default()
            };
            for (i, (radius, fill, stroke)) in styles.iter().copied().enumerate() {
                let x = i as f32 * side as f32 + center;
                let radius = radius * density;
                let width = stroke.width * density;
                let [r, g, b, a] = fill.to_srgba_unmultiplied();
                paint.set_color_rgba8(r, g, b, a);
                if let Some(path) = skia::PathBuilder::from_circle(x, center, radius) {
                    pixmap.fill_path(
                        &path,
                        &paint,
                        skia::FillRule::Winding,
                        skia::Transform::identity(),
                        None,
                    );
                }
                let [r, g, b, a] = stroke.color.to_srgba_unmultiplied();
                paint.set_color_rgba8(r, g, b, a);
                if let Some(path) = skia::PathBuilder::from_circle(x, center, radius + width / 2.) {
                    let stroke = skia::Stroke {
                        width,
                        ..Default::default()
                    };
                    pixmap.stroke_path(&path, &paint, &stroke, skia::Transform::identity(), None);
                }
            }
            let image = egui::ColorImage::from_rgba_premultiplied(
                [side as usize * 4, side as usize],
                pixmap.data(),
            );
            *cache = Some(Self {
                texture: ctx.load_texture("地点マーカー", image, egui::TextureOptions::LINEAR),
                styles,
                pixels_per_point: density,
                side: side as f32 / density,
            });
        }
        cache.as_ref().unwrap()
    }

    pub fn mesh(
        &self,
        positions: impl Iterator<Item = (egui::Pos2, bool, bool)>,
        count: usize,
    ) -> egui::Mesh {
        let mut mesh = egui::Mesh::with_texture(self.texture.id());
        mesh.vertices.reserve(count * 4);
        mesh.indices.reserve(count * 6);
        for (position, matched, active) in positions {
            let u = (usize::from(matched) + 2 * usize::from(active)) as f32 / 4.;
            mesh.add_rect_with_uv(
                egui::Rect::from_center_size(position, egui::Vec2::splat(self.side)),
                egui::Rect::from_min_max(egui::pos2(u, 0.), egui::pos2(u + 0.25, 1.)),
                Color32::WHITE,
            );
        }
        mesh
    }

    pub fn texture_id(&self) -> egui::TextureId {
        self.texture.id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_marker_sizes() {
        for (zoom, matched, count, selected, radius) in [
            (7., true, 1001, false, 2.),
            (8., true, 1001, false, 4.),
            (11., true, 1001, false, 7.),
            (7., true, 1000, false, 7.),
            (10., false, 5, false, 3.),
            (13., false, 5, false, 5.),
            (14., false, 5, false, 6.),
            (5., false, 5, true, 10.),
            (5., true, 5, true, 11.),
        ] {
            assert_eq!(style(zoom, matched, count, selected).0, radius);
        }
    }
    #[test]
    fn large_file_sampling_keeps_selection_and_both_groups() {
        let marks = (0..24_000)
            .map(|index| Mark {
                index,
                world: [0.; 2],
                matched: index < 12_000,
            })
            .collect();
        let shown = displayed(marks, 24_000, Some(23_999));
        assert_eq!(shown.len(), LIMIT);
        assert_eq!(shown.iter().filter(|m| m.matched).count(), 7999);
        assert_eq!(shown.iter().filter(|m| m.index == 23_999).count(), 1);
    }
}
