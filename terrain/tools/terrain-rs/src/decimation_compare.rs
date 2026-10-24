//! 詳細TINからの頂点削除と独立TINを同じDEMで比較する地域試験。
mod adaptive_tin;
mod summit_policy;
mod triangle_frontier;
use adaptive_tin::quality_tin::{generate, minimum_angle};
use anyhow::{Result, ensure};
use clap::Parser;
use rusqlite::{Connection, params};
use serde_json::json;
use std::{
    collections::{BinaryHeap, HashMap, HashSet},
    fs,
    path::PathBuf,
    time::Instant,
};
use summit_policy::summit_scales;
mod decimation_surface;
use decimation_surface::{Face, Point, Surface, importance, orient};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    region: String,
    #[arg(long, default_value_t = 1601)]
    size: usize,
    #[arg(long, default_value_t = 10.)]
    spacing: f64,
    #[arg(long)]
    adaptive_boundary: bool,
    #[arg(long)]
    budget_source: Option<PathBuf>,
    #[arg(long, requires = "budget_source")]
    summit_only: bool,
}
fn save(
    db: &Connection,
    region: &str,
    method: &str,
    error: f64,
    p: &[Point],
    faces: &[Face],
    grid: &[f64],
    n: usize,
    spacing: f64,
    report: serde_json::Value,
) -> Result<serde_json::Value> {
    ensure!(p.len() <= 65535, "16ビット頂点上限");
    let mut points = Vec::new();
    for v in p {
        points.extend((v[0] as u16).to_le_bytes());
        points.extend((v[1] as u16).to_le_bytes());
        let z = (grid[v[1] as usize * n + v[0] as usize] * 4.).round();
        ensure!((-32768. ..=32767.).contains(&z), "高さ範囲");
        points.extend((z as i16).to_le_bytes());
    }
    let indices = triangle_frontier::encode(faces)?;
    let body = json!({"points":p.iter().map(|v|[v[0]as f64*spacing,v[1]as f64*spacing,grid[v[1]as usize*n+v[0]as usize]]).collect::<Vec<_>>(),"faces":faces});
    let mut report = report;
    report["vertices"] = json!(p.len());
    report["boundaryVertices"] = json!(
        p.iter()
            .filter(|v| v.iter().any(|&a| a == 0 || a == n as i64 - 1))
            .count()
    );
    report["triangles"] = json!(faces.len());
    report["bytes"] = json!(points.len() + indices.len());
    report["error"] = json!(error);
    db.execute(
        "INSERT OR REPLACE INTO meshes VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            region,
            method,
            error,
            points,
            indices,
            body.to_string(),
            report.to_string()
        ],
    )?;
    Ok(report)
}
fn budget_comparison(
    args: &Args,
    db: &Connection,
    grid: Vec<f64>,
    weights: Vec<f64>,
) -> Result<()> {
    let source = Connection::open_with_flags(
        args.budget_source.as_ref().unwrap(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let raw: String = source.query_row(
        "SELECT preview FROM meshes WHERE region=?1 AND method='decimated-boundary' AND error=12.5",
        [&args.region],
        |r| r.get(0),
    )?;
    let detailed: serde_json::Value = serde_json::from_str(&raw)?;
    let points = detailed["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            [
                (v[0].as_f64().unwrap() / args.spacing).round() as i64,
                (v[1].as_f64().unwrap() / args.spacing).round() as i64,
            ]
        })
        .collect::<Vec<_>>();
    let faces: Vec<Face> = serde_json::from_value(detailed["faces"].clone())?;
    let (scales, peak_count) = summit_scales(&grid, args.size, args.spacing);
    let methods = if args.summit_only {
        vec![("summit-error", 0.)]
    } else {
        vec![("budget20", 1.2), ("budget0", 1.), ("error-only", 0.)]
    };
    for (method, factor) in methods {
        let mut surface = Surface::new(
            points.clone(),
            faces.clone(),
            grid.clone(),
            weights.clone(),
            args.size,
        );
        surface.adaptive_boundary = true;
        for error in [12.5, 25., 50., 100., 200., 400., 800.] {
            let report: String = source.query_row(
                "SELECT report FROM meshes WHERE region=?1 AND method='independent' AND error=?2",
                params![args.region, error],
                |r| r.get(0),
            )?;
            let baseline = serde_json::from_str::<serde_json::Value>(&report)?["vertices"]
                .as_u64()
                .unwrap() as usize;
            let target = if error == 12.5 {
                points.len()
            } else if factor == 0. {
                4
            } else {
                (baseline as f64 * factor).ceil() as usize
            };
            if method == "summit-error" && error > 12.5 && surface.error_scales.is_none() {
                surface.error_scales = Some(scales.clone());
            }
            let start = Instant::now();
            let (removed, reached) = if error == 12.5 {
                (0, true)
            } else {
                surface.reduce(error, target)
            };
            let (max, weighted, angle) = surface.audit();
            let constrained = surface
                .faces
                .iter()
                .flatten()
                .map(|&f| surface.metric(f).2)
                .fold(0., f64::max);
            ensure!(constrained <= error + 1e-6, "山頂付近の局所許容誤差");
            ensure!(
                max <= error + 1e-6 && angle >= 10. - 1e-6,
                "予算比較の誤差・角度"
            );
            let (p, f, ids) = surface.compact();
            save(
                db,
                &args.region,
                method,
                error,
                &p,
                &f,
                &surface.grid,
                args.size,
                args.spacing,
                json!({"generationMs":start.elapsed().as_secs_f64()*1000.,"maximum":max,"weightedMaximum":weighted,"minimumAngle":angle,"removed":removed,"target":target,"targetReached":reached,"sourceIds":ids,"independentVertices":baseline,"extraFactor":factor,"localConstraintMaximum":constrained,"summitFactor":if method=="summit-error"{0.5}else{1.},"summitRadius":1000,"detectedPeaks":peak_count}),
            )?;
            println!(
                "{} {method} {error}m {} vertices target {target} reached={reached}",
                args.region,
                p.len()
            );
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        (2..=65536).contains(&args.size) && args.spacing.is_finite() && args.spacing > 0.,
        "格子の大きさ・間隔が不正です"
    );
    let start = Instant::now();
    let bytes = fs::read(&args.input)?;
    ensure!(bytes.len() == args.size * args.size * 8, "入力格子の長さ");
    let grid = bytes
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect::<Vec<_>>();
    ensure!(grid.iter().all(|z| z.is_finite()), "欠損標高");
    let weights = importance(&grid, args.size, args.spacing);
    let importance_ms = start.elapsed().as_secs_f64() * 1000.;
    let db = Connection::open(&args.output)?;
    db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE IF NOT EXISTS meshes(region TEXT,method TEXT,error REAL,points BLOB,indices BLOB,preview TEXT,report TEXT,PRIMARY KEY(region,method,error));CREATE TABLE IF NOT EXISTS traces(region TEXT,seq INTEGER,vertex INTEGER,PRIMARY KEY(region,seq));")?;
    if args.budget_source.is_some() {
        return budget_comparison(&args, &db, grid, weights);
    }
    let mut baseline = Vec::new();
    let mut baseline_ms = Vec::new();
    let mut detailed = None;
    for error in [12.5, 25., 50., 100., 200., 400., 800.] {
        let t = Instant::now();
        let mesh = generate(&grid, args.size, error, 10.)?;
        let nv = mesh.points.len();
        println!("{} independent {error}m {nv} vertices", args.region);
        baseline.push(nv);
        baseline_ms.push(t.elapsed().as_secs_f64() * 1000.);
        let report = json!({"generationMs":t.elapsed().as_secs_f64()*1000.,"maximum":mesh.max_error,"minimumAngle":mesh.minimum_angle,"targetReached":true});
        if !args.adaptive_boundary {
            save(
                &db,
                &args.region,
                "independent",
                error,
                &mesh.points,
                &mesh.faces,
                &grid,
                args.size,
                args.spacing,
                report,
            )?;
        }
        if detailed.is_none() {
            detailed = Some(mesh);
        }
    }
    let mesh = detailed.unwrap();
    let mut surface = Surface::new(mesh.points, mesh.faces, grid, weights, args.size);
    surface.adaptive_boundary = args.adaptive_boundary;
    let method = if args.adaptive_boundary {
        "decimated-boundary"
    } else {
        "decimated"
    };
    let mut reports = Vec::new();
    for (i, error) in [12.5, 25., 50., 100., 200., 400., 800.]
        .into_iter()
        .enumerate()
    {
        let t = Instant::now();
        let target = if i == 0 {
            baseline[0]
        } else {
            (baseline[i] as f64 * 1.2).ceil() as usize
        };
        let (before, reached) = if i == 0 {
            (0, true)
        } else {
            surface.reduce(error, target)
        };
        let (max, weighted, angle) = surface.audit();
        ensure!(
            max <= error + 1e-6 && angle >= 10. - 1e-6,
            "誤差・角度の検証失敗"
        );
        let (p, f, ids) = surface.compact();
        let report = json!({"generationMs":t.elapsed().as_secs_f64()*1000.+if i==0 {baseline_ms[0]+importance_ms}else{0.},"maximum":max,"weightedMaximum":weighted,"minimumAngle":angle,"removed":before,"target":target,"targetReached":reached,"sourceIds":ids,"boundaryVertices":(0..surface.p.len()).filter(|&v|surface.live[v]&&surface.is_boundary(v)).count()});
        reports.push(save(
            &db,
            &args.region,
            method,
            error,
            &p,
            &f,
            &surface.grid,
            args.size,
            args.spacing,
            report,
        )?);
        println!(
            "{} {method} {error}m {} vertices target {target} reached={reached}",
            args.region,
            p.len()
        );
    }
    let tx = db.unchecked_transaction()?;
    let trace_region = if args.adaptive_boundary {
        format!("{}:boundary", args.region)
    } else {
        args.region.clone()
    };
    tx.execute("DELETE FROM traces WHERE region=?1", [&trace_region])?;
    for (seq, v) in surface.removed.iter().enumerate() {
        tx.execute(
            "INSERT OR REPLACE INTO traces VALUES(?1,?2,?3)",
            params![trace_region, seq, v],
        )?;
    }
    tx.commit()?;
    println!(
        "{} complete {:.1}s",
        args.region,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removal_preserves_area_boundary_and_subset() {
        let n = 17;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 / 4.).sin() * ((i / n) as f64 / 4.).sin() * 10.)
            .collect::<Vec<_>>();
        let mesh = generate(&grid, n, 0.2, 10.).unwrap();
        let mut s = Surface::new(mesh.points, mesh.faces, grid, vec![1.; n * n], n);
        let count = s.p.len();
        s.reduce(2., 4);
        let (p, f, ids) = s.compact();
        assert!(p.len() < count);
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        let area: i128 = f.iter().map(|f| orient(p[f[0]], p[f[1]], p[f[2]])).sum();
        assert_eq!(area, 2 * ((n - 1) * (n - 1)) as i128);
        let (max, _, angle) = s.audit();
        assert!(max <= 2. + 1e-6 && angle >= 10. - 1e-6);
        for (i, v) in s.p.iter().enumerate() {
            if v.iter().any(|&a| a == 0 || a == n as i64 - 1) {
                assert!(s.live[i]);
            }
        }
    }
}

