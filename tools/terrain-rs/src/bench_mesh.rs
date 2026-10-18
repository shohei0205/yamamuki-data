use crate::{codec, median_ms, packet, unpacket, write_json};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Cursor,
    path::Path,
    time::Instant,
};
type Vertex = [u16; 4];
pub(crate) struct WorldMesh {
    pub(crate) vertices: Vec<[f64; 3]>,
    pub(crate) indices: Vec<u32>,
    pub(crate) source_bytes: usize,
    pub(crate) source_decode_ms: f64,
}
fn canonical<T: Ord + Copy>(a: [T; 3]) -> [T; 3] {
    let b = [a[1], a[2], a[0]];
    let c = [a[2], a[0], a[1]];
    a.min(b).min(c)
}
fn signature(vertices: &[Vertex], indices: &[u32]) -> Vec<[Vertex; 3]> {
    let mut result: Vec<_> = indices
        .chunks_exact(3)
        .map(|f| {
            canonical([
                vertices[f[0] as usize],
                vertices[f[1] as usize],
                vertices[f[2] as usize],
            ])
        })
        .collect();
    result.sort_unstable();
    result
}
pub(crate) fn read_mesh(db: &Connection, meta: &Value, level: u32) -> Result<WorldMesh> {
    let z = (level - 2).max(2);
    let scale = 2f64.powi((14 - z) as i32);
    let meters = meta["meters"].as_f64().unwrap();
    let west = meta["west"].as_f64().unwrap() * 256.;
    let north = meta["north"].as_f64().unwrap() * 256.;
    let cx = (meta["width"].as_f64().unwrap() - 1.) * 0.5;
    let cy = (meta["height"].as_f64().unwrap() - 1.) * 0.5;
    let mut statement = db.prepare("SELECT x,y,data FROM tiles WHERE z=? ORDER BY x,y")?;
    let rows = statement
        .query_map([level], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut source_bytes = 0;
    let mut source_decode_ms = 0.;
    for (x, y, body) in rows {
        source_bytes += body.len();
        let (mesh, ms) = median_ms(|| codec::decode_mesh(&body))?;
        source_decode_ms += ms;
        let base = vertices.len() as u32;
        for p in mesh.vertices {
            vertices.push([
                ((x as f64 * 256. + p[0]) * scale - west - cx) * meters,
                (cy - ((y as f64 * 256. + p[1]) * scale - north)) * meters,
                p[2],
            ]);
        }
        for f in mesh.faces {
            indices.extend(f.map(|i| base + i as u32));
        }
    }
    Ok(WorldMesh {
        vertices,
        indices,
        source_bytes,
        source_decode_ms,
    })
}
fn pack_vertex(p: [f64; 3], origin: [f64; 3], step: f64) -> Result<Vertex> {
    let q = p
        .iter()
        .zip(origin)
        .map(|(p, o)| ((p - o) / step).round() as i64)
        .collect::<Vec<_>>();
    ensure!(
        q.iter().all(|q| *q >= 0) && q[0] < (1 << 24) && q[1] < (1 << 24) && q[2] < 65536,
        "頂点の量子化範囲を超えました"
    );
    Ok([
        q[0] as u16,
        q[1] as u16,
        q[2] as u16,
        ((q[0] >> 16) | ((q[1] >> 16) << 8)) as u16,
    ])
}
fn unpack_vertex(v: Vertex, origin: [f64; 3], step: f64) -> [f64; 3] {
    let q = [
        v[0] as u32 | (((v[3] & 255) as u32) << 16),
        v[1] as u32 | (((v[3] >> 8) as u32) << 16),
        v[2] as u32,
    ];
    std::array::from_fn(|i| origin[i] + q[i] as f64 * step)
}
fn quantize(
    mesh: &WorldMesh,
    origin: [f64; 3],
    step: f64,
) -> Result<(Vec<Vertex>, f64, f64, usize)> {
    let vertices = mesh
        .vertices
        .iter()
        .map(|p| pack_vertex(*p, origin, step))
        .collect::<Result<Vec<_>>>()?;
    let mut maximum = 0f64;
    let mut height_error = 0f64;
    for (&a, &b) in mesh.vertices.iter().zip(&vertices) {
        let p = unpack_vertex(b, origin, step);
        maximum = maximum.max(
            a.iter()
                .zip(p)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt(),
        );
        height_error = height_error.max((a[2] - p[2]).abs());
    }
    let mut collapsed = 0;
    for f in mesh.indices.chunks_exact(3) {
        let p: Vec<_> = f
            .iter()
            .map(|i| unpack_vertex(vertices[*i as usize], origin, step))
            .collect();
        let a: Vec<_> = f.iter().map(|i| mesh.vertices[*i as usize]).collect();
        let before =
            (a[1][0] - a[0][0]) * (a[2][1] - a[0][1]) - (a[1][1] - a[0][1]) * (a[2][0] - a[0][0]);
        let area =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
        if before != 0. && before * area <= 0. {
            collapsed += 1;
        }
    }
    Ok((vertices, maximum, height_error, collapsed))
}
fn encode_mesh(
    vertices: &[Vertex],
    indices: &[u32],
    origin: [f64; 3],
    step: f64,
    level: u32,
) -> Result<Vec<u8>> {
    let mut indices = meshopt::optimize_vertex_cache(indices, vertices.len());
    let vertices = meshopt::optimize_vertex_fetch(&mut indices, vertices);
    let vb = meshopt::encode_vertex_buffer(&vertices)?;
    let ib = meshopt::encode_index_buffer(&indices, vertices.len())?;
    packet(
        b"YMC1",
        &json!({"level":level,"origin":origin,"step":step,"vertexEncoding":"u16x4-xy24-z16","vertexCount":vertices.len(),"indexCount":indices.len(),"vertexBytes":vb.len(),"indexBytes":ib.len()}),
        &[&vb, &ib],
    )
}
fn decode_mesh(data: &[u8]) -> Result<(Vec<Vertex>, Vec<u32>)> {
    let (h, b) = unpacket(data, b"YMC1")?;
    let nv = h["vertexCount"].as_u64().unwrap() as usize;
    let ni = h["indexCount"].as_u64().unwrap() as usize;
    let n = h["vertexBytes"].as_u64().unwrap() as usize;
    ensure!(n <= b.len(), "頂点のデータが不足しています");
    let v = meshopt::decode_vertex_buffer(&b[..n], nv)?;
    let i: Vec<u32> = meshopt::decode_index_buffer(&b[n..], ni)?;
    ensure!(i.iter().all(|i| (*i as usize) < nv), "頂点番号が範囲外です");
    Ok((v, i))
}
fn float_signature(vertices: &[[f32; 3]], indices: &[u32]) -> Vec<[[u32; 3]; 3]> {
    let mut result: Vec<_> = indices
        .chunks_exact(3)
        .map(|f| {
            canonical([
                vertices[f[0] as usize].map(f32::to_bits),
                vertices[f[1] as usize].map(f32::to_bits),
                vertices[f[2] as usize].map(f32::to_bits),
            ])
        })
        .collect();
    result.sort_unstable();
    result
}
fn encode_float(mesh: &WorldMesh, level: u32) -> Result<Vec<u8>> {
    let vertices: Vec<[f32; 3]> = mesh.vertices.iter().map(|p| p.map(|v| v as f32)).collect();
    let mut indices = meshopt::optimize_vertex_cache(&mesh.indices, vertices.len());
    let vertices = meshopt::optimize_vertex_fetch(&mut indices, &vertices);
    let vb = meshopt::encode_vertex_buffer(&vertices)?;
    let ib = meshopt::encode_index_buffer(&indices, vertices.len())?;
    packet(
        b"YMC1",
        &json!({"level":level,"vertexEncoding":"f32x3","vertexCount":vertices.len(),"indexCount":indices.len(),"vertexBytes":vb.len(),"indexBytes":ib.len()}),
        &[&vb, &ib],
    )
}
fn decode_float(data: &[u8]) -> Result<(Vec<[f32; 3]>, Vec<u32>)> {
    let (h, b) = unpacket(data, b"YMC1")?;
    let n = h["vertexBytes"].as_u64().unwrap() as usize;
    ensure!(n <= b.len(), "浮動小数点頂点が不足しています");
    Ok((
        meshopt::decode_vertex_buffer(&b[..n], h["vertexCount"].as_u64().unwrap() as usize)?,
        meshopt::decode_index_buffer(&b[n..], h["indexCount"].as_u64().unwrap() as usize)?,
    ))
}
fn encode_indices(indices: &[u32], vertices: usize) -> Result<Vec<u8>> {
    if indices.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(meshopt::encode_index_buffer(indices, vertices)?)
    }
}
fn decode_indices(data: &[u8], count: usize) -> Result<Vec<u32>> {
    if count == 0 {
        Ok(Vec::new())
    } else {
        Ok(meshopt::decode_index_buffer(data, count)?)
    }
}
fn hierarchy(
    new: &[Vertex],
    before: usize,
    pool_len: usize,
    target: &[u32],
    previous: &[u32],
    origin: [f64; 3],
    step: f64,
    level: u32,
) -> Result<(Vec<u8>, usize, usize)> {
    let vb = if new.is_empty() {
        Vec::new()
    } else {
        meshopt::encode_vertex_buffer(new)?
    };
    let ib = encode_indices(target, pool_len)?;
    let mut old: HashMap<[u32; 3], Vec<usize>> = HashMap::new();
    for (i, f) in previous.chunks_exact(3).enumerate() {
        old.entry(canonical(f.try_into().unwrap()))
            .or_default()
            .push(i);
    }
    let mut keep = vec![false; previous.len() / 3];
    let mut added = Vec::new();
    for f in target.chunks_exact(3) {
        let key = canonical(f.try_into().unwrap());
        if let Some(i) = old.get_mut(&key).and_then(Vec::pop) {
            keep[i] = true;
        } else {
            added.extend_from_slice(f);
        }
    }
    let removed: Vec<u32> = keep
        .iter()
        .enumerate()
        .filter_map(|(i, k)| if !k { Some(i as u32) } else { None })
        .collect();
    let rb = if removed.is_empty() {
        Vec::new()
    } else {
        meshopt::encode_index_sequence(&removed, keep.len())?
    };
    let ab = encode_indices(&added, pool_len)?;
    let common = json!({"level":level,"parentLevel":if level>4{Some(level-1)}else{None},"origin":origin,"step":step,"vertexEncoding":"u16x4-xy24-z16","previousVertexCount":before,"newVertexCount":new.len(),"vertexCount":pool_len,"indexCount":target.len(),"vertexBytes":vb.len()});
    let mut full = common.clone();
    full["indexMode"] = json!("full");
    full["indexBytes"] = json!(ib.len());
    let full = packet(b"YMH1", &full, &[&vb, &ib])?;
    let full = zstd::stream::encode_all(Cursor::new(full), 9)?;
    let mut patch = common;
    patch["indexMode"] = json!("patch");
    patch["removedBytes"] = json!(rb.len());
    patch["removedCount"] = json!(removed.len());
    patch["addedIndexBytes"] = json!(ab.len());
    patch["addedIndexCount"] = json!(added.len());
    let patch = packet(b"YMH1", &patch, &[&vb, &rb, &ab])?;
    let patch = zstd::stream::encode_all(Cursor::new(patch), 9)?;
    let a = full.len();
    let b = patch.len();
    Ok((if a <= b { full } else { patch }, a, b))
}
fn decode_hierarchy(
    data: &[u8],
    previous_vertices: &[Vertex],
    previous_indices: &[u32],
) -> Result<(Vec<Vertex>, Vec<u32>)> {
    let data = zstd::stream::decode_all(Cursor::new(data))?;
    let (h, b) = unpacket(&data, b"YMH1")?;
    ensure!(
        h["previousVertexCount"].as_u64().unwrap() as usize == previous_vertices.len(),
        "階層メッシュの親が違います"
    );
    let nv = h["newVertexCount"].as_u64().unwrap() as usize;
    let vb = h["vertexBytes"].as_u64().unwrap() as usize;
    ensure!(vb <= b.len(), "階層頂点が不足しています");
    let mut vertices = previous_vertices.to_vec();
    if nv > 0 {
        vertices.extend(meshopt::decode_vertex_buffer::<Vertex>(&b[..vb], nv)?);
    }
    let indices = if h["indexMode"] == "full" {
        decode_indices(&b[vb..], h["indexCount"].as_u64().unwrap() as usize)?
    } else {
        let rb = h["removedBytes"].as_u64().unwrap() as usize;
        let rc = h["removedCount"].as_u64().unwrap() as usize;
        ensure!(vb + rb <= b.len(), "階層面のデータが不足しています");
        let removed: Vec<u32> = if rc == 0 {
            Vec::new()
        } else {
            meshopt::decode_index_sequence(&b[vb..vb + rb], rc)?
        };
        let mut deleted = vec![false; previous_indices.len() / 3];
        for r in removed {
            ensure!((r as usize) < deleted.len(), "削除する面が範囲外です");
            deleted[r as usize] = true;
        }
        let mut out = Vec::new();
        for (i, f) in previous_indices.chunks_exact(3).enumerate() {
            if !deleted[i] {
                out.extend_from_slice(f);
            }
        }
        out.extend(decode_indices(
            &b[vb + rb..],
            h["addedIndexCount"].as_u64().unwrap() as usize,
        )?);
        out
    };
    ensure!(
        indices.len() == h["indexCount"].as_u64().unwrap() as usize
            && indices.iter().all(|i| (*i as usize) < vertices.len()),
        "階層メッシュの面が不正です"
    );
    Ok((vertices, indices))
}
pub fn run(root: &Path, output: &Path, database: &Path) -> Result<Value> {
    let meta: Value = serde_json::from_slice(&fs::read(root.join("levels/14/manifest.json"))?)?;
    let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut meshes = BTreeMap::new();
    let mut minimum = [f64::INFINITY; 3];
    for level in 4..=14 {
        let mesh = read_mesh(&db, &meta, level)?;
        for p in &mesh.vertices {
            for j in 0..3 {
                minimum[j] = minimum[j].min(p[j]);
            }
        }
        meshes.insert(level, mesh);
    }
    let step = 0.5;
    let origin = minimum.map(|v| (v / step).floor() * step);
    let dir = output.join("meshes");
    fs::create_dir_all(&dir)?;
    let mut results = Vec::new();
    let mut pool = Vec::<Vertex>::new();
    let mut lookup = HashMap::<Vertex, u32>::new();
    let mut previous_indices = Vec::<u32>::new();
    let mut hierarchy_bytes = 0;
    let mut hierarchy_ms = 0.;
    for (&level, mesh) in &meshes {
        let (vertices, max, height, collapsed) = quantize(mesh, origin, step)?;
        let expected = signature(&vertices, &mesh.indices);
        let floats: Vec<[f32; 3]> = mesh.vertices.iter().map(|p| p.map(|v| v as f32)).collect();
        let expected_float = float_signature(&floats, &mesh.indices);
        let t = Instant::now();
        let float_encoded = encode_float(mesh, level)?;
        let float_encode_ms = t.elapsed().as_secs_f64() * 1000.;
        for compressed in [false, true] {
            let name = if compressed {
                "meshopt-f32-zstd"
            } else {
                "meshopt-f32"
            };
            let data = if compressed {
                zstd::stream::encode_all(Cursor::new(&float_encoded), 9)?
            } else {
                float_encoded.clone()
            };
            let ((v, i), ms) = median_ms(|| {
                let b = if compressed {
                    zstd::stream::decode_all(Cursor::new(&data))?
                } else {
                    data.clone()
                };
                decode_float(&b)
            })?;
            ensure!(
                float_signature(&v, &i) == expected_float,
                "GPU 頂点の可逆圧縮で面が変わりました"
            );
            let file = format!("L{level}-{name}.ymc");
            fs::write(dir.join(&file), &data)?;
            results.push(json!({"level":level,"codec":name,"bytes":data.len(),"sourceBytes":mesh.source_bytes,"vertices":v.len(),"triangles":i.len()/3,"positionMaxErrorMeters":0,"heightMaxErrorMeters":0,"quantizationStepMeters":null,"collapsedOrFlippedFacesAfterQuantization":0,"encodeMs":float_encode_ms,"decodeMsMedian":ms,"sourceDecodeMsMedian":mesh.source_decode_ms,"gpuVertexBitsPreserved":true,"connectivityPreserved":true,"url":format!("/codecs/meshes/{file}")}));
        }
        let t = Instant::now();
        let encoded = encode_mesh(&vertices, &mesh.indices, origin, step, level)?;
        let encode_ms = t.elapsed().as_secs_f64() * 1000.;
        for compressed in [false, true] {
            let name = if compressed {
                "meshopt-zstd"
            } else {
                "meshopt"
            };
            let data = if compressed {
                zstd::stream::encode_all(Cursor::new(&encoded), 9)?
            } else {
                encoded.clone()
            };
            let ((v, i), ms) = median_ms(|| {
                let bytes = if compressed {
                    zstd::stream::decode_all(Cursor::new(&data))?
                } else {
                    data.clone()
                };
                decode_mesh(&bytes)
            })?;
            ensure!(
                signature(&v, &i) == expected,
                "メッシュ圧縮で面の接続・向きが変わりました"
            );
            let file = format!("L{level}-{name}.ymc");
            fs::write(dir.join(&file), &data)?;
            results.push(json!({"level":level,"codec":name,"bytes":data.len(),"sourceBytes":mesh.source_bytes,"vertices":v.len(),"triangles":i.len()/3,"positionMaxErrorMeters":max,"heightMaxErrorMeters":height,"quantizationStepMeters":step,"collapsedOrFlippedFacesAfterQuantization":collapsed,"encodeMs":encode_ms,"decodeMsMedian":ms,"sourceDecodeMsMedian":mesh.source_decode_ms,"connectivityPreserved":true,"url":format!("/codecs/meshes/{file}")}));
        }
        let before = pool.len();
        let mut map = Vec::new();
        for &v in &vertices {
            let index = *lookup.entry(v).or_insert_with(|| {
                let i = pool.len() as u32;
                pool.push(v);
                i
            });
            map.push(index);
        }
        let target: Vec<u32> = mesh.indices.iter().map(|i| map[*i as usize]).collect();
        let target = meshopt::optimize_vertex_cache(&target, pool.len());
        let t = Instant::now();
        let (data, full, patch) = hierarchy(
            &pool[before..],
            before,
            pool.len(),
            &target,
            &previous_indices,
            origin,
            step,
            level,
        )?;
        let encode_ms = t.elapsed().as_secs_f64() * 1000.;
        let ((v, i), ms) =
            median_ms(|| decode_hierarchy(&data, &pool[..before], &previous_indices))?;
        ensure!(
            signature(&v, &i) == expected,
            "階層メッシュの復元で面が変わりました"
        );
        let raw = zstd::stream::decode_all(Cursor::new(&data))?;
        let (header, _) = unpacket(&raw, b"YMH1")?;
        previous_indices = i;
        hierarchy_bytes += data.len();
        hierarchy_ms += ms;
        let file = format!("L{level}-hierarchy.ymh");
        fs::write(dir.join(&file), &data)?;
        results.push(json!({"level":level,"codec":"hierarchy","bytes":data.len(),"requiredBytes":hierarchy_bytes,"sourceBytes":mesh.source_bytes,"vertices":v.len(),"triangles":previous_indices.len()/3,"newVertices":pool.len()-before,"indexMode":header["indexMode"],"fullCandidateBytes":full,"patchCandidateBytes":patch,"encodeMs":encode_ms,"decodeMsMedian":ms,"decodeChainMs":hierarchy_ms,"positionMaxErrorMeters":max,"heightMaxErrorMeters":height,"connectivityPreserved":true,"url":format!("/codecs/meshes/{file}")}));
        println!(
            "{}",
            json!({"part":"mesh","level":level,"state":"complete"})
        );
        write_json(
            &output.join("meshes.partial.json"),
            &json!({"results":results}),
        )?;
    }
    Ok(
        json!({"results":results,"meshoptimizerCrate":"0.7.0","vertexEncoding":"f32 版は元の GPU 頂点をビット単位で保持。他は共通原点・0.5 m 単位で u16x4 に XY 各 24 bit・標高 16 bit を格納。","hierarchy":"粗いメッシュから追加頂点を保存。面は全番号列と削除・追加差分を両方圧縮し、小さい方を選ぶ。頂点分割だけに限定した Progressive Mesh ではない。","timing":"メモリー内の復元を 3 回測った中央値。","originalFilesPreserved":true}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codec_preserves_oriented_faces() {
        let v = vec![[0, 0, 0, 0], [1, 0, 0, 0], [0, 1, 0, 0], [1, 1, 1, 0]];
        let i = vec![0, 1, 2, 1, 3, 2];
        let b = encode_mesh(&v, &i, [0.; 3], 0.5, 9).unwrap();
        let (q, j) = decode_mesh(&b).unwrap();
        assert_eq!(signature(&v, &i), signature(&q, &j));
    }
    #[test]
    fn hierarchy_can_remove_and_add_faces() {
        let v = vec![[0, 0, 0, 0], [1, 0, 0, 0], [0, 1, 0, 0], [1, 1, 1, 0]];
        let i = vec![0, 1, 2];
        let (b, _, _) = hierarchy(&v, 0, 4, &i, &[], [0.; 3], 0.5, 4).unwrap();
        let (p, f) = decode_hierarchy(&b, &[], &[]).unwrap();
        let j = vec![0, 1, 2, 1, 3, 2];
        let (b, _, _) = hierarchy(&[], 4, 4, &j, &f, [0.; 3], 0.5, 5).unwrap();
        let (q, k) = decode_hierarchy(&b, &p, &f).unwrap();
        assert_eq!(signature(&v, &j), signature(&q, &k));
    }
    #[test]
    fn clockwise_faces_are_not_counted_as_quantization_damage() {
        let m = WorldMesh {
            vertices: vec![[0., 0., 0.], [1., 0., 0.], [0., -1., 0.]],
            indices: vec![0, 1, 2],
            source_bytes: 0,
            source_decode_ms: 0.,
        };
        assert_eq!(quantize(&m, [0., -1., 0.], 0.5).unwrap().3, 0);
    }
    #[test]
    fn float_codec_preserves_gpu_vertex_bits() {
        let m = WorldMesh {
            vertices: vec![
                [-0.123456789, 0., 20.],
                [1.123456789, 0., 20.],
                [0., 1.23456789, 30.],
            ],
            indices: vec![0, 1, 2],
            source_bytes: 0,
            source_decode_ms: 0.,
        };
        let data = encode_float(&m, 9).unwrap();
        let (v, i) = decode_float(&data).unwrap();
        let expected: Vec<[f32; 3]> = m.vertices.iter().map(|v| v.map(|x| x as f32)).collect();
        assert_eq!(
            float_signature(&v, &i),
            float_signature(&expected, &m.indices)
        );
    }
}
