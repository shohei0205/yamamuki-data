//! 新しい全国生成の設定検証と処理計画。旧20%プール生成は呼び出さない。
use anyhow::{Context, Result, ensure};
use clap::Parser;
use rusqlite::{Connection, OpenFlags, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
#[allow(dead_code)]
mod adaptive_tin;
#[allow(dead_code)]
mod decimation_surface;
mod mesh_vertex_format;
#[allow(dead_code)]
mod parent_vertex_reference;
mod summit_policy;
#[allow(dead_code)]
mod triangle_frontier;
mod triangle_frontier32;
mod web_mercator;
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "tools/terrain/national_mesh_settings.json")]
    settings: PathBuf,
    #[arg(long, default_value = "build/terrain-lod.sqlite3")]
    source: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long,default_value_t=default_workers())]
    workers: usize,
    #[arg(long)]
    verify_plan: bool,
}
fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 2).max(1))
        .unwrap_or(1)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    version: u32,
    policy: String,
    minimum_zoom: u32,
    maximum_zoom: u32,
    finest_error_meters: f64,
    projection: String,
    tile_scheme: String,
    finest_tile_zoom: u32,
    earth_radius_meters: f64,
    summit_error_factor: f64,
    summit_radius_meters: f64,
    peak_sample_meters: f64,
    peak_search_meters: f64,
    peak_relief_meters: f64,
    finest_uses_shared_mesh: bool,
    minimum_angle_degrees: f64,
    vertex_budget: Option<usize>,
    additional_fraction: f64,
    halo_layers: usize,
    separate_join_candidates: bool,
    generation_order: String,
    packing_order: String,
    vertex_full_zooms: Vec<u32>,
    maximum_vertex_dependency_depth: u32,
    triangle_codec: String,
    triangle_order: String,
    outer_compression: String,
    xy_units_per_tile: i64,
    height_zero_code: u16,
    land_step_meters: f64,
    seabed_step_meters: f64,
    maximum_vertices_per_stored_tile: usize,
    vertex_index_bits: u32,
    parent_reference_encoding: String,
    parent_reference_compression: String,
    coastline_in_mesh: bool,
    dem_normals: bool,
    dem_slope_hints: bool,
}
impl Settings {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 5 && self.policy == "summit-error-decimation-xyz-u32-v5",
            "生成設定の版が異なります"
        );
        ensure!(
            self.minimum_zoom == 6
                && self.maximum_zoom == 14
                && self.finest_error_meters == 12.5
                && self.projection == "EPSG:3857"
                && self.tile_scheme == "XYZ"
                && self.finest_tile_zoom == 14
                && self.earth_radius_meters == web_mercator::R,
            "レベル定義が合意と異なります"
        );
        ensure!(
            self.summit_error_factor == 0.5
                && self.summit_radius_meters == 1000.
                && self.peak_sample_meters == 100.
                && self.peak_search_meters == 500.
                && self.peak_relief_meters == 30.
                && self.finest_uses_shared_mesh,
            "山頂の誤差制約が合意と異なります"
        );
        ensure!(
            self.minimum_angle_degrees == 10.
                && self.vertex_budget.is_none()
                && self.additional_fraction == 0.,
            "旧頂点数枠を使用できません"
        );
        ensure!(
            self.halo_layers == 1 && !self.separate_join_candidates,
            "余白の設定が合意と異なります"
        );
        ensure!(
            self.generation_order == "fine-to-coarse" && self.packing_order == "coarse-to-fine",
            "生成・保存順が不正です"
        );
        ensure!(
            self.vertex_full_zooms == [6, 7, 8, 9, 12] && self.maximum_vertex_dependency_depth == 2,
            "頂点の単独保存レベルまたは親参照の上限が合意と異なります"
        );
        for z in self.minimum_zoom..=self.maximum_zoom {
            self.vertex_storage(z)?;
        }
        ensure!(
            self.triangle_codec == "TFC2"
                && self.triangle_order == "tfc2-shared-edge-greedy"
                && self.outer_compression == "none",
            "三角形の保存方式が不正です"
        );
        ensure!(
            self.xy_units_per_tile == 16384
                && self.height_zero_code == 24000
                && self.land_step_meters == 0.25
                && self.seabed_step_meters == 0.5
                && self.maximum_vertices_per_stored_tile == u32::MAX as usize
                && self.vertex_index_bits == 32
                && self.parent_reference_encoding == "tile8-vertex24"
                && self.parent_reference_compression == "group-delta-varint-or-raw",
            "頂点形式が合意と異なります"
        );
        ensure!(
            !self.coastline_in_mesh && !self.dem_normals && !self.dem_slope_hints,
            "不要な保存情報が有効です"
        );
        Ok(())
    }
    // 全頂点保存で参照を打ち切る。差分レベルでも親なし保存へのフォールバックは可能。
    fn vertex_storage(&self, z: u32) -> Result<(&'static str, u32)> {
        ensure!(
            (self.minimum_zoom..=self.maximum_zoom).contains(&z),
            "ズームが範囲外です"
        );
        let mut ancestor = z;
        while !self.vertex_full_zooms.contains(&ancestor) {
            ensure!(
                ancestor > self.minimum_zoom,
                "単独復元できる祖先がありません"
            );
            ancestor -= 1;
        }
        let depth = z - ancestor;
        ensure!(
            depth <= self.maximum_vertex_dependency_depth,
            "親参照の段数が上限を超えています"
        );
        Ok((if depth == 0 { "full" } else { "delta" }, depth))
    }
    fn levels(&self) -> Value {
        json!(
            (6..=14)
                .rev()
                .map(|z| {
                    let factor = 1_i64 << (14 - z);
                    let (mode, depth) = self.vertex_storage(z).unwrap();
                    json!({"z":z,"widthMeters":web_mercator::width(z),
                "flankErrorMeters":12.5*factor as f64,
                "summitErrorMeters":if z==14{12.5}else{6.25*factor as f64},"haloLayers":1,"vertexStorage":mode,"maximumParentDepth":depth})
                })
                .collect::<Vec<_>>()
        )
    }
}
fn read(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}
fn fingerprint(path: &Path) -> Result<Value> {
    let meta = fs::metadata(path)?;
    Ok(
        json!({"path":path.canonicalize()?.to_string_lossy(),"bytes":meta.len(),"mtimeNs":meta.modified()?.duration_since(UNIX_EPOCH)?.as_nanos().to_string()}),
    )
}
// 入力L14はXYZのz14。旧ランベルト8km目録の番号は流用しない。
fn jobs(base: &[(i64, i64)]) -> Result<BTreeSet<(i64, i64, i64)>> {
    let mut result = BTreeSet::new();
    for &(x, y) in base {
        ensure!(
            (0..16384).contains(&x) && (0..16384).contains(&y),
            "元標高のXYZ番号が範囲外です"
        );
        for z in 6..=14 {
            let divisor = 1_i64 << (14 - z);
            result.insert((z, x / divisor, y / divisor));
        }
    }
    Ok(result)
}

