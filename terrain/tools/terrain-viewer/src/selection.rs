use crate::data::{Key, WORLD};
use glam::DVec3;
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug)]
pub struct Camera {
    pub center: [f64; 2],
    pub height: f64,
    pub distance: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub viewport: [f64; 2],
    pub maximum: u32,
    pub auto: bool,
    pub lod_quality: f64,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            center: lonlat(138.0, 36.0),
            height: 500.0,
            distance: 3600000.0,
            yaw: 0.0,
            pitch: 0.85,
            viewport: [1200.0, 800.0],
            maximum: 14,
            auto: true,
            lod_quality: 2.0,
        }
    }
}
pub fn lonlat(lon: f64, lat: f64) -> [f64; 2] {
    [
        WORLD * (lon + 180.0) / 360.0,
        WORLD * (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0,
    ]
}
impl Camera {
    pub fn pan(&mut self, dx: f64, dy: f64) {
        let scale = self.distance * 0.8 / self.viewport[1];
        let (sin, cos) = self.yaw.sin_cos();
        let x = dx * scale;
        let y = -dy * scale;
        self.center[0] -= x * cos - y * sin;
        self.center[1] += x * sin + y * cos;
    }
    pub fn back(&self) -> DVec3 {
        DVec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        )
    }
    fn projected(&self, key: Key) -> (bool, f64) {
        let span = WORLD / 2f64.powi(key.0 as i32);
        let delta = DVec3::new(
            (key.1 as f64 + 0.5) * span - self.center[0],
            2000.0 - self.height,
            (key.2 as f64 + 0.5) * span - self.center[1],
        ) - self.back() * self.distance;
        let back = self.back();
        let right = DVec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let up = back.cross(right);
        let support = |v: DVec3| span * 0.5 * (v.x.abs() + v.z.abs()) + 2100.0 * v.y.abs();
        let depth = -delta.dot(back);
        let radius = support(back);
        let tangent = (std::f64::consts::PI / 8.0).tan();
        let mut visible = depth + radius > (self.distance / 10000.0).max(1.0);
        for (axis, slope) in [
            (right, tangent * self.viewport[0] / self.viewport[1] * 1.15),
            (up, tangent * 1.15),
        ] {
            for sign in [-1.0, 1.0] {
                let plane = axis * sign + back * slope;
                visible &= delta.dot(plane) <= support(plane);
            }
        }
        (
            visible,
            12.5 * 2f64.powi(14 - key.0 as i32) * self.viewport[1]
                / (2.0 * tangent * (depth - radius * 0.5).max(100.0)),
        )
    }
}
pub fn select(catalog: &HashMap<Key, usize>, camera: &Camera) -> Vec<Key> {
    let mut selected: BTreeSet<_> = catalog
        .keys()
        .filter(|k| k.0 == if camera.auto { 6 } else { camera.maximum } && camera.projected(**k).0)
        .copied()
        .collect();
    if camera.auto {
        let mut faces: usize = selected.iter().map(|k| catalog[k]).sum();
        let mut blocked = BTreeSet::new();
        loop {
            let best = selected
                .iter()
                .filter(|k| k.0 < camera.maximum && !blocked.contains(*k))
                .map(|k| (*k, camera.projected(*k).1))
                .max_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            let Some((key, error)) = best else { break };
            if error <= 3.0 / camera.lod_quality.clamp(0.5, 4.0) {
                break;
            }
            let children: Vec<_> = (0..2)
                .flat_map(|y| (0..2).map(move |x| (key.0 + 1, key.1 * 2 + x, key.2 * 2 + y)))
                .filter(|k| catalog.contains_key(k) && camera.projected(*k).0)
                .collect();
            let cost: usize = children.iter().map(|k| catalog[k]).sum();
            if children.is_empty()
                || faces - catalog[&key] + cost > 350000
                || selected.len() - 1 + children.len() > 192
            {
                blocked.insert(key);
                continue;
            }
            faces = faces - catalog[&key] + cost;
            selected.remove(&key);
            selected.extend(children);
        }
        balance_neighbors(&mut selected, catalog, camera, 350000, 192);
    }
    selected.into_iter().collect()
}

