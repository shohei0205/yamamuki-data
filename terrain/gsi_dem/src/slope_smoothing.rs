//! 山頂・主要な稜線を保護し、斜面の短周期の上下変動を取り除く。
use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use yamamuki_terrain::sqlite::WORLD;
const STEP: f64 = WORLD / (16384. * 256.);
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub sigma_pixels: f64,
    pub max_vertical_change_metres: f64,
    pub feature_radius_pixels: usize,
    pub feature_prominence_metres: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            sigma_pixels: 4.,
            max_vertical_change_metres: 15.,
            feature_radius_pixels: 12,
            feature_prominence_metres: 3.,
        }
    }
}
impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.sigma_pixels.is_finite()
                && (1.0..=12.).contains(&self.sigma_pixels)
                && self.max_vertical_change_metres.is_finite()
                && (0.1..=50.).contains(&self.max_vertical_change_metres)
                && self.feature_radius_pixels >= 2
                && self.feature_radius_pixels <= 32
                && self.feature_prominence_metres.is_finite()
                && self.feature_prominence_metres > 0.,
            "斜面平滑化の設定が不正です"
        );
        Ok(())
    }
}
pub struct Filtered {
    x0: i32,
    y0: i32,
    width: usize,
    height: usize,
    raw: Vec<f64>,
    values: Vec<f64>,
    protection: Vec<f64>,
    peaks: Vec<(usize, u8)>,
    pub profile: Value,
}
fn gaussian(input: &[f64], width: usize, height: usize, sigma: f64) -> Vec<f64> {
    let radius = (3. * sigma).ceil() as isize;
    let kernel = (-radius..=radius)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .collect::<Vec<_>>();
    let pass = |data: &[f64], vertical: bool| {
        let mut out = vec![f64::NAN; data.len()];
        out.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let (mut sum, mut weight) = (0., 0.);
                for (i, &w) in kernel.iter().enumerate() {
                    let offset = i as isize - radius;
                    let xx = x as isize + if vertical { 0 } else { offset };
                    let yy = y as isize + if vertical { offset } else { 0 };
                    if xx < 0 || yy < 0 || xx >= width as isize || yy >= height as isize {
                        continue;
                    }
                    let h = data[yy as usize * width + xx as usize];
                    if h.is_finite() {
                        sum += w * h;
                        weight += w;
                    }
                }
                if weight > 0. {
                    *value = sum / weight;
                }
            }
        });
        out
    };
    pass(&pass(input, false), true)
}
fn derivatives(a: &[f64], i: usize, w: usize) -> Option<([f64; 2], [f64; 3])> {
    let c = a[i];
    let l = a[i - 1];
    let r = a[i + 1];
    let u = a[i - w];
    let d = a[i + w];
    let corners = [a[i - w - 1], a[i - w + 1], a[i + w - 1], a[i + w + 1]];
    if ![c, l, r, u, d]
        .iter()
        .chain(corners.iter())
        .all(|v| v.is_finite())
    {
        return None;
    }
    Some((
        [(r - l) * 0.5, (d - u) * 0.5],
        [
            r + l - 2. * c,
            d + u - 2. * c,
            (corners[3] + corners[0] - corners[1] - corners[2]) * 0.25,
        ],
    ))
}
fn process(
    raw: Vec<f64>,
    width: usize,
    height: usize,
    config: &Config,
) -> (Vec<f64>, Vec<f64>, Vec<(usize, u8)>) {
    let fine = gaussian(&raw, width, height, config.sigma_pixels);
    let coarse = gaussian(&raw, width, height, config.sigma_pixels * 2.);
    let mut protection = vec![0.; raw.len()];
    protection
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            if y == 0 || y + 1 == height {
                return;
            }
            for x in 1..width - 1 {
                let i = y * width + x;
                if !raw[i].is_finite() {
                    continue;
                }
                let (Some((gradient, h)), Some((_, c))) =
                    (derivatives(&fine, i, width), derivatives(&coarse, i, width))
                else {
                    continue;
                };
                let disc = (h[0] - h[1]).hypot(2. * h[2]);
                let pair = [(h[0] + h[1] - disc) * 0.5, (h[0] + h[1] + disc) * 0.5];
                let (strong, weak) = if pair[0].abs() > pair[1].abs() {
                    (pair[0], pair[1])
                } else {
                    (pair[1], pair[0])
                };
                if strong.abs() < 0.02 {
                    continue;
                }
                let mut n = [h[2], strong - h[0]];
                let length = n[0].hypot(n[1]);
                if length < 1e-12 {
                    n = if h[0].abs() >= h[1].abs() {
                        [1., 0.]
                    } else {
                        [0., 1.]
                    };
                } else {
                    n[0] /= length;
                    n[1] /= length;
                }
                let broad = n[0] * n[0] * c[0] + n[1] * n[1] * c[1] + 2. * n[0] * n[1] * c[2];
                if broad * strong <= 0. {
                    continue;
                }
                let coherence = ((broad.abs() / strong.abs() - 0.1) / 0.2).clamp(0., 1.);
                let anisotropy = ((0.6 - weak.abs() / strong.abs()) / 0.4).clamp(0., 1.);
                let distance = (gradient[0] * n[0] + gradient[1] * n[1]).abs() / strong.abs();
                row[x] = coherence * anisotropy * (-0.5 * (distance / 0.7).powi(2)).exp();
            }
        });
    let radius = config.feature_radius_pixels;
    let mut peaks = Vec::new();
    for y in radius..height - radius {
        for x in radius..width - radius {
            let i = y * width + x;
            let z = raw[i];
            if !z.is_finite()
                || !coarse[i].is_finite()
                || (z - coarse[i]).abs() < config.feature_prominence_metres
            {
                continue;
            }
            let high = z > coarse[i];
            let mut selected = true;
            'neighbors: for yy in y - radius..=y + radius {
                for xx in x - radius..=x + radius {
                    if (xx as isize - x as isize).pow(2) + (yy as isize - y as isize).pow(2)
                        > (radius * radius) as isize
                    {
                        continue;
                    }
                    let j = yy * width + xx;
                    let v = raw[j];
                    if v.is_finite() && ((high && v > z) || (!high && v < z) || (j < i && v == z)) {
                        selected = false;
                        break 'neighbors;
                    }
                }
            }
            if selected {
                peaks.push((i, if high { 1 } else { 2 }));
            }
        }
    }
    let bump = (3. * config.sigma_pixels).ceil() as isize;
    for &(i, _) in &peaks {
        let x = (i % width) as isize;
        let y = (i / width) as isize;
        for dy in -bump..=bump {
            for dx in -bump..=bump {
                let xx = x + dx;
                let yy = y + dy;
                if xx < 0 || yy < 0 || xx >= width as isize || yy >= height as isize {
                    continue;
                }
                let j = yy as usize * width + xx as usize;
                let factor = (-0.5 * (dx * dx + dy * dy) as f64
                    / (config.sigma_pixels * config.sigma_pixels))
                    .exp();
                protection[j] = protection[j].max(factor);
            }
        }
    }
    // 広い曲率による標高のずれを相殺し、鉛直方向の短周期成分だけを落とす。
    let moment = |sigma: f64| {
        let radius = (3. * sigma).ceil() as i32;
        let (mut sum, mut variance) = (0., 0.);
        for i in -radius..=radius {
            let weight = (-0.5 * (i as f64 / sigma).powi(2)).exp();
            sum += weight;
            variance += weight * (i * i) as f64;
        }
        variance / sum
    };
    let vf = moment(config.sigma_pixels);
    let vc = moment(config.sigma_pixels * 2.);
    let fine_weight = vc / (vc - vf);
    let coarse_weight = vf / (vc - vf);
    let values = raw
        .iter()
        .enumerate()
        .map(|(i, &z)| {
            if z.is_finite() {
                let smooth = fine_weight * fine[i] - coarse_weight * coarse[i];
                let mut adjusted = z
                    + ((smooth - z) * (1. - protection[i])).clamp(
                        -config.max_vertical_change_metres,
                        config.max_vertical_change_metres,
                    );
                // 補償フィルターの負の裾による新しい極値を、元DEMの3×3範囲へ制限する。
                let x = i % width;
                let y = i / width;
                let (mut low, mut high) = (z, z);
                for yy in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                    for xx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                        let h = raw[yy * width + xx];
                        if h.is_finite() {
                            low = low.min(h);
                            high = high.max(h);
                        }
                    }
                }
                adjusted = adjusted.clamp(low, high);
                (adjusted * 100.).round() / 100.
            } else {
                f64::NAN
            }
        })
        .collect();
    (values, protection, peaks)
}
impl Filtered {
    pub fn load(prepared: &Path, window: &[u32], config: Config) -> Result<Self> {
        config.validate()?;
        ensure!(
            window.len() == 4
                && window[0] < window[2]
                && window[1] < window[3]
                && window[2] <= 16384
                && window[3] <= 16384,
            "平滑化窓が不正です"
        );
        let job: Value = serde_json::from_slice(&std::fs::read(prepared.join("job.json"))?)?;
        let root = Path::new(
            job["identity"]["cache"]
                .as_str()
                .context("元DEMの場所がありません")?,
        );
        let db = Connection::open_with_flags(
            prepared.join("inputs.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let lo = [window[0].saturating_sub(1), window[1].saturating_sub(1)];
        let hi = [(window[2] + 1).min(16384), (window[3] + 1).min(16384)];
        let width = ((hi[0] - lo[0]) * 256) as usize;
        let height = ((hi[1] - lo[1]) * 256) as usize;
        ensure!(
            width.checked_mul(height).is_some_and(|n| n <= 16777216),
            "平滑化領域が予算を超えました"
        );
        let mut raw = vec![f64::NAN; width * height];
        let mut query=db.prepare("SELECT x,y,sha256 FROM inputs WHERE verified=1 AND state='cached' AND x>=?1 AND x<?2 AND y>=?3 AND y<?4 ORDER BY y,x")?;
        let rows = query.query_map(params![lo[0], hi[0], lo[1], hi[1]], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (x, y, sha) = row?;
            let bytes =
                std::fs::read(root.join("14").join(x.to_string()).join(format!("{y}.png")))?;
            ensure!(
                crate::natural_inventory::hash(&bytes) == sha,
                "平滑化元DEMのSHA-256が変化しました"
            );
            let raster = crate::natural_inventory::decode(&bytes)?;
            for row in 0..256 {
                let i = ((y - lo[1]) as usize * 256 + row) * width + (x - lo[0]) as usize * 256;
                raw[i..i + 256].copy_from_slice(&raster[row * 256..row * 256 + 256]);
            }
        }
        let (values, protection, peaks) = process(raw.clone(), width, height, &config);
        let x0 = (lo[0] * 256) as i32;
        let y0 = (lo[1] * 256) as i32;
        let mut result = Self {
            x0,
            y0,
            width,
            height,
            raw,
            values,
            protection,
            peaks,
            profile: Value::Null,
        };
        let mut valid = 0u64;
        let (mut squared, mut maximum, mut raw_rough, mut smooth_rough, mut rough_count) =
            (0., 0f64, 0., 0., 0u64);
        let mut max_raw = f64::NEG_INFINITY;
        let mut max_smooth = f64::NEG_INFINITY;
        let mut protected = 0u64;
        for y in window[1] * 256..window[3] * 256 {
            for x in window[0] * 256..window[2] * 256 {
                let i = (y as i32 - y0) as usize * width + (x as i32 - x0) as usize;
                let a = result.raw[i];
                let b = result.values[i];
                if !a.is_finite() {
                    continue;
                }
                valid += 1;
                squared += (b - a).powi(2);
                maximum = maximum.max((b - a).abs());
                max_raw = max_raw.max(a);
                max_smooth = max_smooth.max(b);
                protected += u64::from(result.protection[i] >= 0.75);
                if i > width && i + width < result.raw.len() && result.protection[i] < 0.25 {
                    let indices = [i - 1, i + 1, i - width, i + width];
                    if indices
                        .iter()
                        .all(|&j| result.raw[j].is_finite() && result.values[j].is_finite())
                    {
                        raw_rough +=
                            (4. * a - indices.iter().map(|&j| result.raw[j]).sum::<f64>()).powi(2);
                        smooth_rough += (4. * b
                            - indices.iter().map(|&j| result.values[j]).sum::<f64>())
                        .powi(2);
                        rough_count += 1;
                    }
                }
            }
        }
        let peaks_in_window = result
            .peaks
            .iter()
            .filter(|(i, _)| {
                let x = x0 + (*i % width) as i32;
                let y = y0 + (*i / width) as i32;
                x >= window[0] as i32 * 256
                    && x < window[2] as i32 * 256
                    && y >= window[1] as i32 * 256
                    && y < window[3] as i32 * 256
            })
            .count();
        let mut target_bytes = Vec::with_capacity(valid as usize * 4);
        for y in window[1] * 256..window[3] * 256 {
            for x in window[0] * 256..window[2] * 256 {
                let i = (y as i32 - y0) as usize * width + (x as i32 - x0) as usize;
                let h = result.values[i];
                target_bytes.extend(
                    if h.is_finite() {
                        (h * 100.).round() as i32
                    } else {
                        i32::MIN
                    }
                    .to_le_bytes(),
                );
            }
        }
        ensure!(valid > 0, "平滑化対象に有効DEMがありません");
        result.profile = json!({"mode":"ridge-preserving-gaussian-v2","targetSha256":crate::natural_inventory::hash(&target_bytes),"config":config,"windowZ14":window,"sourceGenerationId":job["generation_id"],"rawInputManifestSha256":job["input_manifest_sha256"],"validPixels":valid,"rmsChangeMetres":(squared/valid.max(1)as f64).sqrt(),"maximumChangeMetres":maximum,"rawMaximumMetres":max_raw,"smoothedMaximumMetres":max_smooth,"protectedFeaturePixels":protected,"protectedExtrema":peaks_in_window,"slopeRoughnessBefore":(raw_rough/rough_count.max(1)as f64).sqrt(),"slopeRoughnessAfter":(smooth_rough/rough_count.max(1)as f64).sqrt(),"targetQuantizationMetres":0.01});
        Ok(result)
    }
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let xx = x - self.x0;
        let yy = y - self.y0;
        (xx >= 0 && yy >= 0 && (xx as usize) < self.width && (yy as usize) < self.height)
            .then_some(yy.max(0) as usize * self.width + xx.max(0) as usize)
    }
    pub fn sample(&self, x: f64, y: f64) -> Option<f64> {
        let x = x / STEP - 0.5;
        let y = y / STEP - 0.5;
        let ix = x.floor() as i32;
        let iy = y.floor() as i32;
        let mut sum = 0.;
        let mut weight = 0.;
        for (dy, wy) in [(0, 1. - (y - y.floor())), (1, y - y.floor())] {
            for (dx, wx) in [(0, 1. - (x - x.floor())), (1, x - x.floor())] {
                if let Some(i) = self.index(ix + dx, iy + dy) {
                    let h = self.values[i];
                    if h.is_finite() {
                        sum += h * wx * wy;
                        weight += wx * wy;
                    }
                }
            }
        }
        (weight > 0.999).then_some(sum / weight)
    }
    /// 全国生成の作業キャッシュ用。画素中心の平滑化標高と保護マスクを返す。
    pub(super) fn cached_pixel(&self, x: i32, y: i32) -> (f64, bool) {
        self.index(x, y).map_or((f64::NAN, false), |i| {
            (self.values[i], self.protection[i] >= 0.75)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_processing_windows_agree_away_from_their_halo() {
        let width = 193usize;
        let raw = (0..width * width)
            .map(|i| {
                let x = (i % width) as f64;
                let y = (i / width) as f64;
                100. + 0.2 * x
                    + 50. * (-((x - 96.).powi(2) + (y - 96.).powi(2)) / 200.).exp()
                    + 0.7 * (x * 3. + y * 7.).sin()
            })
            .collect::<Vec<_>>();
        let full = process(raw.clone(), width, width, &Config::default()).0;
        let mut cropped = Vec::new();
        for y in 32..161 {
            cropped.extend_from_slice(&raw[y * width + 32..y * width + 161]);
        }
        let cropped = process(cropped, 129, 129, &Config::default()).0;
        for y in 82..110 {
            for x in 82..110 {
                assert_eq!(full[y * width + x], cropped[(y - 32) * 129 + x - 32]);
            }
        }
    }
    #[test]
    fn plane_and_major_peak_are_preserved_while_slope_noise_is_removed() {
        let w = 129;
        let mut raw = Vec::new();
        for y in 0..w {
            for x in 0..w {
                let ridge = 80. * (-((x as f64 - 64.) / 13.).powi(2)).exp();
                let peak =
                    50. * (-((x as f64 - 64.).powi(2) + (y as f64 - 64.).powi(2)) / 100.).exp();
                let noise = if x < 45 {
                    2. * ((x * 19 + y * 13) as f64).sin()
                } else {
                    0.
                };
                raw.push(100. + 0.2 * y as f64 + ridge + peak + noise);
            }
        }
        let config = Config::default();
        let (filtered, _, peaks) = process(raw.clone(), w, w, &config);
        assert!(!peaks.is_empty());
        for &(i, _) in &peaks {
            assert!((filtered[i] - raw[i]).abs() < 0.006);
        }
        let mut before = 0.;
        let mut after = 0.;
        for y in 30..95 {
            for x in 15..25 {
                let i = y * w + x;
                before += (raw[i] - 100. - 0.2 * y as f64).powi(2);
                after += (filtered[i] - 100. - 0.2 * y as f64).powi(2);
                assert!((filtered[i] - raw[i]).abs() <= config.max_vertical_change_metres + 0.006);
            }
        }
        assert!(
            after < before * 0.4,
            "斜面の短周期ノイズが残る: {before} {after}"
        );
        let quadratic = (0..w * w)
            .map(|i| {
                100. + (i % w) as f64 * 0.3
                    + (i / w) as f64 * 0.4
                    + 0.02 * (i % w) as f64 * (i % w) as f64
            })
            .collect::<Vec<_>>();
        let (q, _, _) = process(quadratic.clone(), w, w, &config);
        for y in 40..90 {
            for x in 40..90 {
                assert!(
                    (q[y * w + x] - quadratic[y * w + x]).abs() < 0.006,
                    "広い地形の曲率が変わる"
                );
            }
        }
        let plane = (0..w * w)
            .map(|i| 100. + (i % w) as f64 * 0.3 + (i / w) as f64 * 0.4)
            .collect::<Vec<_>>();
        let (filtered, _, _) = process(plane.clone(), w, w, &config);
        for y in 40..90 {
            for x in 40..90 {
                assert!((filtered[y * w + x] - plane[y * w + x]).abs() < 0.006);
            }
        }
    }
    #[test]
    fn missing_dem_is_not_filled_and_centimetre_output_is_deterministic() {
        let w = 65;
        let mut raw = vec![10.; w * w];
        raw[32 * w + 32] = f64::NAN;
        let (a, _, _) = process(raw.clone(), w, w, &Config::default());
        let (b, _, _) = process(raw, w, w, &Config::default());
        assert!(a[32 * w + 32].is_nan());
        for (x, y) in a.iter().zip(b) {
            assert!(x.is_nan() && y.is_nan() || *x == y);
        }
    }
}
