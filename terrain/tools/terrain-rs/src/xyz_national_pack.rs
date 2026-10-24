//! 全国の面・辺索引と親頂点索引をディスク上に置く最終保存。
use super::*;
use std::collections::BTreeMap;
use xyz_region_pack::{check_faces, intersects, pack_faces, pack_vertices, unpack_vertices};

fn point(db: &Connection, id: i64) -> Result<V> {
    Ok(db
        .prepare_cached("SELECT x,y,h FROM geometry_points WHERE id=?1")?
        .query_row([id], |r| {
            Ok(V {
                x: r.get(0)?,
                y: r.get(1)?,
                z: r.get(2)?,
            })
        })?)
}
fn face(db: &Connection, id: i64) -> Result<[i64; 3]> {
    Ok(db
        .prepare_cached("SELECT a,b,c FROM geometry_faces WHERE id=?1")?
        .query_row([id], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?]))?)
}
fn build_index(args: &Args, config: &Config, work: &Connection, z: u32) -> Result<()> {
    // 前レベルの索引は作り直せる一時情報。確定形状と出力BLOBは残す。
    work.execute_batch("DROP TABLE IF EXISTS geometry_edges;DROP TABLE IF EXISTS geometry_bounds;DROP TABLE IF EXISTS geometry_faces;DROP TABLE IF EXISTS geometry_points;CREATE TABLE geometry_points(id INTEGER PRIMARY KEY,x INTEGER,y INTEGER,h INTEGER,UNIQUE(x,y));CREATE TABLE geometry_faces(id INTEGER PRIMARY KEY,a INTEGER,b INTEGER,c INTEGER);CREATE VIRTUAL TABLE geometry_bounds USING rtree(id,minx,maxx,miny,maxy);CREATE TABLE geometry_edges(u INTEGER,v INTEGER,face INTEGER,PRIMARY KEY(u,v,face)) WITHOUT ROWID;")?;
    for (index, &xy) in config.chunks.iter().enumerate() {
        check_space(&args.output)?;
        let r = chunk_region(config, xy);
        status(
            args,
            json!({"stage":"index-geometry","z":z,"chunkIndex":index,"totalChunks":config.chunks.len(),"generationStarted":true,"complete":false}),
        )?;
        let (p, f, h) = read_level(&chunk_dir(args, &r).join(format!("final-z{z}.mesh")))?;
        let tx = work.unchecked_transaction()?;
        let mut ids = Vec::with_capacity(p.len());
        for (p, h) in p.iter().zip(h) {
            let x = (r.px + p[0]) * 64;
            let y = (r.py + p[1]) * 64;
            let code = (24000. + h / if h < 0. { 0.5 } else { 0.25 }).round() as u16;
            tx.prepare_cached("INSERT OR IGNORE INTO geometry_points(x,y,h) VALUES(?1,?2,?3)")?
                .execute(params![x, y, code])?;
            let (id, saved) = tx
                .prepare_cached("SELECT id,h FROM geometry_points WHERE x=?1 AND y=?2")?
                .query_row(params![x, y], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, u16>(1)?))
                })?;
            ensure!(saved == code, "作業境界の標高が不一致です");
            ids.push(id);
        }
        for f in f {
            let g = f.map(|i| ids[i]);
            tx.prepare_cached("INSERT INTO geometry_faces(a,b,c) VALUES(?1,?2,?3)")?
                .execute(params![g[0], g[1], g[2]])?;
            let id = tx.last_insert_rowid();
            let pp = f.map(|i| [(r.px + p[i][0]) * 64, (r.py + p[i][1]) * 64]);
            tx.prepare_cached("INSERT INTO geometry_bounds VALUES(?1,?2,?3,?4,?5)")?
                .execute(params![
                    id,
                    pp.iter().map(|p| p[0]).min(),
                    pp.iter().map(|p| p[0]).max(),
                    pp.iter().map(|p| p[1]).min(),
                    pp.iter().map(|p| p[1]).max()
                ])?;
            for i in 0..3 {
                let a = g[i].min(g[(i + 1) % 3]);
                let b = g[i].max(g[(i + 1) % 3]);
                tx.prepare_cached("INSERT INTO geometry_edges VALUES(?1,?2,?3)")?
                    .execute(params![a, b, id])?;
            }
        }
        tx.commit()?;
    }
    let nonmanifold: Option<i64> = work
        .query_row(
            "SELECT u FROM geometry_edges GROUP BY u,v HAVING count(*)>2 LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    ensure!(nonmanifold.is_none(), "全国面索引に非多様体辺があります");
    // 両側に作業領域がある内部境界で、共有点の不一致による片面辺を許さない。
    let chunks = config.chunks.iter().copied().collect::<HashSet<_>>();
    let mut edges =
        work.prepare("SELECT e.u,e.v FROM geometry_edges e GROUP BY e.u,e.v HAVING count(*)=1")?;
    let span = config.cells as i64 * 64;
    for row in edges.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (a, b) = row?;
        let a = point(work, a)?;
        let b = point(work, b)?;
        if a.x == b.x && a.x % span == 0 {
            let x = a.x / span;
            let y = a.y.min(b.y) / span;
            ensure!(
                !(chunks.contains(&[x - 1, y]) && chunks.contains(&[x, y])),
                "内部の縦境界が未接続です"
            );
        }
        if a.y == b.y && a.y % span == 0 {
            let y = a.y / span;
            let x = a.x.min(b.x) / span;
            ensure!(
                !(chunks.contains(&[x, y - 1]) && chunks.contains(&[x, y])),
                "内部の横境界が未接続です"
            );
        }
    }
    Ok(())
}
fn parent_vertices(
    db: &Connection,
    z: u32,
    key: (i64, i64),
    cache: &mut HashMap<(i64, i64), Vec<V>>,
) -> Result<()> {
    if cache.contains_key(&key) {
        return Ok(());
    }
    // unpack_verticesのキャッシュは単一レベル用なので、祖父は別の辞書に復元する。
    let vb: Vec<u8> = db.query_row(
        "SELECT vertices FROM tiles WHERE z=?1 AND tile_x=?2 AND tile_y=?3",
        params![z, key.0, key.1],
        |r| r.get(0),
    )?;
    ensure!(vb.len() >= 32, "親頂点BLOBが短すぎます");
    let count = u32::from_le_bytes(vb[12..16].try_into().unwrap()) as usize;
    ensure!(vb.len() >= 32 + count * 12, "親依存リストの長さが不正です");
    let mut grandparents = HashMap::new();
    for d in vb[32..32 + count * 12].chunks_exact(12) {
        let pz = u32::from_le_bytes(d[..4].try_into().unwrap());
        ensure!(pz + 1 == z, "親レベルが不一致です");
        let px = i32::from_le_bytes(d[4..8].try_into().unwrap()) as i64;
        let py = i32::from_le_bytes(d[8..12].try_into().unwrap()) as i64;
        parent_vertices(db, pz, (px, py), &mut grandparents)?;
    }
    let points = unpack_vertices(&vb, z, key, &grandparents)?;
    cache.insert(key, points);
    Ok(())
}
fn metadata(output: &Connection, config: &Config) -> Result<()> {
    output.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;PRAGMA application_id=1498240077;PRAGMA user_version=4;CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS tiles(z INTEGER,tile_x INTEGER,tile_y INTEGER,mesh_id TEXT,vertex_count INTEGER,triangle_count INTEGER,body_triangle_count INTEGER,vertices BLOB,triangles BLOB,PRIMARY KEY(z,tile_x,tile_y)) WITHOUT ROWID;")?;
    let levels=(6..=14u32).map(|z|json!({"z":z,"tile_size_m":web_mercator::width(z),"error_m":12.5*2f64.powi(14-z as i32),"vertex_storage":if [6,7,8,9,12].contains(&z){"full"}else{"delta"},"maximum_parent_depth":match z{10|13=>1,11|14=>2,_=>0}})).collect::<Vec<_>>();
    for (k, v) in [
        ("complete", json!(false)),
        ("distributable", json!(false)),
        ("format", json!("YMTM")),
        ("format_version", json!(4)),
        ("vertex_encoding", json!("YMV4")),
        ("triangle_encoding", json!("YMT3/TFC2")),
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
            "crs",
            json!({"authority":"EPSG:3857","tile_scheme":"XYZ","tile_axis":["east","south"],"local_mesh_axis":["east","south"],"radius_m":6378137}),
        ),
        ("settings", serde_json::from_str(&config.settings)?),
        ("generation_config", serde_json::to_value(config)?),
        (
            "source_manifest",
            json!([{"purpose":"L14 DEM","file":"terrain-lod.sqlite3","sha256":config.source_sha256}]),
        ),
        (
            "test_limitations",
            json!([
                "feature distances use projected metres",
                "mixed-level runtime joins and Loop are not verified here",
                "working chunk edges are constrained and synchronized before saving"
            ]),
        ),
    ] {
        output.execute(
            "INSERT OR REPLACE INTO metadata VALUES(?1,?2)",
            params![k, v.to_string()],
        )?;
    }
    Ok(())
}
pub(super) fn pack_level(args: &Args, config: &Config, work: &Connection, z: u32) -> Result<()> {
    build_index(args, config, work, z)?;
    let output = Connection::open(args.output.join("terrain.sqlite3"))?;
    metadata(&output, config)?;
    work.execute_batch("CREATE TABLE IF NOT EXISTS parent_points(x INTEGER,y INTEGER,h INTEGER,tx INTEGER,ty INTEGER,idx INTEGER,body INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID;DROP TABLE IF EXISTS next_parent_points;CREATE TABLE next_parent_points(x INTEGER,y INTEGER,h INTEGER,tx INTEGER,ty INTEGER,idx INTEGER,body INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID;")?;
    let mut q = work.prepare("SELECT x,y FROM valid_tiles WHERE z=?1 ORDER BY y,x")?;
    let tiles = q
        .query_map([z], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let span = 16384i64 << (14 - z);
    let full = [6, 7, 8, 9, 12].contains(&z);
    let mut bytes = 0u64;
    let mut updated = Instant::now() - std::time::Duration::from_secs(2);
    for (index, &tile) in tiles.iter().enumerate() {
        if index % 100 == 0 {
            check_space(&args.output)?;
        }
        if updated.elapsed().as_secs_f64() >= 1. || index + 1 == tiles.len() {
            status(
                args,
                json!({"stage":"pack-tiles","z":z,"processed":index,"total":tiles.len(),"blobBytes":bytes,"generationStarted":true,"complete":false}),
            )?;
            updated = Instant::now();
        }
        let bounds = [
            tile.0 * span,
            tile.1 * span,
            (tile.0 + 1) * span,
            (tile.1 + 1) * span,
        ];
        let mut query=work.prepare_cached("SELECT f.id,f.a,f.b,f.c FROM geometry_bounds r JOIN geometry_faces f ON f.id=r.id WHERE r.maxx>=?1 AND r.minx<=?2 AND r.maxy>=?3 AND r.miny<=?4 ORDER BY f.id")?;
        let candidates = query
            .query_map(params![bounds[0], bounds[2], bounds[1], bounds[3]], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    [
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ],
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut points = BTreeMap::new();
        let mut body = BTreeMap::new();
        for (id, f) in candidates {
            let mut p = Vec::with_capacity(3);
            for &i in &f {
                let v = *points.entry(i).or_insert(point(work, i)?);
                p.push(v);
            }
            if intersects([0, 1, 2], &p, bounds) {
                body.insert(id, f);
            }
        }
        ensure!(
            !body.is_empty(),
            "有効タイルの形状がありません: z{z} {tile:?}"
        );
        let mut halo = BTreeMap::new();
        for f in body.values() {
            for j in 0..3 {
                let a = f[j].min(f[(j + 1) % 3]);
                let b = f[j].max(f[(j + 1) % 3]);
                let mut query = work.prepare_cached(
                    "SELECT face FROM geometry_edges WHERE u=?1 AND v=?2 ORDER BY face",
                )?;
                for row in query.query_map(params![a, b], |r| r.get::<_, i64>(0))? {
                    let id = row?;
                    if !body.contains_key(&id) {
                        halo.insert(id, face(work, id)?);
                    }
                }
            }
        }
        let body_points = body.values().flatten().copied().collect::<HashSet<_>>();
        let ids = body
            .values()
            .chain(halo.values())
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        points.retain(|id, _| ids.contains(id));
        for &id in &ids {
            if !points.contains_key(&id) {
                points.insert(id, point(work, id)?);
            }
        }
        let mut refs = Vec::new();
        let mut new = Vec::new();
        for &id in &ids {
            let p = points[&id];
            let reference = if full {
                None
            } else {
                work.prepare_cached("SELECT tx,ty,idx,h FROM parent_points WHERE x=?1 AND y=?2")?
                    .query_row(params![p.x, p.y], |r| {
                        Ok((
                            (r.get::<_, i64>(0)?, r.get::<_, i64>(1)?),
                            r.get::<_, u32>(2)?,
                            r.get::<_, u16>(3)?,
                        ))
                    })
                    .optional()?
            };
            if let Some((tile, local, h)) = reference {
                ensure!(p.z == h, "親子で同じ頂点の標高が不一致");
                refs.push((id, tile, local));
            } else {
                new.push(id);
            }
        }
        let mut deps = refs.iter().map(|r| r.1).collect::<Vec<_>>();
        deps.sort_by_key(|p| (p.1, p.0));
        deps.dedup();
        ensure!(deps.len() <= 256, "親参照数が8bitを超えました");
        let slots = deps
            .iter()
            .enumerate()
            .map(|(i, &p)| (p, i))
            .collect::<HashMap<_, _>>();
        let mut refs = refs
            .into_iter()
            .map(|(id, tile, local)| {
                Ok((
                    u32::from_le_bytes(parent_vertex_reference::encode(slots[&tile], local)?),
                    id,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        refs.sort();
        new.sort_by_key(|id| (points[id].y, points[id].x));
        let order = refs
            .iter()
            .map(|r| r.1)
            .chain(new.iter().copied())
            .collect::<Vec<_>>();
        ensure!(
            order.len() <= 0x1000000,
            "親参照の24bit頂点数範囲を超えました"
        );
        let local = order
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect::<HashMap<_, _>>();
        let vertices = order.iter().map(|id| points[id]).collect::<Vec<_>>();
        let references = refs
            .iter()
            .map(|&(r, id)| (r, points[&id]))
            .collect::<Vec<_>>();
        let new = new.iter().map(|id| points[id]).collect::<Vec<_>>();
        let vb = pack_vertices(z, tile, &vertices, &references, &new, &deps)?;
        let mut parents = HashMap::new();
        for &dep in &deps {
            parent_vertices(&output, z - 1, dep, &mut parents)?;
        }
        ensure!(
            unpack_vertices(&vb, z, tile, &parents)? == vertices,
            "全国頂点BLOBの復元不一致"
        );
        let convert = |f: &[i64; 3]| [local[&f[0]], local[&f[2]], local[&f[1]]];
        let bf = body.values().map(convert).collect::<Vec<_>>();
        let hf = halo.values().map(convert).collect::<Vec<_>>();
        let tb = pack_faces(&bf, &hf)?;
        check_faces(&tb, &bf, &hf, vertices.len())?;
        output.execute(
            "INSERT OR REPLACE INTO tiles VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                z,
                tile.0,
                tile.1,
                format!("{}-z{z}", config.mesh_id),
                vertices.len(),
                bf.len() + hf.len(),
                bf.len(),
                vb,
                tb
            ],
        )?;
        bytes += (vb.len() + tb.len()) as u64;
        let tx = work.unchecked_transaction()?;
        for (i, &id) in order.iter().enumerate() {
            let p = points[&id];
            let body = body_points.contains(&id) as i32;
            tx.prepare_cached("INSERT INTO next_parent_points VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(x,y) DO UPDATE SET tx=excluded.tx,ty=excluded.ty,idx=excluded.idx,body=excluded.body WHERE excluded.body>next_parent_points.body OR (excluded.body=next_parent_points.body AND (excluded.ty<next_parent_points.ty OR (excluded.ty=next_parent_points.ty AND excluded.tx<next_parent_points.tx)))")?.execute(params![p.x,p.y,p.z,tile.0,tile.1,i,body])?;
        }
        tx.commit()?;
    }
    // 親索引の交代と完了記録を同じトランザクションで確定する。
    let tx = work.unchecked_transaction()?;
    tx.execute_batch(
        "DROP TABLE parent_points;ALTER TABLE next_parent_points RENAME TO parent_points;",
    )?;
    tx.execute("INSERT INTO packed VALUES(?1)", [z])?;
    tx.commit()?;
    output.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    Ok(())
}
