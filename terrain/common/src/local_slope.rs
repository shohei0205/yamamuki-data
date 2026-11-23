//! 一周近傍の勾配から固定係数1の滑らかな地形曲面を復元する。
use crate::{
    natural_local_gradient::local_gradients,
    natural_regional_surface::{Fast, Work},
};
use anyhow::{Result, ensure};
#[derive(Clone, Copy, Debug)]
pub struct Pair {
    pub original: f64,
    pub corrected: f64,
}
impl Pair {
    pub fn value(self, multiplier: f64) -> f64 {
        self.original + (self.corrected - self.original) * multiplier.clamp(0., 1.)
    }
}
pub struct Prepared<'a> {
    pub model: &'a Fast,
    pub gradients: std::borrow::Cow<'a, [[f64; 2]]>,
}
impl<'a> Prepared<'a> {
    /// 点集合を更新するまで、全国生成の全画素照会で勾配配列を共有する。
    pub fn borrowed(model: &'a Fast, gradients: &'a [[f64; 2]]) -> Self {
        Self {
            model,
            gradients: std::borrow::Cow::Borrowed(gradients),
        }
    }

    pub fn new(model: &'a Fast) -> Self {
        Self {
            model,
            gradients: local_gradients(model).into(),
        }
    }
    pub fn pair(&self, q: [f64; 2], basis: &[(usize, f64)]) -> Result<Pair> {
        let base = self.model.base.points[basis
            .first()
            .ok_or_else(|| anyhow::anyhow!("自然近傍がありません"))?
            .0][2];
        let (mut original, mut tangent, mut den, mut alpha, mut beta) = (0., 0., 0., 0., 0.);
        for &(i, w) in basis {
            let p = self.model.base.points[i];
            let d = [p[0] - q[0], p[1] - q[1]];
            let r2 = d[0] * d[0] + d[1] * d[1];
            if r2 < 1e-16 {
                return Ok(Pair {
                    original: p[2],
                    corrected: p[2],
                });
            }
            let r = r2.sqrt();
            let inverse = w / r;
            original += w * (p[2] - base);
            den += inverse;
            tangent +=
                inverse * (p[2] - base - self.gradients[i][0] * d[0] - self.gradients[i][1] * d[1]);
            alpha += w * r;
            beta += w * r2;
        }
        ensure!(den > 0., "傾き補間の重みが不正です");
        alpha /= den;
        let corrected = base + (alpha * original + beta * tangent / den) / (alpha + beta);
        let result = Pair {
            original: base + original,
            corrected,
        };
        ensure!(
            result.original.is_finite() && result.corrected.is_finite(),
            "傾き補間が非有限です"
        );
        Ok(result)
    }
    pub fn evaluate(&self, q: [f64; 2], multiplier: f64, work: &mut Work) -> Result<f64> {
        let basis = self.model.basis_borrowed(q, work)?;
        Ok(self.pair(q, basis)?.value(multiplier))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_hermite_peak_is_round_and_does_not_overshoot() -> Result<()> {
        let model = Fast::new(
            (-3..=3)
                .flat_map(|y| {
                    (-3..=3).map(move |x| {
                        [
                            x as f64 * 100.,
                            y as f64 * 100.,
                            1000. - 10. * (x * x + y * y) as f64,
                        ]
                    })
                })
                .collect(),
        )?;
        let prepared = Prepared::new(&model);
        let mut work = Work::default();
        for direction in [[1., 0.], [0., 1.], [0.6, 0.8], [-0.8, 0.6]] {
            let mut slopes = Vec::new();
            for r in [10., 1., 0.1, 0.01] {
                let h = prepared.evaluate([direction[0] * r, direction[1] * r], 1., &mut work)?;
                assert!(h <= 1000. + 1e-8);
                slopes.push((1000. - h) / r);
            }
            assert!(
                slopes[3] < slopes[0] * 0.01,
                "山頂へ近づいても円錐状の傾きが残る: {slopes:?}"
            );
        }
        for y in -40..=40 {
            for x in -40..=40 {
                let h = prepared.evaluate([x as f64 * 5., y as f64 * 5.], 1., &mut work)?;
                assert!(h <= 1000. + 1e-8, "山頂を超える補間突起: {h}");
            }
        }
        Ok(())
    }
}
