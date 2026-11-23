//! 固定曲面の余分な局所突起を全画素で検査し、粗品質は細品質の点だけを復帰する。
use super::*;
const GUARD: f64 = 0.5;
fn bad_peak_with(
    mut evaluate: impl FnMut([f64; 2], &mut Work) -> Result<f64>,
    input: &Input,
    source: &HashMap<(i32, i32), &Input>,
    dem: &mut Dem,
) -> Result<(Option<Vec<[f64; 3]>>, u64)> {
    let mut target = vec![f64::NAN; 258 * 258];
    for dy in -1..=1 {
        for dx in -1..=1 {
            let Some(i) = source.get(&(input.x + dx, input.y + dy)) else {
                continue;
            };
            let a = dem.read(i)?;
            for yy in 0..258i32 {
                let ry = yy - 1 - dy * 256;
                if !(0..256).contains(&ry) {
                    continue;
                }
                for xx in 0..258i32 {
                    let rx = xx - 1 - dx * 256;
                    if (0..256).contains(&rx) {
                        target[(yy * 258 + xx) as usize] = a[(ry * 256 + rx) as usize];
                    }
                }
            }
        }
    }
    let mut values = vec![f64::NAN; target.len()];
    let mut work = Work::default();
    for y in 0..258 {
        for x in 0..258 {
            let k = y * 258 + x;
            if !target[k].is_finite() {
                continue;
            }
            values[k] = evaluate(
                [
                    (input.x as f64 * 256. + x as f64 - 0.5) * STEP,
                    (input.y as f64 * 256. + y as f64 - 0.5) * STEP,
                ],
                &mut work,
            )?;
        }
    }
    let mut tested = 0;
    for y in 1..257 {
        for x in 1..257 {
            let k = y * 258 + x;
            let v = values[k];
            if !v.is_finite() {
                continue;
            }
            let mut high = f64::NEG_INFINITY;
            let mut raw_high = f64::NEG_INFINITY;
            let mut valid = true;
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let j = (k as isize + dy * 258 + dx) as usize;
                    valid &= values[j].is_finite() && target[j].is_finite();
                    high = high.max(values[j]);
                    raw_high = raw_high.max(target[j]);
                }
            }
            if !valid {
                continue;
            }
            tested += 1;
            if (v - high).max(0.) - (target[k] - raw_high).max(0.) > GUARD + 1e-7 {
                let mut candidates = Vec::new();
                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        let xx = x as isize + dx;
                        let yy = y as isize + dy;
                        let j = (yy * 258 + xx) as usize;
                        candidates.push([
                            (input.x as f64 * 256. + xx as f64 - 0.5) * STEP,
                            (input.y as f64 * 256. + yy as f64 - 0.5) * STEP,
                            target[j],
                        ]);
                    }
                }
                return Ok((Some(candidates), tested));
            }
        }
    }
    Ok((None, tested))
}
pub(super) fn certify(
    s: &Store,
    z: i64,
    inputs: &[Input],
    dem: &mut Dem,
    root: &Path,
) -> Result<()> {
    if !s.fixed_slope()? {
        return Ok(());
    }
    let name = format!("shape-{z}");
    if state(s, &name)?.is_some_and(|v| v["epoch"].as_i64() == Some(s.epoch(z).unwrap_or(-1))) {
        return Ok(());
    }
    let source = inputs
        .iter()
        .map(|i| ((i.x, i.y), i))
        .collect::<HashMap<_, _>>();
    let mut verified = fit_cache::Verified::empty(inputs);
    let mut added = 0u64;
    let mut valid = 0u64;
    let mut pass = 0;
    loop {
        pass += 1;

        for (n, input) in inputs.iter().enumerate() {
            if verified.has(n) {
                continue;
            }
            let b = rect(input);
            let bounds = [
                b[0] - STEP * 2.,
                b[1] - STEP * 2.,
                b[2] + STEP * 2.,
                b[3] + STEP * 2.,
            ];
            loop {
                ensure!(
                    !root.join("stop-requested").exists(),
                    "停止要求を受けました"
                );
                let mut p = s.patch(z, bounds)?;
                let (bad, tested) = bad_peak_with(|q, w| p.evaluate(q, w), input, &source, dem)?;
                valid += tested;
                let Some(candidates) = bad else {
                    verified.mark(input);
                    break;
                };
                let mut count = 0;
                for q in candidates {
                    let q = if z == 13 {
                        quant(q)?
                    } else {
                        let Some(id) = nearest_fine(s, z, [q[0], q[1]])? else {
                            continue;
                        };
                        quant(s.point(id)?)?
                    };
                    if z == 13 && s.replace_finest_height(q)? {
                        s.db.execute(
                            "UPDATE points SET guard=2 WHERE x=?1 AND y=?2 AND guard=0",
                            params![q[0], q[1]],
                        )?;
                        verified.clear();
                        p = s.patch(z, bounds)?;
                        count += 1;
                        continue;
                    }
                    let xy = [q[0] as f64 / 16., q[1] as f64 / 16.];
                    let mut circles = repair_support_circles(&p, xy)?;
                    if s.insert_cached(z, q, &mut p)? {
                        if z == 13 {
                            s.db.execute(
                                "UPDATE points SET guard=2 WHERE x=?1 AND y=?2 AND guard=0",
                                params![q[0], q[1]],
                            )?;
                        }
                        circles.extend(repair_support_circles(&p, xy)?);
                        verified.invalidate(&circles);
                        count += 1;
                    }
                }
                ensure!(
                    count > 0,
                    "突起補修に使える点がありません z{z} {}/{}",
                    input.x,
                    input.y
                );
                added += count;
            }
            if n % 16 == 0 || n + 1 == inputs.len() {
                progress(
                    root,
                    json!({"stage":"fixed-shape-audit","z":z,"pass":pass,"done":n+1,"total":inputs.len(),"verifiedInputs":verified.count(),"added":added,"complete":false}),
                )?;
            }
        }
        if verified.count() < inputs.len() {
            continue;
        }
        let certified = state(s, &format!("certify-{z}"))?.and_then(|v| v["epoch"].as_i64());
        if certified != Some(s.epoch(z)?) {
            // 突起補修で角度や許容誤差を壊さない。既存の全画素検査も再実行する。
            super::certify(s, z, inputs, dem, root)?;
            verified.clear();
            continue;
        }
        done(
            s,
            &name,
            json!({"epoch":s.epoch(z)?,"peakGuardMetres":GUARD,"newSharpPeaks":0,"auditedPixels":valid,"added":added,"passes":pass}),
        )?;
        return Ok(());
    }
}

/// 配布用の整数点から復元した曲面を同じ基準で再監査する。
pub(super) fn audit_saved(
    model: &crate::natural_regional_surface::Fast,
    origin: [f64; 2],
    input: &Input,
    source: &HashMap<(i32, i32), &Input>,
    dem: &mut Dem,
) -> Result<u64> {
    let (bad, tested) = bad_peak_with(
        |q, w| model.evaluate([q[0] - origin[0], q[1] - origin[1]], w),
        input,
        source,
        dem,
    )?;
    ensure!(
        bad.is_none(),
        "保存後の曲面に余分な突起があります {}/{}",
        input.x,
        input.y
    );
    Ok(tested)
}
