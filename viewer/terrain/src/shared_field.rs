//! 保存タイルとGPU評価領域を分離し、拡大した部分だけ同じ制御点から再評価する。
use crate::{
    selection::Camera,
    shared_data::{Key, bounds},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Field {
    pub depth: u32,
    pub x: u32,
    pub y: u32,
}
impl Field {
    pub fn world_bounds(self, key: Key) -> [f64; 4] {
        let b = bounds(key);
        let [x, y, side] = self.relative(b[2] - b[0]);
        [b[0] + x, b[1] + y, b[0] + x + side, b[1] + y + side]
    }
    pub fn pixels(self, key: Key, camera: &Camera) -> Option<f64> {
        let b = self.world_bounds(key);
        let depth = camera.visible_depth(b)?;
        Some(
            (b[2] - b[0]) * camera.scale() * camera.viewport[1]
                / (2. * camera.tan_half_fov() * depth),
        )
    }
    /// 視点中心の一枚だけでなく、可視部分をすべて覆う四分木の葉を返す。
    pub fn visible(key: Key, camera: &Camera) -> Vec<Self> {
        let root = Self::default();
        let Some(pixels) = root.pixels(key, camera) else {
            return Vec::new();
        };
        let mut leaves = vec![(root, pixels)];
        loop {
            let candidate = leaves
                .iter()
                .enumerate()
                .filter(|(_, (f, p))| {
                    *p > 1024. * (camera.mesh_pixels / 3.).min(1.) && f.depth < (30 - key.0).min(24)
                })
                .max_by(|a, b| a.1.1.total_cmp(&b.1.1))
                .map(|(i, _)| i);
            let Some(i) = candidate else {
                break;
            };
            let (field, pixels) = leaves.swap_remove(i);
            let children = (0..2)
                .flat_map(|y| {
                    (0..2).map(move |x| Self {
                        depth: field.depth + 1,
                        x: field.x * 2 + x * 2,
                        y: field.y * 2 + y * 2,
                    })
                })
                .filter_map(|f| f.pixels(key, camera).map(|p| (f, p)))
                .collect::<Vec<_>>();
            if leaves.len() + children.len() > 64 {
                leaves.push((field, pixels));
                break;
            }
            leaves.extend(children);
        }
        leaves.into_iter().map(|(field, _)| field).collect()
    }
    pub fn relative(self, side: f64) -> [f64; 3] {
        let size = side / 2f64.powi(self.depth as i32);
        [self.x as f64 * size / 2., self.y as f64 * size / 2., size]
    }
}
/// 目標タイルで完全に覆われた背景だけを省く。未収録の子領域は背景評価を残す。
pub struct Coverage {
    keys: std::collections::HashSet<Key>,
}
impl Coverage {
    pub fn new(keys: &[Key]) -> Self {
        let mut covered = keys
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        for z in (1..=keys.iter().map(|k| k.0).max().unwrap_or(0)).rev() {
            let parents = covered
                .iter()
                .filter(|k| k.0 == z)
                .map(|k| (z - 1, k.1 / 2, k.2 / 2))
                .collect::<std::collections::HashSet<_>>();
            for parent in parents {
                if (0..2).all(|y| {
                    (0..2).all(|x| covered.contains(&(z, parent.1 * 2 + x, parent.2 * 2 + y)))
                }) {
                    covered.insert(parent);
                }
            }
        }
        Self { keys: covered }
    }
    pub fn covers(&self, key: Key, field: Field) -> bool {
        let mut node = (
            key.0 + field.depth,
            (key.1 << field.depth) + field.x as i32 / 2,
            (key.2 << field.depth) + field.y as i32 / 2,
        );
        loop {
            if self.keys.contains(&node) {
                return true;
            }
            if node.0 == 0 {
                return false;
            }
            node = (node.0 - 1, node.1 / 2, node.2 / 2);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_child_keeps_background_refinement() {
        let parent = (6, 2, 3);
        let children = [(7, 4, 6), (7, 5, 6), (7, 4, 7), (7, 5, 7)];
        assert!(Coverage::new(&children).covers(parent, Field::default()));
        let partial = Coverage::new(&children[..3]);
        assert!(!partial.covers(parent, Field::default()));
        assert!(!partial.covers(
            parent,
            Field {
                depth: 1,
                x: 2,
                y: 2
            }
        ));
        assert!(partial.covers(
            parent,
            Field {
                depth: 1,
                x: 0,
                y: 0
            }
        ));
    }
    #[test]
    fn visible_fields_cover_off_center_terrain() {
        let center = crate::selection::lonlat(138.7274, 35.3606);
        let side = crate::shared_data::WORLD / 64.;
        let key = (6, (center[0] / side) as i32, (center[1] / side) as i32);
        let camera = Camera {
            center,
            distance: 2000.,
            pitch: 0.25,
            height: 500.,
            ..Default::default()
        };
        let fields = Field::visible(key, &camera);
        assert!(fields.len() > 1 && fields.len() <= 64);
        let rect = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(1200., 800.),
        );
        let projection = crate::Projection { camera };
        for x in [0., 0.5, 1.] {
            for y in [0.25, 0.5, 0.75, 1.] {
                if let Some(world) =
                    projection.world_at(rect, eframe::egui::pos2(x * 1200., y * 800.))
                {
                    let world = world.map(|v| v * crate::shared_data::WORLD);
                    assert!(
                        fields.iter().any(|f| {
                            let b = f.world_bounds(key);
                            world[0] >= b[0] - 0.01
                                && world[0] <= b[2] + 0.01
                                && world[1] >= b[1] - 0.01
                                && world[1] <= b[3] + 0.01
                        }),
                        "画面({x},{y})の地表が評価範囲外"
                    );
                }
            }
        }
    }
    #[test]
    fn visible_fields_partition_source_without_overlapping() {
        let camera = Camera {
            center: [20000000., 17000000.],
            distance: 10000.,
            ..Default::default()
        };
        let key = (6, 31, 27);
        let fields = Field::visible(key, &camera);
        for (i, a) in fields.iter().enumerate() {
            for b in &fields[i + 1..] {
                let a = a.world_bounds(key);
                let b = b.world_bounds(key);
                assert!(
                    a[2] <= b[0] + 0.01
                        || b[2] <= a[0] + 0.01
                        || a[3] <= b[1] + 0.01
                        || b[3] <= a[1] + 0.01
                );
            }
        }
    }
}