fn adjacent(a: Key, b: Key) -> bool {
    let sa = 1i64 << (14 - a.0);
    let sb = 1i64 << (14 - b.0);
    let (ax, ay) = (a.1 as i64 * sa, a.2 as i64 * sa);
    let (bx, by) = (b.1 as i64 * sb, b.2 as i64 * sb);
    ((ax + sa == bx || bx + sb == ax) && ay.max(by) < (ay + sa).min(by + sb))
        || ((ay + sa == by || by + sb == ay) && ax.max(bx) < (ax + sa).min(bx + sb))
}
// 細かい側を粗くしない。予算があれば隣接する粗い側を詳細化する。
fn balance_neighbors(
    selected: &mut BTreeSet<Key>,
    catalog: &HashMap<Key, usize>,
    camera: &Camera,
    budget: usize,
    tile_budget: usize,
) {
    loop {
        let keys: Vec<_> = selected.iter().copied().collect();
        let mut coarse = BTreeSet::new();
        for (i, &a) in keys.iter().enumerate() {
            for &b in &keys[i + 1..] {
                if a.0.abs_diff(b.0) > 2 && adjacent(a, b) {
                    coarse.insert(if a.0 < b.0 { a } else { b });
                }
            }
        }
        let faces: usize = selected.iter().map(|k| catalog[k]).sum();
        let mut changed = false;
        for key in coarse {
            let children: Vec<_> = (0..2)
                .flat_map(|y| (0..2).map(move |x| (key.0 + 1, key.1 * 2 + x, key.2 * 2 + y)))
                .filter(|k| catalog.contains_key(k) && camera.projected(*k).0)
                .collect();
            let cost: usize = children.iter().map(|k| catalog[k]).sum();
            if children.is_empty()
                || faces - catalog[&key] + cost > budget
                || selected.len() - 1 + children.len() > tile_budget
            {
                continue;
            }
            selected.remove(&key);
            selected.extend(children);
            changed = true;
            break;
        }
        if !changed {
            break;
        }
    }
}
pub fn level_at(keys: &[Key], center: [f64; 2]) -> Option<u32> {
    keys.iter()
        .filter(|&&(z, x, y)| {
            let span = WORLD / 2f64.powi(z as i32);
            center[0] >= x as f64 * span
                && center[0] < (x + 1) as f64 * span
                && center[1] >= y as f64 * span
                && center[1] < (y + 1) as f64 * span
        })
        .map(|k| k.0)
        .max()
}
pub fn stale_zoom_would_coarsen(
    current: &Camera,
    requested: &Camera,
    previous: &[Key],
    incoming: &[Key],
    previous_divisions: u32,
    incoming_divisions: u32,
) -> bool {
    if requested.distance <= current.distance * 1.02 {
        return false;
    }
    match (
        level_at(previous, current.center),
        level_at(incoming, current.center),
    ) {
        (Some(old), Some(new)) => {
            new < old || (new == old && incoming_divisions < previous_divisions)
        }
        (Some(_), None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quality_slider_refines_auto_lod_but_does_not_change_fixed_level() {
        let mut camera = Camera::default();
        camera.center = lonlat(138.727, 35.36);
        camera.height = 0.;
        camera.distance = 100000.;
        camera.pitch = 1.56;
        let key = |z| {
            let span = WORLD / 2f64.powi(z as i32);
            (
                z,
                (camera.center[0] / span).floor() as i32,
                (camera.center[1] / span).floor() as i32,
            )
        };
        let catalog: HashMap<_, _> = (6..=14).map(|z| (key(z), 10)).collect();
        camera.lod_quality = 1.;
        let previous = select(&catalog, &camera);
        camera.lod_quality = 2.;
        let standard = select(&catalog, &camera);
        camera.lod_quality = 4.;
        let finest = select(&catalog, &camera);
        assert!(standard[0].0 > previous[0].0);
        assert!(finest[0].0 > standard[0].0);
        camera.auto = false;
        camera.maximum = 12;
        camera.lod_quality = 0.5;
        let fixed = select(&catalog, &camera);
        camera.lod_quality = 4.;
        assert_eq!(fixed, select(&catalog, &camera));
        assert_eq!(fixed[0].0, 12);
    }
    #[test]
    fn neighbor_budget_never_collapses_detailed_tiles() {
        let coarse = (6, 0, 0);
        let fine = (14, 256, 0);
        let mut camera = Camera::default();
        camera.center = [WORLD / 64., WORLD / 32768.];
        camera.distance = 12000.;
        camera.pitch = 1.56;
        let mut catalog = HashMap::from([(coarse, 50), (fine, 50), ((7, 1, 0), 1000)]);
        let mut selected = BTreeSet::from([coarse, fine]);
        balance_neighbors(&mut selected, &catalog, &camera, 100, 192);
        assert!(selected.contains(&fine));
        assert!(selected.contains(&coarse));
        for z in 7..=12 {
            catalog.insert((z, (1 << (z - 6)) - 1, 0), 50);
        }
        balance_neighbors(&mut selected, &catalog, &camera, 350000, 192);
        assert!(selected.contains(&fine));
        assert!(selected.contains(&(12, 63, 0)));
        assert!(selected.iter().map(|k| catalog[k]).sum::<usize>() <= 350000);
    }
    #[test]
    fn stale_wide_view_cannot_replace_current_detail_while_zooming_in() {
        let mut current = Camera::default();
        current.center = lonlat(138.727, 35.36);
        current.distance = 1000.;
        let tile = |z| {
            let s = WORLD / 2f64.powi(z);
            (
                z as u32,
                (current.center[0] / s).floor() as i32,
                (current.center[1] / s).floor() as i32,
            )
        };
        let mut requested = current.clone();
        requested.distance = 4000.;
        assert!(stale_zoom_would_coarsen(
            &current,
            &requested,
            &[tile(14)],
            &[tile(11)],
            4,
            4
        ));
        assert!(stale_zoom_would_coarsen(
            &current,
            &requested,
            &[tile(14)],
            &[tile(14)],
            4,
            2
        ));
        assert!(!stale_zoom_would_coarsen(
            &current,
            &requested,
            &[tile(12)],
            &[tile(14)],
            4,
            2
        ));
        assert!(!stale_zoom_would_coarsen(
            &current,
            &current,
            &[tile(14)],
            &[tile(11)],
            4,
            4
        ));
    }
    #[test]
    fn dragged_ground_follows_pointer_at_all_headings() {
        for yaw in [0., 0.5, 1.57, 3.14, 4.71] {
            for (dx, dy) in [(10., 0.), (0., 10.)] {
                let mut c = Camera::default();
                c.yaw = yaw;
                let before = c.center;
                c.pan(dx, dy);
                let ground = DVec3::new(before[0] - c.center[0], 0., before[1] - c.center[1]);
                let right = DVec3::new(yaw.cos(), 0., -yaw.sin());
                let up = c.back().cross(right);
                if dx > 0. {
                    assert!(ground.dot(right) > 0.);
                    assert!(ground.dot(up).abs() < 1e-7);
                }
                if dy > 0. {
                    assert!(-ground.dot(up) > 0.);
                    assert!(ground.dot(right).abs() < 1e-7);
                }
            }
        }
    }
    #[test]
    fn projection_origin() {
        let p = lonlat(0.0, 0.0);
        assert!((p[0] - WORLD / 2.0).abs() < 1e-8);
        assert!((p[1] - WORLD / 2.0).abs() < 1e-8);
    }
    #[test]
    fn no_parent_overlap() {
        let c = Camera::default();
        let mut map = HashMap::new();
        for z in 6..=8 {
            for y in 20 * (1 << (z - 6))..22 * (1 << (z - 6)) {
                for x in 55 * (1 << (z - 6))..57 * (1 << (z - 6)) {
                    map.insert((z, x, y), 1000);
                }
            }
        }
        let selected = select(&map, &c);
        for a in &selected {
            for b in &selected {
                assert!(a.0 >= b.0 || a.1 != b.1 >> (b.0 - a.0) || a.2 != b.2 >> (b.0 - a.0));
            }
        }
    }
}
