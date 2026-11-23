//! 制御点ごとの局所傾き調整。Sibson面積座標で係数も滑らかに補間する。
use crate::{
    natural_local_gradient::local_gradients,
    natural_regional_surface::{Fast, Work},
};
use anyhow::{Result, ensure};
#[derive(Clone, Copy, Debug)]
pub struct Pair {
    pub original: f64,
    pub corrected: f64,
    pub amount: f64,
}
impl Pair {
    pub fn value(self, multiplier: f64) -> f64 {
        self.original + (self.corrected - self.original) * self.amount * multiplier.clamp(0., 1.)
    }
}
pub struct Prepared<'a> {
    pub model: &'a Fast,
    pub gradients: std::borrow::Cow<'a, [[f64; 2]]>,
    pub automatic_amounts: Option<Vec<f64>>,
}
impl<'a> Prepared<'a> {
    /// 点集合を更新するまで、全国生成の全画素照会で勾配配列を共有する。
    pub fn borrowed(model: &'a Fast, gradients: &'a [[f64; 2]]) -> Self {
        Self {
            model,
            gradients: std::borrow::Cow::Borrowed(gradients),
            automatic_amounts: None,
        }
    }

    pub fn new(model: &'a Fast) -> Self {
        Self {
            model,
            gradients: local_gradients(model).into(),
            automatic_amounts: None,
        }
    }
    pub fn automatic(model: &'a Fast) -> Self {
        let gradients = local_gradients(model);
        let amounts = crate::natural_local_gradient::automatic_amounts(model, &gradients);
        Self {
            model,
            gradients: gradients.into(),
            automatic_amounts: Some(amounts),
        }
    }
    pub fn pair(
        &self,
        q: [f64; 2],
        basis: &[(usize, f64)],
        amounts: Option<&[u8]>,
    ) -> Result<Pair> {
        ensure!(
            amounts.is_none_or(|v| v.len() == self.model.base.points.len()),
            "傾き調整量と点数が一致しません"
        );
        let base = self.model.base.points[basis
            .first()
            .ok_or_else(|| anyhow::anyhow!("自然近傍がありません"))?
            .0][2];
        let (mut original, mut tangent, mut den, mut alpha, mut beta, mut amount) =
            (0., 0., 0., 0., 0., 0.);
        for &(i, w) in basis {
            let p = self.model.base.points[i];
            let d = [p[0] - q[0], p[1] - q[1]];
            let r2 = d[0] * d[0] + d[1] * d[1];
            let a = amounts.map_or_else(
                || self.automatic_amounts.as_ref().map_or(1., |v| v[i]),
                |v| f64::from(v[i]) / 255.,
            );
            if r2 < 1e-16 {
                return Ok(Pair {
                    original: p[2],
                    corrected: p[2],
                    amount: a,
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
            amount += w * a;
        }
        ensure!(den > 0., "傾き補間の重みが不正です");
        alpha /= den;
        let corrected = base + (alpha * original + beta * tangent / den) / (alpha + beta);
        let result = Pair {
            original: base + original,
            corrected,
            amount: amount.clamp(0., 1.),
        };
        ensure!(
            result.original.is_finite() && result.corrected.is_finite(),
            "傾き補間が非有限です"
        );
        Ok(result)
    }
    pub fn evaluate(
        &self,
        q: [f64; 2],
        amounts: Option<&[u8]>,
        multiplier: f64,
        work: &mut Work,
    ) -> Result<f64> {
        let basis = self.model.basis_borrowed(q, work)?;
        Ok(self.pair(q, basis, amounts)?.value(multiplier))
    }
}
/// 倍率1での全画素の許容範囲を制御点へ交差し、8bit量子化後も誤差を保つ。
pub struct Fit {
    pub numerator: Vec<f64>,
    pub denominator: Vec<f64>,
    pub floor: Vec<f64>,
    pub cap: Vec<f64>,
    pub samples: u64,
}
impl Fit {
    pub fn new(points: usize) -> Self {
        Self {
            numerator: vec![0.; points],
            denominator: vec![0.; points],
            floor: vec![0.; points],
            cap: vec![1.; points],
            samples: 0,
        }
    }
    pub fn observe(
        &mut self,
        prepared: &Prepared<'_>,
        q: [f64; 2],
        height: f64,
        error_down: f64,
        error_up: f64,
        work: &mut Work,
    ) -> Result<()> {
        ensure!(
            height.is_finite()
                && error_down.is_finite()
                && error_up.is_finite()
                && error_down >= 0.
                && error_up >= 0.,
            "傾き評価の標高・誤差が不正です"
        );
        let basis = prepared.model.basis_borrowed(q, work)?;
        let pair = prepared.pair(q, basis, None)?;
        let residual = height - pair.original;
        let delta = pair.corrected - pair.original;
        self.samples += 1;
        if delta.abs() < 1e-12 {
            ensure!(
                residual <= error_down + 1e-7 && -residual <= error_up + 1e-7,
                "補正量0の位置が許容誤差を超えています"
            );
            return Ok(());
        }
        let a = (height - error_down - pair.original) / delta;
        let b = (height + error_up - pair.original) / delta;
        let minimum = a.min(b).max(0.);
        let maximum = a.max(b).min(1.);
        ensure!(
            minimum <= maximum + 1e-12,
            "局所係数0〜1では許容誤差を満たせません"
        );
        for &(i, w) in basis {
            self.numerator[i] += w * delta * residual;
            self.denominator[i] += w * delta * delta;
            self.floor[i] = self.floor[i].max(minimum);
            self.cap[i] = self.cap[i].min(maximum);
        }
        Ok(())
    }
    pub fn quantize(numerator: f64, denominator: f64, lower: f64, upper: f64) -> Result<u8> {
        let minimum = (lower * 255. - 1e-10).ceil().clamp(0., 255.);
        let maximum = (upper * 255. + 1e-10).floor().clamp(0., 255.);
        ensure!(
            lower <= upper + 1e-12 && minimum <= maximum,
            "共有点の係数を8bitの許容範囲へ収められません"
        );
        let value = if denominator > 1e-24 {
            (numerator / denominator).clamp(0., 1.)
        } else {
            1.
        };
        Ok((value * 255.).floor().clamp(minimum, maximum) as u8)
    }
    pub fn finish(&self) -> Result<Vec<u8>> {
        (0..self.cap.len())
            .map(|i| {
                Self::quantize(
                    self.numerator[i],
                    self.denominator[i],
                    self.floor[i],
                    self.cap[i],
                )
            })
            .collect()
    }
    pub fn merge(&mut self, other: &Self) -> Result<()> {
        ensure!(
            self.cap.len() == other.cap.len(),
            "傾き評価の点数が一致しません"
        );
        for i in 0..self.cap.len() {
            self.numerator[i] += other.numerator[i];
            self.denominator[i] += other.denominator[i];
            self.floor[i] = self.floor[i].max(other.floor[i]);
            self.cap[i] = self.cap[i].min(other.cap[i]);
        }
        self.samples += other.samples;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Fast {
        Fast::new(
            (0..9)
                .flat_map(|y| {
                    (0..9).map(move |x| {
                        [
                            x as f64 * 10.,
                            y as f64 * 10.,
                            (x * x + 2 * y * y + x * y) as f64,
                        ]
                    })
                })
                .collect(),
        )
        .unwrap()
    }
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
                let h =
                    prepared.evaluate([direction[0] * r, direction[1] * r], None, 1., &mut work)?;
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
                let h = prepared.evaluate([x as f64 * 5., y as f64 * 5.], None, 1., &mut work)?;
                assert!(h <= 1000. + 1e-8, "山頂を超える補間突起: {h}");
            }
        }
        Ok(())
    }
    #[test]
    fn multiplier_one_can_correct_an_invalid_original_surface() -> Result<()> {
        let m = model();
        let prepared = Prepared::new(&m);
        let mut work = Work::default();
        let q = [35., 36.];
        let pair = prepared.pair(q, m.basis_borrowed(q, &mut work)?, None)?;
        let error = (pair.corrected - pair.original).abs() * 0.2;
        assert!(error > 1e-6);
        let mut fit = Fit::new(m.base.points.len());
        fit.observe(&prepared, q, pair.corrected, error, error, &mut work)?;
        let amounts = fit.finish()?;
        assert!(
            (prepared.evaluate(q, Some(&amounts), 1., &mut work)? - pair.corrected).abs()
                <= error + 1e-8
        );
        assert!((pair.original - pair.corrected).abs() > error);
        assert!(fit.floor.iter().any(|v| *v > 0.));
        assert!(Fit::quantize(1., 1., 0.5001, 0.5002).is_err());
        Ok(())
    }
    #[test]
    fn locally_fitted_amounts_preserve_error_and_improve_squared_error() -> Result<()> {
        let m = model();
        let prepared = Prepared::new(&m);
        let mut work = Work::default();
        let mut fit = Fit::new(m.base.points.len());
        let mut samples = Vec::new();
        for y in 1..80 {
            for x in 1..80 {
                let q = [x as f64, y as f64];
                let basis = m.basis_borrowed(q, &mut work)?;
                let p = prepared.pair(q, basis, None)?;
                let h = p.original + (p.corrected - p.original) * (0.1 + 0.8 * q[0] / 80.);
                let error = (p.corrected - p.original).abs() + 1.;
                fit.observe(&prepared, q, h, error, error, &mut work)?;
                samples.push((q, h, error));
            }
        }
        let amounts = fit.finish()?;
        assert!(amounts.iter().any(|a| *a > 0 && *a < 255));
        let (mut before, mut after) = (0., 0.);
        for (q, h, error) in samples {
            let p = prepared.pair(q, m.basis_borrowed(q, &mut work)?, Some(&amounts))?;
            before += (p.original - h).powi(2);
            after += (p.value(1.) - h).powi(2);
            for multiplier in [0., 0.25, 0.5, 0.75, 1.] {
                let e = p.value(multiplier) - h;
                assert!((-error - 1e-7..=error + 1e-7).contains(&e));
            }
        }
        assert!(after < before * 0.95, "{before} -> {after}");
        for (i, p) in m.base.points.iter().enumerate() {
            assert!(
                (prepared.evaluate([p[0], p[1]], Some(&amounts), 1., &mut work)? - p[2]).abs()
                    < 1e-8,
                "{i}"
            );
        }
        Ok(())
    }
    #[test]
    fn zero_and_full_and_interpolated_amounts_are_consistent() -> Result<()> {
        let m = model();
        let prepared = Prepared::new(&m);
        let mut work = Work::default();
        let q = [35., 36.];
        let basis = m.basis_borrowed(q, &mut work)?;
        let p = prepared.pair(q, basis, None)?;
        for a in [0, 64, 127, 255] {
            let v = vec![a; m.base.points.len()];
            let actual = prepared.pair(q, basis, Some(&v))?;
            assert!(
                (actual.value(1.)
                    - (p.original + (p.corrected - p.original) * f64::from(a) / 255.))
                    .abs()
                    < 1e-10
            );
        }
        let mut fit = Fit::new(m.base.points.len());
        let mut other = Fit::new(m.base.points.len());
        fit.observe(&prepared, q, p.original, 0., 0., &mut work)?;
        other.observe(&prepared, q, p.original, 0., 0., &mut work)?;
        fit.merge(&other)?;
        assert_eq!(fit.samples, 2);
        assert!(
            (prepared.evaluate(q, Some(&fit.finish()?), 1., &mut work)? - p.original).abs() < 1e-10
        );
        Ok(())
    }
}