fn prepare(args: &Args) -> Result<Value> {
    let raw = fs::read_to_string(&args.settings)?;
    let setting: Settings = serde_json::from_str(&raw)?;
    setting.validate()?;
    ensure!(args.workers > 0, "並列数は1以上が必要です");
    let source = read(&args.source)?;
    let _: i64 = source
        .query_row("SELECT count(*) FROM tiles WHERE z=14", [], |r| r.get(0))
        .context("元標高のtilesがありません")?;
    let base = source
        .prepare("SELECT DISTINCT x,y FROM tiles WHERE z=14 ORDER BY y,x")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<(i64, i64)>>>()?;
    ensure!(!base.is_empty(), "元標高のXYZ目録が空です");
    let plan = jobs(&base)?;
    let config = json!({"settings":serde_json::from_str::<Value>(&raw)?,"source":fingerprint(&args.source)?});
    let output = args.output.canonicalize().ok();
    ensure!(
        output.as_ref() != Some(&args.source.canonicalize()?),
        "元データを出力に指定できません"
    );
    if args.verify_plan {
        let db = read(&args.output)?;
        let saved: String =
            db.query_row("SELECT value FROM metadata WHERE key='config'", [], |r| {
                r.get(0)
            })?;
        ensure!(
            serde_json::from_str::<Value>(&saved)? == config,
            "計画の設定または元データが変更されています"
        );
        let count: usize = db.query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))?;
        ensure!(count == plan.len(), "計画のジョブ数が不一致です");
        let saved = db
            .prepare("SELECT z,x,y FROM jobs")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<BTreeSet<(i64, i64, i64)>>>()?;
        ensure!(saved == plan, "計画のタイル座標が不一致です");
        return Ok(json!({"verified":true,"jobs":count}));
    }
    ensure!(
        !args.output.exists(),
        "既存出力を上書きしません。別の計画名か--verify-planを指定してください"
    );
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut db = Connection::open(&args.output)?;
    let tx = db.transaction()?;
    tx.execute_batch("CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE jobs(z INTEGER,x INTEGER,y INTEGER,state TEXT NOT NULL,PRIMARY KEY(z,x,y));CREATE TABLE stages(sequence INTEGER PRIMARY KEY,name TEXT NOT NULL,state TEXT NOT NULL);")?;
    tx.execute(
        "INSERT INTO metadata VALUES('config',?1)",
        [config.to_string()],
    )?;
    tx.execute(
        "INSERT INTO metadata VALUES('stage','prepared-not-generating')",
        [],
    )?;
    for &(z, x, y) in &plan {
        tx.execute(
            "INSERT INTO jobs VALUES(?1,?2,?3,'pending')",
            params![z, x, y],
        )?;
    }
    for (i, name) in [
        "finest-z14",
        "coarsen-z13-z6",
        "synchronize-shared-boundaries",
        "bake-halo-repairs",
        "extract-one-halo-layer",
        "quantize-coarse-to-fine",
        "pack-vertex-deltas-and-tfc2",
        "verify-final",
    ]
    .iter()
    .enumerate()
    {
        tx.execute(
            "INSERT INTO stages VALUES(?1,?2,'pending')",
            params![i, name],
        )?;
    }
    tx.commit()?;
    Ok(
        json!({"prepared":true,"generationStarted":false,"complete":false,"distributable":false,"workers":args.workers,"levels":setting.levels().as_array().unwrap().iter().map(|l|{let mut l=l.clone();let z=l["z"].as_i64().unwrap();l["tiles"]=json!(plan.iter().filter(|j|j.0==z).count());l}).collect::<Vec<_>>(),"jobs":plan.len(),"readiness":{"summitPolicy":"shared-with-regional-trial","vertexCodec":"implemented","tileReduction":"regional-only","nationalCoarsening":"not-integrated","sharedBoundaryAndHaloRepair":"not-integrated","finalDeltaPacking":"not-integrated"}}),
    )
}
fn main() -> Result<()> {
    let args = Args::parse();
    println!("{}", serde_json::to_string_pretty(&prepare(&args)?)?);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_reducer_obeys_summit_constraints() {
        let n = 25;
        let grid = (0..n * n)
            .map(|i| {
                40. * (-(((i % n) as f64 - 12.).powi(2) + ((i / n) as f64 - 12.).powi(2)) / 25.)
                    .exp()
            })
            .collect::<Vec<_>>();
        let mesh = adaptive_tin::quality_tin::generate(&grid, n, 1., 10.).unwrap();
        let mut surface =
            decimation_surface::Surface::new(mesh.points, mesh.faces, grid, vec![1.; n * n], n);
        surface.adaptive_boundary = true;
        surface.error_scales = Some(vec![0.5; n * n]);
        surface.reduce(5., 4);
        assert!(
            surface
                .faces
                .iter()
                .flatten()
                .all(|&f| surface.metric(f).2 <= 5. + 1e-6)
        );
        assert!(surface.audit().2 >= 10. - 1e-6);
    }
    fn settings() -> Settings {
        serde_json::from_str(include_str!("../../terrain/national_mesh_settings.json")).unwrap()
    }
    #[test]
    fn accepted_settings_and_levels() {
        let s = settings();
        s.validate().unwrap();
        let l = s.levels();
        assert_eq!(l[0]["summitErrorMeters"], 12.5);
        assert_eq!(l[0]["z"], 14);
        assert_eq!(l[8]["z"], 6);
        assert_eq!(l[8]["widthMeters"], web_mercator::width(6));
        assert_eq!(l[8]["summitErrorMeters"], 1600.);
    }
    #[test]
    fn checkpoints_bound_dependencies_and_report_each_level() {
        let s = settings();
        for (z, mode, depth) in [
            (6, "full", 0),
            (7, "full", 0),
            (8, "full", 0),
            (9, "full", 0),
            (10, "delta", 1),
            (11, "delta", 2),
            (12, "full", 0),
            (13, "delta", 1),
            (14, "delta", 2),
        ] {
            assert_eq!(s.vertex_storage(z).unwrap(), (mode, depth));
            let rows = s.levels();
            let row = rows
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["z"] == z)
                .unwrap();
            assert_eq!(row["vertexStorage"], mode);
            assert_eq!(row["maximumParentDepth"], depth);
        }
        let mut bad = settings();
        bad.vertex_full_zooms = vec![6];
        assert!(bad.vertex_storage(14).is_err());
        assert!(bad.validate().is_err());
        let mut bad = settings();
        bad.maximum_vertex_dependency_depth = 1;
        assert!(bad.validate().is_err());
    }
    #[test]
    fn narrow_index_configuration_is_rejected() {
        let mut s = settings();
        s.vertex_index_bits = 16;
        assert!(s.validate().is_err());
        s = settings();
        s.triangle_codec = "TFC1".into();
        assert!(s.validate().is_err());
        s = settings();
        s.maximum_vertices_per_stored_tile = 65536;
        assert!(s.validate().is_err());
    }
    #[test]
    fn old_projection_is_rejected() {
        let mut s = settings();
        s.projection = "Lambert".into();
        assert!(s.validate().is_err());
        s = settings();
        s.version = 1;
        assert!(s.validate().is_err());
    }
    #[test]
    fn old_pool_is_rejected() {
        let mut s = settings();
        s.additional_fraction = 0.2;
        assert!(s.validate().is_err());
        s.additional_fraction = 0.;
        s.vertex_budget = Some(1024);
        assert!(s.validate().is_err());
    }
    #[test]
    fn xyz_source_tiles_map_to_their_parents() {
        let p = jobs(&[(14500, 6500), (14501, 6500)]).unwrap();
        assert_eq!(p.iter().filter(|j| j.0 == 14).count(), 2);
        assert!(p.contains(&(6, 14500 / 256, 6500 / 256)));
        for &(z, x, y) in &p {
            if z > 6 {
                assert!(p.contains(&(z - 1, x / 2, y / 2)));
            }
        }
        assert!(jobs(&[(-1, 0)]).is_err());
        assert!(jobs(&[(16384, 0)]).is_err());
    }
}
