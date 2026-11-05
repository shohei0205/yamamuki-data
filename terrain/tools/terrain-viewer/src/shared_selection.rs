use crate::shared_data::{ERRORS, Key, WORLD};
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
            maximum: 13,
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
    pub fn scale(&self) -> f64 {
        (std::f64::consts::PI * (1. - 2. * self.center[1] / WORLD))
            .sinh()
            .atan()
            .cos()
    }
    pub fn visible_depth(&self, bounds: [f64; 4]) -> Option<f64> {
        let right = DVec3::new(self.yaw.cos(), 0., -self.yaw.sin());
        let back = self.back();
        let up = back.cross(right);
        let scale = self.scale();
        let tan = (std::f64::consts::PI / 8.).tan();
        let aspect = self.viewport[0] / self.viewport[1];
        let near = (self.distance / 10000.).max(0.2);
        let mut minimum = f64::INFINITY;
        // Clipping a box face rather than its centre handles a tile containing the camera.
        let vertices = (0..8)
            .map(|i| {
                let d = DVec3::new(
                    (bounds[if i & 1 == 0 { 0 } else { 2 }] - self.center[0]) * scale,
                    if i & 4 == 0 {
                        -500. - self.height
                    } else {
                        4500. - self.height
                    },
                    (bounds[if i & 2 == 0 { 1 } else { 3 }] - self.center[1]) * scale,
                );
                [d.dot(right), d.dot(up), self.distance - d.dot(back)]
            })
            .collect::<Vec<_>>();
        let planes: [[f64; 4]; 5] = [
            [0., 0., 1., -near],
            [1., 0., tan * aspect * 1.15, 0.],
            [-1., 0., tan * aspect * 1.15, 0.],
            [0., 1., tan * 1.15, 0.],
            [0., -1., tan * 1.15, 0.],
        ];
        for face in [
            [0, 1, 3, 2],
            [4, 6, 7, 5],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 5, 7, 3],
        ] {
            let mut polygon = face.iter().map(|&i| vertices[i]).collect::<Vec<_>>();
            for plane in planes {
                let mut out = Vec::new();
                for i in 0..polygon.len() {
                    let a = polygon[i];
                    let b = polygon[(i + 1) % polygon.len()];
                    let d = |p: [f64; 3]| {
                        plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3]
                    };
                    let da = d(a);
                    let db = d(b);
                    if da >= 0. {
                        out.push(a);
                    }
                    if (da >= 0.) != (db >= 0.) {
                        let t = da / (da - db);
                        out.push(std::array::from_fn(|j| a[j] + t * (b[j] - a[j])));
                    }
                }
                polygon = out;
                if polygon.is_empty() {
                    break;
                }
            }
            for p in polygon {
                minimum = minimum.min(p[2]);
            }
        }
        minimum.is_finite().then_some(minimum.max(200.))
    }
    pub(crate) fn projected(&self, key: Key) -> (bool, f64) {
        let depth = self.visible_depth(crate::shared_data::bounds(key));
        (
            depth.is_some(),
            ERRORS[(key.0.clamp(6, 13) - 6) as usize] * self.viewport[1]
                / (2. * (std::f64::consts::PI / 8.).tan() * depth.unwrap_or(f64::INFINITY)),
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
                || faces - catalog[&key] + cost > 250000
                || selected.len() - 1 + children.len() > 96
            {
                blocked.insert(key);
                continue;
            }
            faces = faces - catalog[&key] + cost;
            selected.remove(&key);
            selected.extend(children);
        }
        balance_neighbors(&mut selected, catalog, camera, 250000, 96);
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
#[cfg(test)]
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
#[cfg(test)]
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
        balance_neighbors(&mut selected, &catalog, &camera, 100, 96);
        assert!(selected.contains(&fine));
        assert!(selected.contains(&coarse));
        for z in 7..=12 {
            catalog.insert((z, (1 << (z - 6)) - 1, 0), 50);
        }
        balance_neighbors(&mut selected, &catalog, &camera, 250000, 96);
        assert!(selected.contains(&fine));
        assert!(selected.contains(&(12, 63, 0)));
        assert!(selected.iter().map(|k| catalog[k]).sum::<usize>() <= 250000);
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

/// Temporarily retain one finer level if all four children are already decoded.
pub fn prefer_cached(
    keys: Vec<Key>,
    cached: &std::collections::HashSet<Key>,
    catalog: &HashMap<Key, usize>,
    maximum: u32,
) -> Vec<Key> {
    let mut result = keys.iter().copied().collect::<BTreeSet<_>>();
    let mut controls = result.iter().map(|k| catalog[k]).sum::<usize>();
    for k in keys {
        if k.0 >= maximum {
            continue;
        }
        let children = (0..2)
            .flat_map(|y| (0..2).map(move |x| (k.0 + 1, k.1 * 2 + x, k.2 * 2 + y)))
            .collect::<Vec<_>>();
        if children
            .iter()
            .all(|c| cached.contains(c) && catalog.contains_key(c))
        {
            let cost = children.iter().map(|c| catalog[c]).sum::<usize>();
            if result.len() + 3 <= 96 && controls - catalog[&k] + cost <= 250000 {
                controls = controls - catalog[&k] + cost;
                result.remove(&k);
                result.extend(children);
            }
        }
    }
    result.into_iter().collect()
}
#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn complete_cached_children_only_and_one_level() {
        let k = (8, 2, 3);
        let children = [(9, 4, 6), (9, 5, 6), (9, 4, 7), (9, 5, 7)];
        let mut catalog = HashMap::from([(k, 10)]);
        for c in children {
            catalog.insert(c, 10);
        }
        let mut cache = children
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(prefer_cached(vec![k], &cache, &catalog, 13).len(), 4);
        cache.remove(&children[0]);
        assert_eq!(prefer_cached(vec![k], &cache, &catalog, 13), vec![k]);
        assert_eq!(prefer_cached(vec![k], &cache, &catalog, 8), vec![k]);
    }
}
