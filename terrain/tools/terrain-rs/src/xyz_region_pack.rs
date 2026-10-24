//! 地域試験用YMV4/YMT3の保存と、各タイルの即時復元照合。
use super::*;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct V {
    pub(super) x: i64,
    pub(super) y: i64,
    pub(super) z: u16,
}
type Key = (i64, i64);
#[derive(Clone, Copy)]
struct Ref {
    tile: Key,
    index: u32,
    body: bool,
}
pub(super) fn height(u: u16) -> f64 {
    let v = u as i32 - 24000;
    v as f64 * if v < 0 { 0.5 } else { 0.25 }
}
pub(super) fn intersects(f: [usize; 3], p: &[V], b: [i64; 4]) -> bool {
    let a = f.map(|i| p[i]);
    for axis in 0..2 {
        let c = a.map(|v| if axis == 0 { v.x } else { v.y });
        if *c.iter().max().unwrap() < b[axis] || *c.iter().min().unwrap() > b[axis + 2] {
            return false;
        }
    }
    for i in 0..3 {
        let u = a[i];
        let v = a[(i + 1) % 3];
        let dx = -(v.y - u.y) as i128;
        let dy = (v.x - u.x) as i128;
        let t = a.map(|p| p.x as i128 * dx + p.y as i128 * dy);
        let r = [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])]
            .map(|(x, y)| x as i128 * dx + y as i128 * dy);
        if t.iter().max().unwrap() < r.iter().min().unwrap()
            || r.iter().max().unwrap() < t.iter().min().unwrap()
        {
            return false;
        }
    }
    true
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
pub(super) fn pack_vertices(
    z: u32,
    tile: Key,
    vertices: &[V],
    refs: &[(u32, V)],
    new: &[V],
    deps: &[Key],
) -> Result<Vec<u8>> {
    let values = refs.iter().map(|v| v.0).collect::<Vec<_>>();
    let (mode, stream) = parent_vertex_reference::encode_stream(&values)?;
    let mut b = Vec::new();
    b.extend(b"YMV4");
    b.extend(4u16.to_le_bytes());
    b.extend((mode as u16).to_le_bytes());
    for n in [
        vertices.len(),
        deps.len(),
        refs.len(),
        new.len(),
        stream.len(),
        0,
    ] {
        b.extend(u32::try_from(n)?.to_le_bytes());
    }
    for &(x, y) in deps {
        b.extend((z - 1).to_le_bytes());
        b.extend((x as i32).to_le_bytes());
        b.extend((y as i32).to_le_bytes());
    }
    b.extend(stream);
    let k = 1i64 << (14 - z);
    let origin = [tile.0 * 16384 * k, tile.1 * 16384 * k];
    for v in new {
        ensure!(v.x % k == 0 && v.y % k == 0, "保存格子が不整合");
        let x = v.x / k - origin[0] / k;
        let y = v.y / k - origin[1] / k;
        b.extend(
            i16::try_from(x)
                .context("余白Xが16bit範囲外")?
                .to_le_bytes(),
        );
        b.extend(
            i16::try_from(y)
                .context("余白Yが16bit範囲外")?
                .to_le_bytes(),
        );
        b.extend(v.z.to_le_bytes());
    }
    Ok(b)
}
pub(super) fn unpack_vertices(
    b: &[u8],
    z: u32,
    tile: Key,
    parents: &HashMap<Key, Vec<V>>,
) -> Result<Vec<V>> {
    ensure!(
        b.len() >= 32 && &b[..4] == b"YMV4" && u16::from_le_bytes(b[4..6].try_into().unwrap()) == 4,
        "YMV4ヘッダー不正"
    );
    let mode = u16::from_le_bytes(b[6..8].try_into().unwrap());
    ensure!(mode <= 1, "YMV4方式不正");
    let (v, d, r, a, p) = (
        u32_at(b, 8) as usize,
        u32_at(b, 12) as usize,
        u32_at(b, 16) as usize,
        u32_at(b, 20) as usize,
        u32_at(b, 24) as usize,
    );
    ensure!(
        d <= 256 && v == r + a && u32_at(b, 28) == 0 && b.len() == 32 + 12 * d + p + 6 * a,
        "YMV4長さ不正"
    );
    let mut deps = Vec::new();
    let mut counts = Vec::new();
    for i in 0..d {
        let o = 32 + 12 * i;
        ensure!(u32_at(b, o) == z - 1, "YMV4親ズーム不正");
        let key = (
            i32::from_le_bytes(b[o + 4..o + 8].try_into().unwrap()) as i64,
            i32::from_le_bytes(b[o + 8..o + 12].try_into().unwrap()) as i64,
        );
        let parent = parents.get(&key).context("YMV4親がありません")?;
        counts.push(u32::try_from(parent.len())?);
        deps.push(key);
    }
    let at = 32 + 12 * d;
    let refs = parent_vertex_reference::decode_stream(mode as u8, &b[at..at + p], r, &counts)?;
    let mut out = Vec::with_capacity(v);
    for value in refs {
        let (slot, index) = parent_vertex_reference::decode(value.to_le_bytes(), &counts)?;
        out.push(parents[&deps[slot]][index as usize]);
    }
    let k = 1i64 << (14 - z);
    for i in 0..a {
        let o = at + p + 6 * i;
        let x = i16::from_le_bytes(b[o..o + 2].try_into().unwrap()) as i64;
        let y = i16::from_le_bytes(b[o + 2..o + 4].try_into().unwrap()) as i64;
        out.push(V {
            x: (tile.0 * 16384 + x) * k,
            y: (tile.1 * 16384 + y) * k,
            z: u16::from_le_bytes(b[o + 4..o + 6].try_into().unwrap()),
        });
    }
    Ok(out)
}
pub(super) fn pack_faces(body: &[[usize; 3]], halo: &[[usize; 3]]) -> Result<Vec<u8>> {
    let a = triangle_frontier32::encode(body)?;
    let h = triangle_frontier32::encode(halo)?;
    let mut out = Vec::new();
    out.extend(b"YMT3");
    out.extend(3u16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    for n in [body.len() + halo.len(), body.len(), a.len(), h.len()] {
        out.extend(u32::try_from(n)?.to_le_bytes());
    }
    out.extend(a);
    out.extend(h);
    Ok(out)
}
fn canonical(mut f: [usize; 3]) -> [usize; 3] {
    let i = (0..3).min_by_key(|&i| f[i]).unwrap();
    f.rotate_left(i);
    f
}
pub(super) fn check_faces(
    b: &[u8],
    body: &[[usize; 3]],
    halo: &[[usize; 3]],
    nv: usize,
) -> Result<()> {
    ensure!(
        b.len() >= 24 && &b[..4] == b"YMT3" && b[4..8] == [3, 0, 0, 0],
        "YMT3ヘッダー不正"
    );
    let (t, n, l, h) = (
        u32_at(b, 8) as usize,
        u32_at(b, 12) as usize,
        u32_at(b, 16) as usize,
        u32_at(b, 20) as usize,
    );
    ensure!(
        t == body.len() + halo.len() && n == body.len() && b.len() == 24 + l + h,
        "YMT3長さ不正"
    );
    for (bytes, want) in [(&b[24..24 + l], body), (&b[24 + l..], halo)] {
        let mut got = triangle_frontier32::decode(bytes, want.len(), nv)?
            .into_iter()
            .map(canonical)
            .collect::<Vec<_>>();
        let mut want = want.iter().copied().map(canonical).collect::<Vec<_>>();
        got.sort();
        want.sort();
        ensure!(got == want, "TFC2復元面不一致");
    }
    Ok(())
}
pub(super) fn quantized_audit(
    r: &Region,
    points: &[V],
    faces: &[[usize; 3]],
    grid: &[f64],
    scales: &[f64],
    z: u32,
) -> serde_json::Value {
    // 座標配列は全体で一度だけ作り、スレッド間で共有する。
    let p = points
        .iter()
        .map(|v| [(v.x - r.px * 64) / 64, (v.y - r.py * 64) / 64])
        .collect::<Vec<_>>();
    let results = faces
        .par_chunks(64)
        .map(|part| quantized_audit_serial(r, points, &p, part, grid, scales, z))
        .collect::<Vec<_>>();
    let mut maximum = 0f64;
    let mut ratio = 0f64;
    let mut angle = 180f64;
    let mut bad = 0u64;
    for value in results {
        maximum = maximum.max(value["maximumErrorMeters"].as_f64().unwrap());
        ratio = ratio.max(value["maximumLocalErrorRatio"].as_f64().unwrap());
        angle = angle.min(value["minimumAngleDegrees"].as_f64().unwrap());
        bad += value["invalidFaces"].as_u64().unwrap();
    }
    json!({"maximumErrorMeters":maximum,"maximumLocalErrorRatio":ratio,"minimumAngleDegrees":angle,"invalidFaces":bad,"passed":ratio<=1.+1e-6&&angle>=10.-1e-7&&bad==0})
}
fn quantized_audit_serial(
    r: &Region,
    points: &[V],
    p: &[[i64; 2]],
    faces: &[[usize; 3]],
    grid: &[f64],
    scales: &[f64],
    z: u32,
) -> serde_json::Value {
    let tolerance = 12.5 * 2f64.powi(14 - z as i32);
    let mut maximum = 0f64;
    let mut ratio = 0f64;
    let mut angle = 180f64;
    let mut bad = 0usize;
    for &f in faces {
        let [a, b, c] = f.map(|v| p[v]);
        let den = decimation_surface::orient(a, b, c) as f64;
        if den <= 0. {
            bad += 1;
            continue;
        }
        angle = angle.min(adaptive_tin::quality_tin::minimum_angle(p, f));
        let za = height(points[f[0]].z);
        let zb = height(points[f[1]].z) - za;
        let zc = height(points[f[2]].z) - za;
        for y in a[1].min(b[1]).min(c[1]).max(0)..=a[1].max(b[1]).max(c[1]).min(r.n as i64 - 1) {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for (u, v) in [(a, b), (b, c), (c, a)] {
                if y < u[1].min(v[1]) || y > u[1].max(v[1]) {
                    continue;
                }
                if u[1] == v[1] {
                    lo = lo.min(u[0].min(v[0]) as f64);
                    hi = hi.max(u[0].max(v[0]) as f64);
                } else {
                    let x = u[0] as f64
                        + (y - u[1]) as f64 * (v[0] - u[0]) as f64 / (v[1] - u[1]) as f64;
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
            for x in ((lo - 1e-8).ceil() as i64).max(0)
                ..=((hi + 1e-8).floor() as i64).min(r.n as i64 - 1)
            {
                let u = decimation_surface::orient(c, a, [x, y]);
                let v = decimation_surface::orient(a, b, [x, y]);
                if u < 0 || v < 0 || (u + v) as f64 > den {
                    continue;
                }
                let i = y as usize * r.n + x as usize;
                let e = (za + u as f64 / den * zb + v as f64 / den * zc - grid[i]).abs();
                maximum = maximum.max(e);
                let limit = if z == 14 {
                    12.5
                } else {
                    (tolerance * scales[i]).max(12.5)
                };
                ratio = ratio.max(e / limit);
            }
        }
    }
    json!({"maximumErrorMeters":maximum,"maximumLocalErrorRatio":ratio,"minimumAngleDegrees":angle,"invalidFaces":bad,"passed":ratio<=1.+1e-6&&angle>=10.-1e-7&&bad==0})
}
fn valid_tiles(r: &Region, dir: &Path) -> Result<HashSet<(u32, i64, i64)>> {
    let mask = fs::read(dir.join("valid.bits"))?;
    let stride = r.n.div_ceil(8);
    ensure!(mask.len() == stride * r.n, "有効標高マスクの長さが不正です");
    let mut finest = HashSet::new();
    for y in 0..r.n {
        let ty = (r.py + y as i64).div_euclid(256);
        let mut x = 0;
        while x < r.n {
            let gx = r.px + x as i64;
            let tx = gx.div_euclid(256);
            let end = (x + (256 - gx.rem_euclid(256)) as usize).min(r.n);
            if !finest.contains(&(tx, ty))
                && (x..end).any(|xx| mask[y * stride + xx / 8] & (1 << (xx % 8)) != 0)
            {
                finest.insert((tx, ty));
            }
            x = end;
        }
    }
    let mut all = HashSet::new();
    for (x, y) in finest {
        for z in 6..=14 {
            let d = 1i64 << (14 - z);
            all.insert((z, x / d, y / d));
        }
    }
    Ok(all)
}
pub(crate) fn pack(r: &Region, dir: &Path) -> Result<()> {
    let valid_tiles = valid_tiles(r, dir)?;
    let output = dir.join("terrain.sqlite3");
    let db = Connection::open(&output)?;
    db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA application_id=1498240077;PRAGMA user_version=4;CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS tiles(z INTEGER,tile_x INTEGER,tile_y INTEGER,mesh_id TEXT,vertex_count INTEGER,triangle_count INTEGER,body_triangle_count INTEGER,vertices BLOB,triangles BLOB,PRIMARY KEY(z,tile_x,tile_y)) WITHOUT ROWID;")?;
    let config = serde_json::to_value(r)?;
    let saved: Option<String> = db
        .query_row("SELECT value FROM metadata WHERE key='region'", [], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(saved) = saved {
        ensure!(
            same_region(&serde_json::from_str::<Region>(&saved)?, r),
            "地域出力の設定が不一致です"
        );
    }
    db.execute(
        "INSERT OR REPLACE INTO metadata VALUES('region',?1)",
        [config.to_string()],
    )?;
    for (k, v) in [
        ("complete", "false"),
        ("distributable", "false"),
        ("format", "\"YMTM\""),
        ("format_version", "4"),
        ("vertex_encoding", "\"YMV4\""),
        ("triangle_encoding", "\"YMT3/TFC2\""),
    ] {
        db.execute("INSERT OR REPLACE INTO metadata VALUES(?1,?2)", [k, v])?;
    }
    let generation = format!(
        "xyz-region-test-{}-{}-{}",
        r.name,
        r.n,
        &r.source_sha256[..16]
    );
    let levels=(6..=14u32).map(|z|json!({"z":z,"tile_size_m":web_mercator::width(z),"error_m":12.5*2f64.powi(14-z as i32),"summit_min_error_m":12.5f64.max(6.25*2f64.powi(14-z as i32)),"vertex_storage":if [6,7,8,9,12].contains(&z){"full"}else{"delta"},"maximum_parent_depth":match z {10|13=>1,11|14=>2,_=>0}})).collect::<Vec<_>>();
    for (key, value) in [
        ("generation_id", json!(generation)),
        (
            "crs",
            json!({"authority":"EPSG:3857","type":"web_mercator","input":"WGS84_longitude_latitude_degrees","radius_m":6378137,"axis":["east","north"],"tile_scheme":"XYZ","tile_axis":["east","south"],"local_mesh_axis":["east","south"],"unit":"metre"}),
        ),
        ("origin_x", json!(-web_mercator::half_world())),
        ("origin_y", json!(web_mercator::half_world())),
        ("levels", json!(levels)),
        ("vertex_index_bits", json!(32)),
        ("xy_units_per_tile", json!(16384)),
        (
            "height_encoding",
            json!({"zero":24000,"negative_step_m":0.5,"positive_step_m":0.25}),
        ),
        ("halo_layers", json!(1)),
        ("outer_compression", json!("none")),
        (
            "settings",
            serde_json::from_str(include_str!("../../terrain/national_mesh_settings.json"))?,
        ),
        (
            "source_manifest",
            json!([{"purpose":"L14 DEM","file":"terrain-lod.sqlite3","sha256":r.source_sha256}]),
        ),
        (
            "test_limitations",
            json!([
                "200 km square is measured in EPSG:3857 projected metres",
                "regional crop; coarse XYZ tiles do not contain their entire global extent",
                "regional feature distances use projected metres; geodesic feature metric not yet integrated",
                "mixed-level runtime joins and Loop are not tested by this generator"
            ]),
        ),
    ] {
        db.execute(
            "INSERT OR REPLACE INTO metadata VALUES(?1,?2)",
            params![key, value.to_string()],
        )?;
    }
    let mut grid = vec![0f64; r.n * r.n];
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(grid.as_mut_ptr().cast::<u8>(), grid.len() * 8) };
    File::open(dir.join("dem.f64"))?.read_exact(bytes)?;
    let (scales, _) = summit_policy::summit_scales(&grid, r.n, r.spacing);
    let mut coarser_points = Vec::<[i64; 2]>::new();
    let mut parents = HashMap::<Key, Vec<V>>::new();
    let mut lookup = HashMap::<[i64; 2], Ref>::new();
    let mut reports = Vec::new();
    let mut all_pass = true;
    for z in 6..=14u32 {
        let level_started = Instant::now();
        let mut timings = BTreeMap::<String, f64>::new();
        let mut phase_started = Instant::now();
        progress(
            dir,
            json!({"stage":"quantize-and-pack","part":"quantization-repair","z":z,"region":r.name,"complete":false}),
        )?;
        let (seeds, _, _) = read_level(&dir.join(format!("z{z}.mesh")))?;
        let k = 1i64 << (14 - z);
        let span = 16384 * k;
        let step = (k / 64).max(1) as usize;
        let n = (r.n - 1) / step + 1;
        let mut selected = BTreeSet::new();
        for xy in seeds.iter().chain(coarser_points.iter()) {
            let x = ((r.px + xy[0]) as f64 / step as f64).round() as i64 * step as i64 - r.px;
            let y = ((r.py + xy[1]) as f64 / step as f64).round() as i64 * step as i64 - r.py;
            selected.insert([x / step as i64, y / step as i64]);
        }
        let downsample = |field: &[f64]| {
            (0..n)
                .flat_map(|y| (0..n).map(move |x| field[y * step * r.n + x * step]))
                .collect::<Vec<_>>()
        };
        let low_grid = if step == 1 {
            std::borrow::Cow::Borrowed(&grid[..])
        } else {
            std::borrow::Cow::Owned(downsample(&grid))
        };
        let low_scales = if step == 1 {
            std::borrow::Cow::Borrowed(&scales[..])
        } else {
            std::borrow::Cow::Owned(downsample(&scales))
        };
        let tolerance = 12.5 * 2f64.powi(14 - z as i32);
        let fixed = adaptive_tin::quality_tin::refine_seeded_with_limits(
            &low_grid,
            n,
            selected.into_iter().collect(),
            tolerance - 0.125,
            10.,
            || false,
            if z == 14 { None } else { Some(&low_scales) },
        )?;
        timings.insert(
            "repairSeconds".into(),
            phase_started.elapsed().as_secs_f64(),
        );
        phase_started = Instant::now();
        progress(
            dir,
            json!({"stage":"quantize-and-pack","part":"quantized-audit","z":z,"region":r.name,"timings":timings,"complete":false}),
        )?;
        let original = fixed
            .points
            .iter()
            .map(|xy| [xy[0] * step as i64, xy[1] * step as i64])
            .collect::<Vec<_>>();
        let faces = fixed.faces;
        coarser_points = original.clone();
        let mut p = Vec::with_capacity(original.len());
        for &xy in &original {
            let h = grid[xy[1] as usize * r.n + xy[0] as usize];
            ensure!((-12000. ..=10383.75).contains(&h), "標高の符号化範囲外");
            let zcode = (24000. + h / if h < 0. { 0.5 } else { 0.25 }).round() as u16;
            p.push(V {
                x: (r.px + xy[0]) * 64,
                y: (r.py + xy[1]) * 64,
                z: zcode,
            });
        }
        drop(low_grid);
        drop(low_scales);
        let audit = quantized_audit(r, &p, &faces, &grid, &scales, z);
        timings.insert("auditSeconds".into(), phase_started.elapsed().as_secs_f64());
        phase_started = Instant::now();
        progress(
            dir,
            json!({"stage":"quantize-and-pack","part":"tile-partition-and-encode","z":z,"region":r.name,"timings":timings,"complete":false}),
        )?;
        all_pass &= audit["passed"].as_bool().unwrap();
        let mut adjacency = HashMap::<[usize; 2], [usize; 2]>::new();
        for (i, f) in faces.iter().enumerate() {
            for j in 0..3 {
                let mut e = [f[j], f[(j + 1) % 3]];
                e.sort();
                let a = adjacency.entry(e).or_insert([usize::MAX; 2]);
                if a[0] == usize::MAX {
                    a[0] = i;
                } else {
                    ensure!(a[1] == usize::MAX, "面が非多様体です");
                    a[1] = i;
                }
            }
        }
        let mut bodies = BTreeMap::<Key, Vec<usize>>::new();
        for (i, &f) in faces.iter().enumerate() {
            let a = f.map(|id| p[id]);
            for y in a.iter().map(|v| v.y / span).min().unwrap()
                ..=a.iter().map(|v| v.y / span).max().unwrap()
            {
                for x in a.iter().map(|v| v.x / span).min().unwrap()
                    ..=a.iter().map(|v| v.x / span).max().unwrap()
                {
                    if intersects(f, &p, [x * span, y * span, (x + 1) * span, (y + 1) * span]) {
                        bodies.entry((x, y)).or_default().push(i);
                    }
                }
            }
        }
        bodies.retain(|&(x, y), _| valid_tiles.contains(&(z, x, y)));
        let full = [6, 7, 8, 9, 12].contains(&z);
        let mut current_parents = HashMap::new();
        let mut current_lookup = HashMap::<[i64; 2], Ref>::new();
        let mut vb_total = 0usize;
        let mut tb_total = 0usize;
        let mut reference_bytes = 0usize;
        let mut reference_raw = 0usize;
        let mut max_vertices = 0usize;
        for (&tile, body) in &bodies {
            let body_set = body.iter().copied().collect::<HashSet<_>>();
            let mut halo = BTreeSet::new();
            for &i in body {
                for j in 0..3 {
                    let mut e = [faces[i][j], faces[i][(j + 1) % 3]];
                    e.sort();
                    for &other in &adjacency[&e] {
                        if other != usize::MAX && !body_set.contains(&other) {
                            halo.insert(other);
                        }
                    }
                }
            }
            let ids = body
                .iter()
                .chain(halo.iter())
                .flat_map(|&i| faces[i])
                .collect::<BTreeSet<_>>();
            let body_points = body.iter().flat_map(|&i| faces[i]).collect::<HashSet<_>>();
            let mut choices = Vec::new();
            let mut new_ids = Vec::new();
            for id in ids {
                if !full {
                    if let Some(reference) = lookup.get(&original[id]) {
                        choices.push((id, *reference));
                        continue;
                    }
                }
                new_ids.push(id);
            }
            let mut deps = choices.iter().map(|v| v.1.tile).collect::<Vec<_>>();
            deps.sort_by_key(|&(x, y)| (y, x));
            deps.dedup();
            ensure!(deps.len() <= 256, "親タイル数が8bitを超えました");
            let slots = deps
                .iter()
                .enumerate()
                .map(|(i, &k)| (k, i))
                .collect::<HashMap<_, _>>();
            let mut references = choices
                .into_iter()
                .map(|(id, r)| {
                    Ok((
                        u32::from_le_bytes(parent_vertex_reference::encode(
                            slots[&r.tile],
                            r.index,
                        )?),
                        id,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            references.sort();
            new_ids.sort_by_key(|&id| (p[id].y, p[id].x));
            let order = references
                .iter()
                .map(|v| v.1)
                .chain(new_ids.iter().copied())
                .collect::<Vec<_>>();
            let local = order
                .iter()
                .enumerate()
                .map(|(i, &id)| (id, i))
                .collect::<HashMap<_, _>>();
            let vertices = order.iter().map(|&id| p[id]).collect::<Vec<_>>();
            ensure!(
                vertices
                    .iter()
                    .map(|v| (v.x, v.y))
                    .collect::<HashSet<_>>()
                    .len()
                    == vertices.len(),
                "量子化で頂点が合流しました"
            );
            let references = references
                .iter()
                .map(|&(value, id)| (value, p[id]))
                .collect::<Vec<_>>();
            let new = new_ids.iter().map(|&id| p[id]).collect::<Vec<_>>();
            let vb = pack_vertices(z, tile, &vertices, &references, &new, &deps)?;
            let restored = unpack_vertices(&vb, z, tile, &parents)?;
            ensure!(restored == vertices, "YMV4頂点復元不一致");
            let convert = |i: usize| {
                let f = faces[i];
                [local[&f[0]], local[&f[2]], local[&f[1]]]
            };
            let bf = body.iter().map(|&i| convert(i)).collect::<Vec<_>>();
            let hf = halo.iter().map(|&i| convert(i)).collect::<Vec<_>>();
            let tb = pack_faces(&bf, &hf)?;
            check_faces(&tb, &bf, &hf, vertices.len())?;
            db.execute(
                "INSERT OR REPLACE INTO tiles VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    z,
                    tile.0,
                    tile.1,
                    format!("{generation}:z{z}"),
                    vertices.len(),
                    bf.len() + hf.len(),
                    bf.len(),
                    vb,
                    tb
                ],
            )?;
            vb_total += vb.len();
            tb_total += tb.len();
            reference_bytes += u32_at(&vb, 24) as usize;
            reference_raw += references.len() * 4;
            max_vertices = max_vertices.max(vertices.len());
            for (i, &id) in order.iter().enumerate() {
                let candidate = Ref {
                    tile,
                    index: i as u32,
                    body: body_points.contains(&id),
                };
                let key = original[id];
                let replace = current_lookup.get(&key).is_none_or(|old| {
                    (!candidate.body, candidate.tile.1, candidate.tile.0)
                        < (!old.body, old.tile.1, old.tile.0)
                });
                if replace {
                    current_lookup.insert(key, candidate);
                }
            }
            current_parents.insert(tile, vertices);
        }
        lookup = current_lookup;
        parents = current_parents;
        timings.insert(
            "partitionEncodeSaveSeconds".into(),
            phase_started.elapsed().as_secs_f64(),
        );
        timings.insert("totalSeconds".into(), level_started.elapsed().as_secs_f64());
        let report = json!({"timings":timings,"z":z,"tiles":bodies.len(),"uniqueVertices":p.len(),"uniqueTriangles":faces.len(),"maximumTileVertices":max_vertices,"vertexBytes":vb_total,"triangleBytes":tb_total,"parentReferenceBytes":reference_bytes,"rawParentReferenceBytes":reference_raw,"quantizedAudit":audit,"vertexStorage":if full{"full"}else{"delta"}});
        reports.push(report.clone());
        fs::write(
            dir.join("packing-report.json"),
            serde_json::to_vec_pretty(&reports)?,
        )?;
        progress(
            dir,
            json!({"stage":"level-packed","z":z,"region":r.name,"report":report,"complete":false}),
        )?;
    }
    ensure!(
        db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? == "ok",
        "SQLite検査失敗"
    );
    db.execute(
        "INSERT OR REPLACE INTO metadata VALUES('complete','true')",
        [],
    )?;
    db.execute("INSERT OR REPLACE INTO metadata VALUES('validation',?1)",[json!({"geometryPassed":all_pass,"tileRestorePassed":true,"partialRegion":true,"distributable":false}).to_string()])?;
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    progress(
        dir,
        json!({"stage":"test-complete","region":r.name,"complete":true,"distributable":false,"quantizedGeometryPassed":all_pass,"report":reports}),
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_vertex_modes_and_faces_roundtrip() {
        let parent = vec![
            V {
                x: 100,
                y: 100,
                z: 24000,
            },
            V {
                x: 200,
                y: 200,
                z: 25000,
            },
        ];
        let parents = HashMap::from([((0, 0), parent.clone())]);
        let new = V {
            x: 300,
            y: 400,
            z: 26000,
        };
        let mut all = parent.clone();
        all.push(new);
        let b = pack_vertices(
            14,
            (0, 0),
            &all,
            &[(0, parent[0]), (1, parent[1])],
            &[new],
            &[(0, 0)],
        )
        .unwrap();
        assert_eq!(unpack_vertices(&b, 14, (0, 0), &parents).unwrap(), all);
        let b = pack_vertices(14, (0, 0), &all, &[], &all, &[]).unwrap();
        assert_eq!(
            unpack_vertices(&b, 14, (0, 0), &HashMap::new()).unwrap(),
            all
        );
        let body = [[0, 2, 1]];
        let b = pack_faces(&body, &[]).unwrap();
        check_faces(&b, &body, &[], 3).unwrap();
    }
    #[test]
    fn intersection_includes_boundary_and_rejects_bbox_only() {
        let p = [
            V { x: 0, y: 0, z: 0 },
            V { x: 10, y: 0, z: 0 },
            V { x: 0, y: 10, z: 0 },
        ];
        assert!(intersects([0, 1, 2], &p, [0, 0, 1, 1]));
        assert!(intersects([0, 1, 2], &p, [10, 0, 11, 1]));
        assert!(!intersects([0, 1, 2], &p, [8, 8, 9, 9]));
    }
}

#[cfg(test)]
mod parallel_quantized_tests {
    use super::*;
    #[test]
    fn parallel_audit_matches_serial() {
        let r = Region {
            name: "test".into(),
            latitude: 0.,
            longitude: 0.,
            n: 17,
            px: 0,
            py: 0,
            spacing: 1.,
            extent: 16.,
            source_sha256: String::new(),
        };
        let p = (0..=4)
            .flat_map(|y| (0..=4).map(move |x| [x * 4, y * 4]))
            .collect::<Vec<[i64; 2]>>();
        let v = p
            .iter()
            .map(|xy| V {
                x: xy[0] * 64,
                y: xy[1] * 64,
                z: (24000 + xy[0] * 4 + xy[1] * 8) as u16,
            })
            .collect::<Vec<_>>();
        let mut f = Vec::new();
        for y in 0..4 {
            for x in 0..4 {
                let a = y * 5 + x;
                f.push([a, a + 1, a + 6]);
                f.push([a, a + 6, a + 5]);
            }
        }
        f = f.repeat(5);
        let grid = (0..17 * 17)
            .map(|i| (i % 17) as f64 + 2. * (i / 17) as f64 + if i % 7 == 0 { 3. } else { 0. })
            .collect::<Vec<_>>();
        let scales = vec![0.7; grid.len()];
        for z in [6, 14] {
            assert_eq!(
                quantized_audit(&r, &v, &f, &grid, &scales, z),
                quantized_audit_serial(&r, &v, &p, &f, &grid, &scales, z)
            );
        }
    }
}
