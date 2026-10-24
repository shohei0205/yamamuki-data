// 100m間隔の局所最高点から、半径1kmで滑らかな許容誤差倍率を作る。
pub fn summit_scales(grid: &[f64], n: usize, spacing: f64) -> (Vec<f64>, usize) {
    let step = (100. / spacing).round().max(1.) as usize;
    let size = (n - 1) / step + 1;
    let mut sampled = vec![0.; size * size];
    for y in 0..size {
        for x in 0..size {
            let (cx, cy) = (x * step, y * step);
            let mut total = 0.;
            let mut count = 0;
            for yy in cy.saturating_sub(step / 2)..=(cy + step / 2).min(n - 1) {
                for xx in cx.saturating_sub(step / 2)..=(cx + step / 2).min(n - 1) {
                    total += grid[yy * n + xx];
                    count += 1;
                }
            }
            sampled[y * size + x] = total / count as f64;
        }
    }
    let radius = (500. / (step as f64 * spacing)).ceil() as usize;
    let mut peaks = Vec::new();
    for y in radius..size.saturating_sub(radius) {
        for x in radius..size.saturating_sub(radius) {
            let z = sampled[y * size + x];
            let mut highest = true;
            let mut low = z;
            for yy in y - radius..=y + radius {
                for xx in x - radius..=x + radius {
                    let q = sampled[yy * size + xx];
                    low = low.min(q);
                    if q > z || (q == z && (yy, xx) < (y, x)) {
                        highest = false;
                    }
                }
            }
            if highest && z - low >= 30. {
                peaks.push((x * step, y * step));
            }
        }
    }
    let mut scale = vec![1.; n * n];
    let r = (1000. / spacing).ceil() as usize;
    for &(cx, cy) in &peaks {
        for y in cy.saturating_sub(r)..=(cy + r).min(n - 1) {
            for x in cx.saturating_sub(r)..=(cx + r).min(n - 1) {
                let t =
                    ((x as f64 - cx as f64).hypot(y as f64 - cy as f64) * spacing / 1000.).min(1.);
                let value = 0.5 + 0.5 * t * t * (3. - 2. * t);
                scale[y * n + x] = f64::min(scale[y * n + x], value);
            }
        }
    }
    (scale, peaks.len())
}
