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
    /// 描画・曲面評価の目標間隔。カメラ停止後に段階的に小さくする（論理px）。
    pub mesh_pixels: f64,
    pub fov_y: f64,
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
            mesh_pixels: 3.0,
            fov_y: std::f64::consts::FRAC_PI_4,
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
    /// 地図の操作を反映する。地形への追従や衝突回避で基準標高を補正しない。
    pub fn set_map_view(
        &mut self,
        center: [f64; 2],
        zoom: f64,
        yaw: f64,
        tilt: f64,
        viewport: [f64; 2],
    ) -> bool {
        let next = center.map(|v| v * WORLD);
        let scale = (std::f64::consts::PI * (1. - 2. * center[1]))
            .sinh()
            .atan()
            .cos();
        let distance = (WORLD / (256. * zoom.exp2()) * viewport[1] * scale
            / (2. * (std::f64::consts::PI / 8.).tan()))
        .min(10_000_000.);
        let pitch = (std::f64::consts::FRAC_PI_2 - tilt).clamp(0.05, 1.56);
        let changed = self.fov_y != std::f64::consts::FRAC_PI_4
            || self.center != next
            || self.distance != distance
            || self.yaw != yaw
            || self.pitch != pitch;
        self.fov_y = std::f64::consts::FRAC_PI_4;
        self.center = next;
        self.distance = distance;
        self.yaw = yaw;
        self.pitch = pitch;
        changed
    }
    pub fn tan_half_fov(&self) -> f64 {
        (self.fov_y / 2.).tan()
    }
    pub fn eye(&self) -> [f64; 3] {
        let back = self.back();
        let scale = self.scale();
        [
            self.center[0] + back.x * self.distance / scale,
            self.center[1] + back.z * self.distance / scale,
            self.height + back.y * self.distance,
        ]
    }
    pub fn set_observer(
        &mut self,
        world: [f64; 2],
        height: f64,
        yaw: f64,
        pitch: f64,
        fov: f64,
        viewport: [f64; 2],
    ) -> bool {
        let before = (
            self.center,
            self.height,
            self.yaw,
            self.pitch,
            self.fov_y,
            self.viewport,
            self.distance,
        );
        self.yaw = yaw;
        self.pitch = pitch.clamp(-1.5, 1.5);
        self.fov_y = fov.clamp(15f64.to_radians(), 100f64.to_radians());
        self.distance = 1000.;
        self.viewport = viewport;
        self.center = world;
        let back = self.back();
        // Mercatorの縮尺変化を含めても、回転で視点のXYZが移動しないようにする。
        for _ in 0..4 {
            let scale = self.scale();
            self.center = [
                world[0] - back.x * self.distance / scale,
                world[1] - back.z * self.distance / scale,
            ];
        }
        self.height = height - back.y * self.distance;
        before
            != (
                self.center,
                self.height,
                self.yaw,
                self.pitch,
                self.fov_y,
                self.viewport,
                self.distance,
            )
    }
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
    /// 遠景の上限は視点と日本の最高標高を余裕をもって含む地平線距離から求める。
    /// 俯瞰で広域を表示するときは、注視距離に応じて範囲を広げる。
    pub fn far_depth(&self) -> f64 {
        let radius = 6_371_000.;
        let eye_height = self.eye()[2].max(0.);
        let horizon = (2. * radius * eye_height + eye_height * eye_height).sqrt()
            + (2. * radius * 4500. + 4500f64.powi(2)).sqrt();
        (horizon * 1.15).max(self.distance * 8.).max(10000.)
    }
    pub fn visible_depth(&self, bounds: [f64; 4]) -> Option<f64> {
        let right = DVec3::new(self.yaw.cos(), 0., -self.yaw.sin());
        let back = self.back();
        let up = back.cross(right);
        let scale = self.scale();
        let tan = self.tan_half_fov();
        let aspect = self.viewport[0] / self.viewport[1];
        let near = (self.distance / 10000.).max(0.2);
        let mut minimum = f64::INFINITY;
        // 箱の中心ではなく面を切り、カメラを内包するタイルも評価する。
        let vertices: [[f64; 3]; 8] = std::array::from_fn(|i| {
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
        });
        let planes: [[f64; 4]; 6] = [
            [0., 0., 1., -near],
            [0., 0., -1., self.far_depth()],
            [1., 0., tan * aspect * 1.15, 0.],
            [-1., 0., tan * aspect * 1.15, 0.],
            [0., 1., tan * 1.15, 0.],
            [0., -1., tan * 1.15, 0.],
        ];
        let mut crossing = [false; 6];
        for (i, plane) in planes.iter().enumerate() {
            let inside = vertices
                .iter()
                .filter(|p| plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3] >= 0.)
                .count();
            if inside == 0 {
                return None;
            }
            crossing[i] = inside != 8;
        }
        if !crossing.iter().any(|v| *v) {
            return Some(
                vertices
                    .iter()
                    .map(|v| v[2])
                    .fold(f64::INFINITY, f64::min)
                    .max(near),
            );
        }
        for face in [
            [0, 1, 3, 2],
            [4, 6, 7, 5],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 5, 7, 3],
        ] {
            let mut polygon = face.iter().map(|&i| vertices[i]).collect::<Vec<_>>();
            for (plane_index, plane) in planes.iter().enumerate() {
                if !crossing[plane_index] {
                    continue;
                }
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
        minimum.is_finite().then_some(minimum.max(near))
    }
    pub(crate) fn projected(&self, key: Key) -> (bool, f64) {
        let depth = self.visible_depth(crate::shared_data::bounds(key));
        (
            depth.is_some(),
            ERRORS[(key.0.clamp(6, 13) - 6) as usize] * self.viewport[1]
                / (2. * self.tan_half_fov() * depth.unwrap_or(f64::INFINITY)),
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

/// 4子タイルがすべて復号済みなら、一段細かいレベルを一時的に維持する。
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

#[cfg(test)]
mod map_camera_tests {
    use super::*;
    #[test]
    fn dragging_and_zooming_never_raise_the_camera_to_terrain() {
        let mut camera = Camera {
            height: 100.,
            ..Default::default()
        };
        for (lon, lat) in [(138.7, 35.3), (138.7274, 35.3606), (137.647, 36.342)] {
            let center = lonlat(lon, lat).map(|v| v / WORLD);
            for zoom in [12., 18., 24.] {
                camera.set_map_view(center, zoom, 0., 1.2, [900., 600.]);
                assert_eq!(camera.height, 100.);
            }
        }
        assert!(camera.distance < 100.);
    }
}

#[cfg(test)]
mod culling_benchmarks {
    use super::*;
    #[test]
    fn observer_culls_far_distance_but_keeps_nearby_mountains() {
        let mut camera = Camera::default();
        camera.set_observer(lonlat(137.638, 36.249), 1502., 0., 0., 0.8, [1920., 1080.]);
        let eye = camera.eye();
        let scale = camera.scale();
        let ahead = |distance: f64| {
            let x = eye[0];
            let y = eye[1] - distance / scale;
            [x - 1000., y - 1000., x + 1000., y + 1000.]
        };
        assert!(camera.far_depth() > 350000. && camera.far_depth() < 500000.);
        assert!(camera.visible_depth(ahead(50000.)).is_some());
        assert!(camera.visible_depth(ahead(300000.)).is_some());
        assert!(
            camera
                .visible_depth(ahead(camera.far_depth() + 20000.))
                .is_none()
        );
        let behind = [
            eye[0] - 1000.,
            eye[1] + 10000.,
            eye[0] + 1000.,
            eye[1] + 12000.,
        ];
        assert!(camera.visible_depth(behind).is_none());
        // 上空から全国を見る操作では、景色モードの距離に固定しない。
        camera.set_map_view(
            lonlat(138., 36.).map(|v| v / WORLD),
            5.,
            0.,
            0.8,
            [1920., 1080.],
        );
        assert!(camera.far_depth() > 1_000_000.);
    }
    #[test]
    #[ignore = "視錐台判定のCPU時間を実測する"]
    fn benchmark_visible_depth() {
        let mut camera = Camera::default();
        camera.set_observer(lonlat(137.638, 36.249), 1502., 0., 0., 0.8, [1920., 1080.]);
        let eye = camera.eye();
        let boxes = (0..20000)
            .map(|i| {
                let x = eye[0] + ((i % 200) as f64 - 100.) * 4000.;
                let y = eye[1] - ((i / 200) as f64 - 25.) * 10000.;
                [x, y, x + 2000., y + 2000.]
            })
            .collect::<Vec<_>>();
        let start = std::time::Instant::now();
        let mut visible = 0;
        for _ in 0..5 {
            for b in &boxes {
                if std::hint::black_box(&camera)
                    .visible_depth(std::hint::black_box(*b))
                    .is_some()
                {
                    visible += 1;
                }
            }
        }
        eprintln!(
            "視錐台判定100000回: {:.2} ms / 可視 {}",
            start.elapsed().as_secs_f64() * 1000.,
            visible
        );
    }
}
