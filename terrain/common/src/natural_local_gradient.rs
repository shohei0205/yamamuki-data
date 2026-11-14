use crate::natural_regional_surface::Fast;
// 共有するDelaunay一周近傍だけで傾きを決める。全域連立方程式は作らない。
pub fn local_gradients(m: &Fast) -> Vec<[f64; 2]> {
    m.base
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut xx = 0.;
            let mut xy = 0.;
            let mut yy = 0.;
            let mut xz = 0.;
            let mut yz = 0.;
            let mut positive = false;
            let mut negative = false;
            for &j in &m.base.neighbors[i] {
                let q = m.base.points[j];
                let x = q[0] - p[0];
                let y = q[1] - p[1];
                let z = q[2] - p[2];
                let r2 = x * x + y * y;
                if r2 <= 0. {
                    continue;
                }
                // 共円四角形の対角線はVoronoi辺の長さが0で、選び方が一意でない。
                // その辺を傾きの近傍から除き、部分集合の再三角化にも依存させない。
                let common = m.base.incident[i]
                    .iter()
                    .copied()
                    .filter(|t| m.base.incident[j].contains(t))
                    .take(2)
                    .collect::<Vec<_>>();
                if common.len() == 2 {
                    let a = m.base.circles[common[0]];
                    let b = m.base.circles[common[1]];
                    if (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) <= 1e-14 * r2.max(1.) {
                        continue;
                    }
                }
                let w = 1. / r2;
                xx += w * x * x;
                xy += w * x * y;
                yy += w * y * y;
                xz += w * x * z;
                yz += w * y * z;
                positive |= z > 1e-8;
                negative |= z < -1e-8;
            }
            // 極値の点へ斜面を与えて尖らせない。通常点では正規化した2×2最小二乗。
            if !positive || !negative {
                return [0., 0.];
            }
            let ridge = (xx + yy) * 1e-12;
            xx += ridge;
            yy += ridge;
            let d = xx * yy - xy * xy;
            if d <= 1e-20 {
                return [0., 0.];
            }
            [(yy * xz - xy * yz) / d, (xx * yz - xy * xz) / d]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cocircular_grid_gradient_is_independent_of_subset_and_order() {
        let points = (0..11)
            .flat_map(|y| {
                (0..11).map(move |x| {
                    [
                        x as f64 * 10.,
                        y as f64 * 10.,
                        (x * x + 3 * y * y + x * y) as f64,
                    ]
                })
            })
            .collect::<Vec<_>>();
        let m = Fast::new(points.clone()).unwrap();
        let g = local_gradients(&m);
        let subset = points
            .iter()
            .copied()
            .filter(|p| p[0] >= 10. && p[0] <= 90. && p[1] >= 10. && p[1] <= 90.)
            .rev()
            .collect();
        let n = Fast::new(subset).unwrap();
        let ng = local_gradients(&n);
        for (i, p) in n.base.points.iter().enumerate() {
            if p[0] < 30. || p[0] > 70. || p[1] < 30. || p[1] > 70. {
                continue;
            }
            let j = points.iter().position(|q| p == q).unwrap();
            assert!((g[j][0] - ng[i][0]).abs() < 1e-10);
            assert!((g[j][1] - ng[i][1]).abs() < 1e-10);
        }
    }
}
