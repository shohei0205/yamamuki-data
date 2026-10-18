use crate::mesh::Mesh;
use anyhow::{Result, ensure};
use std::io::{Read, Write};

fn u16_at(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes(b[p..p + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn f32_at(b: &[u8], p: usize) -> f32 {
    f32::from_bits(u32_at(b, p))
}
fn unpack(b: &[u8], gzip: bool, limit: usize) -> Result<Vec<u8>> {
    let reader: Box<dyn Read + '_> = if gzip {
        Box::new(flate2::read::GzDecoder::new(b))
    } else {
        Box::new(lzma_rust2::XzReader::new(b, false))
    };
    let mut raw = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut raw)?;
    ensure!(raw.len() <= limit, "展開後のデータが上限を超えています");
    Ok(raw)
}
pub fn decode_mesh(b: &[u8]) -> Result<Mesh> {
    ensure!(
        b.len() >= 19 && &b[..4] == b"YMP4",
        "YMP4 ヘッダーが不正です"
    );
    let (nv, nt) = (u32_at(b, 10) as usize, u32_at(b, 14) as usize);
    let step = f32_at(b, 6);
    ensure!(
        u16_at(b, 4) == 257
            && b[18] == 0
            && nv <= 1_000_000
            && nt <= 3_000_000
            && step.is_finite()
            && step > 0.,
        "YMP4 の保存条件が不正です"
    );
    let length = (nv + nt) * 12;
    let raw = unpack(&b[19..], false, length)?;
    ensure!(raw.len() == length, "ポリゴンのデータ長が不正です");
    let vertices = (0..nv)
        .map(|i| {
            [
                f32_at(&raw, i * 12) as f64,
                f32_at(&raw, i * 12 + 4) as f64,
                (f32_at(&raw, i * 12 + 8) * step) as f64,
            ]
        })
        .collect();
    let mut previous = 0i64;
    let mut faces = Vec::with_capacity(nt);
    for i in 0..nt {
        let mut face = [0; 3];
        for (j, v) in face.iter_mut().enumerate() {
            previous += u32_at(&raw, nv * 12 + i * 12 + j * 4) as i32 as i64;
            ensure!(
                previous >= 0 && previous < nv as i64,
                "頂点番号が保存範囲を超えています"
            );
            *v = previous as usize;
        }
        faces.push(face);
    }
    let mesh = Mesh { vertices, faces };
    mesh.validate()?;
    Ok(mesh)
}
pub fn encode_mesh(mesh: &Mesh) -> Result<Vec<u8>> {
    mesh.validate()?;
    ensure!(
        mesh.vertices.len() <= 1_000_000 && mesh.faces.len() <= 3_000_000,
        "ポリゴン数が保存範囲を超えています"
    );
    let mut body = Vec::new();
    body.extend(b"YMP4");
    body.extend(257u16.to_le_bytes());
    body.extend(5f32.to_le_bytes());
    body.extend((mesh.vertices.len() as u32).to_le_bytes());
    body.extend((mesh.faces.len() as u32).to_le_bytes());
    body.push(0);
    let mut writer = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(3))?;
    for p in &mesh.vertices {
        for (axis, value) in p.iter().enumerate() {
            let value = if axis == 2 {
                (*value as f32 / 5.).round_ties_even()
            } else {
                *value as f32
            };
            ensure!(value.is_finite(), "頂点が保存範囲を超えています");
            writer.write_all(&value.to_le_bytes())?;
        }
    }
    let mut previous = 0i32;
    for f in &mesh.faces {
        for &v in f {
            let v = v as i32;
            writer.write_all(&(v - previous).to_le_bytes())?;
            previous = v;
        }
    }
    body.extend(writer.finish()?);
    Ok(body)
}
pub struct Grid {
    pub rows: usize,
    pub columns: usize,
    pub values: Vec<f32>,
}
pub fn decode_grid(b: &[u8]) -> Result<Grid> {
    ensure!(
        b.len() >= 18 && &b[..4] == b"YMD1",
        "標高のヘッダーが不正です"
    );
    let (rows, columns) = (u16_at(b, 4) as usize, u16_at(b, 6) as usize);
    let step = f32_at(b, 8);
    let base = u32_at(b, 12) as i32;
    let (flags, codec) = (b[16], b[17]);
    let n = rows * columns;
    ensure!(
        n > 0 && n <= 4_000_000 && flags <= 3 && codec <= 1 && step > 0. && step <= 5.,
        "標高の保存条件が不正です"
    );
    let width = if flags & 1 != 0 { 4 } else { 2 };
    let length = n * width;
    let mask = if flags & 2 != 0 { n.div_ceil(8) } else { 0 };
    let raw = unpack(&b[18..], codec == 0, length + mask)?;
    ensure!(raw.len() == length + mask, "標高のデータ長が不正です");
    let mut sums = vec![0i64; n];
    let mut values = vec![0.; n];
    for y in 0..rows {
        for x in 0..columns {
            let i = y * columns + x;
            let delta = if width == 4 {
                u32_at(&raw, i * 4) as i32 as i64
            } else {
                u16_at(&raw, i * 2) as i16 as i64
            };
            sums[i] = delta
                + if y > 0 { sums[i - columns] } else { 0 }
                + if x > 0 { sums[i - 1] } else { 0 }
                - if x > 0 && y > 0 {
                    sums[i - columns - 1]
                } else {
                    0
                };
            values[i] = ((sums[i] + base as i64) as f64 * step as f64) as f32;
            if mask > 0 && raw[length + i / 8] & (1 << (i % 8)) != 0 {
                values[i] = f32::NAN;
            }
        }
    }
    Ok(Grid {
        rows,
        columns,
        values,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let m = Mesh {
            vertices: vec![[0., 0., 12.5], [256., 0., 17.5], [0., 256., -12.5]],
            faces: vec![[0, 1, 2]],
        };
        let body = encode_mesh(&m).unwrap();
        let r = decode_mesh(&body).unwrap();
        assert_eq!(r.faces, m.faces);
        assert_eq!(r.vertices[0][2], 10.);
        assert_eq!(r.vertices[1][2], 20.);
        assert_eq!(r.vertices[2][2], -10.);
    }
    #[test]
    fn rejects_truncation() {
        for n in 0..19 {
            assert!(decode_mesh(&vec![0; n]).is_err());
        }
        assert!(decode_grid(b"YMD1").is_err());
    }
}