#[cfg(test)]
mod nested_tests {
    use super::*;
    #[test]
    fn stages_are_nested_and_deterministic() {
        let n = 25;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 / 3.).sin() * ((i / n) as f64 / 4.).cos() * 20.)
            .collect::<Vec<_>>();
        let mesh = generate(&grid, n, 0.5, 10.).unwrap();
        let mut a = Surface::new(
            mesh.points.clone(),
            mesh.faces.clone(),
            grid.clone(),
            vec![1.; n * n],
            n,
        );
        let mut b = Surface::new(mesh.points, mesh.faces, grid, vec![1.; n * n], n);
        let mut previous = (0..a.p.len()).collect::<HashSet<_>>();
        for error in [1., 2., 5., 10.] {
            a.reduce(error, 4);
            b.reduce(error, 4);
            let (p, f, ids) = a.compact();
            assert_eq!(a.compact(), b.compact());
            let ids = ids.into_iter().collect::<HashSet<_>>();
            assert!(ids.is_subset(&previous));
            previous = ids;
            let area: i128 = f.iter().map(|t| orient(p[t[0]], p[t[1]], p[t[2]])).sum();
            assert_eq!(area, 2 * ((n - 1) * (n - 1)) as i128);
            let (e, _, angle) = a.audit();
            assert!(e <= error + 1e-6 && angle >= 10. - 1e-6);
        }
    }
}

#[cfg(test)]
mod summit_tests {
    use super::*;
    #[test]
    fn peaks_get_smooth_stricter_limits_but_flat_ground_does_not() {
        let n = 51;
        let grid = (0..n * n)
            .map(|i| {
                80. * (-(((i % n) as f64 - 25.).powi(2) + ((i / n) as f64 - 25.).powi(2)) / 50.)
                    .exp()
            })
            .collect::<Vec<_>>();
        let (scale, count) = summit_scales(&grid, n, 40.);
        assert!(count > 0);
        assert!(scale[25 * n + 25] < 0.51);
        assert_eq!(scale[0], 1.);
        assert!(scale.iter().all(|&v| (0.5..=1.).contains(&v)));
        let (flat, count) = summit_scales(&vec![0.; n * n], n, 40.);
        assert_eq!(count, 0);
        assert!(flat.iter().all(|&v| v == 1.));
    }
    #[test]
    fn removal_respects_local_error_limits() {
        let n = 25;
        let grid = (0..n * n)
            .map(|i| {
                30. * (-(((i % n) as f64 - 12.).powi(2) + ((i / n) as f64 - 12.).powi(2)) / 25.)
                    .exp()
            })
            .collect::<Vec<_>>();
        let mesh = generate(&grid, n, 1., 10.).unwrap();
        let mut s = Surface::new(mesh.points, mesh.faces, grid, vec![1.; n * n], n);
        s.error_scales = Some(
            (0..n * n)
                .map(|i| {
                    if (i % n).abs_diff(12) < 5 && (i / n).abs_diff(12) < 5 {
                        0.5
                    } else {
                        1.
                    }
                })
                .collect(),
        );
        s.adaptive_boundary = true;
        s.reduce(5., 4);
        let (max, _, angle) = s.audit();
        assert!(max <= 5. + 1e-6 && angle >= 10. - 1e-6);
        assert!(
            s.faces
                .iter()
                .flatten()
                .all(|&f| s.metric(f).2 <= 5. + 1e-6)
        );
    }
}
#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn boundary_can_coarsen_without_holes_or_losing_corners() {
        let n = 25;
        let grid = (0..n * n)
            .map(|i| ((i % n) as f64 / 4.).sin() * ((i / n) as f64 / 3.).cos() * 20.)
            .collect::<Vec<_>>();
        let mesh = generate(&grid, n, 0.5, 10.).unwrap();
        let mut s = Surface::new(mesh.points, mesh.faces, grid, vec![1.; n * n], n);
        s.adaptive_boundary = true;
        let before = (0..s.p.len()).filter(|&v| s.is_boundary(v)).count();
        let mut previous = (0..s.p.len()).collect::<HashSet<_>>();
        let mut plan_previous = previous.clone();
        for error in [1., 2., 5., 10., 40.] {
            let plan = s.boundary_plan(error);
            assert!(plan.is_subset(&plan_previous));
            plan_previous = plan;
            s.reduce(error, 4);
            let (p, f, ids) = s.compact();
            let ids = ids.into_iter().collect::<HashSet<_>>();
            assert!(ids.is_subset(&previous));
            previous = ids;
            let area: i128 = f.iter().map(|t| orient(p[t[0]], p[t[1]], p[t[2]])).sum();
            assert_eq!(area, 2 * ((n - 1) * (n - 1)) as i128);
            let (e, _, angle) = s.audit();
            assert!(e <= error + 1e-6 && angle >= 10. - 1e-6);
            for corner in [
                [0, 0],
                [0, n as i64 - 1],
                [n as i64 - 1, 0],
                [n as i64 - 1, n as i64 - 1],
            ] {
                assert!(p.contains(&corner));
            }
        }
        let after = (0..s.p.len())
            .filter(|&v| s.live[v] && s.is_boundary(v))
            .count();
        assert!(after < before, "boundary {} -> {}", before, after);
    }
}
